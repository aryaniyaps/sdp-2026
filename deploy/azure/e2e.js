// End to end check of the hosted page with a real headless Chrome: tell Pi a fact in one session,
// open a session in the other directory, ask about it, and watch the graph change.
// Usage: node e2e.js <base url> <user> <password>
const { spawn } = require('child_process');
const fs = require('fs'), os = require('os'), path = require('path');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const [, , base, user, pass] = process.argv;
  const port = 9571, profile = fs.mkdtempSync(path.join(os.tmpdir(), 'e2e-'));
  const proc = spawn('google-chrome-stable', ['--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, '--window-size=1600,900',
    '--no-first-run', '--mute-audio', 'about:blank'], { stdio: 'ignore' });
  let page;
  for (let i = 0; i < 80 && !page; i++) { try { page = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === 'page'); } catch {} await sleep(150); }
  const ws = new WebSocket(page.webSocketDebuggerUrl); await new Promise((r) => ws.onopen = r);
  let id = 0; const pend = new Map();
  ws.onmessage = (e) => { const m = JSON.parse(e.data); if (pend.has(m.id)) pend.get(m.id)(m); if (m.method === 'Page.javascriptDialogOpening') ws.send(JSON.stringify({ id: ++id, method: 'Page.handleJavaScriptDialog', params: { accept: true } })); };
  const send = (method, params = {}) => new Promise((res, rej) => { const i = ++id; const t = setTimeout(() => rej(new Error('cdp timeout ' + method)), 45000); pend.set(i, (m) => { clearTimeout(t); res(m); }); ws.send(JSON.stringify({ id: i, method, params })); });
  const ev = async (x) => { const r = await send('Runtime.evaluate', { expression: x, returnByValue: true, awaitPromise: true }); if (r.result.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 300)); return r.result.result.value; };
  const results = []; const check = (name, ok, detail = '') => { results.push({ name, ok }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ': ' + String(detail).slice(0, 220) : ''}`); };
  let termText = async () => '';
  try {
    await send('Page.enable'); await send('Network.enable');
    await send('Network.setExtraHTTPHeaders', { headers: { Authorization: 'Basic ' + Buffer.from(`${user}:${pass}`).toString('base64') } });
    const nav = await send('Page.navigate', { url: base + '/' }); console.log('navigate', JSON.stringify(nav.result).slice(0, 120)); await sleep(4000); console.log('page', await ev(`document.readyState + ' ' + location.href + ' frames=' + document.querySelectorAll('iframe').length`));
    termText = () => ev(`(() => { const t = document.getElementById('term').contentWindow.term; if (!t) return ''; const b = t.buffer.active; let s = ''; for (let i = 0; i < b.length; i++) s += b.getLine(i).translateToString(true) + '\\n'; return s; })()`);
    const graphStats = () => ev(`(() => { const g = document.getElementById('graph').contentWindow.__graphView; return g ? g.stats() : null; })()`);
    const waitFor = async (fn, ms, label) => { const end = Date.now() + ms; let last; while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch {} await sleep(1000); } throw new Error('timeout: ' + label); };
    const type = async (text) => {
      await ev(`document.getElementById('term').contentDocument.querySelector('textarea.xterm-helper-textarea').focus()`);
      for (const ch of text) { await send('Input.insertText', { text: ch }); await sleep(15); }
      await sleep(300);
      await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r' });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
    };
    const busy = (t) => /Working/.test(t.split('\n').filter((l) => l.trim()).slice(-14).join('\n'));
    // Wait until Pi is idle again after a prompt: no spinner for 6 seconds and the text stopped changing.
    // Returns what Pi printed after the first echo of the typed prompt (its last words are the marker) found at or after position `from`.
    const settled = async (marker, min, from = 0) => { let last = '', since = Date.now(); const start = Date.now();
      for (;;) { const t = await termText(); if (t !== last) { last = t; since = Date.now(); }
        else if (Date.now() - since > 6000 && Date.now() - start > min && !busy(t) && t.indexOf(marker, from) >= 0) return t.slice(t.indexOf(marker, from) + marker.length);
        await sleep(800); if (Date.now() - start > 170000) throw new Error('terminal never settled'); } };

    const g0 = await waitFor(async () => { const s = await graphStats(); return s && s.nodes > 100 ? s : null; }, 60000, 'graph nodes');
    check('graph panel loads and shows nodes', g0.nodes > 100, JSON.stringify({ nodes: g0.nodes, links: g0.links }));
    await waitFor(async () => /\bpi\b|claude|sonnet|payments-api/i.test(await termText()), 60000, 'pi start');
    check('terminal shows a running Pi in payments-api', /payments-api/.test(await termText()));

    const word = 'walnut-ledger-' + Math.floor(Math.random() * 9000 + 1000);
    const told = `Remember this for later: our billing queue is called ${word}, and Rowan owns the nightly export.`;
    const from1 = (await termText()).length;
    await type(told);
    const r1 = await settled('nightly export.', 8000, from1);
    check('session 1 replied after the prompt', r1.trim().length > 40 && /stored|saved|save|memory|remember|noted/i.test(r1), r1.split('\n').filter(Boolean).slice(0, 6).join(' | '));

    console.log('stage: waiting for extraction'); await sleep(25000);
    console.log('stage: opening mobile-app'); await ev(`document.getElementById('b-mobile-app').click()`);
    await waitFor(async () => /mobile-app/.test(await termText()), 60000, 'second session');
    check('second session opened in mobile-app with an empty screen', !(await termText()).includes(word));
    console.log('stage: asking'); await sleep(5000);
    const from2 = (await termText()).length;
    await type('What is our billing queue called and who owns the nightly export?');
    const tail = await settled('who owns the nightly export?', 8000, from2);
    check('session 2 answers from memory', tail.includes(word) && /rowan/i.test(tail), tail.split('\n').filter(Boolean).slice(-8).join(' | '));
    const g1 = await graphStats();
    check('graph grew while the sessions ran', g1.nodes > g0.nodes, `${g0.nodes} -> ${g1.nodes}`);
    await send('Page.captureScreenshot', { format: 'png' }).then((r) => fs.writeFileSync(path.join(os.tmpdir(), 'e2e-final.png'), Buffer.from(r.result.data, 'base64')));
    console.log('screenshot', path.join(os.tmpdir(), 'e2e-final.png'), 'fact word', word);
  } catch (e) { check('run completed', false, e.message); try { const t = await termText(); console.log('terminal tail:', t.split('\n').filter(Boolean).slice(-12).join(' | ')); const r = await send('Page.captureScreenshot', { format: 'png' }); fs.writeFileSync(path.join(os.tmpdir(), 'e2e-fail.png'), Buffer.from(r.result.data, 'base64')); } catch {} }
  ws.close(); proc.kill();
  for (const l of fs.readdirSync(os.tmpdir())) {}
  const failed = results.filter((r) => !r.ok).length;
  console.log(`${results.length - failed}/${results.length} checks passed`);
  process.exit(failed ? 1 : 0);
})();
