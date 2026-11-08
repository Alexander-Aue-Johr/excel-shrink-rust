/* Standalone configuration and bounded progress. Scientific views are shared static pages. */
'use strict';

import { createTargetedPanel } from '/observatory-assets/targeted-panel.js';
import { optionValue, setOption } from '/observatory-assets/processor-options.js';

const byId = id => document.getElementById(id);
const node = (tag, text, className) => {
  const element = document.createElement(tag);
  if (text != null) element.textContent = String(text);
  if (className) element.className = className;
  return element;
};

let token = '';
let configuration = null;
let stageModels = [];
let workingDirectory = '';
let browseDirectoryDefault = '';
let browserTarget = null;
let browserState = null;
let pollTimer = null;
let polling = false;
let resultsSignature = '';
let lastError = '';
let isActive = false;
let hasReport = false;
let reportBase = '';
let nativePicker = false;
let processorSchema = null;
let shrinkPreset = null;
let logicalProcessors = 1;
let workbookOffset = 0;
let workbookRevision = '';
let workbookRequest = 0;
let workbookSearchTimer;
const targetedPanel = createTargetedPanel(byId('targeted-comparisons'));

function message(text, error = false) {
  byId('message').textContent = text || '';
  byId('message').classList.toggle('error', error);
  byId('configuration-message').textContent = text || '';
}

