import { age, duration, phaseLabel, progressSummary, startupSummary, timestamp, workerRows } from './worker-monitor-model.js';

const element = (tag, className, text) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
};
const timeText = value => timestamp(value) == null ? 'Nicht beobachtet' : new Date(timestamp(value)).toLocaleString('de-DE');
const sinceText = value => age(value) == null ? 'Nicht beobachtet' : `vor ${duration(age(value))}`;
const message = event => typeof event === 'string' ? event : event?.message || event?.kind || event?.error || 'Nicht beobachtet';
const observedPhase = row => row.alive === false && !['closed', 'finished', 'complete', 'completed'].includes(row.state)
  ? `Prozess beendet · zuletzt ${phaseLabel(row.state)}` : phaseLabel(row.state);

export function initWorkerMonitor({ selectGroup } = {}) {
  const stylesheet = element('link');
  stylesheet.rel = 'stylesheet'; stylesheet.href = new URL('./worker-monitor.css', import.meta.url).href;
  document.head.append(stylesheet);
  const button = element('button', 'wm-open', 'Worker-Übersicht');
  button.id = 'workerMonitor'; button.type = 'button';
  document.querySelector('#activeGroups')?.after(button);
  const progress = element('section', 'wm-progress');
  progress.setAttribute('aria-label', 'Fortschritt der ausgewählten Korpora und Partition');
  document.querySelector('.controls')?.after(progress);

  const dialog = element('dialog', 'wm-dialog');
  dialog.id = 'workerMonitorDialog'; dialog.setAttribute('aria-labelledby', 'wm-title');
  dialog.innerHTML = `<header class="wm-header"><div><div class="eyebrow">LAUFZEITDIAGNOSE</div><h2 id="wm-title">Worker-Übersicht</h2><p>Heartbeats zeigen Erreichbarkeit. Fortschritt zeigt abgeschlossene Schritte.</p></div><button class="wm-close" aria-label="Worker-Übersicht schließen">×</button></header>
    <div class="wm-toolbar"><label>Anzeigen <select class="wm-filter"><option value="all">Alle Zustände</option><option value="working">Auftrag läuft / startet</option><option value="waiting">Wartend / pausiert</option><option value="ended">Beendet</option></select></label><label><input type="checkbox" class="wm-recent"> Letzte 8 beendete Gruppen</label><span class="wm-summary"></span></div>
    <p class="wm-status" role="status"></p><div class="wm-content"><div class="wm-overview"><div class="wm-groups"></div><div class="wm-pager"><button class="wm-prev">Vorherige Gruppen</button><span></span><button class="wm-next">Weitere Gruppen</button></div></div><aside class="wm-detail" aria-label="Worker-Details"><p>Einen Worker auswählen, um seinen letzten erfolgreichen Schritt und die Ereignisse zu sehen.</p></aside></div>`;
  document.body.append(dialog);
  const query = selector => dialog.querySelector(selector);
  const list = query('.wm-groups'), detail = query('.wm-detail'), status = query('.wm-status');
  let queue, snapshot, selected, offset = 0, timer, request, generation = 0;
  const heartbeats = new Map();

  function field(container, label, value, title) {
    const block = element('div', 'wm-field');
    block.append(element('dt', null, label), element('dd', null, value ?? 'Nicht beobachtet'));
    if (title) block.lastChild.title = title;
    container.append(block);
  }

  function showDetail(row, current = true) {
    selected = row;
    const scroll = detail.scrollTop;
    const heading = element('div', 'wm-detail-heading');
    heading.append(element('h3', null, row.clientId), element('span', 'wm-kind', row.kind));
    const workbook = element('p', 'wm-workbook', row.workbook);
    const fields = element('dl', 'wm-detail-fields');
    field(fields, 'Status / Phase', observedPhase(row));
    field(fields, 'Seit diesem Zustand', duration(age(row.stateSince)), timeText(row.stateSince));
    field(fields, 'Sheet', row.sheet);
    field(fields, 'Warteursache', row.waitReason || (row.category === 'working'
      ? 'Auftrag wurde übergeben; Excel-interne Wartezeiten sind nicht beobachtbar.' : 'Nicht beobachtet'));
    field(fields, 'Letzter Heartbeat', row.kind === 'Rust / Host' ? 'Kein Add-in-Heartbeat für den Host' : sinceText(row.heartbeat), timeText(row.heartbeat));
    field(fields, 'Letzter tatsächlicher Fortschritt', sinceText(row.lastProgress), timeText(row.lastProgress));
    field(fields, 'Letzter erfolgreicher Schritt', message(row.lastSuccess), timeText(row.lastSuccess));
    field(fields, 'Letztes Ereignis', `${message(row.lastEvent)} · ${sinceText(row.lastEvent)}`, timeText(row.lastEvent));
    field(fields, 'Prozess', row.pid ? `PID ${row.pid} · ${row.alive === true ? 'läuft' : row.alive === false ? 'beendet' : 'Lebenszeichen nicht separat geprüft'}` : 'Noch keine Prozess-ID');
    field(fields, 'Prozessstart', timeText(row.processStarted));
    field(fields, 'Worker-ID', row.id);
    field(fields, 'Gruppe / Versuch', `${row.groupId} / ${row.attempt || 'älteres Format'}`);
    if (row.job) {
      field(fields, 'Aktueller Auftrag', `${row.job.kind} · ${row.job.id}`);
      field(fields, 'Erfasste Auftragsbereiche', (row.job.locations || []).map(item => [item.sheet_name || item.name, item.range].filter(Boolean).join('!')).join(', ') || 'Nicht im Auftrag angegeben');
    }
    if (row.uploadedImages != null) field(fields, 'Bilder empfangen / Aufträge fertig', `${row.uploadedImages} / ${row.completedCommands}`);
    if (row.error) field(fields, 'Gemeldeter Fehler', message(row.error));
    const open = element('button', null, 'Gruppe im Atelier zeigen');
    open.addEventListener('click', () => { dialog.close(); selectGroup?.(row.groupId); });
    const events = element('ol', 'wm-events');
    for (const event of row.events || []) {
      const item = element('li');
      item.append(element('time', null, timeText(event)), element('span', null, message(event)));
      if (event.details) item.append(element('small', null, [event.details.sheet_name, event.details.range, event.details.command_id || event.details.id].filter(Boolean).join(' · ')));
      events.append(item);
    }
    detail.replaceChildren(heading, workbook);
    if (!current) detail.append(element('p', 'wm-muted', 'Letzter verfügbarer Stand; dieser Worker ist auf der aktuellen Seite nicht mehr enthalten.'));
    if (!row.active) detail.append(element('p', 'wm-muted', 'Diese Gruppe ist beendet. Die Ereignisse sind der letzte gespeicherte Stand.'));
    detail.append(fields, open, element('h4', null, 'Letzte Ereignisse · maximal 24'), events);
    detail.scrollTop = scroll;
  }

  function render() {
    if (!snapshot) return;
    const focusedWorker = document.activeElement?.dataset?.workerId;
    const filter = query('.wm-filter').value, fragment = document.createDocumentFragment();
    const allRows = snapshot.groups.flatMap(workerRows);
    const selectedNow = allRows.find(row => row.id === selected?.id);
    if (selectedNow) showDetail(selectedNow);
    else if (selected) showDetail(selected, false);
    for (const group of snapshot.groups) {
      const rows = workerRows(group).filter(row => filter === 'all' || row.category === filter);
      if (filter !== 'all' && !rows.length) continue;
      const card = element('section', 'wm-group');
      const groupTitle = element('div', 'wm-group-title');
      groupTitle.append(element('strong', null, group.workbook || group.id), element('span', null, `${group.corpus || ''} · ${phaseLabel(group.status)}`));
      card.append(groupTitle);
      if (!rows.length) card.append(element('p', 'wm-muted', 'Prozessdaten für diesen Versuch werden noch bereitgestellt.'));
      for (const row of rows) {
        const item = element('button', `wm-worker wm-${row.category}${selected?.id === row.id ? ' wm-selected' : ''}`);
        item.dataset.workerId = row.id;
        item.type = 'button'; item.setAttribute('aria-pressed', String(selected?.id === row.id));
        const identity = element('span', 'wm-identity');
        const heartbeat = element('span', 'wm-heart', row.kind === 'Rust / Host' ? '◇' : '♥');
        const heartbeatAge = age(row.heartbeat), oldCount = heartbeats.get(row.id);
        heartbeat.style.setProperty('--wm-heart-strength', heartbeatAge == null ? '.12' : String(Math.max(.12, 1 - heartbeatAge / 30)));
        heartbeat.title = row.kind === 'Rust / Host' ? 'Host-Telemetrie' : `Heartbeat ${sinceText(row.heartbeat)}`;
        if (oldCount != null && row.heartbeatCount > oldCount) heartbeat.classList.add('wm-beat');
        heartbeats.set(row.id, row.heartbeatCount);
        identity.append(heartbeat, element('strong', null, row.clientId), element('small', null, `${row.kind}${row.pid ? ` · PID ${row.pid}` : ''}`));
        const phase = element('span', 'wm-phase');
        phase.append(element('strong', null, observedPhase(row)), element('small', null, `${duration(age(row.stateSince))} seit Zustandswechsel${row.sheet ? ` · ${row.sheet}` : ''}`));
        const activity = element('span', 'wm-activity');
        activity.append(element('span', null, `Fortschritt ${sinceText(row.lastProgress)}`), element('small', null, row.kind === 'Rust / Host' ? `Ereignis ${sinceText(row.lastEvent)}` : `Heartbeat ${sinceText(row.heartbeat)}`));
        item.append(identity, phase, activity);
        item.addEventListener('click', () => { showDetail(row); render(); });
        card.append(item);
      }
      fragment.append(card);
    }
    if (!fragment.childNodes.length) fragment.append(element('p', 'wm-muted', 'Keine Worker mit diesem Filter.'));
    list.replaceChildren(fragment);
    if (focusedWorker) [...list.querySelectorAll('.wm-worker')]
      .find(item => item.dataset.workerId === focusedWorker)?.focus({ preventScroll: true });
    // Retain only this bounded page, so a day-long run does not grow browser memory.
    const ids = new Set(allRows.map(row => row.id));
    for (const id of heartbeats.keys()) if (!ids.has(id)) heartbeats.delete(id);
    query('.wm-summary').textContent = `${snapshot.active_groups} aktive Gruppen · ${allRows.filter(row => row.kind !== 'Rust / Host').length} Excel-Worker auf dieser Seite`;
    query('.wm-prev').disabled = offset === 0;
    query('.wm-next').disabled = snapshot.next_offset == null;
    query('.wm-pager span').textContent = snapshot.total_groups ? `Gruppen ${offset + 1}–${Math.min(offset + 64, snapshot.total_groups)} von ${snapshot.total_groups}` : 'Keine aktiven Gruppen';
    const warning = snapshot.runner_alive === false ? 'Runner beendet; gespeicherter letzter Stand.' : snapshot.running === false ? 'Kein aktiver Corpuslauf; gespeicherter letzter Stand.' : 'Lange Wartezeiten allein sind kein Fehler.';
    status.textContent = `${warning} Stand ${timeText(snapshot.sampled_at)}${snapshot.diagnostics?.length ? ` · ${snapshot.diagnostics.length} Hinweis(e): ${snapshot.diagnostics.map(message).join('; ')}` : ''}`;
  }

  async function refresh() {
    clearTimeout(timer);
    if (!dialog.open) return;
    request?.abort(); request = new AbortController();
    const token = ++generation;
    try {
      const response = await fetch(`/api/workers?offset=${offset}&include_recent=${query('.wm-recent').checked}`, { signal: request.signal, cache: 'no-store' });
      if (!response.ok) throw new Error(`Worker-API: HTTP ${response.status}`);
      const data = await response.json();
      if (token !== generation || !dialog.open) return;
      snapshot = data;
      if (offset > 0 && offset >= data.total_groups) { offset = 0; return refresh(); }
      render();
    } catch (error) {
      if (error.name !== 'AbortError') status.textContent = `Worker-Status konnte nicht aktualisiert werden: ${error.message}. Angezeigte Daten sind der letzte empfangene Stand.`;
    } finally {
      if (token === generation && dialog.open) timer = setTimeout(refresh, 2000);
    }
  }

  button.addEventListener('click', () => { dialog.showModal(); status.textContent = 'Worker laden …'; refresh(); });
  query('.wm-close').addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => { clearTimeout(timer); request?.abort(); generation++; button.focus(); });
  query('.wm-filter').addEventListener('change', render);
  query('.wm-recent').addEventListener('change', () => { offset = 0; refresh(); });
  query('.wm-prev').addEventListener('click', () => { offset = Math.max(0, offset - 64); refresh(); });
  query('.wm-next').addEventListener('click', () => { offset = snapshot?.next_offset ?? 0; refresh(); });

  return { update(nextQueue) {
    queue = nextQueue;
    const startup = startupSummary(queue.startup);
    if (startup) {
      const heading = element('div', 'wm-progress-heading');
      heading.append(element('strong', null, 'Vorverarbeitung'), element('span', null, startup.corpus));
      const label = element('p', null, `${startup.label}: ${startup.count}${startup.percent == null ? '' : ` (${startup.percent} %)`}`);
      const bar = element('progress', 'wm-startup-bar');
      bar.max = 100;
      if (startup.percent != null) bar.value = startup.percent;
      bar.setAttribute('aria-label', startup.label);
      const note = element('p', 'wm-progress-note', `Aktuelle Phase seit ${startup.elapsed}. Die Anzeige bezieht sich auf diese Phase; weitere Vorbereitungsschritte folgen. Vergleiche starten danach.`);
      if (queue.runner_alive === false || queue.running === false) note.textContent = 'Runner inaktiv: letzter gespeicherter Stand der Vorverarbeitung.';
      progress.replaceChildren(heading, label, bar, note);
      return;
    }
    const summary = progressSummary(queue.progress);
    if (!summary) {
      progress.replaceChildren(element('p', 'wm-muted', 'Corpusfortschritt wird vom Runner bereitgestellt; ältere Läufe enthalten noch keine Geschwindigkeitsmessung.'));
      return;
    }
    const heading = element('div', 'wm-progress-heading');
    heading.append(element('strong', null, summary.scope), element('span', null, summary.phase));
    const numbers = element('dl', 'wm-progress-numbers');
    field(numbers, 'Verglichen', summary.completed);
    field(numbers, 'Übersprungen', summary.skipped);
    field(numbers, 'Übrig / insgesamt', `${summary.remaining} / ${summary.total}`);
    field(numbers, 'In Bearbeitung', summary.inProgress);
    field(numbers, 'Verglichen / min', summary.perMinute);
    field(numbers, 'Verglichen / h', summary.perHour);
    const footer = element('p', 'wm-progress-note', `${summary.eta}${summary.finish ? ` · etwa ${timeText(summary.finish)}` : ''}. Rate erfolgreich verglichener Arbeitsmappen im aktiven Zeitfenster von ${summary.window}. Laufmittel: ${summary.averagePerHour}/h. Noch vorzubereiten: ${summary.preparationRemaining} (${summary.preparationPerHour}/h). Vollständig ruhende Pausen (Soll 0) zählen nicht mit; Capture-Pausen mit aktiven Gruppen zählen weiter.`);
    if (Number(queue.progress.diagnosis_failed) > 0) footer.append(document.createTextNode(` Offene Diagnosefehler: ${summary.diagnosisFailed}.`));
    if (queue.runner_alive === false || queue.running === false) footer.append(document.createTextNode(' Runner inaktiv: letzter gespeicherter Stand.'));
    progress.replaceChildren(heading, numbers, footer);
  } };
}
