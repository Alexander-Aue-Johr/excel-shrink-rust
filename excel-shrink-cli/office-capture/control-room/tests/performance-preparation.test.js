import test from 'node:test';
import assert from 'node:assert/strict';
import { preparationJobRequest, installPerformancePreparation } from '../performance-preparation.js';

test('preparation workers are independent and can exceed logical cores', () => {
  const fields = {
    source_manifest: { value: ' G:\\selected sources.json ' },
    output: { value: ' V:\\new preparation ' },
    workers: { value: '32' },
    reuse_from: { value: '' },
  };
  const form = { elements: { namedItem: name => fields[name] } };
  assert.deepEqual(preparationJobRequest(form), {
    source_manifest: 'G:\\selected sources.json', output: 'V:\\new preparation', workers: 32, reuse_from: null,
  });
  for (const invalid of ['', '0', '-1', '2.5', 'Infinity']) {
    fields.workers.value = invalid;
    assert.throws(() => preparationJobRequest(form), /positive integer/);
  }
  fields.workers.value = '2';
  fields.output.value = '';
  assert.throws(() => preparationJobRequest(form), /new preparation directory/);
});

test('directory preparation binds an optional corpus without a custom manifest', () => {
  const values = { source_kind: 'directory', source_directory: ' V:/originals ', corpus_profile: 'destatis',
    output: ' V:/prepared ', workers: '16', reuse_from: '' };
  const form = { elements: { namedItem: name => ({ value: values[name] }) } };
  assert.deepEqual(preparationJobRequest(form), {
    source_manifest: null, source_directory: 'V:/originals', corpus_profile: 'destatis',
    output: 'V:/prepared', workers: 16, reuse_from: null,
  });
  values.source_directory = '';
  assert.throws(() => preparationJobRequest(form), /XLSX originals directory/);
  values.source_kind = 'invalid';
  assert.throws(() => preparationJobRequest(form), /input selection/);
});

test('job status distinguishes request counts from workers and exposes terminal errors', () => {
  const fields = ['performance-job', 'performance-job-fields', 'performance-job-observation',
    'performance-job-status', 'performance-job-progress', 'performance-job-detail',
    'performance-worker-control', 'performance-worker-target', 'performance-worker-apply'];
  const nodes = Object.fromEntries(fields.map(name => [name, {
    disabled: false, hidden: true, textContent: '', addEventListener() {},
  }]));
  const previousDocument = globalThis.document;
  globalThis.document = { getElementById: name => nodes[name] };
  try {
    const view = installPerformancePreparation({ api: () => {}, message: () => {} });
    view.render({ status: 'running', total: 10, workers: 32, output: 'V:\\prepared',
      progress: { total: 10, prepared: 3, failed: 1, in_flight: 2, queued: 4 } }, true);
    assert.equal(nodes['performance-job-fields'].disabled, true);
    assert.equal(nodes['performance-job-progress'].value, 4);
    assert.match(nodes['performance-job-detail'].textContent, /32 requested workers/);
    assert.match(nodes['performance-job-detail'].textContent, /2 in-flight requests/);
    view.render({ status: 'failed', total: 10, workers: 32, error: 'native fixture failure' }, false);
    assert.equal(nodes['performance-job-fields'].disabled, false);
    assert.match(nodes['performance-job-detail'].textContent, /native fixture failure/);
    view.render(null, false);
    assert.equal(nodes['performance-job-observation'].hidden, true);
  } finally {
    globalThis.document = previousDocument;
  }
});

test('live control binds the requested target to the observed job identity', async () => {
  const names = ['performance-job', 'performance-job-fields', 'performance-job-observation',
    'performance-job-status', 'performance-job-progress', 'performance-job-detail',
    'performance-worker-control', 'performance-worker-target', 'performance-worker-apply'];
  const nodes = Object.fromEntries(names.map(name => [name, {
    value: '48', handlers: {}, addEventListener(type, handler) { this.handlers[type] = handler; },
  }]));
  const previous = globalThis.document;
  globalThis.document = { getElementById: name => nodes[name] };
  const calls = [];
  try {
    const view = installPerformancePreparation({ api: async (...args) => calls.push(args), message() {} });
    view.render({ id: 'selected-job', status: 'running', workers: 2, total: 10 }, true);
    assert.equal(nodes['performance-worker-target'].value, '2');
    nodes['performance-worker-target'].value = '48';
    view.render({ id: 'selected-job', status: 'running', workers: 2, total: 10,
      progress: { requested_workers: 3 } }, true);
    assert.equal(nodes['performance-worker-target'].value, '48');
    await nodes['performance-worker-control'].handlers.submit({ preventDefault() {} });
    assert.deepEqual(calls, [['/api/preparation/control', { job_id: 'selected-job', workers: 48 }]]);
    view.render({ id: 'selected-job', status: 'completed', workers: 2, total: 10 }, false);
    assert.equal(nodes['performance-worker-control'].hidden, true);
    await nodes['performance-worker-control'].handlers.submit({ preventDefault() {} });
    assert.equal(calls.length, 1);
  } finally {
    globalThis.document = previous;
  }
});

test('a fresh page reattaches the server job and restores its current worker target', () => {
  const names = ['performance-job', 'performance-job-fields', 'performance-job-observation',
    'performance-job-status', 'performance-job-progress', 'performance-job-detail',
    'performance-worker-control', 'performance-worker-target', 'performance-worker-apply'];
  const nodes = Object.fromEntries(names.map(name => [name, {
    value: '', addEventListener() {},
  }]));
  const previous = globalThis.document;
  globalThis.document = { getElementById: name => nodes[name] };
  try {
    const view = installPerformancePreparation({ api() {}, message() {} });
    view.render({ id: 'retained-job', status: 'running', workers: 2, total: 10,
      progress: { requested_workers: 16, prepared: 4, failed: 1 } }, true);
    assert.equal(nodes['performance-worker-target'].value, '16');
    assert.equal(nodes['performance-job-progress'].value, 5);
    assert.equal(nodes['performance-job-fields'].disabled, true);
    assert.equal(nodes['performance-worker-control'].hidden, false);
    view.render({ id: 'next-job', status: 'running', workers: 8, total: 20 }, true);
    assert.equal(nodes['performance-worker-target'].value, '8');
  } finally {
    globalThis.document = previous;
  }
});
