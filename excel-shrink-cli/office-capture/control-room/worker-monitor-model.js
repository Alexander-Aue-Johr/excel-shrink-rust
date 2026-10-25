// Read-only presentation of observations. A live heartbeat is never progress,
// and a long-running command alone is never classified as an error.
const phases = {
  starting: 'Starting', staging: 'Staging workbook', waiting_for_open_slot: 'Waiting to open',
  waiting_for_addin: 'Opening Excel / connecting add-in', ready: 'Add-in ready',
  waiting_for_job: 'Waiting for job', waiting_for_peers: 'Waiting for other add-ins',
  paused: 'Capture paused', settling: 'Waiting after Excel startup', planning_sheet: 'Determining sheet geometry',
  capturing: 'Capture job running', providing_images: 'Providing image evidence', executing_job: 'Office.js job running',
  releasing: 'Releasing workbook', closed: 'Closed', client_error: 'Add-in reported an error',
  blocked_edit_mode: 'Excel is in cell edit mode', waiting_cleanup: 'Waiting for cleanup',
  restart_waiting_cleanup: 'Restart waiting for cleanup', prepare: 'Preparing', preparing: 'Preparing',
  extract: 'Extracting file', shrink: 'Generating variants', compare: 'Coordinating comparison',
  save: 'Saving result', capture_planning: 'Planning captures', capture_upload: 'Waiting for captures',
  host_pixel_comparison: 'Comparing pixels on host', complete: 'Completed', completed: 'Completed',
  failed: 'Failed', interrupted: 'Interrupted', draining: 'Finishing groups in progress',
  finished: 'Completed', extracting: 'Extracting file', converting: 'Converting legacy Excel format',
  validating: 'Validating file and capture plan', shrinking: 'Shrinking and validating variant', saving: 'Saving result',
  measuring_open: 'Measuring Excel opening time', validating_values: 'Checking cell values and recalculation',
  opening: 'Starting Excel clients', comparing: 'Coordinating profile comparison',
  comparing_images: 'Comparing image data on host', processing_result: 'Processing Excel response',
  waiting_client: 'Waiting for Office.js response', waiting_images: 'Waiting for images / batch responses',
  whitespace_diagnosis: 'Whitespace cross-check', closing: 'Closing Excel processes',
};
const waits = { coordinator: 'Next coordinator job', primary_addins: 'Other add-ins in group',
  capture_pause: 'Capture pause', startup_quiet_period: 'Quiet period after Excel startup' };

