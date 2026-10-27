import { age, commandTimingSummary, duration, phaseLabel, progressSummary, retainedImageSummary, startupSummary, timestamp, workerRows } from './worker-monitor-model.js';
import { createSignalMonitor } from './signal-monitor.js';

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

export function initWorkerMonitor({ selectGroup, navigateToCaptures } = {}) {
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
  dialog.innerHTML = `<header class="wm-header"><div><div class="eyebrow">LIVE TELEMETRY</div><h2 id="wm-title">Worker overview</h2><p>ECG-shaped heartbeat events, spatial capture miniatures and a compact state strip on a shared timeline.</p></div><button class="wm-close" aria-label="Close worker overview">×</button></header>
    <div class="wm-toolbar"><label>Show <select class="wm-filter"><option value="all">All states</option><option value="working">Job running / starting</option><option value="waiting">Waiting / paused</option><option value="ended">Ended</option></select></label><label><input type="checkbox" class="wm-recent"> Last 8 ended groups</label><span class="wm-summary"></span></div>
    <p class="wm-status" role="status"></p><div class="wm-content"><div class="wm-overview"><div class="wm-groups"></div><div class="wm-pager"><button class="wm-prev">Previous groups</button><span></span><button class="wm-next">More groups</button></div></div><aside class="wm-detail" aria-label="Worker details"><p>Select a worker to see its last successful step and events.</p></aside></div>`;
  document.body.append(dialog);
  const query = selector => dialog.querySelector(selector);
  const list = query('.wm-groups'), detail = query('.wm-detail'), status = query('.wm-status');
  let queue, snapshot, selected, offset = 0, timer, request, generation = 0;
  const groupViews = new Map();
  const signalViews = new Map();
  const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
  let detailSignal, detailChrome;

  function openCaptures(row, capture = null, follow = true) {
    if (row.kind === 'Rust / Host' || !row.groupId) return;
    dialog.close();
    if (navigateToCaptures) navigateToCaptures({ groupId: row.groupId, clientId: row.clientId, capture, follow });
    else selectGroup?.(row.groupId);
  }

  function field(container, label, value, title) {
    const block = element('div', 'wm-field');
    block.append(element('dt', null, label), element('dd', null, value ?? 'Not observed'));
    if (title) block.lastChild.title = title;
    container.append(block);
  }

  function showDetail(row, current = true) {
    selected = row;
    const scroll = detail.scrollTop;
    if (!detailChrome) {
      const heading = element('div', 'wm-detail-heading');
      const name = element('h3');
      const kind = element('span', 'wm-kind');
      const workbook = element('p', 'wm-workbook');
      const notices = element('div');
      const body = element('div');
      const timing = element('div');
      const events = element('ol', 'wm-events');
      const open = element('button', null, 'Show group in studio');
      open.addEventListener('click', () => {
        if (!selected) return;
        dialog.close();
        selectGroup?.(selected.groupId);
      });
      const jump = element('button', 'wm-jump', '↗ Follow captures');
      jump.title = 'Follow this Excel profile directly in the studio.';
      jump.addEventListener('click', () => { if (selected) openCaptures(selected); });
      heading.append(name, kind);
      detailSignal = createSignalMonitor({ onCaptureNavigate: ({ row, capture, follow }) => openCaptures(row, capture, follow) });
      // Build the detail shell once. Its signal root stays connected while
      // snapshots change, including when its fullscreen view is open.
      detail.replaceChildren(heading, workbook, notices, detailSignal.element, body,
        timing, open, jump, element('h4', null, 'Recent events · up to 24'), events);
      detailChrome = { name, kind, workbook, notices, body, timing, jump, events };
    }
    detailChrome.name.textContent = row.clientId;
    detailChrome.kind.textContent = row.kind;
    detailChrome.workbook.textContent = row.workbook;
    const notices = [];
    if (!current) notices.push(element('p', 'wm-muted', 'Last available snapshot; this worker is no longer on the current page.'));
    if (!row.active) notices.push(element('p', 'wm-muted', 'This group has ended. Events show the last saved snapshot.'));
    detailChrome.notices.replaceChildren(...notices);
    detailChrome.jump.disabled = row.kind === 'Rust / Host';
    detailSignal.update(row, { stale: !current, live: current && snapshot?.running !== false && snapshot?.runner_alive !== false });
    const fields = element('dl', 'wm-detail-fields');
    field(fields, 'Status / phase', observedPhase(row));
    field(fields, 'Time in this state', duration(age(row.stateSince)), timeText(row.stateSince));
    field(fields, 'Sheet', row.sheet);
    if (row.profile) field(fields, 'Profile / input', row.profile);
    if (Number.isInteger(row.externalWorkbookLinks)) {
      field(fields, 'External workbook links', row.externalWorkbookLinks.toLocaleString('en-US'),
        row.linkUpdates === 'disabled' ? 'Link updates are disabled for this recalculation.' : null);
    }
    field(fields, 'Wait reason', row.waitReason || (row.category === 'working'
      ? 'Job submitted; waits inside Excel cannot be observed.' : 'Not observed'));
    field(fields, 'Last heartbeat', row.kind === 'Rust / Host' ? 'No add-in heartbeat for the host' : sinceText(row.heartbeat), timeText(row.heartbeat));
    field(fields, 'Last actual progress', sinceText(row.lastProgress), timeText(row.lastProgress));
    field(fields, 'Last successful step', message(row.lastSuccess), timeText(row.lastSuccess));
    field(fields, 'Last event', `${message(row.lastEvent)} · ${sinceText(row.lastEvent)}`, timeText(row.lastEvent));
    field(fields, 'Process', row.pid ? `PID ${row.pid} · ${row.alive === true ? 'running' : row.alive === false ? 'exited' : 'liveness not checked separately'}` : 'No process ID yet');
    field(fields, 'Process start', timeText(row.processStarted));
    if (row.windowMode) field(fields, 'Excel desktop', row.windowMode === 'private_desktop'
      ? 'Private desktop (hidden from your workspace)' : row.windowMode.replaceAll('_', ' '));
    field(fields, 'Worker ID', row.id);
    field(fields, 'Group / attempt', `${row.groupId} / ${row.attempt || 'legacy format'}`);
    if (row.runtime) {
      field(fields, 'Taskpane document', row.runtime.document_id);
      field(fields, 'Taskpane initialization', row.runtime.run_id);
    }
    if (row.paneResources) {
      const retained = retainedImageSummary(row.paneResources);
      if (retained) field(fields, 'Retained capture images', retained, 'Exact PNGs kept until the coordinator requests evidence or releases them.');
      const counts = Object.entries(row.paneResources).filter(([name,value]) => typeof value === 'number' &&
        !['retained_image_tiles', 'retained_image_bytes'].includes(name));
      for (const [name,value] of counts) field(fields, `Pane: ${name.replaceAll('_',' ')}`, value.toLocaleString('en-US'));
    }
    if (row.job) {
      field(fields, 'Current job', `${row.job.kind} · ${row.job.id}`);
      field(fields, 'Job regions', (row.job.locations || []).map(item => [item.sheet_name || item.name, item.range].filter(Boolean).join('!')).join(', ') || 'Not specified in job');
    }
    if (row.uploadedImages != null) field(fields, 'Images received / jobs completed', `${row.uploadedImages} / ${row.completedCommands}`);
    if (row.error) field(fields, 'Reported error', message(row.error));
    const events = [];
    for (const event of row.events || []) {
      const item = element('li');
      item.append(element('time', null, timeText(event)), element('span', null, message(event)));
      if (event.details) item.append(element('small', null, [event.details.sheet_name, event.details.range, event.details.command_id || event.details.id].filter(Boolean).join(' · ')));
      events.push(item);
    }
    detailChrome.body.replaceChildren(fields);
    detailChrome.events.replaceChildren(...events);
    const timing = commandTimingSummary(row.paneResources?.last_command_timing);
    if (timing) {
      const timingFields = element('dl', 'wm-detail-fields');
      for (const [label, value] of timing.fields) field(timingFields, label, value);
      detailChrome.timing.replaceChildren(element('h4', null, 'Last measured command'), element('p', 'wm-muted', timing.command),
        timingFields, element('p', 'wm-muted', timing.note));
    } else detailChrome.timing.replaceChildren();
    detail.scrollTop = scroll;
  }

  const emptyList = element('p', 'wm-muted', 'No workers match this filter.');
  list.append(emptyList);

  function createGroupView(group, key) {
    const card = element('section', 'wm-group');
    card.dataset.groupId = group.id;
    card.dataset.attemptId = group.attempt_id || 'legacy';
    const title = element('div', 'wm-group-title');
    const workbook = element('strong');
    const phase = element('span');
    const pending = element('p', 'wm-muted', 'Process data for this attempt is still being provided.');
    title.append(workbook, phase);
    card.append(title, pending);
    list.append(card);
    const view = { element: card, workbook, phase, pending, workers: new Map() };
    groupViews.set(key, view);
    return view;
  }

  function createWorkerView(group, row) {
    // Keep interactive capture controls outside the worker selection button.
    const item = element('div', 'wm-worker');
    const select = element('button', 'wm-worker-select');
    select.dataset.workerId = row.id;
    select.type = 'button';
    const identity = element('span', 'wm-identity');
    const name = element('strong');
    const process = element('small');
    identity.append(name, process);
    const phase = element('span', 'wm-phase');
    const phaseName = element('strong');
    const phaseAge = element('small');
    phase.append(phaseName, phaseAge);
    select.append(identity, phase);
    const view = { element: item, select, name, process, phaseName, phaseAge, row };
    view.signals = createSignalMonitor({ compact: true,
      onCaptureNavigate: ({ capture, follow }) => openCaptures(view.row, capture, follow) });
    item.append(select, view.signals.element);
    select.addEventListener('click', () => { selected = view.row; render(); });
    group.element.append(item);
    group.workers.set(row.id, view);
    signalViews.set(row.id, view);
    return view;
  }

  function updateWorkerView(view, row, filter) {
    view.row = row;
    const isSelected = selected?.id === row.id;
    view.element.className = `wm-worker wm-${row.category}${isSelected ? ' wm-selected' : ''}`;
    view.element.hidden = filter !== 'all' && row.category !== filter;
    view.select.setAttribute('aria-pressed', String(isSelected));
    view.select.setAttribute('aria-label', `Select ${row.clientId}, ${observedPhase(row)}`);
    view.name.textContent = row.clientId;
    view.process.textContent = `${row.kind}${row.pid ? ` · PID ${row.pid}` : ''}`;
    view.phaseName.textContent = observedPhase(row);
    view.phaseAge.textContent = `${duration(age(row.stateSince))} since state change${row.sheet ? ` · ${row.sheet}` : ''}`;
    view.signals.update(row, { live: snapshot.running !== false && snapshot.runner_alive !== false });
  }

  function captureLayout() {
    const groups = new Map(), workers = new Map();
    const overview = query('.wm-overview');
    const viewport = overview.getBoundingClientRect();
    let anchor;
    for (const view of groupViews.values()) {
      if (view.element.hidden) continue;
      const rect = view.element.getBoundingClientRect();
      groups.set(view, rect);
      if (!anchor && rect.bottom > viewport.top && rect.top < viewport.bottom) {
        anchor = { view, top: rect.top };
      }
      for (const worker of view.workers.values()) {
        if (worker.element.hidden) continue;
        const position = worker.element.getBoundingClientRect();
        workers.set(worker, { left: position.left - rect.left, top: position.top - rect.top });
      }
    }
    // Read current animated positions before cancelling an interrupted move.
    for (const view of [...groups.keys(), ...workers.keys()]) view.animation?.cancel();
    return { groups, workers, anchor, overview };
  }

  function animateLayout(before) {
    const { anchor, overview } = before;
    if (anchor?.view.element.isConnected && !anchor.view.element.hidden) {
      overview.scrollTop += anchor.view.element.getBoundingClientRect().top - anchor.top;
    }
    if (reducedMotion.matches || typeof list.animate !== 'function') return;
    const move = (view, old, position) => {
      const dx = old ? old.left - position.left : 0;
      const dy = old ? old.top - position.top : 0;
      if (old && Math.abs(dx) < .5 && Math.abs(dy) < .5) return;
      const frames = old
        ? [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'translate(0, 0)' }]
        : [{ opacity: 0 }, { opacity: 1 }];
      view.animation = view.element.animate(frames, { duration: 220, easing: 'ease-out' });
    };
    for (const view of groupViews.values()) {
      if (view.element.hidden) continue;
      const rect = view.element.getBoundingClientRect();
      const workers = [];
      for (const worker of view.workers.values()) {
        if (worker.element.hidden) continue;
        const position = worker.element.getBoundingClientRect();
        workers.push([worker, {
          left: position.left - rect.left, top: position.top - rect.top,
        }]);
      }
      move(view, before.groups.get(view), rect);
      for (const [worker, position] of workers) move(worker, before.workers.get(worker), position);
    }
  }

  function render() {
    if (!snapshot) return;
    const before = captureLayout();
    const filter = query('.wm-filter').value;
    const allRows = snapshot.groups.flatMap(workerRows);
    const selectedNow = allRows.find(row => row.id === selected?.id);
    if (selectedNow) showDetail(selectedNow);
    else if (selected) showDetail(selected, false);
    const seenGroups = new Set();
    for (const group of snapshot.groups) {
      const key = JSON.stringify([group.id, group.attempt_id ?? null]);
      seenGroups.add(key);
      const view = groupViews.get(key) || createGroupView(group, key);
      const rows = workerRows(group);
      view.workbook.textContent = group.workbook || group.id;
      view.phase.textContent = `${group.corpus || ''} · ${phaseLabel(group.status)}`;
      view.pending.hidden = rows.length > 0;
      view.element.hidden = filter !== 'all' && !rows.some(row => row.category === filter);
      const seenWorkers = new Set();
      for (const row of rows) {
        seenWorkers.add(row.id);
        const worker = view.workers.get(row.id) || createWorkerView(view, row);
        updateWorkerView(worker, row, filter);
      }
      for (const [id, worker] of view.workers) {
        if (seenWorkers.has(id)) continue;
        worker.animation?.cancel();
        worker.signals.destroy();
        worker.element.remove();
        view.workers.delete(id);
        signalViews.delete(id);
      }
    }
    for (const [key, view] of groupViews) {
      if (seenGroups.has(key)) continue;
      for (const [id, worker] of view.workers) {
        worker.animation?.cancel();
        worker.signals.destroy();
        signalViews.delete(id);
      }
      view.animation?.cancel();
      view.element.remove();
      groupViews.delete(key);
    }
    // Existing cards retain their admission order. Heartbeats and phase changes
    // never move the canvas to another DOM position.
    emptyList.hidden = [...groupViews.values()].some(view => !view.element.hidden);
    animateLayout(before);
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
      if (!response.ok) throw new Error(`Worker API: HTTP ${response.status}`);
      const data = await response.json();
      if (token !== generation || !dialog.open) return;
      snapshot = data;
      if (offset > 0 && offset >= data.total_groups) { offset = 0; return refresh(); }
      render();
    } catch (error) {
      if (error.name !== 'AbortError') {
        status.textContent = `Could not refresh worker status: ${error.message}. Displayed data is the last received snapshot.`;
        for (const item of signalViews.values()) item.signals.update(item.row, { connected: false });
        if (selected) detailSignal?.update(selected, { connected: false });
      }
    } finally {
      if (token === generation && dialog.open) timer = setTimeout(refresh, 2000);
    }
  }

  button.addEventListener('click', () => { dialog.showModal(); status.textContent = 'Loading workers …'; refresh(); });
  query('.wm-close').addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => {
    clearTimeout(timer); request?.abort(); generation++;
    for (const item of signalViews.values()) {
      item.animation?.cancel();
      item.signals.destroy();
    }
    signalViews.clear();
    for (const view of groupViews.values()) { view.animation?.cancel(); view.element.remove(); }
    groupViews.clear();
    detailSignal?.destroy(); detailSignal = null; detailChrome = null;
    detail.replaceChildren(element('p', null, 'Select a worker to see its last successful step and events.'));
    button.focus();
  });
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
    const activity = element('div', 'wm-run-activity');
    if (summary.activity) {
      const current = summary.activity;
      const attempts = element('dl', 'wm-progress-numbers');
      field(attempts, current.observed ? 'Observed outcomes since restart' : 'Finished attempts this run', current.finished);
      field(attempts, 'Compared this run', current.compared);
      field(attempts, 'Skipped this run', current.skipped);
      field(attempts, 'Interrupted this run', current.interrupted);
      field(attempts, 'Retries finished this run', current.retried);
      field(attempts, 'Attempts / h', current.perHour);
      activity.append(attempts, element('p', 'wm-progress-note',
        `${current.observed ? 'Observed current-run outcomes; older runner snapshots cannot reconstruct repeated attempts or their rate. ' : ''}Attempts include unsuccessful retries. Retrying an already skipped file can leave corpus totals unchanged; attempts are not successful validations.`));
      for (const outcome of current.outcomes) {
        const result = element('p', 'wm-progress-note');
        result.append(element('strong', null, `${outcome.label}${outcome.retried ? ' (retry)' : ''}`),
          document.createTextNode(` · ${timeText(outcome.at)} · ${outcome.workbook}${outcome.reason ? ` — ${outcome.reason}` : ''}`));
        activity.append(result);
      }
    }
    const footer = element('p', 'wm-progress-note', `${summary.eta}${summary.finish ? ` · around ${timeText(summary.finish)}` : ''}. Rate of successfully compared workbooks in the active window of ${summary.window}. Run average: ${summary.averagePerHour}/h. Remaining preparation: ${summary.preparationRemaining} (${summary.preparationPerHour}/h). Fully idle pauses (target 0) are excluded; capture pauses with active groups still count.`);
    if (Number(queue.progress.diagnosis_failed) > 0) footer.append(document.createTextNode(` Unresolved diagnostic failures: ${summary.diagnosisFailed}.`));
    if (queue.runner_alive === false || queue.running === false) footer.append(document.createTextNode(' Runner inactive: last saved snapshot.'));
    progress.replaceChildren(heading, numbers, activity, footer);
  } };
}
