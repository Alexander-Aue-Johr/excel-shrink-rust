import assert from 'node:assert/strict';
import { test } from 'node:test';
import { openingMetrics, processTimings, relativeCost } from '../process-metrics-model.js';

test('rebuilt cards use the one actual opening observation', () => {
  const sample = {startup_seconds: 2, startup_memory_working_set_bytes: 1024};
  assert.equal(openingMetrics({lanes: [{id: 'O', measurements: sample}, {id: 'OR'}]}, 'OR'), sample);
  assert.equal(openingMetrics({lanes: [{id: 'O'}]}, 'O'), null);
  assert.equal(relativeCost(4, 2).tone, 'high');
  assert.equal(relativeCost(2, 4).tone, 'lower');
  assert.equal(relativeCost(1, 0), null);
});

test('live clocks aggregate the same process across rebuild without inventing sheet timings', () => {
  const comparison = {lanes: [
    {id: 'O', phase_timings: {open: {calls: 1, elapsed_ms: 100, started_at: null}}},
    {id: 'OR', phase_timings: {discover: {calls: 1, elapsed_ms: 20, started_at: 1000}}},
  ], operation_timings: [{lane: 'OR', sheet: 'Sheet1', operation: 'measure_batch', elapsed_ms: 50, calls: 1}],
  current_operation: {started_at: 1100, jobs: [{lane: 'OR', sheet: 'Sheet1', kind: 'measure_batch'}]}};
  const timings = processTimings(comparison, 'O', 1200);
  assert.equal(timings.phases.find(item => item.id === 'discover').elapsed_ms, 220);
  assert.equal(timings.operations.length, 1);
  assert.equal(timings.operations[0].elapsed_ms, 150);
  assert.equal(timings.operations[0].running, true);
  comparison.finished_at = 1150;
  assert.equal(processTimings(comparison, 'O', 9000).phases.find(item => item.id === 'discover').elapsed_ms, 170);
  assert.equal(processTimings(comparison, 'O', 9000).operations[0].elapsed_ms, 50);
});
