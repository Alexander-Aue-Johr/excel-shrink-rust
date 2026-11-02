/** A finished report alone cannot certify that its Excel processes exited. */
export function comparisonFinished(comparison) {
  return Number.isFinite(comparison.finished_at) && comparison.status !== 'running'
    && comparison.lanes.every(lane => !lane.pid || lane.lifecycle === 'closed' && lane.process_alive !== true);
}

/** Preserve the real event time; repeated snapshots never restart an afterglow. */
export function iconActivity(lane, kind, now = Date.now()) {
  if (kind === 'excel') {
    if (lane.process_alive === false || lane.lifecycle === 'closed') return { level: decay(lane.closed_at, now), pulse: false };
    if (lane.lifecycle === 'opening') return { level: 1, pulse: true };
    if (lane.lifecycle === 'open' || lane.process_alive === true) return { level: 1, pulse: false };
    return { level: decay(lane.closed_at, now), pulse: false };
  }
  if (Number.isFinite(lane.xml_started_at) && lane.xml_started_at <= now && lane.xml_finished_at == null) return { level: 1, pulse: false };
  return { level: decay(lane.xml_finished_at, now), pulse: false };
}

function decay(ended, now) {
  return Number.isFinite(ended) && ended <= now ? Math.exp(-(now - ended) / 6500) : 0;
}

/** Unknown and invalid capture totals stay indeterminate rather than becoming 100%. */
export function captureProgress(lane) {
  const total = lane.captures_total;
  const done = lane.captures_done;
  if (!Number.isSafeInteger(total) || total < 0 || !Number.isSafeInteger(done) || done < 0 || done > total) {
    return { known: false, text: 'Candidate discovery in progress' };
  }
  return { known: true, total, done, remaining: total - done,
    text: `${done} / ${total} captures · ${total - done} remaining` };
}

/** Native process probes intentionally have a different source from add-in beats. */
export function heartbeatRow(comparison, lane) {
  return {
    id: `targeted/${comparison.run_id}/${lane.id}/${lane.pid || 'pending'}/${lane.process_started_at || 0}`,
    kind: 'Excel / process probe', active: !comparisonFinished(comparison), alive: lane.process_alive,
    processStarted: lane.process_started_at,
    heartbeat: lane.process_heartbeat_at,
    heartbeatCount: lane.process_heartbeat_at == null ? undefined : lane.process_heartbeat_count,
    heartbeatSource: 'PID liveness · not Excel responsiveness or progress',
    state: lane.lifecycle, events: [],
  };
}

/** Keep the latest server revision so a slow response cannot reverse a lifecycle. */
export function acceptComparison(current, incoming) {
  if (!incoming?.run_id || !Array.isArray(incoming.lanes)) return current;
  if (current && (incoming.revision ?? 0) < (current.revision ?? 0)) return current;
  return incoming;
}

