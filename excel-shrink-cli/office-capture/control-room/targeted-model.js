/** A finished report alone cannot certify that its Excel processes exited. */
export function comparisonFinished(comparison) {
  return Number.isFinite(comparison.finished_at) && comparison.status !== 'running'
    && comparison.lanes.every(lane => !lane.pid || lane.lifecycle === 'closed' && lane.process_alive !== true);
}

/** Preserve the real event time; repeated snapshots never restart an afterglow. */
export function iconActivity(lane, kind, now = Date.now()) {
  if (kind === 'process') {
    return { level: lane.process_alive === true ? 1 : decay(lane.closed_at, now), pulse: false };
  }
  if (kind === 'excel') {
    if (lane.process_alive === false || lane.lifecycle === 'closed') return { level: decay(lane.closed_at, now), pulse: false };
    if (lane.lifecycle === 'opening') return { level: 1, pulse: true };
    if (lane.lifecycle === 'open') return { level: 1, pulse: false };
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

/** Only Office.js messages received by the resident host drive the heart trace. */
export function heartbeatRow(comparison, lane) {
  return {
    id: `targeted/${comparison.run_id}/${lane.id}/${lane.pid || 'pending'}/${lane.process_started_at || 0}`,
    kind: 'Excel / Office.js', active: !comparisonFinished(comparison), alive: lane.process_alive,
    processStarted: lane.process_started_at,
    heartbeat: lane.addin_heartbeat_at,
    heartbeatCount: lane.addin_heartbeat_at == null ? undefined : lane.addin_heartbeat_count,
    heartbeatSource: 'Office.js heartbeat received by host · not calculation progress',
    state: lane.lifecycle, events: [],
  };
}

/** Keep the latest server revision so a slow response cannot reverse a lifecycle. */
export function acceptComparison(current, incoming) {
  if (!incoming?.run_id || !Array.isArray(incoming.lanes)) return current;
  if (current && (incoming.revision ?? 0) < (current.revision ?? 0)) return current;
  return incoming;
}

/** A column is complete only when every applicable lane has completed it. */
export function phaseComplete(lanes, phase) {
  const statuses = lanes.filter(lane => lane.lifecycle !== 'unavailable')
    .map(lane => lane.steps?.[phase] || 'pending')
    .filter(status => status !== 'skipped' && status !== 'unavailable');
  return statuses.length > 0 && statuses.every(status => status === 'done');
}
