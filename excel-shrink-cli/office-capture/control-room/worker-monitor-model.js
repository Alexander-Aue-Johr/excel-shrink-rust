// Read-only presentation of observations. A live heartbeat is never progress,
// and a long-running command alone is never classified as an error.
const phases = {
  starting: 'Startet', staging: 'Arbeitsmappe bereitstellen', waiting_for_open_slot: 'Wartet auf Öffnen',
  waiting_for_addin: 'Excel öffnen / Add-in verbinden', ready: 'Add-in bereit',
  waiting_for_job: 'Wartet auf Auftrag', waiting_for_peers: 'Wartet auf andere Add-ins',
  paused: 'Capture pausiert', settling: 'Wartet nach Excel-Start', planning_sheet: 'Sheet-Geometrie ermitteln',
  capturing: 'Capture-Auftrag läuft', providing_images: 'Bildbelege liefern', executing_job: 'Office.js-Auftrag läuft',
  releasing: 'Arbeitsmappe freigeben', closed: 'Geschlossen', client_error: 'Add-in meldet Fehler',
  blocked_edit_mode: 'Excel ist im Zellenbearbeitungsmodus', waiting_cleanup: 'Wartet auf Aufräumen',
  restart_waiting_cleanup: 'Neustart wartet auf Aufräumen', prepare: 'Vorbereiten', preparing: 'Vorbereiten',
  extract: 'Datei entpacken', shrink: 'Varianten erzeugen', compare: 'Vergleich koordinieren',
  save: 'Ergebnis speichern', capture_planning: 'Capture planen', capture_upload: 'Auf Captures warten',
  host_pixel_comparison: 'Pixel auf dem Host vergleichen', complete: 'Abgeschlossen', completed: 'Abgeschlossen',
  failed: 'Fehlgeschlagen', interrupted: 'Unterbrochen', draining: 'Begonnene Gruppen fertigstellen',
  finished: 'Abgeschlossen', extracting: 'Datei entpacken', converting: 'Altes Excel-Format konvertieren',
  validating: 'Datei und Capture-Plan prüfen', shrinking: 'Variante shrinken und prüfen', saving: 'Ergebnis speichern',
  measuring_open: 'Öffnungsdauer in Excel messen', validating_values: 'Zellwerte und Neuberechnung prüfen',
  opening: 'Excel-Clients starten', comparing: 'Profilvergleich koordinieren',
  comparing_images: 'Bilddaten auf dem Host vergleichen', processing_result: 'Excel-Antwort verarbeiten',
  waiting_client: 'Wartet auf Office.js-Antwort', waiting_images: 'Wartet auf Bilder / Batch-Antworten',
  whitespace_diagnosis: 'Whitespace-Gegenprobe', closing: 'Excel-Prozesse schließen',
};
const waits = { coordinator: 'Nächster Auftrag des Orchestrierers', primary_addins: 'Andere Add-ins der Gruppe',
  capture_pause: 'Capture-Pause', startup_quiet_period: 'Ruhezeit nach Excel-Start' };

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
export function phaseLabel(phase) { return phases[phase] || phase || 'Nicht beobachtet'; }
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
        ? 'Excel-Öffnen oder Add-in-Initialisierung; interne Warteursache unbekannt' : null),
      job, sheet: job?.locations?.map(item => item.sheet_name || item.name || (item.sheet_index != null ? `Sheet ${item.sheet_index}` : null)).filter(Boolean).filter((name, i, names) => names.indexOf(name) === i).join(', ') || monitor.sheet_name,
      events: [...(monitor.events || []), ...(client.last_launcher_event ? [client.last_launcher_event] : [])]
        .sort((a, b) => (timestamp(b) || 0) - (timestamp(a) || 0)).slice(0, 24),
      uploadedImages: monitor.uploaded_images, completedCommands: monitor.completed_commands, error: client.error,
    };
  });
  if (group.coordinator) {
    const monitor = group.coordinator, state = monitor.phase || monitor.state;
    rows.unshift({ ...common, id: `${group.id}/${group.attempt_id}/coordinator/${monitor.pid}/${monitor.process_started_unix_ms}`,
      clientId: 'Orchestrierer', kind: 'Rust / Host', pid: monitor.pid, processStarted: monitor.process_started_unix_ms,
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
  const number = value => Number.isFinite(value) ? new Intl.NumberFormat('de-DE', { maximumFractionDigits: 1 }).format(value) : '—';
  const statuses = { insufficient_samples: 'Warte auf genügend Abschlüsse', preparation_pending: 'Vorbereitung läuft; noch keine Gesamtschätzung',
    paused: 'Pausiert', complete: 'Auswahl abgeschlossen', limited: 'Lauflimit: keine Schätzung für die ganze Auswahl',
    unavailable: 'Noch keine belastbare Schätzung' };
  const scopeLabel = [scope.corpora?.join(', '), scope.size_part ? `Partition ${scope.size_part.part}/${scope.size_part.parts}` : 'Gesamter Korpus',
    ['descending', 'largest-first', 'size-desc'].includes(scope.order) ? 'größte zuerst' : ['ascending', 'smallest-first', 'size-asc'].includes(scope.order) ? 'kleinste zuerst' : scope.order === 'priority' ? 'Prioritätsreihenfolge' : null].filter(Boolean).join(' · ');
  return { scope: scopeLabel, completed: number(counts.completed), skipped: number(counts.skipped),
    remaining: number(counts.remaining), total: number(counts.total), inProgress: number(counts.in_progress),
    preparationRemaining: number(counts.preparation_remaining), prepared: number(counts.prepared),
    perMinute: number(rate.completed_per_minute), perHour: number(rate.completed_per_hour),
    averagePerHour: number(rate.average_completed_per_hour),
    preparationPerHour: number(rate.preparation_per_hour), window: duration(rate.window_seconds),
    eta: eta.status === 'estimating' && Number.isFinite(eta.remaining_seconds) ? `Noch etwa ${duration(eta.remaining_seconds)}` : statuses[eta.status] || 'Noch keine Schätzung',
    finish: eta.status === 'estimating' ? timestamp(eta.finish_unix_ms) : null,
    diagnosisFailed: number(progress.diagnosis_failed), phase: progress.phase === 'paused' ? 'Lauf pausiert' : phaseLabel(progress.phase),
  };
}
