import { age, duration, progressSummary, startupSummary, timestamp, workerRows } from '../worker-monitor-model.js';

export function runTests() {
  let passed = 0;
  const assert = (condition, message) => { if (!condition) throw new Error(message); passed++; };
  const start = Date.parse('2026-10-24T09:00:00Z');
  assert(startupSummary(null) === null, 'Legacy queues must not invent startup progress');
  const startup = startupSummary({ stage: 'scanning_archive', corpus: 'Enron', completed: 250, total: 1000, started_unix_ms: start }, start + 60000);
  assert(startup.percent === 25 && startup.count === '250 / 1,000' && startup.elapsed === '1 min 0 s', 'Startup must report measured stage progress');
  assert(startupSummary({ stage: 'opening_archive', completed: 0, total: null }).percent === null, 'Unknown totals must stay indeterminate');
  assert(startupSummary({ stage: 'loading_records', completed: 0, total: 0 }).percent === null, 'Empty phases must not divide by zero');
  const group = { id: 'group', attempt_id: 'attempt-2', active: true, workbook: 'Book.xlsx', clients: [{
    worker_id: 'group/attempt-2/original/123/456', client_id: 'original', stage: 'capturing', pid: 123,
    process_alive: true, last_heartbeat_at: '2026-10-24T10:00:00Z', heartbeat_count: 50,
    last_activity_at: '2026-10-24T10:00:00Z',
    monitor: { state: 'capturing', state_since: '2026-10-24T09:00:00Z',
      last_progress_at: '2026-10-24T09:10:00Z', last_event: { at: '2026-10-24T09:20:00Z', kind: 'waiting' },
      active_job: { kind: 'capture', id: 'command-7', locations: [{ sheet_name: 'Sheet1', range: 'A1:F10' }] }, events: [] },
  }] };
  const row = workerRows(group)[0];
  assert(row.lastProgress === '2026-10-24T09:10:00Z', 'A fresh heartbeat must not reset actual progress.');
  assert(row.stateSince === '2026-10-24T09:00:00Z', 'A fresh heartbeat must not reset the phase age.');
  assert(row.category === 'working' && row.state === 'capturing', 'A long command alone is not an error.');
  assert(row.sheet === 'Sheet1' && row.job.id === 'command-7', 'Actual command locations identify the current sheet.');
  assert(row.id.includes('attempt-2'), 'Worker identity must distinguish retries.');
  const noProgress = workerRows({ ...group, clients: [{ ...group.clients[0], monitor: undefined }] })[0];
  assert(noProgress.lastProgress === null, 'Legacy LastActivity must not be relabelled as progress.');
  const waiting = workerRows({ ...group, clients: [{ ...group.clients[0], stage: 'waiting_for_addin', monitor: {} }] })[0];
  assert(waiting.category === 'waiting' && waiting.waitReason.includes('unknown'), 'Excel internal wait reasons remain unknown.');
  const coordinator = workerRows({ ...group, clients: [], coordinator: { phase: 'shrink', pid: 42,
    state_since_unix_ms: start, last_progress_unix_ms: start + 5000,
    last_event: { at_unix_ms: start + 5000, kind: 'shrunk' }, events: [] } })[0];
  assert(coordinator.kind === 'Rust / Host' && coordinator.heartbeat === null, 'Host phases must not invent an Office.js heartbeat.');
  assert(timestamp(coordinator.lastEvent) === start + 5000 && age(coordinator.stateSince, start + 5000) === 5, 'Numeric Rust timestamps retain their actual age.');
  assert(duration(null) === '—' && age(null) === null, 'Missing timestamps remain unknown.');
  const summary = progressSummary({ counts: { total: 100, completed: 70, skipped: 5, remaining: 25, in_progress: 2 },
    rates: { completed_per_minute: 1.2, completed_per_hour: 72, average_completed_per_hour: 50, window_seconds: 900 },
    scope: { corpora: ['Enron'], size_part: { part: 2, parts: 4 }, order: 'descending' },
    eta: { status: 'estimating', remaining_seconds: 1000, finish_unix_ms: start + 1000000 } });
  assert(summary.completed === '70' && summary.skipped === '5' && summary.total === '100', 'Skips and successful comparisons stay separate.');
  assert(summary.scope.includes('Partition 2/4') && summary.scope.includes('largest first'), 'Progress is scoped to the selected corpus and partition.');
  assert(summary.perMinute === '1.2' && summary.averagePerHour === '50' && summary.finish === start + 1000000, 'Window rate, run average and actual ETA remain distinct.');
  for (const status of ['insufficient_samples', 'preparation_pending', 'paused', 'complete', 'limited', 'unavailable']) {
    const result = progressSummary({ counts: {}, eta: { status, remaining_seconds: 100, finish_unix_ms: start } });
    assert(result.finish == null && !result.eta.startsWith('About'), `${status} must not display a misleading completion estimate.`);
  }
  assert(progressSummary({}) === null, 'Old runs do not fabricate rate data.');
  return { passed };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) console.log(`Worker monitor: ${result.passed} checks passed.`);
