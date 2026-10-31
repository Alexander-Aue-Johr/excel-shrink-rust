// Inspect a real WebView2 breakpoint using Node's built-in WebSocket client.
const fs = require('node:fs');
const [url, output] = process.argv.slice(2);
const socket = new WebSocket(url);
const pending = new Map();
let sequence = 0;
const report = {breakpoint_hit: false};

function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++sequence;
    pending.set(id, {resolve, reject});
    socket.send(JSON.stringify({id, method, params}));
  });
}

async function inspectPause(event) {
  report.breakpoint_hit = true;
  report.frames = event.callFrames.map(frame => ({
    function: frame.functionName, line: frame.location.lineNumber + 1
  }));
  const probe = await call('Debugger.evaluateOnCallFrame', {
    callFrameId: event.callFrames[0].callFrameId,
    expression: 'globalThis.cpProbe', returnByValue: false
  });
  const properties = await call('Runtime.getProperties', {
    objectId: probe.result.objectId, ownProperties: true
  });
  report.objects = {};
  for (const property of properties.result) {
    if (!property.value?.objectId) continue;
    const description = await call('Runtime.getProperties', {
      objectId: property.value.objectId, ownProperties: false,
      generatePreview: false
    });
    report.objects[property.name] = description.result.map(member => ({
      name: member.name, type: member.value?.type, getter: Boolean(member.get)
    }));
  }
  fs.writeFileSync(output, JSON.stringify(report, null, 2));
  await call('Debugger.resume');
}

socket.addEventListener('open', async () => {
  try {
    await call('Runtime.enable');
    await call('Debugger.enable');
    fs.writeFileSync(output + '.ready', 'ready');
  } catch (error) {
    report.error = String(error);
    fs.writeFileSync(output, JSON.stringify(report, null, 2));
    socket.close();
  }
});
socket.addEventListener('message', async event => {
  const message = JSON.parse(event.data);
  if (message.id) {
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error) request.reject(message.error);
    else request.resolve(message.result);
  } else if (message.method === 'Debugger.paused') {
    try { await inspectPause(message.params); }
    catch (error) {
      report.error = String(error);
      fs.writeFileSync(output, JSON.stringify(report, null, 2));
      await call('Debugger.resume').catch(() => {});
    }
  }
});
setTimeout(() => socket.close(), 180000).unref();
