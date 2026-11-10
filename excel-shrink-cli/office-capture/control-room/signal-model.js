// Small, bounded histories of observed protocol signals. These are event
// timestamps, not simulated physiology or estimates of Excel's internal work.
const CHANNELS = ['heartbeat', 'progress', 'capture'];
const COMMAND_PROGRESS = new Set(['addin_ready', 'job_completed']);
const HOST_PROGRESS = new Set([
  'extraction_complete', 'conversion_complete', 'profile_complete', 'bundle_complete',
  'open_measurement_complete', 'recalculation_complete', 'comparison_complete',
]);
const NON_PROGRESS = new Set(['state_changed', 'launcher_state_changed', 'job_dispatched', 'job_failed']);

function time(value) {
  if (value == null || value === '') return null;
  if (typeof value === 'object') return time(value.at ?? value.at_unix_ms ?? value.unix_ms ?? value.updated_unix_ms);
  const result = typeof value === 'number' ? value : typeof value === 'string' ? Date.parse(value) : NaN;
  return Number.isFinite(result) && result >= 0 ? result : null;
}
const counter = value => Number.isSafeInteger(value) && value >= 0 ? value : undefined;
const positive = (value, fallback) => Number.isSafeInteger(value) && value > 0 ? value : fallback;
const isHost = row => row.kind === 'Rust / Host';

/**
 * observe() and snapshot() return independent snapshots, or null for an unknown
 * worker. Points contain actual Unix milliseconds; unknown counters stay
 * undefined. `since` is when this browser first observed this worker/session,
 * not the beginning of an assumed complete historical record.
 */
