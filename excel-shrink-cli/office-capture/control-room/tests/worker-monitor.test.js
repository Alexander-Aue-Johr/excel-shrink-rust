import { age, commandTimingSummary, duration, phaseLabel, progressSummary, retainedImageSummary, startupSummary, timestamp, workerRows } from '../worker-monitor-model.js';

export function runTests() {
  let passed = 0;
  const assert = (condition, message) => { if (!condition) throw new Error(message); passed++; };
  assert(phaseLabel('recalc_opening') === 'Opening workbook for recalculation', 'Opening remains distinct from calculation.');
  assert(phaseLabel('recalculating') === 'Recalculating workbook', 'The calculation phase starts only when reported.');
  assert(phaseLabel('recalc_saving') === 'Saving recalculated workbook', 'Saving has its own observed phase.');
  assert(retainedImageSummary(null) === null, 'Legacy workers do not invent retained-image counts');
  assert(retainedImageSummary({ retained_image_tiles: 0, retained_image_bytes: 0 }) === '0 tiles ? 0 MiB of retained PNGs', 'Released image buffers remain a measured zero');
  assert(retainedImageSummary({ retained_image_tiles: 3, retained_image_bytes: 1572864 }) === '3 tiles ? 1.5 MiB of retained PNGs', 'Retained image memory uses explicit binary units');
  assert(retainedImageSummary({ retained_image_tiles: -1, retained_image_bytes: 10 }) === null, 'Invalid retained counts are not rendered as observations');
  assert(phaseLabel('releasing_images') === 'Releasing capture images', 'Discarding PNG buffers is not reported as closing the workbook');
  const start = Date.parse('2026-10-24T09:00:00Z');
  assert(startupSummary(null) === null, 'Legacy queues must not invent startup progress');
  assert(startupSummary({ stage: 'planning_comparisons', completed: 256, total: 1000 }).label === 'Planning comparison groups',
    'Comparison planning has its own startup stage');
  const startup = startupSummary({ stage: 'scanning_archive', corpus: 'Enron', completed: 250, total: 1000, started_unix_ms: start }, start + 60000);
  assert(startup.percent === 25 && startup.count === '250 / 1,000' && startup.elapsed === '1 min 0 s', 'Startup must report measured stage progress');
  assert(startupSummary({ stage: 'opening_archive', completed: 0, total: null }).percent === null, 'Unknown totals must stay indeterminate');
  assert(startupSummary({ stage: 'loading_records', completed: 0, total: 0 }).percent === null, 'Empty phases must not divide by zero');
  const group = { id: 'group', attempt_id: 'attempt-2', active: true, workbook: 'Book.xlsx', launcher_sampled_unix_ms: start, clients: [{
    worker_id: 'group/attempt-2/original/123/456', client_id: 'original', stage: 'capturing', pid: 123,
    process_alive: true, window_mode: 'private-desktop', last_heartbeat_at: '2026-10-24T10:00:00Z', heartbeat_count: 50,
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
  assert(row.sampledAt === start && row.windowMode === 'private-desktop', 'Signal observations retain their sample time and Excel desktop mode.');
  const unknownHeartbeat = workerRows({ ...group, clients: [{ ...group.clients[0], heartbeat_count: undefined }] })[0];
  assert(unknownHeartbeat.heartbeatCount === undefined, 'Missing heartbeat counters must remain unknown.');
  const noProgress = workerRows({ ...group, clients: [{ ...group.clients[0], monitor: undefined }] })[0];
  assert(noProgress.lastProgress === null, 'Legacy LastActivity must not be relabelled as progress.');
  const waiting = workerRows({ ...group, clients: [{ ...group.clients[0], stage: 'waiting_for_addin', monitor: {} }] })[0];
  assert(waiting.category === 'waiting' && waiting.waitReason.includes('unknown'), 'Excel internal wait reasons remain unknown.');
  const unconfirmed = workerRows({ ...group, clients: [{ ...group.clients[0], stage: 'exit_unconfirmed', process_alive: null }] })[0];
  assert(unconfirmed.category === 'waiting' && unconfirmed.alive === null, 'An unconfirmed exit is neither active work nor proof that the process exited.');
  const coordinator = workerRows({ ...group, clients: [], coordinator: { phase: 'shrink', pid: 42,
    state_since_unix_ms: start, last_progress_unix_ms: start + 5000,
    last_event: { at_unix_ms: start + 5000, kind: 'shrunk' }, events: [] } })[0];
  assert(coordinator.kind === 'Rust / Host' && coordinator.heartbeat === null, 'Host phases must not invent an Office.js heartbeat.');
  assert(coordinator.heartbeatCount === undefined, 'Host workers must not invent an Office.js heartbeat counter.');
  const recalculating = workerRows({ ...group, clients: [], coordinator: { phase: 'recalc_opening',
    last_event: { details: { profile: 'maximum-cleaning', external_workbook_links: 2, link_updates: 'disabled' } } } })[0];
  assert(recalculating.profile === 'maximum-cleaning' && recalculating.externalWorkbookLinks === 2
    && recalculating.linkUpdates === 'disabled', 'Recalculation dependencies retain their observed count and update policy.');
  assert(timestamp(coordinator.lastEvent) === start + 5000 && age(coordinator.stateSince, start + 5000) === 5, 'Numeric Rust timestamps retain their actual age.');
  const blocked = workerRows({ ...group, clients: [], cleanup_pending: {state:'waiting',since_unix_ms:start,error:'Access denied'},
    coordinator: {phase:'finished',process_alive:true,pid:42} })[0];
  assert(blocked.state === 'cleanup_blocked' && blocked.category === 'waiting', 'A saved result must not hide unresolved process cleanup.');
  assert(blocked.error === 'Access denied' && blocked.waitReason.includes('slot stays reserved'), 'Cleanup ownership failures expose their reason and retained admission slot.');
  const exhausted = workerRows({ ...group, active: false, clients: [],
    cleanup_pending: { state: 'waiting', retry_exhausted: true, error: 'Access denied' },
    coordinator: { phase: 'finished', pid: 42 } })[0];
  assert(exhausted.state === 'cleanup_failed' && exhausted.waitReason.includes('run stopped'), 'Exhausted cleanup must not claim retries are still running.');
  assert(duration(null) === '—' && age(null) === null, 'Missing timestamps remain unknown.');
  const summary = progressSummary({ counts: { total: 100, completed: 70, skipped: 5, remaining: 25, in_progress: 2 },
    rates: { completed_per_minute: 1.2, completed_per_hour: 72, average_completed_per_hour: 50, window_seconds: 900 },
    scope: { corpora: ['Enron'], size_part: { part: 2, parts: 4 }, order: 'descending' },
    eta: { status: 'estimating', remaining_seconds: 1000, finish_unix_ms: start + 1000000 } });
  assert(summary.completed === '70' && summary.skipped === '5' && summary.total === '100', 'Skips and successful comparisons stay separate.');
  assert(summary.scope.includes('Partition 2/4') && summary.scope.includes('largest first'), 'Progress is scoped to the selected corpus and partition.');
  assert(summary.perMinute === '1.2' && summary.averagePerHour === '50' && summary.finish === start + 1000000, 'Window rate, run average and actual ETA remain distinct.');
  assert(summary.activity === null, 'Legacy progress must not invent attempt counts.');
  const phases = progressSummary({ visual_scope_frozen: true,
    counts: { preparation_total: 10, preparation_terminal: 10, visual_total: 4,
      visual_completed: 2, visual_remaining: 1, visual_deferred: 1, visual_excluded: 6 },
    rates: { preparation_per_minute: 2, preparation_per_hour: 120 },
    eta: { preparation_remaining_seconds: 0 },
    active_preparation: [{ id: 'a', workbook: 'sample.xlsx', profile: 'maximum-cleaning' }] });
  assert(phases.preparationRatio === 1 && phases.visualRatio === 0.5,
    'Preparation and visual comparison keep separate denominators.');
  assert(phases.visualDeferred === '1' && phases.visualRemaining === '1',
    'Deferred full scans remain separate from completed visual comparisons.');
  assert(phases.preparationPerMinute === '2'
    && phases.activePreparation[0] === 'sample.xlsx · maximum cleaning',
  'The shrink phase exposes throughput, current workbook, and profile.');
  const sweep = progressSummary({ counts: { preparation_total: 10, preparation_terminal: 10 },
    hash_sweep: { total: 4, completed: 2, failed: 1 } });
  assert(sweep.preparationRatio === 1 && sweep.hashSweep.completed === 2 && sweep.hashSweep.total === 4,
    'Completed shrinking and half-completed sweeps retain independent progress.');
  assert(sweep.hashSweep.failed === 1, 'Failed sweeps expose the visual-fallback count.');
  const retryActivity = progressSummary({ counts: { completed: 70, skipped: 5, remaining: 25 },
    rates: { new_completions: 0, completed_per_hour: 0 }, activity: {
      finished_attempts: 8, compared_attempts: 0, skipped_attempts: 8, interrupted_attempts: 0,
      retried_attempts: 8, attempts_per_hour: 16, recent_outcomes: [{ workbook: 'Retry.xlsx',
        status: 'skipped', reason: 'Excel opening timed out', retried: true, at_unix_ms: start }],
    } });
  assert(retryActivity.completed === '70' && retryActivity.activity.finished === '8', 'Retry activity stays distinct from corpus validation totals.');
  assert(retryActivity.perHour === '0' && retryActivity.activity.perHour === '16', 'Attempt throughput cannot replace successful comparison throughput.');
  assert(retryActivity.activity.outcomes[0].reason === 'Excel opening timed out'
    && retryActivity.activity.outcomes[0].retried && retryActivity.activity.outcomes[0].at === start,
  'Recent retry outcomes preserve their cause and completion timestamp.');
  assert(retryActivity.activity.outcomes[0].label === 'Skipped / not validated', 'A skipped attempt must never claim visual identity.');
  const observedActivity = progressSummary({ counts: {}, activity: { source: 'observed_current_run',
    finished_attempts: 4, retried_attempts: null, attempts_per_hour: null } });
  assert(observedActivity.activity.observed && observedActivity.activity.retried === '—'
    && observedActivity.activity.perHour === '—', 'Fallback observations disclose unknown retry counts and throughput.');
  for (const status of ['insufficient_samples', 'preparation_pending', 'paused', 'complete', 'limited', 'unavailable']) {
    const result = progressSummary({ counts: {}, eta: { status, remaining_seconds: 100, finish_unix_ms: start } });
    assert(result.finish == null && !result.eta.startsWith('About'), `${status} must not display a misleading completion estimate.`);
  }
  assert(progressSummary({}) === null, 'Old runs do not fabricate rate data.');
  assert(commandTimingSummary(null) === null, 'Old runs do not fabricate command timings.');
  const timing = commandTimingSummary({ command_id: '000003', kind: 'capture_images',
    execution_ms: 1250.5, capture_ms: 0, upload_ms: -1, result_ack_ms: '100', result_attempts: 2 });
  const timings = Object.fromEntries(timing.fields);
  assert(timing.command === 'capture_images · 000003', 'Timing identity names the measured command, not the currently active job.');
  assert(timings.Execution === '1.3 s' && timings['Excel image capture'] === '0 ms', 'Measured zero is distinct from a missing duration.');
  assert(timings['Image finalization'] === 'Not measured' && timings['Image upload / receipt'] === 'Not measured', 'Absent and negative intervals remain unknown.');
  assert(timings['Result delivery / acknowledgment'] === 'Not measured' && timings['Result delivery attempts'] === '2', 'Strings are not silently converted to measured durations.');
  assert(timing.note.includes('do not add') && timing.note.includes('pauses'), 'Nested durations and intentional waiting are explained.');
  const hashes = Object.fromEntries(commandTimingSummary({ command_id: '000004', kind: 'capture_hashes', hash_ms: 2.5 }).fields);
  assert(hashes['PNG checksum (within finalization)'] === '2.5 ms', 'Native digest time is identified as a part of finalization');
  return { passed };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) console.log(`Worker monitor: ${result.passed} checks passed.`);
