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

test('job status distinguishes request counts from workers and exposes terminal errors', () => {
  const fields = ['performance-job', 'performance-job-fields', 'performance-job-observation',
    'performance-job-status', 'performance-job-progress', 'performance-job-detail'];
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