export function createSignalStore({ windowMs = 60000, maxWorkers = 512, maxEvents = 256 } = {}) {
  windowMs = positive(windowMs, 60000);
  maxWorkers = positive(maxWorkers, 512);
  maxEvents = positive(maxEvents, 256);
  const recentMs = Math.min(15000, windowMs);
  const workers = new Map();

  function fresh(row, now, identity) {
    return { id: row.id, identity, since: now, lastObserved: now, sampledAt: null,
      points: { heartbeat: [], progress: [], capture: [] },
      totals: { heartbeat: undefined, progress: undefined, capture: undefined },
      supported: { heartbeat: false, progress: false, capture: false }, ended: false,
      captureEvents: [], captureHistory: [], stateEvents: [], currentState: null, seenEventKeys: new Map() };
  }
  function trim(entry, now) {
    for (const channel of CHANNELS) entry.points[channel] = entry.points[channel]
      .filter(at => at >= now - windowMs && at <= now).slice(-maxEvents);
    entry.captureEvents = entry.captureEvents.filter(event => event.at >= now - windowMs && event.at <= now).slice(-maxEvents);
    // The current worksheet miniature keeps its last observed areas during a
    // quiet period; the time lane still expires normally. Both remain bounded.
    entry.captureHistory = entry.captureHistory.slice(-maxEvents);
    entry.stateEvents = entry.stateEvents.filter(event => event.at >= now - windowMs && event.at <= now).slice(-maxEvents);
    for (const [key, at] of entry.seenEventKeys) if (at < now - windowMs) entry.seenEventKeys.delete(key);
    while (entry.seenEventKeys.size > maxEvents * 3) entry.seenEventKeys.delete(entry.seenEventKeys.keys().next().value);
  }
  function result(entry, now) {
    trim(entry, now);
    const recent = channel => entry.points[channel].some(at => at >= now - recentMs);
    const state = entry.ended ? 'ended'
      : recent('progress') || recent('capture') ? 'working'
      : recent('heartbeat') ? 'heartbeat-only'
      : CHANNELS.some(channel => entry.supported[channel]) ? 'quiet' : 'unknown';
    return { id: entry.id, points: Object.fromEntries(CHANNELS.map(channel => [channel, [...entry.points[channel]]])),
      totals: { ...entry.totals }, supported: { ...entry.supported },
      captureEvents: entry.captureEvents.map(event => ({ ...event, details: { ...event.details } })),
      captureHistory: entry.captureHistory.map(event => ({ ...event, details: { ...event.details } })),
      stateEvents: entry.stateEvents.map(event => ({ ...event })),
      currentState: entry.currentState && { ...entry.currentState },
      since: entry.since, lastObserved: entry.lastObserved, state };
  }

  return {
    observe(row, now = Date.now()) {
      if (!row || typeof row.id !== 'string' || !row.id || !Number.isFinite(now)) return null;
      const identity = [row.processStarted, row.pid, row.attempt].map(value => value ?? '').join('/');
      const totals = { heartbeat: isHost(row) ? undefined : counter(row.heartbeatCount),
        progress: counter(row.completedCommands), capture: counter(row.uploadedImages) };
      let entry = workers.get(row.id);
      const sampledAt = time(row.sampledAt);
      // The selected workbook and Worker overview poll independently. Browser
      // arrival order cannot turn an older launcher file into a process reset.
      if (entry && sampledAt != null && entry.sampledAt != null && sampledAt < entry.sampledAt)
        return result(entry, Math.max(now, entry.lastObserved));
      // Ignore an out-of-order browser observation rather than moving its
      // history backwards. A new process/retry must have a fresh observation.
      if (entry && now < entry.lastObserved) return result(entry, entry.lastObserved);
      const reset = entry && (entry.identity !== identity || CHANNELS.some(channel =>
        totals[channel] != null && entry.totals[channel] != null && totals[channel] < entry.totals[channel]));
      if (!entry || reset) entry = fresh(row, now, identity);
      if (sampledAt != null) entry.sampledAt = sampledAt;
      entry.lastObserved = now;
      entry.ended = row.active === false || row.alive === false;
      // A phase and its start time are independently observed facts. A long
      // waiting state is not a failed worker without coordinator evidence.
      entry.currentState = typeof row.state === 'string' ? {
        state: row.state, at: time(row.stateSince), waitReason: row.waitReason || null,
      } : null;
      const started = time(row.processStarted);
      const valid = value => {
        const at = time(value);
        return at != null && at <= now && at >= now - windowMs && (started == null || at >= started) ? at : null;
      };
      const add = (channel, value) => {
        const at = valid(value);
        if (at == null) return;
        entry.supported[channel] = true;
        if (!entry.points[channel].includes(at)) entry.points[channel].push(at);
      };
      for (const channel of CHANNELS) if (totals[channel] != null) {
        entry.totals[channel] = totals[channel]; entry.supported[channel] = true;
      }
      if (!isHost(row)) add('heartbeat', row.heartbeat);
      const events = [...(Array.isArray(row.events) ? row.events.slice(-maxEvents) : []), row.lastEvent, row.lastSuccess,
        typeof row.lastProgress === 'object' ? row.lastProgress : null].filter(Boolean);
      for (const event of events) {
        const at = valid(event);
        if (event.kind === 'image_received') {
          add('capture', event);
          if (at != null) {
            const details = Object.fromEntries(['sheet_index', 'sheet_name', 'tile_index', 'range', 'command_id', 'capture_id']
              .filter(key => typeof event.details?.[key] === 'string' || Number.isFinite(event.details?.[key]))
              .map(key => [key, typeof event.details[key] === 'string' ? event.details[key].slice(0, 256) : event.details[key]]));
            const key = `capture:${event.sequence ?? ''}:${at}:${JSON.stringify(details)}`;
            if (!entry.seenEventKeys.has(key)) {
              entry.seenEventKeys.set(key, at);
              const capture = { at, sequence: event.sequence ?? null, details };
              entry.captureEvents.push(capture);
              entry.captureHistory.push(capture);
            }
          }
        } else if (COMMAND_PROGRESS.has(event.kind) || (isHost(row) && HOST_PROGRESS.has(event.kind))) add('progress', event);
        if (event.kind === 'state_changed' && at != null) {
          const key = `state:${event.sequence ?? ''}:${at}:${event.state || ''}`;
          if (!entry.seenEventKeys.has(key)) {
            entry.seenEventKeys.set(key, at);
            entry.stateEvents.push({ at, state: typeof event.state === 'string' ? event.state : null,
              message: typeof event.message === 'string' ? event.message.slice(0, 256) : '' });
          }
        }
      }
      entry.captureEvents.sort((a, b) => a.at - b.at);
      entry.captureHistory.sort((a, b) => a.at - b.at);
      entry.stateEvents.sort((a, b) => a.at - b.at);
      // The backend's last_success / last_progress fields are direct evidence
      // of completion. Dispatch and phase transitions must never become work
      // just because they occur in a legacy launcher's last_success field.
      const successKind = row.lastSuccess?.kind;
      const successIsProgress = row.lastSuccess && successKind !== 'image_received' && !NON_PROGRESS.has(successKind);
      if (successIsProgress) add('progress', row.lastSuccess);
      const progressKind = typeof row.lastProgress === 'object' ? row.lastProgress?.kind : null;
      // C# records last_progress just before its last_success event; they can
      // differ by a millisecond but still describe one completed operation.
      if (!(successIsProgress && valid(row.lastSuccess) != null) && successKind !== 'image_received'
        && progressKind !== 'image_received' && !NON_PROGRESS.has(successKind) && !NON_PROGRESS.has(progressKind)) add('progress', row.lastProgress);
      // Null last_progress is supported only when actual command counters or
      // successful events prove the telemetry exists; old queues remain unknown.
      if (row.lastProgress != null && time(row.lastProgress) != null) entry.supported.progress = true;
      for (const channel of CHANNELS) entry.points[channel].sort((a, b) => a - b);
      trim(entry, now);
      workers.delete(row.id); workers.set(row.id, entry);
      while (workers.size > maxWorkers) workers.delete(workers.keys().next().value);
      return result(entry, now);
    },
    snapshot(id, now = Date.now()) {
      const entry = workers.get(id);
      return entry && Number.isFinite(now) ? result(entry, Math.max(now, entry.lastObserved)) : null;
    },
    retain(ids) {
      const keep = new Set(ids);
      for (const id of workers.keys()) if (!keep.has(id)) workers.delete(id);
    },
    clear() { workers.clear(); },
  };
}
