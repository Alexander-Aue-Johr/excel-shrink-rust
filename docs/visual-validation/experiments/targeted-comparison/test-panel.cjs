/** Exercise real shaders, stable canvases, history and fullscreen in isolated Edge. */
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { spawn } = require('node:child_process');

const repository = path.resolve(__dirname, '../../../..');
const output = path.resolve(process.argv[2]);
const source = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
fs.mkdirSync(output, { recursive: false });
const now = Date.now();
const clone = value => JSON.parse(JSON.stringify(value));
const sample = (id, finished) => {
  const value = clone(source);
  value.run_id = id;
  value.workbook = `UI verification · ${id}.xlsx`;
  value.status = finished ? 'completed_with_limitations' : 'running';
  value.started_at = now - 60000;
  value.finished_at = finished ? now - 20000 : null;
  value.revision = 100;
  value.verification_fixture = true;
  for (const lane of value.lanes) if (lane.lifecycle !== 'unavailable') {
    lane.lifecycle = finished ? 'closed' : lane.id === 'O' ? 'opening' : 'open';
    lane.process_alive = !finished;
    lane.process_heartbeat_at = now - 500;
    lane.xml_started_at = finished ? now - 20001 : now - 501;
    lane.xml_finished_at = finished ? now - 20000 : now - 500;
    lane.closed_at = finished ? now - 20000 : null;
    lane.captures_done = finished ? lane.captures_total : 1;
    if (!finished) {
      for (const key of Object.keys(lane.steps)) lane.steps[key] = key === 'discover' ? 'done' : 'pending';
      lane.steps.open = lane.id === 'O' ? 'running' : 'done';
    }
  }
  return value;
};
const samples = [sample('active', false), ...Array.from({ length: 5 }, (_, index) => sample(`completed-${index}`, true))];
const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Targeted cards · UI verification</title>
<link rel="stylesheet" href="/excel-shrink-cli/office-capture/control-room/targeted-panel.css">
<link rel="stylesheet" href="/excel-shrink-cli/office-capture/control-room/signal-monitor.css">
<style>body { margin:24px; background:#08131e; font:14px system-ui; color:#cde3ed; }
button { border:1px solid #456776; background:#143040; color:#cde3ed; padding:8px 12px; border-radius:6px; cursor:pointer; }</style>
</head><body><p>Synthetic UI verification; this page does not operate Excel.</p>
<section id="panel"></section>
<script type="module">
  const original = HTMLCanvasElement.prototype.getContext;
  window.webglContexts = 0;
  HTMLCanvasElement.prototype.getContext = function(type, ...args) {
    if (type === 'webgl2') window.webglContexts++;
    return original.call(this, type, ...args);
  };
  const {createTargetedPanel} = await import('/excel-shrink-cli/office-capture/control-room/targeted-panel.js');
  window.panel = createTargetedPanel(document.getElementById('panel'));
  await window.panel.refresh();
  window.ready = true;
</script></body></html>`;

const server = http.createServer((request, response) => {
  if (request.url === '/') { response.setHeader('Content-Type', 'text/html'); response.end(html); return; }
  if (request.url === '/api/targeted-comparisons') {
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify({ comparisons: samples })); return;
  }
  const file = path.resolve(repository, '.' + decodeURIComponent(request.url.split('?')[0]));
  if (!file.startsWith(repository + path.sep) || !fs.existsSync(file)) { response.writeHead(404).end(); return; }
  response.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : 'text/css');
  fs.createReadStream(file).pipe(response);
});

async function connect(url) {
  const ws = new WebSocket(url);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let sequence = 0;
  const pending = new Map();
  const errors = [];
  ws.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails);
    if (message.id) {
      const waiter = pending.get(message.id); pending.delete(message.id);
      clearTimeout(waiter.timer);
      if (message.error) waiter.reject(new Error(message.error.message)); else waiter.resolve(message.result);
    }
  };
  return {
    errors,
    call(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 15000);
        pending.set(id, { resolve, reject, timer });
        ws.send(JSON.stringify({ id, method, params }));
      });
    },
    close() { ws.close(); },
  };
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function main() {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = `http://127.0.0.1:${server.address().port}/`;
  const edge = 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
  const browser = spawn(edge, ['--headless=new', '--no-first-run', '--no-default-browser-check',
    '--remote-debugging-port=0', `--user-data-dir=${path.join(output, 'profile')}`,
    '--use-angle=swiftshader', '--enable-unsafe-swiftshader', 'about:blank'], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
  const stderr = [];
  browser.stderr.on('data', data => stderr.push(String(data)));
  let cdp;
  try {
    const portFile = path.join(output, 'profile/DevToolsActivePort');
    for (let attempt = 0; attempt < 100 && !fs.existsSync(portFile); attempt++) await sleep(100);
    const port = fs.readFileSync(portFile, 'utf8').split('\n')[0];
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    cdp = await connect(targets.find(target => target.type === 'page').webSocketDebuggerUrl);
    await cdp.call('Runtime.enable');
    await cdp.call('Page.enable');
    await cdp.call('Emulation.setDeviceMetricsOverride', { width: 1600, height: 1200, deviceScaleFactor: 1, mobile: false });
    const evaluate = async expression => {
      const result = await cdp.call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, userGesture: true });
      if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    await cdp.call('Page.navigate', { url: address });
    for (let attempt = 0; attempt < 100 && !await evaluate('Boolean(window.ready)'); attempt++) await sleep(100);
    await sleep(600);
    assert.equal(await evaluate('document.querySelectorAll(".targeted-active .targeted-workbook").length'), 1);
    assert.equal(await evaluate('[...document.querySelectorAll(".targeted-completed .targeted-workbook")].filter(node => !node.hidden).length'), 3);
    assert.equal(await evaluate('document.querySelectorAll(".targeted-active .targeted-flow input").length'), source.columns.length * source.lanes.length);
    assert.equal(await evaluate('document.querySelectorAll(".targeted-active .signal-capture, .targeted-active .signal-progress").length'), 0);
    assert.equal(await evaluate('[...document.querySelectorAll(".targeted-active .targeted-icon")].every(node => node.dataset.renderer === "webgl2")'), true);
    assert.ok(await evaluate('window.webglContexts <= 2'), 'All icons and heartbeat traces share at most two WebGL contexts.');
    await evaluate('window.savedCanvas = document.querySelector(".targeted-active .signal-canvas")');
    const update = clone(samples[0]); update.revision++;
    await evaluate(`window.panel.update(${JSON.stringify(update)})`);
    assert.equal(await evaluate('window.savedCanvas === document.querySelector(".targeted-active .signal-canvas")'), true);
    const screenshot = await cdp.call('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(path.join(output, 'cards.png'), Buffer.from(screenshot.data, 'base64'));
    const finished = sample('active', true); finished.revision = 102; finished.finished_at = Date.now();
    await evaluate(`window.panel.update(${JSON.stringify(finished)})`);
    assert.equal(await evaluate('document.querySelectorAll(".targeted-active .targeted-workbook").length'), 0);
    assert.equal(await evaluate('window.savedCanvas.isConnected && window.savedCanvas.closest(".targeted-completed") !== null'), true);
    await evaluate('document.querySelector(".targeted-more").click()');
    assert.equal(await evaluate('[...document.querySelectorAll(".targeted-completed .targeted-workbook")].filter(node => !node.hidden).length'), 6);
    await evaluate('document.getElementById("panel").requestFullscreen()');
    assert.equal(await evaluate('document.fullscreenElement === document.getElementById("panel")'), true);
    assert.equal(await evaluate('getComputedStyle(document.fullscreenElement).overflowY'), 'auto');
    await evaluate('document.exitFullscreen()');
    assert.deepEqual(cdp.errors, []);
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ passed: true, checked: [
      'real WebGL shader compilation', 'shared contexts', 'stable heartbeat canvases',
      'heartbeat-only graph', 'workflow checkboxes', 'completed-card relocation',
      'bounded recent history', 'expand history', 'fullscreen scrolling', 'no JavaScript exceptions',
    ] }, null, 2));
    console.log('Targeted cards browser checks passed.');
  } finally {
    cdp?.close();
    browser.kill();
    server.close();
    fs.writeFileSync(path.join(output, 'browser.stderr.log'), stderr.join(''));
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