async function api(path, body) {
  const response = await fetch(path, body === undefined ? {} : {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Sheet-Scope-Token': token },
    body: JSON.stringify(body),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `Request failed (${response.status})`);
  return result;
}

function mode() {
  return document.querySelector('input[name=mode]:checked').value;
}

function showSections() {
  const selected = mode();
  byId('configuration').hidden = selected === 'report' || selected === 'study';
  byId('saved-report').hidden = selected !== 'report';
  byId('saved-study').hidden = selected !== 'study';
  byId('source-b-row').hidden = selected !== 'compare';
  byId('processing').hidden = selected !== 'process';
  byId('output-settings').hidden = selected !== 'process';
  byId('output-path-row').hidden = byId('output-kind').value === 'temporary';
  byId('stage-cache-row').hidden = !byId('persist-stages').checked;
  byId('processing-key-row').hidden = !byId('persist-stages').checked;
  byId('reuse-path-row').hidden = !byId('reuse').checked;
  if (byId('reuse').checked) byId('reuse-details').open = true;
}

function populate(config) {
  configuration = structuredClone(config);
  const set = (id, value) => { byId(id).value = value ?? ''; };
  const check = (id, value) => { byId(id).checked = Boolean(value); };
  set('source-a', config.source_a);
  set('source-b', config.source_b);
  set('report-dir', config.report_dir);
  set('parallel', config.max_processes ?? 1);
  set('baseline-step', config.baseline_after_step);
  set('output-kind', config.output?.kind || 'temporary');
  set('output-path', config.output?.path);
  check('persist-stages', config.stage_cache_dir);
  set('stage-cache', config.stage_cache_dir);
  set('processing-key', config.processing_cache_key);
  check('continue-errors', config.continue_on_error);
  set('environment-key', config.environment_key);
  check('reuse', config.reuse_from);
  set('reuse-path', config.reuse_from);
  for (const input of document.querySelectorAll('[data-part]')) {
    input.checked = config.reuse_parts?.[input.dataset.part] ?? true;
  }
  set('extra-parts', (config.reuse_parts?.extra_parts || []).join('\n'));
  set('download-cache', config.cache_dir || '.tmp/sheet-scope/downloads');
  check('refresh-downloads', config.refresh_downloads);
  set('operation-timeout', config.capture?.operation_timeout_seconds ?? 120);
  set('idle-timeout', config.capture?.idle_timeout_seconds ?? 120);
  check('serial-captures', config.capture?.serial_captures);
  check('retain-equal-images', config.capture?.retain_equal_images);
  document.querySelector(`input[name=mode][value=${config.steps?.length ? 'process' : 'compare'}]`).checked = true;
  stageModels = structuredClone(config.steps || []);
  renderStages();
  showSections();
}

function readConfiguration() {
  const config = structuredClone(configuration);
  const value = id => byId(id).value.trim();
  config.source_a = value('source-a');
  config.source_b = mode() === 'compare' ? value('source-b') : null;
  config.steps = mode() === 'process' ? readStages() : [];
  config.baseline_after_step = mode() === 'process' && value('baseline-step') ? Number(value('baseline-step')) : null;
  config.report_dir = value('report-dir');
  config.max_processes = Number(value('parallel'));
  config.output = mode() === 'process' && byId('output-kind').value !== 'temporary'
    ? { kind: byId('output-kind').value, path: value('output-path') }
    : { kind: 'temporary' };
  config.stage_cache_dir = mode() === 'process' && byId('persist-stages').checked ? value('stage-cache') : null;
  config.processing_cache_key = value('processing-key');
  config.continue_on_error = byId('continue-errors').checked;
  config.environment_key = value('environment-key');
  config.reuse_from = byId('reuse').checked ? value('reuse-path') : null;
  config.reuse_parts = {};
  for (const input of document.querySelectorAll('[data-part]')) config.reuse_parts[input.dataset.part] = input.checked;
  config.reuse_parts.extra_parts = value('extra-parts').split(/\r?\n/).map(line => line.trim()).filter(Boolean);
  config.cache_dir = value('download-cache');
  config.refresh_downloads = byId('refresh-downloads').checked;
  config.capture = {
    operation_timeout_seconds: Number(value('operation-timeout')),
    idle_timeout_seconds: Number(value('idle-timeout')),
    serial_captures: byId('serial-captures').checked,
    retain_equal_images: byId('retain-equal-images').checked,
  };
  return config;
}

function field(title, control) {
  const label = node('label', title);
  label.append(control);
  return label;
}

function stageInput(card, key, value, type = 'text') {
  const input = node('input');
  input.type = type;
  input.dataset.key = key;
  input.value = value ?? '';
  if (type === 'number') input.min = '1';
  return input;
}

function stageSelect(key, value, values) {
  const select = node('select');
  select.dataset.key = key;
  for (const [id, text] of values) {
    const option = node('option', text);
    option.value = id;
    select.append(option);
  }
  select.value = value;
  return select;
}

function renderStages() {
  const root = byId('steps');
  root.replaceChildren();
  stageModels.forEach((stage, index) => {
    const shrink = processorSchema && (stage.name === 'Excel Shrink'
      || /(?:^|[\\/])excel-shrink-cli(?:\.exe)?$/i.test(stage.program));
    const card = node('section', null, 'step');
    card.dataset.index = String(index);
    const heading = node('div', null, 'section-title');
    heading.append(node('h3', `Step ${index + 1}`));
    const remove = node('button', 'Remove');
    remove.type = 'button';
    remove.addEventListener('click', () => {
      try { stageModels = readStages(); stageModels.splice(index, 1); renderStages(); }
      catch (error) { message(error.message, true); }
    });
    heading.append(remove);
    const grid = node('div', null, 'grid');
    grid.append(field('Step name', stageInput(card, 'name', stage.name)));
    const program = stageInput(card, 'program', stage.program);
    const browse = node('button', 'Browse…');
    browse.type = 'button';
    browse.addEventListener('click', () => openBrowser(program, 'program'));
    const programPath = node('span', null, 'path');
    programPath.append(program, browse);
    grid.append(field('Executable', programPath));
    grid.append(field('Input per invocation', stageSelect('input', stage.input || 'file', [
      ['file', 'One workbook'], ['directory', 'Complete folder'], ['archive', 'Complete ZIP archive'],
    ])));
    grid.append(field('Program output', stageSelect('output', stage.output || 'directory', [
      ['file', 'One workbook file'], ['directory', 'Folder of workbooks'], ['archive', 'ZIP archive'],
    ])));
    const argumentsInput = node('textarea');
    argumentsInput.dataset.key = 'arguments';
    argumentsInput.rows = 4;
    argumentsInput.spellcheck = false;
    argumentsInput.value = JSON.stringify(stage.arguments || ['{input}', '{output}'], null, 2);
    const argumentsLabel = field('Arguments as a JSON array', argumentsInput);
    argumentsLabel.className = 'wide';
    if (shrink) {
      const advanced = node('details', null, 'wide');
      advanced.append(node('summary', 'Advanced invocation arguments'), argumentsLabel);
      grid.append(advanced);
    } else grid.append(argumentsLabel);
    grid.append(field('Output extension (optional)', stageInput(card, 'output_extension', stage.output_extension)));
    grid.append(field('Timeout per invocation (seconds)', stageInput(card, 'timeout_seconds', stage.timeout_seconds ?? 3600, 'number')));
    card.append(heading, grid);
    if (shrink) {
      renderProcessorOptions(card, argumentsInput);
    }
    card.append(node('p', 'Available placeholders: {input}, {output}, {output_dir}, {output_file}. Paths with spaces remain one argument. Additional environment and working-directory settings are available in the complete JSON.', 'help'));
    root.append(card);
  });
}

function renderProcessorOptions(card, argumentsInput) {
  const details = node('details', null, 'processor-options');
  details.append(node('summary', 'Excel Shrink parameters'));
  const grid = node('div', null, 'grid');
  const controls = [];
  for (const option of processorSchema.options) {
    let control;
    if (option.kind === 'enum') {
      control = stageSelect(option.name, '', [['', 'CLI default'], ...option.choices.map(value => [value, value])]);
    } else if (option.kind === 'list') {
      control = node('textarea');
      control.rows = 2;
      control.placeholder = 'One value per line';
    } else {
      control = node('input');
      control.type = option.kind === 'boolean' ? 'checkbox' : option.kind === 'integer' ? 'number' : 'text';
      if (option.kind === 'integer') control.step = '1';
      if (['rayon-threads', 'writer-workers', 'max-active-workbooks'].includes(option.name)) control.min = '1';
      if (option.name === 'rayon-threads') control.max = String(logicalProcessors);
      if (option.defaults.length && option.kind !== 'boolean') control.placeholder = `CLI default: ${option.defaults.join(', ')}`;
    }
    const label = field(`--${option.name}`, control);
    if (option.kind === 'boolean') label.className = 'check';
    label.append(node('span', option.help, 'processor-help'));
    control.addEventListener('change', () => {
      try {
        const args = JSON.parse(argumentsInput.value);
        argumentsInput.value = JSON.stringify(setOption(args, option,
          option.kind === 'boolean' ? control.checked : control.value), null, 2);
      } catch (error) { message(`Cannot update arguments: ${error.message}`, true); }
    });
    controls.push({ option, control });
    // Thread count is visible without opening the complete option catalogue.
    if (option.name === 'rayon-threads') card.append(label);
    else grid.append(label);
  }
  function sync() {
    try {
      const args = JSON.parse(argumentsInput.value);
      for (const { option, control } of controls) {
        const value = optionValue(args, option);
        if (option.kind === 'boolean') control.checked = value;
        else control.value = value;
      }
    } catch { /* The JSON editor shows its parse error when the run is submitted. */ }
  }
  argumentsInput.addEventListener('input', sync);
  sync();
  details.append(grid);
  card.append(details);
}

function readStages() {
  return [...byId('steps').children].map((card, index) => {
    const stage = structuredClone(stageModels[index]);
    const value = key => card.querySelector(`[data-key="${key}"]`).value;
    stage.name = value('name').trim();
    stage.program = value('program').trim();
    stage.input = value('input');
    stage.output = value('output');
    try { stage.arguments = JSON.parse(value('arguments')); }
    catch { throw new Error(`Step ${index + 1}: arguments must be a JSON array of strings.`); }
    if (!Array.isArray(stage.arguments) || stage.arguments.some(argument => typeof argument !== 'string')) {
      throw new Error(`Step ${index + 1}: every argument must be a string.`);
    }
    stage.output_extension = value('output_extension').trim() || null;
    stage.timeout_seconds = Number(value('timeout_seconds'));
    return stage;
  });
}

function addStage(shrink = false) {
  try {
    stageModels = readStages();
    stageModels.push(shrink ? structuredClone(shrinkPreset) : {
      name: `Program ${stageModels.length + 1}`,
      program: '', input: 'file', output: 'directory', output_extension: null,
      arguments: ['{input}', '{output}'],
      timeout_seconds: 3600,
    });
    renderStages();
  } catch (error) { message(error.message, true); }
}

function dateLabel() {
  return new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
}

function appendPath(parent, name) {
  const separator = parent.includes('\\') ? '\\' : '/';
  return `${parent.replace(/[\\/]$/, '')}${separator}${name}`;
}

async function openBrowser(input, purpose = input.id, kind = purpose === 'program' ? 'program' : 'folder') {
  if (nativePicker && browserTarget) return;
  browserTarget = { input, purpose };
  if (nativePicker) {
    try {
      const selected = await api('/api/pick', { kind, initial_path: input.value.trim() || browseDirectoryDefault });
      if (selected.path) choosePath(selected.path, kind === 'folder');
    } catch (error) { message(error.message, true); }
    finally { browserTarget = null; }
    return;
  }
  byId('browser').showModal();
  const path = input.value.trim();
  try { await browseDirectory(path && !/^https?:/.test(path) ? path : browseDirectoryDefault); }
  catch { await browseDirectory(browseDirectoryDefault).catch(error => { byId('browse-message').textContent = error.message; }); }
}

async function browseDirectory(path, offset = 0) {
  const result = await api(`/api/browse?path=${encodeURIComponent(path)}&offset=${offset}`);
  browserState = result;
  byId('browse-path').value = result.path;
  byId('browse-message').textContent = `${result.total} entries · ${offset + 1}–${Math.min(offset + 200, result.total)}`;
  byId('browse-prev').disabled = offset === 0;
  byId('browse-next').disabled = offset + 200 >= result.total;
  byId('browse-up').disabled = !result.parent;
  const entries = byId('browse-entries');
  entries.replaceChildren();
  for (const entry of result.entries) {
    const button = node('button', `${entry.directory ? 'Folder · ' : ''}${entry.name}`);
    button.type = 'button';
    button.addEventListener('click', () => {
      if (entry.directory) browseDirectory(entry.path).catch(error => { byId('browse-message').textContent = error.message; });
      else choosePath(entry.path, false);
    });
    entries.append(button);
  }
}

function choosePath(path, directory) {
  if (!browserTarget) return;
  const { input, purpose } = browserTarget;
  if (directory && ['report-dir', 'export-path'].includes(purpose)) path = appendPath(path, `sheet-scope-${dateLabel()}`);
  if (directory && purpose === 'output-path') {
    path = appendPath(path, byId('output-kind').value === 'archive' ? `processed-${dateLabel()}.zip` : `processed-${dateLabel()}`);
  }
  input.value = path;
  byId('browser').close();
  browserTarget = null;
}

function renderProgress(state) {
  const progress = state.progress;
  isActive = state.active;
  if (state.report_base && reportBase !== state.report_base) {
    reportBase = state.report_base;
    targetedPanel.reset();
    workbookOffset = 0;
    byId('evidence-panel').hidden = true;
    byId('evidence-viewer').src = 'about:blank';
  }
  byId('report-link').href = `${reportBase}/index.html`;
  byId('status').textContent = progress.state.replaceAll('_', ' ');
  byId('current').textContent = progress.current || 'No active run.';
  const processing = progress.state === 'processing' ? progress.processing : null;
  byId('completion').max = Math.max(processing?.total ?? progress.total, 1);
  byId('completion').value = processing?.completed ?? progress.completed;
  byId('counts').textContent = processing
    ? `${processing.completed} / ${processing.total} processing invocations finished · visual comparison has not started`
    : `${progress.completed} / ${progress.total} visual comparisons · ${progress.reused} reused · ${progress.failed} failed or missing`;
  renderProcessing(progress.processing);
  byId('run-fields').disabled = state.active;
  byId('start').disabled = state.active;
  byId('open-report').disabled = state.active;
  byId('edit-json').disabled = state.active;
  byId('configure').disabled = state.active;
  document.querySelectorAll('input[name=mode]').forEach(input => { input.disabled = state.active; });
  hasReport = Boolean(progress.report_dir);
  byId('report-link').hidden = !hasReport || state.active;
  byId('export-settings').hidden = !hasReport || state.active;

  byId('connection-status').textContent = state.active ? 'Live · local runner' : 'Connected · ready';
  targetedPanel.updateMany([...(state.comparisons || [])].reverse().concat(state.telemetry || []));
  renderKpis(progress);
  const revision = JSON.stringify([progress.state, progress.total, progress.completed, progress.current,
    progress.processing?.completed, Object.keys(progress.processing?.running || {}), reportBase]);
  if (revision !== workbookRevision) {
    workbookRevision = revision;
    void loadWorkbooks();
  }
  const signature = reportBase + JSON.stringify(progress.recent);
  if (signature !== resultsSignature) {
    resultsSignature = signature;
    const results = byId('results');
    results.replaceChildren();
    for (const record of progress.recent || []) {
      const row = node('tr');
      row.append(node('td', record.name), node('td', record.status), node('td', record.different_regions ?? '—'));
      const evidence = node('td');
      if (record.report) {
        const link = node('a', 'View comparison');
        link.href = `${reportBase}/${record.report.replace(/report\.json$/, 'index.html').split('/').map(encodeURIComponent).join('/')}`;
        link.target = '_blank';
        link.rel = 'noopener';
        link.addEventListener('click', event => { event.preventDefault(); openEvidence(link.href, record.name); });
        evidence.append(link);
      } else evidence.textContent = (record.errors || []).join(' · ');
      row.append(evidence);
      results.append(row);
    }
  }
  if (progress.error && progress.error !== lastError) {
    lastError = progress.error;
    message(progress.error, true);
  }
}

function renderProcessing(processing) {
  byId('processing-progress').hidden = !processing;
  if (!processing) return;
  const running = Object.values(processing.running);
  const waiting = Math.max(0, processing.total - processing.completed - running.length);
  byId('processing-title').textContent = `Step ${processing.step}: ${processing.stage}`;
  byId('processing-counts').textContent = `${processing.succeeded} / ${processing.total} succeeded · ${running.length} running · ${waiting} waiting · ${processing.failed} failed · ${processing.skipped} skipped`;
  byId('processing-rows').replaceChildren(...[...running, ...processing.recent].map(process => {
    const row = node('tr');
    const name = process.input_count > 1 ? `${process.input_count} workbooks` : process.name;
    const milliseconds = process.status === 'running' && process.started_at
      ? Date.now() - Date.parse(process.started_at) : process.elapsed_ms;
    const seconds = Math.max(0, Math.floor(milliseconds / 1000));
    row.append(node('td', name), node('td', process.status.replaceAll('_', ' ')), node('td', process.pid || '—'),
      node('td', `${Math.floor(seconds / 60)}m ${seconds % 60}s`));
    if (process.error) row.title = process.error;
    const logs = node('td');
    for (const stream of ['stdout', 'stderr']) {
      const link = node('a', stream);
      link.href = `${reportBase}/logs/stage-${processing.step}/invocations/${String(process.invocation).padStart(6, '0')}/${stream}.log`;
      link.target = '_blank';
      link.rel = 'noopener';
      logs.append(link, document.createTextNode(' '));
    }
    row.append(logs);
    return row;
  }));
}

function renderKpis(progress) {
  const pending = Math.max(0, progress.total - progress.completed);
  const values = [
    ['all', 'All selected workbooks', progress.total],
    ['pending', 'Pending / running', pending],
    ['equal', 'Checked: no differences', progress.checked_equal_pairs || 0],
    ['differences', 'Differences found', progress.different_pairs || 0],
    ['failed', 'Failed / missing', progress.failed],
    ['reused', 'Reused observations', progress.reused],
  ];
  byId('workbook-kpis').replaceChildren(...values.map(([filter, title, count]) => {
    const button = node('button', null, 'workbook-kpi');
    button.type = 'button';
    button.append(node('strong', `${count} / ${progress.total}`), node('span', title));
    button.addEventListener('click', () => {
      byId('workbook-filter').value = filter;
      workbookOffset = 0;
      void loadWorkbooks();
    });
    return button;
  }));
  byId('workbook-scope').textContent = progress.total
    ? `${progress.completed} / ${progress.total} comparisons completed`
    : 'Waiting for source acquisition and processing';
}

async function loadWorkbooks() {
  const request = ++workbookRequest;
  const query = new URLSearchParams({ offset: String(workbookOffset), search: byId('workbook-search').value,
    filter: byId('workbook-filter').value });
  try {
    const page = await api(`/api/workbooks?${query}`);
    if (request !== workbookRequest) return;
    byId('workbook-count').textContent = page.total
      ? `${page.offset + 1}–${page.offset + page.rows.length} / ${page.total} workbooks` : '0 workbooks';
    byId('workbook-prev').disabled = page.offset === 0;
    byId('workbook-next').disabled = page.offset + page.rows.length >= page.total;
    byId('workbook-list').replaceChildren(...page.rows.map(record => {
      const preparation = record.processing_status ? `Processing: ${record.processing_status.replaceAll('_', ' ')} · ` : '';
      const button = node('button', `${record.name} · ${preparation}${record.status.replaceAll('_', ' ')}`);
      button.type = 'button';
      button.disabled = !record.report;
      button.addEventListener('click', () => openEvidence(
        `${reportBase}/${record.report.replace(/report\.json$/, 'index.html').split('/').map(encodeURIComponent).join('/')}`, record.name));
      return button;
    }));
  } catch (error) { message(error.message, true); }
}

function openEvidence(url, name) {
  byId('evidence-title').textContent = name;
  byId('evidence-viewer').src = url;
  byId('evidence-panel').hidden = false;
  byId('evidence-panel').scrollIntoView({ behavior: 'smooth', block: 'start' });
}

async function poll() {
  clearTimeout(pollTimer);
  if (polling) return;
  polling = true;
  try { renderProgress(await api('/api/state')); }
  catch (error) { message(error.message, true); }
  finally {
    polling = false;
    pollTimer = setTimeout(poll, document.hidden ? 10000 : isActive ? 1000 : 5000);
  }
}

document.querySelectorAll('input[name=mode], #output-kind, #persist-stages, #reuse').forEach(input => input.addEventListener('change', showSections));
document.querySelectorAll('[data-browse]').forEach(button => button.addEventListener('click', () => openBrowser(byId(button.dataset.browse), button.dataset.browse, button.dataset.kind || 'folder')));
byId('configure').addEventListener('click', () => byId('configuration-panel').showModal());
byId('close-configuration').addEventListener('click', () => byId('configuration-panel').close());
byId('close-evidence').addEventListener('click', () => {
  byId('evidence-panel').hidden = true;
  byId('evidence-viewer').src = 'about:blank';
});
byId('workbook-filter').addEventListener('change', () => { workbookOffset = 0; void loadWorkbooks(); });
byId('workbook-search').addEventListener('input', () => {
  clearTimeout(workbookSearchTimer);
  workbookSearchTimer = setTimeout(() => { workbookOffset = 0; void loadWorkbooks(); }, 200);
});
byId('workbook-prev').addEventListener('click', () => { workbookOffset = Math.max(0, workbookOffset - 100); void loadWorkbooks(); });
byId('workbook-next').addEventListener('click', () => { workbookOffset += 100; void loadWorkbooks(); });
byId('add-step').addEventListener('click', () => addStage());
byId('add-shrink').addEventListener('click', () => addStage(true));
byId('close-browser').addEventListener('click', () => byId('browser').close());
byId('use-folder').addEventListener('click', () => browserState && choosePath(browserState.path, true));
byId('browse-up').addEventListener('click', () => browseDirectory(browserState.parent).catch(error => { byId('browse-message').textContent = error.message; }));
byId('browse-prev').addEventListener('click', () => browseDirectory(browserState.path, Math.max(0, browserState.offset - 200)).catch(error => { byId('browse-message').textContent = error.message; }));
byId('browse-next').addEventListener('click', () => browseDirectory(browserState.path, browserState.offset + 200).catch(error => { byId('browse-message').textContent = error.message; }));
byId('browse-form').addEventListener('submit', event => {
  event.preventDefault();
  browseDirectory(byId('browse-path').value).catch(error => { byId('browse-message').textContent = error.message; });
});
byId('configuration').addEventListener('submit', async event => {
  event.preventDefault();
  try {
    byId('start').disabled = true;
    await api('/api/run', readConfiguration());
    byId('configuration-panel').close();
    hasReport = false;
    lastError = '';
    message('Validation started. Completed pairs become available below.');
    await poll();
  } catch (error) { byId('start').disabled = false; message(error.message, true); }
});
byId('resume-study').addEventListener('click', async () => {
  const button = byId('resume-study');
  button.disabled = true;
  try {
    const result = await api('/api/study/resume', {
      receipt: byId('study-receipt').value.trim(),
    });
    byId('study-link').href = result.url;
    byId('study-link').hidden = false;
    message(`Study runner started with PID ${result.pid}. Open its Observatory after startup.`);
  } catch (error) {
    message(error.message, true);
  } finally {
    button.disabled = false;
  }
});
byId('open-report').addEventListener('click', async () => {
  try {
    const opened = await api('/api/open', { path: byId('open-path').value.trim() });
    reportBase = opened.report_base;
    byId('report-link').href = opened.url;
    hasReport = true;
    byId('report-link').hidden = false;
    byId('export-settings').hidden = false;
    byId('report-link').click();
    message('Saved evidence is ready. The report opens independently of Excel.');
  } catch (error) { message(error.message, true); }
});
byId('export-report').addEventListener('click', async () => {
  try {
    const result = await api('/api/export', { output: byId('export-path').value.trim() });
    message(`Static website exported: ${result.index}`);
  } catch (error) { message(error.message, true); }
});
byId('save-config').addEventListener('click', () => {
  try {
    const blob = new Blob([JSON.stringify(readConfiguration(), null, 2) + '\n'], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const link = node('a');
    link.href = url;
    link.download = 'sheet-scope.json';
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch (error) { message(error.message, true); }
});
byId('edit-json').addEventListener('click', () => {
  try { byId('json-value').value = JSON.stringify(readConfiguration(), null, 2); byId('json-editor').showModal(); }
  catch (error) { message(error.message, true); }
});
byId('apply-json').addEventListener('click', () => {
  try { populate(JSON.parse(byId('json-value').value)); byId('json-editor').close(); }
  catch (error) { message(error.message, true); }
});
byId('close-json').addEventListener('click', () => byId('json-editor').close());
document.addEventListener('visibilitychange', () => { if (!document.hidden) poll(); });

(async function initialise() {
  try {
    const session = await api('/api/session');
    token = session.token;
    nativePicker = session.native_picker;
    processorSchema = session.processor_schema;
    shrinkPreset = session.shrink_preset;
    logicalProcessors = session.logical_processors;
    workingDirectory = session.working_directory;
    browseDirectoryDefault = session.browse_directory || workingDirectory;
    populate(session.config);
    byId('parallel-help').textContent = `${logicalProcessors} logical processors. This limits external programs; --rayon-threads controls threads inside each Excel Shrink process.`;
    await poll();
    byId('configuration-panel').showModal();
  } catch (error) { message(error.message, true); }
})();
