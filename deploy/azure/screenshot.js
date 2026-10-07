// Screenshot of the hosted page after Pi has started and the graph has loaded. Usage: node screenshot.js <base url> <user> <password> <out.png>
const { spawn } = require('child_process');
const fs = require('fs'), os = require('os'), path = require('path');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
(async () => {
  const [, , base, user, pass, out] = process.argv;
  const port = 9572, profile = fs.mkdtempSync(path.join(os.tmpdir(), 'shot-'));
  const proc = spawn('google-chrome-stable', ['--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, '--window-size=1800,1000', '--force-device-scale-factor=1', '--no-first-run', 'about:blank'], { stdio: 'ignore' });
  let page; for (let i = 0; i < 80 && !page; i++) { try { page = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === 'page'); } catch {} await sleep(150); }
  const ws = new WebSocket(page.webSocketDebuggerUrl); await new Promise((r) => ws.onopen = r);
  let id = 0; const pend = new Map(); ws.onmessage = (e) => { const m = JSON.parse(e.data); if (pend.has(m.id)) pend.get(m.id)(m); };
  const send = (method, params = {}) => new Promise((res) => { const i = ++id; pend.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
  const ev = async (x) => (await send('Runtime.evaluate', { expression: x, returnByValue: true })).result.result.value;
  await send('Page.enable'); await send('Network.enable');
  await send('Network.setExtraHTTPHeaders', { headers: { Authorization: 'Basic ' + Buffer.from(`${user}:${pass}`).toString('base64') } });
  await send('Page.navigate', { url: base + '/' });
  for (let i = 0; i < 60; i++) { const ok = await ev(`(() => { const g = document.getElementById('graph').contentWindow.__graphView; const t = document.getElementById('term').contentWindow.term; if (!g || !t) return false; const b = t.buffer.active; let s = ''; for (let i = 0; i < b.length; i++) s += b.getLine(i).translateToString(true); return g.stats().nodes > 300 && g.stats().alpha < 0.02 && /payments-api/.test(s); })()`).catch(() => false); if (ok) break; await sleep(1500); }
  await sleep(3000);
  fs.writeFileSync(out, Buffer.from((await send('Page.captureScreenshot', { format: 'png' })).result.data, 'base64'));
  ws.close(); proc.kill(); fs.rmSync(profile, { recursive: true, force: true }); console.log('wrote', out); process.exit(0);
})().catch((e) => { console.error(e.message); process.exit(1); });
