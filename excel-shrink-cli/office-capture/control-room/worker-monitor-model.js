// Read-only presentation of observations. A live heartbeat is never progress,
// and a long-running command alone is never classified as an error.
const phases = {
  starting: 'Starting', staging: 'Staging workbook', waiting_for_open_slot: 'Waiting to open',
  waiting_for_addin: 'Opening Excel / connecting add-in', ready: 'Add-in ready',
  verifying_workbook: 'Verifying workbook identity', waiting_for_startup_sample: 'Sampling startup memory',
  waiting_for_job: 'Waiting for job', waiting_for_peers: 'Waiting for other add-ins',
  paused: 'Capture paused', settling: 'Waiting after Excel startup', planning_sheet: 'Determining sheet geometry',
  capturing: 'Capture job running', providing_images: 'Providing image evidence', executing_job: 'Office.js job running',
  releasing_images: 'Releasing capture images', releasing: 'Releasing workbook', closed: 'Closed', client_error: 'Add-in reported an error',
  blocked_edit_mode: 'Excel is in cell edit mode', waiting_cleanup: 'Waiting for cleanup',
  cleanup_blocked: 'Cleanup blocked', cleanup_failed: 'Cleanup could not be confirmed', exit_unconfirmed: 'Excel exit not confirmed',
  restart_waiting_cleanup: 'Restart waiting for cleanup', prepare: 'Preparing', preparing: 'Preparing',
  extract: 'Extracting file', shrink: 'Generating variants', compare: 'Coordinating comparison',
  save: 'Saving result', capture_planning: 'Planning captures', capture_upload: 'Waiting for captures',
  host_pixel_comparison: 'Comparing pixels on host', complete: 'Completed', completed: 'Completed',
  failed: 'Failed', interrupted: 'Interrupted', draining: 'Finishing groups in progress',
  finished: 'Completed', extracting: 'Extracting file', converting: 'Converting legacy Excel format',
  validating: 'Validating file and capture plan', shrinking: 'Shrinking and validating variant', saving: 'Saving result',
  measuring_open: 'Measuring Excel opening time', validating_values: 'Checking cell values and recalculation',
  recalc_starting_excel: 'Starting Excel for recalculation', recalc_opening: 'Opening workbook for recalculation',
  recalculating: 'Recalculating workbook', recalc_saving: 'Saving recalculated workbook',
  recalc_closing: 'Closing recalculation workbook',
  opening: 'Starting Excel clients', comparing: 'Coordinating profile comparison',
  comparing_images: 'Comparing image data on host', processing_result: 'Processing Excel response',
  waiting_client: 'Waiting for Office.js response', waiting_images: 'Waiting for images / batch responses',
  waiting_classic: 'Classic comparison subprocess running',
  preparing_sheet: 'Preparing worksheet', checking_row_heights: 'Reading Excel row heights',
  whitespace_diagnosis: 'Whitespace cross-check', closing: 'Closing Excel processes',
};
const waits = { coordinator: 'Next coordinator job', primary_addins: 'Other add-ins in group',
  capture_pause: 'Capture pause', startup_quiet_period: 'Quiet period after Excel startup',
  excel: 'Excel API response', launcher: 'Launcher startup sample' };

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

export function commandTimingSummary(timing) {
  if (!timing?.command_id || !timing.kind) return null;
  const milliseconds = value => {
    if (!Number.isFinite(value) || value < 0) return 'Not measured';
    const seconds = value >= 1000;
    const formatted = (seconds ? value / 1000 : value).toLocaleString('en-US', { maximumFractionDigits: 1 });
    return `${formatted} ${seconds ? 's' : 'ms'}`;
  };
  return {
    command: `${timing.kind} · ${timing.command_id}`,
    fields: [
      ['Execution', milliseconds(timing.execution_ms)],
      ['Excel image capture', milliseconds(timing.capture_ms)],
      ['Image finalization', milliseconds(timing.finish_image_ms)],
      ['PNG checksum (within finalization)', milliseconds(timing.hash_ms)],
      ['Image upload / receipt', milliseconds(timing.upload_ms)],
      ['Waiting for job', milliseconds(timing.job_wait_ms)],
      ['Result delivery / acknowledgment', milliseconds(timing.result_ack_ms)],
      ['Result delivery attempts', Number.isSafeInteger(timing.result_attempts) && timing.result_attempts >= 0
        ? timing.result_attempts.toLocaleString('en-US') : 'Not measured'],
    ],
    note: 'Subphases are included in execution time; do not add the rows. Image capture measures getImage round trips; finalization includes overflow checks and checksum time. Waiting for a job includes pauses and peer readiness, not just delivery time.',
  };
}

