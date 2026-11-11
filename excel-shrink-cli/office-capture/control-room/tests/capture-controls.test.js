import test from 'node:test';
import assert from 'node:assert/strict';
import { captureControlRequest, installCaptureControls } from '../capture-controls.js';

test('live capture requests keep the three independent settings and comparison identity', () => {
  const values = { geometry_batch_ranges: '128', capture_batch_ranges: '4', capture_batch_megapixels: '128' };
  const form = { elements: { namedItem: name => ({ value: values[name] }) } };
  assert.deepEqual(captureControlRequest(form, 'pair'), { run_id: 'pair',
    geometry_batch_ranges: 128, capture_batch_ranges: 4, capture_batch_megapixels: 128 });
  assert.throws(() => captureControlRequest(form, null), /active comparison/);
  for (const [key, invalid] of [['geometry_batch_ranges', '4097'], ['capture_batch_ranges', '0'],
    ['capture_batch_ranges', '2.5'], ['capture_batch_megapixels', '31'], ['capture_batch_megapixels', '257']]) {
    const previous = values[key];
    values[key] = invalid;
    assert.throws(() => captureControlRequest(form, 'pair'), /integer/);
    values[key] = previous;
  }
});

test('live fields reload actual settings and ignore a late response from the preceding pair', async () => {
  const inputs = Object.fromEntries(['geometry_batch_ranges', 'capture_batch_ranges', 'capture_batch_megapixels']
    .map(key => [key, { value: '' }]));
  const listeners = {};
  const form = { hidden: true, elements: { namedItem: key => inputs[key] },
    addEventListener: (event, handler) => { listeners[event] = handler; } };
  const fields = { disabled: true };
  const status = { textContent: '' };
  const reload = { addEventListener: (_event, handler) => { listeners.reload = handler; } };
  const nodes = { 'live-capture-controls': form, 'live-capture-fields': fields,
    'live-capture-status': status, 'reload-capture-controls': reload };
  const previousDocument = globalThis.document;
  globalThis.document = { getElementById: key => nodes[key] };
  const pending = [];
  const sent = [];
  let finishPost;
  try {
    const view = installCaptureControls({ api: (path, body) => {
      if (body) {
        sent.push({ path, body });
        return new Promise(resolve => { finishPost = resolve; });
      }
      return new Promise(resolve => { pending.push(resolve); });
    } });
    const active = run => ({ active: true, telemetry: { run_id: run, status: 'running' } });
    view.render(active('first'));
    view.render(active('second'));
    pending[1]({ available: true, run_id: 'second', geometry_batch_ranges: 128,
      capture_batch_ranges: 4, capture_batch_megapixels: 128 });
    await Promise.resolve();
    pending[0]({ available: true, run_id: 'first', geometry_batch_ranges: 64,
      capture_batch_ranges: 1, capture_batch_megapixels: 32 });
    await Promise.resolve();
    assert.equal(fields.disabled, false);
    assert.equal(inputs.capture_batch_megapixels.value, 128);
    const submission = listeners.submit({ preventDefault() {} });
    assert.equal(sent[0].path, '/api/capture/control');
    assert.equal(sent[0].body.run_id, 'second');
    assert.equal(sent[0].body.capture_batch_megapixels, 128);
    view.render(active('third'));
    finishPost({ accepted: true });
    await submission;
    assert.equal(fields.disabled, true);
    pending[2]({ available: true, run_id: 'third', geometry_batch_ranges: 256,
      capture_batch_ranges: 8, capture_batch_megapixels: 64 });
    await Promise.resolve();
    assert.equal(fields.disabled, false);
    assert.equal(inputs.capture_batch_ranges.value, 8);
    view.render({ active: false });
    assert.equal(form.hidden, true);
    assert.equal(fields.disabled, true);
  } finally {
    globalThis.document = previousDocument;
  }
});
