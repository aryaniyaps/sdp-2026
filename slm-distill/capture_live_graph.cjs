// Capture the real graph UI for the namespace created by verify_live_memory.py.
// Usage: node slm-distill/capture_live_graph.cjs RECEIPT_JSON UI_BASE OUTPUT_PNG [--allow-incomplete]
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('../frontend/node_modules/@playwright/test');
(async () => {
  const [receiptFile, uiBase, outputFile, preview] = process.argv.slice(2);
  if (!receiptFile || !uiBase || !outputFile) throw new Error('Expected receipt, UI base URL, output PNG');
  const receipt = JSON.parse(fs.readFileSync(receiptFile, 'utf8'));
  if (!receipt.passed && preview !== '--allow-incomplete') throw new Error('Acceptance has not passed; use --allow-incomplete only for a clearly labelled preview');
  const url = `${uiBase.replace(/\/$/, '')}/graph?namespace=${encodeURIComponent(receipt.namespace)}&readonly=1&poll=0`;
  fs.mkdirSync(path.dirname(outputFile), { recursive: true });
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    const errors = [];
    page.on('pageerror', error => errors.push(String(error)));
    const projectionResponse = page.waitForResponse(response => response.url().includes('/api/v2/graph/projection?'));
    await page.goto(url);
    const response = await projectionResponse;
    if (!response.ok()) throw new Error(`Graph projection HTTP ${response.status()}`);
    const projection = await response.json();
    const observations = projection.nodes.filter(node => node.assertion_kind === 'observation' && node.status === 'active');
    const supports = projection.relationships.filter(edge => ['supports', 'derives'].includes(edge.relation));
    if (!observations.length || !supports.length) throw new Error('Projection has no active derived observation/support relationship');
    await page.waitForFunction(() => Number(document.querySelector('#c-fact')?.textContent) > 0);
    await page.getByRole('button', { name: 'Fit', exact: true }).click();
    await page.waitForTimeout(3000);
    await page.screenshot({ path: outputFile, fullPage: true });
    await page.getByRole('searchbox', { name: 'Search nodes' }).fill('observation');
    await page.waitForTimeout(700);
    const detailFile = outputFile.replace(/\.png$/, '-observations.png');
    await page.screenshot({ path: detailFile, fullPage: true });
    const metadata = { url, receipt: path.resolve(receiptFile), acceptance_passed: receipt.passed,
      screenshot: path.resolve(outputFile), observation_screenshot: path.resolve(detailFile),
      source: projection.source, counts: projection.counts, active_observations: observations.length,
      support_relationships: supports.length, page_errors: errors,
      captured_at: new Date().toISOString(), visible_text: await page.locator('body').innerText() };
    fs.writeFileSync(outputFile.replace(/\.png$/, '.json'), JSON.stringify(metadata, null, 2));
    if (errors.length) throw new Error(`Page errors: ${errors.join('; ')}`);
    console.log(JSON.stringify(metadata, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exit(1); });