export function retainedImageSummary(resources) {
  const tiles = resources?.retained_image_tiles, bytes = resources?.retained_image_bytes;
  if (!Number.isSafeInteger(tiles) || tiles < 0 || !Number.isFinite(bytes) || bytes < 0) return null;
  const mib = (bytes / (1024 * 1024)).toLocaleString('en-US', { maximumFractionDigits: 2 });
  return `${tiles.toLocaleString('en-US')} tiles ? ${mib} MiB of retained PNGs`;
}

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
  if (state?.startsWith('waiting') || ['paused', 'settling', 'ready', 'blocked_edit_mode', 'client_error', 'cleanup_blocked', 'exit_unconfirmed'].includes(state)) return 'waiting';
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
    const plannedSheets = job?.locations?.map(item => item.sheet_name || item.name ||
      (item.sheet_index != null ? `Sheet ${item.sheet_index}` : null))
      .filter(Boolean).filter((name, index, names) => names.indexOf(name) === index).join(', ');
    const sheet = monitor.pane_resources?.capture_progress?.sheet_name || plannedSheets || monitor.sheet_name;
    return { ...common, id: client.worker_id, clientId: client.client_id, kind: 'Excel / Office.js',
      pid: client.pid, processStarted: client.process_started_unix_ms, alive: client.process_alive, windowMode: client.window_mode,
      sampledAt: group.launcher_sampled_unix_ms,
      state, stateSince: monitor.state === state ? monitor.state_since : client.stage_since,
      category: category(state, client.process_alive, common.active), lastProgress, lastSuccess, lastEvent,
      heartbeat: client.last_heartbeat_at, heartbeatCount: client.heartbeat_count,
      waitReason: waits[monitor.wait_reason] || monitor.wait_reason || (state === 'waiting_for_addin'
        ? 'Excel opening or add-in initialization; internal wait reason unknown' : null),
      job, sheet,
      events: [...(monitor.events || []), ...(client.last_launcher_event ? [client.last_launcher_event] : [])]
        .sort((a, b) => (timestamp(b) || 0) - (timestamp(a) || 0)).slice(0, 24),
      uploadedImages: monitor.uploaded_images, completedCommands: monitor.completed_commands, error: client.error,
      runtime: client.runtime, paneResources: monitor.pane_resources,
    };
  });
  if (group.coordinator) {
    const monitor = group.coordinator, blocked = group.cleanup_pending?.state === 'waiting';
    const exhausted = blocked && group.cleanup_pending.retry_exhausted === true;
    const state = blocked ? (exhausted ? 'cleanup_failed' : 'cleanup_blocked') : monitor.phase || monitor.state;
    rows.unshift({ ...common, id: `${group.id}/${group.attempt_id}/coordinator/${monitor.pid}/${monitor.process_started_unix_ms}`,
      clientId: 'Coordinator', kind: 'Rust / Host', pid: monitor.pid, processStarted: monitor.process_started_unix_ms,
      alive: monitor.process_alive,
      state, stateSince: blocked ? group.cleanup_pending.since_unix_ms : monitor.state_since ?? monitor.state_since_unix_ms, category: category(state, monitor.process_alive, common.active),
      lastEvent: monitor.last_event, lastProgress: monitor.last_progress_at ?? monitor.last_progress_unix_ms ?? monitor.last_progress,
      lastSuccess: monitor.last_success, waitReason: exhausted ? 'Cleanup retries exhausted. The run stopped; process journals and working files were retained.'
        : blocked ? 'Process ownership or exit could not be confirmed; this slot stays reserved while cleanup retries.' : monitor.wait_reason,
      error: blocked ? group.cleanup_pending.error : null,
      sheet: monitor.sheet_name || monitor.last_event?.details?.sheet_name,
      profile: monitor.last_event?.details?.profile,
      externalWorkbookLinks: monitor.last_event?.details?.external_workbook_links,
      linkUpdates: monitor.last_event?.details?.link_updates,
      events: [...(monitor.events || [])].reverse().slice(0, 24), heartbeat: null, heartbeatCount: undefined });
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
  const activity = progress.activity;
  const preparationTotal = counts.preparation_total;
  const preparationTerminal = counts.preparation_terminal;
  const visualTotal = counts.visual_total;
  const visualCompleted = counts.visual_completed;
  const outcomeLabels = {
    unchanged: 'No differences detected', differences: 'Differences found',
    skipped: 'Skipped / not validated', interrupted: 'Interrupted',
  };
  return { scope: scopeLabel, completed: number(counts.completed), skipped: number(counts.skipped),
    remaining: number(counts.remaining), total: number(counts.total), inProgress: number(counts.in_progress),
    preparationRemaining: number(counts.preparation_remaining), prepared: number(counts.prepared),
    preparationTotal: number(preparationTotal), preparationTerminal: number(preparationTerminal),
    preparationRatio: Number.isFinite(preparationTotal) && preparationTotal > 0
      ? Math.min(1, Number(preparationTerminal || 0) / preparationTotal) : null,
    visualScopeFrozen: progress.visual_scope_frozen === true,
    visualTotal: number(visualTotal), visualCompleted: number(visualCompleted),
    visualRemaining: number(counts.visual_remaining), visualExcluded: number(counts.visual_excluded),
    visualDeferred: number(counts.visual_deferred),
    visualRatio: progress.visual_scope_frozen === true && Number.isFinite(visualTotal) && visualTotal > 0
      ? Math.min(1, Number(visualCompleted || 0) / visualTotal) : null,
    activePreparation: (progress.active_preparation || []).filter(item => item.workbook).map(item => {
      const profile = ({ default: 'default', 'maximum-cleaning': 'maximum cleaning',
        'too-aggressive-cleaning': 'too aggressive cleaning' })[item.profile] || item.profile;
      return profile ? `${item.workbook} · ${profile}` : item.workbook;
    }),
    perMinute: number(rate.completed_per_minute), perHour: number(rate.completed_per_hour),
    averagePerHour: number(rate.average_completed_per_hour),
    preparationPerMinute: number(rate.preparation_per_minute),
    preparationPerHour: number(rate.preparation_per_hour),
    preparationEta: Number.isFinite(eta.preparation_remaining_seconds)
      ? `About ${duration(eta.preparation_remaining_seconds)}` : 'Estimating after more workbooks',
    window: duration(rate.window_seconds),
    eta: eta.status === 'estimating' && Number.isFinite(eta.remaining_seconds) ? `About ${duration(eta.remaining_seconds)}` : statuses[eta.status] || 'No estimate yet',
    finish: eta.status === 'estimating' ? timestamp(eta.finish_unix_ms) : null,
    diagnosisFailed: number(progress.diagnosis_failed), phase: progress.phase === 'paused' ? 'Run paused' : phaseLabel(progress.phase),
    activity: activity ? {
      observed: activity.source === 'observed_current_run',
      finished: number(activity.finished_attempts), compared: number(activity.compared_attempts),
      skipped: number(activity.skipped_attempts), interrupted: number(activity.interrupted_attempts),
      retried: number(activity.retried_attempts), perHour: number(activity.attempts_per_hour),
      outcomes: (activity.recent_outcomes || []).slice(0, 3).map(outcome => ({
        workbook: outcome.workbook, label: outcomeLabels[outcome.status] || outcome.status,
        reason: outcome.reason, retried: outcome.retried === true, at: outcome.at_unix_ms,
      })),
    } : null,
  };
}