export function timestamp(value) {
  if (value == null || value === '') return null;
  if (typeof value === 'object') return timestamp(value.at ?? value.at_unix_ms ?? value.unix_ms ?? value.updated_unix_ms);
  const number = typeof value === 'number' ? value : Date.parse(value);
  return Number.isFinite(number) ? number : null;
}
export function age(value, now = Date.now()) {
  const time = timestamp(value);
  return time == null ? null : Math.max(0, (now - time) / 1000);
}
export function duration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  if (seconds < 60) return `${Math.floor(seconds)} s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ${Math.floor(seconds % 60)} s`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ${Math.floor(seconds / 60) % 60} min`;
  return `${Math.floor(seconds / 86400)} d ${Math.floor(seconds / 3600) % 24} h`;
}
export function phaseLabel(phase) { return phases[phase] || phase || 'Not observed'; }
export function startupSummary(startup, now = Date.now()) {
  if (!startup?.stage) return null;
  const labels = { loading_records: 'Loading existing results', opening_archive: 'Opening ZIP catalog',
    scanning_archive: 'Reading ZIP entries', checking_catalog: 'Checking catalog',
    importing_results: 'Importing results', selecting_partition: 'Determining file sizes and partition',
    initializing_run: 'Initializing run' };
  const total = startup.total, done = startup.completed;
  const measured = Number.isInteger(total) && total > 0 && Number.isInteger(done) && done >= 0 && done <= total;
  return { label: labels[startup.stage] || startup.stage, corpus: startup.corpus || '',
    count: measured ? `${done.toLocaleString('en-US')} / ${total.toLocaleString('en-US')}` : 'Determining scope',
    percent: measured ? Math.floor(done / total * 100) : null,
    elapsed: duration(age(startup.started_unix_ms, now)) };
}
function latest(...events) {
  return events.filter(event => event && timestamp(event) != null).sort((a, b) => timestamp(b) - timestamp(a))[0] || null;
}
function category(state, alive, active) {
  if (!active || alive === false || ['closed', 'completed', 'complete', 'finished', 'failed', 'interrupted'].includes(state)) return 'ended';
  if (state?.startsWith('waiting') || ['paused', 'settling', 'ready', 'blocked_edit_mode', 'client_error'].includes(state)) return 'waiting';
  return 'working';
}
export function workerRows(group) {
  const common = { groupId: group.id, workbook: group.workbook || group.id, corpus: group.corpus,
    attempt: group.attempt_id, active: group.active === true, groupStatus: group.status };
  const rows = (group.clients || []).map(client => {
    const monitor = client.monitor || {}, state = client.stage || monitor.state || client.status || 'starting';
    const lastSuccess = latest(monitor.last_success, client.last_launcher_success);
    const lastProgress = latest(monitor.last_progress_at, monitor.last_progress, lastSuccess);
    const lastEvent = latest(monitor.last_event, client.last_launcher_event);
    const job = monitor.active_job;
    return { ...common, id: client.worker_id, clientId: client.client_id, kind: 'Excel / Office.js',
      pid: client.pid, processStarted: client.process_started_unix_ms, alive: client.process_alive,
      state, stateSince: monitor.state === state ? monitor.state_since : client.stage_since,
      category: category(state, client.process_alive, common.active), lastProgress, lastSuccess, lastEvent,
      heartbeat: client.last_heartbeat_at, heartbeatCount: client.heartbeat_count ?? 0,
      waitReason: waits[monitor.wait_reason] || monitor.wait_reason || (state === 'waiting_for_addin'
        ? 'Excel opening or add-in initialization; internal wait reason unknown' : null),
      job, sheet: job?.locations?.map(item => item.sheet_name || item.name || (item.sheet_index != null ? `Sheet ${item.sheet_index}` : null)).filter(Boolean).filter((name, i, names) => names.indexOf(name) === i).join(', ') || monitor.sheet_name,
      events: [...(monitor.events || []), ...(client.last_launcher_event ? [client.last_launcher_event] : [])]
        .sort((a, b) => (timestamp(b) || 0) - (timestamp(a) || 0)).slice(0, 24),
      uploadedImages: monitor.uploaded_images, completedCommands: monitor.completed_commands, error: client.error,
    };
  });
  if (group.coordinator) {
    const monitor = group.coordinator, state = monitor.phase || monitor.state;
    rows.unshift({ ...common, id: `${group.id}/${group.attempt_id}/coordinator/${monitor.pid}/${monitor.process_started_unix_ms}`,
      clientId: 'Coordinator', kind: 'Rust / Host', pid: monitor.pid, processStarted: monitor.process_started_unix_ms,
      alive: monitor.process_alive,
      state, stateSince: monitor.state_since ?? monitor.state_since_unix_ms, category: category(state, monitor.process_alive, common.active),
      lastEvent: monitor.last_event, lastProgress: monitor.last_progress_at ?? monitor.last_progress_unix_ms ?? monitor.last_progress,
      lastSuccess: monitor.last_success, waitReason: monitor.wait_reason,
      sheet: monitor.sheet_name || monitor.last_event?.details?.sheet_name,
      events: [...(monitor.events || [])].reverse().slice(0, 24), heartbeat: null, heartbeatCount: 0 });
  }
  return rows;
}

export function progressSummary(progress) {
  if (!progress?.counts) return null;
  const counts = progress.counts, rate = progress.rates || {}, eta = progress.eta || {}, scope = progress.scope || {};
  const number = value => Number.isFinite(value) ? new Intl.NumberFormat('en-US', { maximumFractionDigits: 1 }).format(value) : '—';
  const statuses = { insufficient_samples: 'Waiting for enough completions', preparation_pending: 'Preparation in progress; no overall estimate yet',
    paused: 'Paused', complete: 'Selection completed', limited: 'Run limit: no estimate for the full selection',
    unavailable: 'No reliable estimate yet' };
  const scopeLabel = [scope.corpora?.join(', '), scope.size_part ? `Partition ${scope.size_part.part}/${scope.size_part.parts}` : 'Entire corpus',
    ['descending', 'largest-first', 'size-desc'].includes(scope.order) ? 'largest first' : ['ascending', 'smallest-first', 'size-asc'].includes(scope.order) ? 'smallest first' : scope.order === 'priority' ? 'priority order' : null].filter(Boolean).join(' · ');
  return { scope: scopeLabel, completed: number(counts.completed), skipped: number(counts.skipped),
    remaining: number(counts.remaining), total: number(counts.total), inProgress: number(counts.in_progress),
    preparationRemaining: number(counts.preparation_remaining), prepared: number(counts.prepared),
    perMinute: number(rate.completed_per_minute), perHour: number(rate.completed_per_hour),
    averagePerHour: number(rate.average_completed_per_hour),
    preparationPerHour: number(rate.preparation_per_hour), window: duration(rate.window_seconds),
    eta: eta.status === 'estimating' && Number.isFinite(eta.remaining_seconds) ? `About ${duration(eta.remaining_seconds)}` : statuses[eta.status] || 'No estimate yet',
    finish: eta.status === 'estimating' ? timestamp(eta.finish_unix_ms) : null,
    diagnosisFailed: number(progress.diagnosis_failed), phase: progress.phase === 'paused' ? 'Run paused' : phaseLabel(progress.phase),
  };
}
