import { age, duration, phaseLabel, progressSummary, startupSummary, timestamp, workerRows } from './worker-monitor-model.js';

const element = (tag, className, text) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
};
const timeText = value => timestamp(value) == null ? 'Not observed' : new Date(timestamp(value)).toLocaleString('en-US');
const sinceText = value => age(value) == null ? 'Not observed' : `${duration(age(value))} ago`;
const message = event => typeof event === 'string' ? event : event?.message || event?.kind || event?.error || 'Not observed';
const observedPhase = row => row.alive === false && !['closed', 'finished', 'complete', 'completed'].includes(row.state)
  ? `Process exited · last ${phaseLabel(row.state)}` : phaseLabel(row.state);

export function initWorkerMonitor({ selectGroup } = {}) {
  const stylesheet = element('link');
  stylesheet.rel = 'stylesheet'; stylesheet.href = new URL('./worker-monitor.css', import.meta.url).href;
  document.head.append(stylesheet);
  const button = element('button', 'wm-open', 'Worker overview');
  button.id = 'workerMonitor'; button.type = 'button';
  document.querySelector('#activeGroups')?.after(button);
  const progress = element('section', 'wm-progress');
  progress.setAttribute('aria-label', 'Progress for selected corpora and partition');
  document.querySelector('.controls')?.after(progress);

  const dialog = element('dialog', 'wm-dialog');
  dialog.id = 'workerMonitorDialog'; dialog.setAttribute('aria-labelledby', 'wm-title');
  dialog.innerHTML = `<header class="wm-header"><div><div class="eyebrow">RUNTIME DIAGNOSTICS</div><h2 id="wm-title">Worker overview</h2><p>Heartbeats show connectivity. Progress shows completed steps.</p></div><button class="wm-close" aria-label="Close worker overview">×</button></header>
    <div class="wm-toolbar"><label>Show <select class="wm-filter"><option value="all">All states</option><option value="working">Job running / starting</option><option value="waiting">Waiting / paused</option><option value="ended">Ended</option></select></label><label><input type="checkbox" class="wm-recent"> Last 8 ended groups</label><span class="wm-summary"></span></div>
    <p class="wm-status" role="status"></p><div class="wm-content"><div class="wm-overview"><div class="wm-groups"></div><div class="wm-pager"><button class="wm-prev">Previous groups</button><span></span><button class="wm-next">More groups</button></div></div><aside class="wm-detail" aria-label="Worker details"><p>Select a worker to see its last successful step and events.</p></aside></div>`;
  document.body.append(dialog);
  const query = selector => dialog.querySelector(selector);
  const list = query('.wm-groups'), detail = query('.wm-detail'), status = query('.wm-status');
  let queue, snapshot, selected, offset = 0, timer, request, generation = 0;
  const heartbeats = new Map();

  function field(container, label, value, title) {
    const block = element('div', 'wm-field');
    block.append(element('dt', null, label), element('dd', null, value ?? 'Not observed'));
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
    field(fields, 'Status / phase', observedPhase(row));
    field(fields, 'Time in this state', duration(age(row.stateSince)), timeText(row.stateSince));
    field(fields, 'Sheet', row.sheet);
    field(fields, 'Wait reason', row.waitReason || (row.category === 'working'
      ? 'Job submitted; waits inside Excel cannot be observed.' : 'Not observed'));
    field(fields, 'Last heartbeat', row.kind === 'Rust / Host' ? 'No add-in heartbeat for the host' : sinceText(row.heartbeat), timeText(row.heartbeat));
    field(fields, 'Last actual progress', sinceText(row.lastProgress), timeText(row.lastProgress));
    field(fields, 'Last successful step', message(row.lastSuccess), timeText(row.lastSuccess));
    field(fields, 'Last event', `${message(row.lastEvent)} · ${sinceText(row.lastEvent)}`, timeText(row.lastEvent));
    field(fields, 'Process', row.pid ? `PID ${row.pid} · ${row.alive === true ? 'running' : row.alive === false ? 'exited' : 'liveness not checked separately'}` : 'No process ID yet');
    field(fields, 'Process start', timeText(row.processStarted));
    field(fields, 'Worker ID', row.id);
    field(fields, 'Group / attempt', `${row.groupId} / ${row.attempt || 'legacy format'}`);
    if (row.runtime) {
      field(fields, 'Taskpane document', row.runtime.document_id);
      field(fields, 'Taskpane initialization', row.runtime.run_id);
    }
    if (row.paneResources) {
      const counts = Object.entries(row.paneResources).filter(([,value]) => typeof value === 'number');
      for (const [name,value] of counts) field(fields, `Pane: ${name.replaceAll('_',' ')}`, value.toLocaleString('en-US'));
    }
    if (row.job) {
      field(fields, 'Current job', `${row.job.kind} · ${row.job.id}`);
      field(fields, 'Job regions', (row.job.locations || []).map(item => [item.sheet_name || item.name, item.range].filter(Boolean).join('!')).join(', ') || 'Not specified in job');
    }
    if (row.uploadedImages != null) field(fields, 'Images received / jobs completed', `${row.uploadedImages} / ${row.completedCommands}`);
    if (row.error) field(fields, 'Reported error', message(row.error));
    const open = element('button', null, 'Show group in studio');
    open.addEventListener('click', () => { dialog.close(); selectGroup?.(row.groupId); });
    const events = element('ol', 'wm-events');
    for (const event of row.events || []) {
      const item = element('li');
      item.append(element('time', null, timeText(event)), element('span', null, message(event)));
      if (event.details) item.append(element('small', null, [event.details.sheet_name, event.details.range, event.details.command_id || event.details.id].filter(Boolean).join(' · ')));
      events.append(item);
    }
    detail.replaceChildren(heading, workbook);
    if (!current) detail.append(element('p', 'wm-muted', 'Last available snapshot; this worker is no longer on the current page.'));
    if (!row.active) detail.append(element('p', 'wm-muted', 'This group has ended. Events show the last saved snapshot.'));
    detail.append(fields, open, element('h4', null, 'Recent events · up to 24'), events);
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
      if (!rows.length) card.append(element('p', 'wm-muted', 'Process data for this attempt is still being provided.'));
      for (const row of rows) {
        const item = element('button', `wm-worker wm-${row.category}${selected?.id === row.id ? ' wm-selected' : ''}`);
        item.dataset.workerId = row.id;
        item.type = 'button'; item.setAttribute('aria-pressed', String(selected?.id === row.id));
        const identity = element('span', 'wm-identity');
        const heartbeat = element('span', 'wm-heart', row.kind === 'Rust / Host' ? '◇' : '♥');
        const heartbeatAge = age(row.heartbeat), oldCount = heartbeats.get(row.id);
        heartbeat.style.setProperty('--wm-heart-strength', heartbeatAge == null ? '.12' : String(Math.max(.12, 1 - heartbeatAge / 30)));
        heartbeat.title = row.kind === 'Rust / Host' ? 'Host telemetry' : `Heartbeat ${sinceText(row.heartbeat)}`;
        if (oldCount != null && row.heartbeatCount > oldCount) heartbeat.classList.add('wm-beat');
        heartbeats.set(row.id, row.heartbeatCount);
        identity.append(heartbeat, element('strong', null, row.clientId), element('small', null, `${row.kind}${row.pid ? ` · PID ${row.pid}` : ''}`));
        const phase = element('span', 'wm-phase');
        phase.append(element('strong', null, observedPhase(row)), element('small', null, `${duration(age(row.stateSince))} since state change${row.sheet ? ` · ${row.sheet}` : ''}`));
        const activity = element('span', 'wm-activity');
        activity.append(element('span', null, `Progress ${sinceText(row.lastProgress)}`), element('small', null, row.kind === 'Rust / Host' ? `Event ${sinceText(row.lastEvent)}` : `Heartbeat ${sinceText(row.heartbeat)}`));
        item.append(identity, phase, activity);
        item.addEventListener('click', () => { showDetail(row); render(); });
        card.append(item);
      }
      fragment.append(card);
    }
    if (!fragment.childNodes.length) fragment.append(element('p', 'wm-muted', 'No workers match this filter.'));
    list.replaceChildren(fragment);
    if (focusedWorker) [...list.querySelectorAll('.wm-worker')]
      .find(item => item.dataset.workerId === focusedWorker)?.focus({ preventScroll: true });
    // Retain only this bounded page, so a day-long run does not grow browser memory.
    const ids = new Set(allRows.map(row => row.id));
    for (const id of heartbeats.keys()) if (!ids.has(id)) heartbeats.delete(id);
    query('.wm-summary').textContent = `${snapshot.active_groups} active groups · ${allRows.filter(row => row.kind !== 'Rust / Host').length} Excel workers on this page`;
    query('.wm-prev').disabled = offset === 0;
    query('.wm-next').disabled = snapshot.next_offset == null;
    query('.wm-pager span').textContent = snapshot.total_groups ? `Groups ${offset + 1}–${Math.min(offset + 64, snapshot.total_groups)} of ${snapshot.total_groups}` : 'No active groups';
    const warning = snapshot.runner_alive === false ? 'Runner exited; showing last saved snapshot.' : snapshot.running === false ? 'No active corpus run; showing last saved snapshot.' : 'Long waits alone do not indicate an error.';
    status.textContent = `${warning} Snapshot ${timeText(snapshot.sampled_at)}${snapshot.diagnostics?.length ? ` · ${snapshot.diagnostics.length} note(s): ${snapshot.diagnostics.map(message).join('; ')}` : ''}`;
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
      if (error.name !== 'AbortError') status.textContent = `Could not refresh worker status: ${error.message}. Displayed data is the last received snapshot.`;
    } finally {
      if (token === generation && dialog.open) timer = setTimeout(refresh, 2000);
    }
  }

  button.addEventListener('click', () => { dialog.showModal(); status.textContent = 'Loading workers …'; refresh(); });
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
      heading.append(element('strong', null, 'Preparation'), element('span', null, startup.corpus));
      const label = element('p', null, `${startup.label}: ${startup.count}${startup.percent == null ? '' : ` (${startup.percent} %)`}`);
      const bar = element('progress', 'wm-startup-bar');
      bar.max = 100;
      if (startup.percent != null) bar.value = startup.percent;
      bar.setAttribute('aria-label', startup.label);
      const note = element('p', 'wm-progress-note', `Current phase has run for ${startup.elapsed}. Progress refers to this phase; further preparation steps follow. Comparisons start afterward.`);
      if (queue.runner_alive === false || queue.running === false) note.textContent = 'Runner inactive: last saved preparation snapshot.';
      progress.replaceChildren(heading, label, bar, note);
      return;
    }
    const summary = progressSummary(queue.progress);
    if (!summary) {
      progress.replaceChildren(element('p', 'wm-muted', 'Corpus progress is provided by the runner; older runs do not include throughput measurements.'));
      return;
    }
    const heading = element('div', 'wm-progress-heading');
    heading.append(element('strong', null, summary.scope), element('span', null, summary.phase));
    const numbers = element('dl', 'wm-progress-numbers');
    field(numbers, 'Compared', summary.completed);
    field(numbers, 'Skipped', summary.skipped);
    field(numbers, 'Remaining / total', `${summary.remaining} / ${summary.total}`);
    field(numbers, 'In progress', summary.inProgress);
    field(numbers, 'Compared / min', summary.perMinute);
    field(numbers, 'Compared / h', summary.perHour);
    const footer = element('p', 'wm-progress-note', `${summary.eta}${summary.finish ? ` · around ${timeText(summary.finish)}` : ''}. Rate of successfully compared workbooks in the active window of ${summary.window}. Run average: ${summary.averagePerHour}/h. Remaining preparation: ${summary.preparationRemaining} (${summary.preparationPerHour}/h). Fully idle pauses (target 0) are excluded; capture pauses with active groups still count.`);
    if (Number(queue.progress.diagnosis_failed) > 0) footer.append(document.createTextNode(` Unresolved diagnostic failures: ${summary.diagnosisFailed}.`));
    if (queue.runner_alive === false || queue.running === false) footer.append(document.createTextNode(' Runner inactive: last saved snapshot.'));
    progress.replaceChildren(heading, numbers, footer);
  } };
}
