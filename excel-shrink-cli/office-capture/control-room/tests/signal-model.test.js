import { createSignalStore } from '../signal-model.js';

export function runTests() {
  let passed = 0;
  const assert = (value, message) => { if (!value) throw new Error(message); passed++; };
  const equal = (actual, expected, message) => assert(JSON.stringify(actual) === JSON.stringify(expected), message);
  const start = Date.parse('2026-10-25T12:00:00Z');
  const at = delta => new Date(start + delta).toISOString();
  const row = { id: 'group/attempt-1/original/12/123', kind: 'Excel / Office.js', active: true, alive: true,
    heartbeatCount: 1, heartbeat: at(0), uploadedImages: 0, completedCommands: 0, events: [] };
  const store = createSignalStore();
  let signal = store.observe(row, start);
  equal(signal.points.heartbeat, [start], 'The first actual heartbeat uses its real timestamp.');
  assert(signal.state === 'heartbeat-only' && signal.since === start, 'First observation is explicit and heartbeat is separate from work.');
  signal = store.observe(row, start + 2000);
  equal(signal.points.heartbeat, [start], 'Repeated polls never replay an existing heartbeat.');
  signal = store.observe({ ...row, heartbeatCount: 50, heartbeat: at(10000) }, start + 10000);
  equal(signal.points.heartbeat, [start, start + 10000], 'A count jump cannot invent missing heartbeat timestamps.');
  assert(signal.totals.heartbeat === 50 && signal.points.progress.length === 0, 'Heartbeat totals do not count as progress.');
  const capture = { sequence: 1, kind: 'image_received', at: at(11000),
    details: { sheet_index: 1, sheet_name: 'Sheet2', tile_index: 4, range: 'B2:E10', command_id: 'batch-1' } };
  signal = store.observe({ ...row, heartbeatCount: 50, heartbeat: at(10000), uploadedImages: 25,
    events: [capture], lastSuccess: capture, lastProgress: at(11000) }, start + 11000);
  equal(signal.points.capture, [start + 11000], 'Capture count jumps only draw the one observed capture timestamp.');
  assert(signal.captureEvents.length === 1 && signal.captureEvents[0].details.range === 'B2:E10',
    'Capture events retain authentic worksheet geometry for a visual tile glyph.');
  assert(signal.captureHistory.length === 1 && signal.captureHistory[0].details.range === 'B2:E10',
    'The spatial context retains the same authentic capture as the time lane.');
  equal(signal.points.progress, [], 'A capture must not also be drawn as a successful command.');
  assert(store.observe({ ...row, heartbeatCount: 50, heartbeat: at(10000), events: [capture], uploadedImages: 25 }, start + 11500).captureEvents.length === 1,
    'Repeated polls never fabricate additional capture packages.');
  assert(store.snapshot(row.id, start + 11500).captureHistory.length === 1,
    'Repeated capture polls do not duplicate spatial history.');
  const spatialCopy = store.snapshot(row.id, start + 11500);
  spatialCopy.captureHistory[0].details.range = 'MUTATED'; spatialCopy.captureHistory.push({ at: 0 });
  const spatialOriginal = store.snapshot(row.id, start + 11500);
  assert(spatialOriginal.captureHistory.length === 1 && spatialOriginal.captureHistory[0].details.range === 'B2:E10'
    && spatialOriginal.captureEvents[0].details.range === 'B2:E10',
    'Returned spatial histories and their geometry cannot mutate retained captures or the time lane.');
  assert(signal.state === 'working' && signal.totals.capture === 25, 'A received image is real activity.');
  const complete = { sequence: 2, kind: 'job_completed', at: at(12000) };
  signal = store.observe({ ...row, heartbeatCount: 50, heartbeat: at(10000), uploadedImages: 25, completedCommands: 1,
    events: [capture, complete], lastSuccess: complete, lastProgress: at(12000) }, start + 12000);
  equal(signal.points.progress, [start + 12000], 'The event and last_progress timestamp are deduplicated.');
  assert(signal.totals.progress === 1, 'Progress totals preserve the actual completed-command counter.');
  signal = store.observe({ ...row, heartbeatCount: 50, heartbeat: at(10000), uploadedImages: 25, completedCommands: 1,
    events: [complete], lastSuccess: complete, lastProgress: at(11999) }, start + 12500);
  equal(signal.points.progress, [start + 12000], 'The backend success timestamp and preceding progress timestamp describe only one operation.');
  const dispatch = { kind: 'job_dispatched', at: at(13000) };
  const state = { kind: 'state_changed', at: at(14000), sequence: 3, state: 'waiting_for_job' };
  signal = store.observe({ ...row, heartbeatCount: 50, uploadedImages: 25, completedCommands: 1,
    events: [dispatch, state], lastSuccess: state, lastProgress: dispatch }, start + 14000);
  equal(signal.points.progress, [start + 12000], 'Dispatch and state changes are never successful work.');
  assert(signal.stateEvents.length === 1 && signal.stateEvents[0].state === 'waiting_for_job',
    'Status events preserve their actual phase instead of pretending to be work.');
  signal = store.snapshot(row.id, start + 30000);
  assert(signal.state === 'quiet', 'Old signals fade to quiet rather than a fabricated error.');
  signal = store.snapshot(row.id, start + 80000);
  equal(signal.points, { heartbeat: [], progress: [], capture: [] }, 'Signals fall out of the bounded time window.');
  assert(signal.captureEvents.length === 0 && signal.captureHistory.length === 1
    && signal.captureHistory[0].at === start + 11000,
    'A quiet period expires the time lane while retaining the last observed spatial context.');
  assert(signal.state === 'quiet' && signal.since === start, 'Observed quiet is different from unsupported telemetry.');

  const legacy = store.observe({ id: 'legacy', last_activity_at: at(80000), events: [] }, start + 80000);
  assert(legacy.state === 'unknown' && Object.values(legacy.supported).every(value => !value), 'Legacy activity never masquerades as heartbeat or progress.');
  assert(legacy.totals.heartbeat === undefined && legacy.totals.capture === undefined, 'Missing counts remain unknown.');
  const captureOnly = store.observe({ id: 'capture-only', lastProgress: { kind: 'image_received', at: at(80000) } }, start + 80000);
  assert(captureOnly.points.capture.length === 1 && captureOnly.points.progress.length === 0, 'Typed legacy last_progress capture evidence keeps its own lane.');
  const absent = store.observe({ id: 'empty', heartbeatCount: 0, uploadedImages: 0, completedCommands: 0 }, start + 80000);
  assert(absent.state === 'quiet' && absent.supported.heartbeat && absent.since === start + 80000, 'Supported empty observations start a real observed interval.');
  const host = store.observe({ id: 'host', kind: 'Rust / Host', heartbeat: at(80000), heartbeatCount: 20,
    lastSuccess: { kind: 'custom_saved_step', at_unix_ms: start + 79000 }, lastProgress: start + 79000,
    events: [{ kind: 'conversion_complete', at_unix_ms: start + 78000 }, { kind: 'comparison_started', at_unix_ms: start + 79500 }] }, start + 80000);
  assert(!host.supported.heartbeat && host.points.heartbeat.length === 0, 'Host workers never receive an invented add-in heartbeat.');
  equal(host.points.progress, [start + 78000, start + 79000], 'Known host completions and explicit backend success are progress.');

  const invalid = store.observe({ id: 'invalid', heartbeat: at(90001), heartbeatCount: 1, processStarted: start + 89500,
    events: [{ kind: 'image_received', at: at(90001) }, { kind: 'job_completed', at: 'not a date' },
      { kind: 'image_received', at: at(89000) }, { kind: 'job_completed', at: at(0) }] }, start + 90000);
  equal(invalid.points, { heartbeat: [], progress: [], capture: [] }, 'Invalid, future, pre-process and out-of-window events never draw pulses.');
  const exposed = store.snapshot('host', start + 80000);
  exposed.points.progress.push(1); exposed.supported.heartbeat = true;
  exposed.captureEvents.push({ at: 0 });
  assert(store.snapshot('host', start + 80000).points.progress.length === 2 && !store.snapshot('host', start + 80000).supported.heartbeat
    && store.snapshot('host', start + 80000).captureEvents.length === 0, 'Returned snapshots cannot mutate the retained history.');

  const resets = createSignalStore();
  resets.observe({ ...row, processStarted: start - 1000, heartbeatCount: 50 }, start);
  signal = resets.observe({ ...row, processStarted: start - 1000, heartbeatCount: 1, heartbeat: at(5000) }, start + 5000);
  equal(signal.points.heartbeat, [start + 5000], 'A restarted counter gets a fresh session history.');
  assert(signal.since === start + 5000 && signal.totals.heartbeat === 1, 'A reset is not mistaken for a counter increment.');
  signal = resets.observe({ ...row, processStarted: start + 6000, heartbeat: at(7000) }, start + 7000);
  equal(signal.points.heartbeat, [start + 7000], 'A different process identity cannot retain the old process pulses.');
  const captureResets = createSignalStore();
  captureResets.observe({ ...row, processStarted: start, uploadedImages: 1, events: [capture] }, start + 11000);
  const newProcess = captureResets.observe({ ...row, processStarted: start + 12000, uploadedImages: 1, events: [] }, start + 13000);
  assert(newProcess.captureHistory.length === 0 && newProcess.captureEvents.length === 0,
    'A different process identity discards the preceding process spatial context.');
  const retryId = row.id.replace('attempt-1', 'attempt-2');
  signal = resets.observe({ ...row, id: retryId, heartbeat: at(8000) }, start + 8000);
  equal(signal.points.heartbeat, [start + 8000], 'Attempts have independent histories.');
  signal = resets.observe({ ...row, id: retryId, heartbeatCount: 0, heartbeat: at(6000) }, start + 6000);
  equal(signal.points.heartbeat, [start + 8000], 'Out-of-order observations cannot erase or move the current history.');
  assert(resets.observe({ ...row, active: false }, start + 10000).state === 'ended', 'An explicit ended group ends the signal panel.');
  assert(resets.observe({ ...row, alive: false }, start + 11000).state === 'ended', 'An observed exited process ends the signal panel.');

  const feeds = createSignalStore();
  feeds.observe({ ...row, sampledAt: start, heartbeatCount: 10 }, start);
  feeds.observe({ ...row, sampledAt: start + 1000, heartbeatCount: 11, heartbeat: at(1000) }, start + 1000);
  signal = feeds.observe({ ...row, sampledAt: start, heartbeatCount: 10 }, start + 1500);
  equal(signal.points.heartbeat, [start, start + 1000], 'An older launcher snapshot from the other UI feed cannot reset newer heartbeats.');
  assert(signal.since === start && signal.totals.heartbeat === 11, 'Stale source snapshots preserve the observation window and latest counter.');
  signal = feeds.observe({ ...row, sampledAt: start + 1000, heartbeatCount: 11, heartbeat: at(1000) }, start + 2000);
  equal(signal.points.heartbeat, [start, start + 1000], 'Repeated equal source revisions stay idempotent.');

  const bounded = createSignalStore({ windowMs: 60000, maxWorkers: 2, maxEvents: 3 });
  const many = Array.from({ length: 20 }, (_, index) => ({ kind: 'image_received', at_unix_ms: start + index * 1000 }));
  signal = bounded.observe({ id: 'a', events: many }, start + 20000);
  equal(signal.points.capture, [start + 17000, start + 18000, start + 19000], 'Dense event histories are bounded without inventing timestamps.');
  equal(signal.captureHistory.map(event => event.at), [start + 17000, start + 18000, start + 19000],
    'Dense spatial history is bounded to the same explicit capture limit.');
  for (let index = 20; index < 30; index++) bounded.observe({ id: 'a',
    events: [{ kind: 'image_received', at_unix_ms: start + index * 1000 }] }, start + index * 1000);
  equal(bounded.snapshot('a', start + 100000).captureHistory.map(event => event.at),
    [start + 27000, start + 28000, start + 29000], 'Spatial history stays bounded across polls and after the time lane expires.');
  bounded.observe({ id: 'b' }, start + 20000); bounded.observe({ id: 'c' }, start + 20000);
  assert(bounded.snapshot('a', start + 20000) === null && bounded.snapshot('b', start + 20000) != null, 'The worker cache evicts its least recently observed worker.');
  bounded.retain(['c']);
  assert(bounded.snapshot('b', start + 20000) === null && bounded.snapshot('c', start + 20000) != null, 'Retaining the visible page releases all other worker histories.');
  bounded.clear();
  assert(bounded.snapshot('c', start + 20000) === null, 'Clear releases all histories.');
  assert(bounded.observe(null, start) === null && bounded.observe({ id: 'bad' }, NaN) === null, 'Malformed observations are ignored.');
  return { passed };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) console.log(`Signal model: ${result.passed} checks passed.`);
