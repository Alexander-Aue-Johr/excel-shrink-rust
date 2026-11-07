const families = { O: ['O', 'OR'], SD: ['SD', 'SDR'], CP: ['CP', 'CPR'] };

export function processFamily(laneId) {
  return Object.values(families).find(ids => ids.includes(laneId)) || [laneId];
}

/** In-place rebuilds reuse the original launch observation, not a second opening. */
export function openingMetrics(comparison, laneId) {
  const ids = processFamily(laneId);
  return comparison.lanes.find(lane => ids.includes(lane.id)
    && (Number.isFinite(lane.measurements?.startup_seconds)
      || Number.isFinite(lane.measurements?.startup_memory_working_set_bytes)))?.measurements || null;
}

export function relativeCost(value, reference) {
  if (!Number.isFinite(value) || value < 0 || !Number.isFinite(reference) || reference <= 0) return null;
  const ratio = value / reference;
  return {
    ratio,
    tone: ratio >= 2 ? 'high' : ratio > 1.2 ? 'higher' : ratio < 0.8 ? 'lower' : 'similar',
    text: ratio >= 1 ? `${ratio.toFixed(2)}× the other file` : `${((1 - ratio) * 100).toFixed(0)}% less`,
  };
}

/** Phase wall times can overlap. Do not present their sum as total process time. */
export function processTimings(comparison, laneId, now = Date.now()) {
  const family = processFamily(laneId);
  const end = Number.isFinite(comparison.finished_at) ? Math.min(now, comparison.finished_at) : now;
  const phases = new Map();
  for (const lane of comparison.lanes.filter(item => family.includes(item.id))) {
    for (const [id, timing] of Object.entries(lane.phase_timings || {})) {
      if (!timing.calls) continue;
      const current = phases.get(id) || { id, elapsed_ms: 0, running: false };
      current.elapsed_ms += timing.elapsed_ms || 0;
      if (Number.isFinite(timing.started_at)) {
        current.elapsed_ms += Math.max(0, end - timing.started_at);
        current.running ||= !Number.isFinite(comparison.finished_at);
      }
      phases.set(id, current);
    }
  }
  const operations = (comparison.operation_timings || [])
    .filter(item => family.includes(item.lane)).map(item => ({...item}));
  const current = comparison.current_operation;
  if (!Number.isFinite(comparison.finished_at) && Number.isFinite(current?.started_at)) {
    for (const job of current.jobs || [current]) {
      if (!family.includes(job.lane)) continue;
      let timing = operations.find(item => item.lane === job.lane
        && item.sheet === job.sheet && item.operation === job.kind);
      if (!timing) {
        timing = {lane: job.lane, sheet: job.sheet, operation: job.kind, calls: 0,
          elapsed_ms: 0, geometry_ms: 0, render_ms: 0, upload_ms: 0};
        operations.push(timing);
      }
      timing.elapsed_ms += Math.max(0, end - current.started_at);
      timing.calls++;
      timing.running = true;
    }
  }
  return { phases: [...phases.values()], operations };
}

export function seconds(milliseconds) {
  return `${(milliseconds / 1000).toFixed(milliseconds < 10000 ? 2 : 1)} s`;
}
