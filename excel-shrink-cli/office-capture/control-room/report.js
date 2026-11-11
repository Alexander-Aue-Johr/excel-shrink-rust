import { atelierRegions, shouldInspectFormulas } from './atelier-model.js';
import { createPixelInspector } from './atelier-pixels.js';
import { createRetainedFormulaInspector, retainedFormulaDiagnosis } from './formula-inspector.js';
import { installReportReplay } from './report-replay.js';

/** Only immutable package images are addressable, on file:// or a static host. */
export function portableImageUrl(path) {
  return typeof path === 'string' && /^evidence\/[a-f0-9]{64}\.(png|jpg)$/.test(path)
    ? path : null;
}

/** Keep navigation below the exported report root without interpreting a URL. */
export function portablePageUrl(path) {
  if (typeof path !== 'string' || !path.endsWith('/index.html')
      || /[\\:\u0000-\u001f?#]/.test(path)) return null;
  const parts = path.split('/');
  if (parts.some(part => !part || part === '.' || part === '..')) return null;
  return parts.map(encodeURIComponent).join('/');
}

/** A reused observation keeps its previous scientific scope; it is not a new pass. */
export function reportRowMatches(row, query, filter) {
  if (!String(row.name || row.id || '').toLocaleLowerCase().includes(query.toLocaleLowerCase())) return false;
  const status = String(row.status || 'unknown');
  if (filter === 'differences') return Number(row.different_regions || 0) > 0;
  if (filter === 'reused') return Boolean(row.reuse);
  if (filter === 'unavailable') return !row.report || /fail|error|incomplete|interrupted/.test(status);
  return true;
}

const reportElement = (tag, className = '', text = null) => {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text != null) element.textContent = text;
  return element;
};

function reportLink(href, label) {
  const link = reportElement('a', '', label);
  link.href = href;
  return link;
}

function reportDetails(label, content) {
  const details = reportElement('details', 'report-details');
  details.append(reportElement('summary', '', label));
  if (content instanceof Element) details.append(content);
  else details.append(reportElement('pre', '', JSON.stringify(content, null, 2)));
  return details;
}

/** Public pages share their adjacent JSON; native portable pages carry it inline. */
export async function loadComparisonReport(data, fetchReport = fetch) {
  if (data.report) return data.report;
  if (data.report_url !== 'report.json') throw new Error('Unsupported comparison report location');
  const response = await fetchReport('report.json', { credentials: 'omit', redirect: 'error' });
  if (!response.ok) throw new Error(`Report request failed (${response.status})`);
  return response.json();
}

async function startReportPage() {
  const content = document.getElementById('report-content');
  const links = document.getElementById('report-links');
  try {
    const data = JSON.parse(document.getElementById('report-data').textContent);
    document.getElementById('report-title').textContent = data.title || 'DeltaSieve report';
    content.replaceChildren();
    if (data.kind === 'run') {
      links.append(reportLink('run.json', 'Run metadata'));
      showRunReport(content, data);
    } else if (data.kind === 'comparison') {
      // The back path is generated from the manifest depth, never from workbook text.
      if (typeof data.back === 'string' && /^(\.\.\/)+index\.html$/.test(data.back)) {
        links.append(reportLink(data.back, 'All workbooks'));
      }
      links.append(reportLink('report.json', 'Scientific report'));
      if (data.report) links.append(reportLink('bundle.json', 'Integrity manifest'));
      content.append(reportElement('p', 'report-note', 'Loading retained comparison…'));
      const report = await loadComparisonReport(data);
      content.replaceChildren();
      showComparisonReport(content, report);
    } else {
      throw new Error('Unknown report page type');
    }
  } catch (error) {
    content.replaceChildren(reportElement('p', 'report-warning', `Cannot display retained report: ${error.message}`));
  }
}

function showRunReport(content, data) {
  const rows = Array.isArray(data.pairs) ? data.pairs : [];
  if (data.merge) {
    content.append(reportElement('p', 'report-warning',
      `Merged collection · ${data.merge.distinct_run_snapshots} source snapshots · ${rows.length} retained observations · `
      + `${data.merge.potential_conflicts} potential outcome conflicts. Counts do not represent unique corpus coverage.`));
    content.append(reportDetails('Merge provenance and overlap checks', data.merge));
    const sources = reportElement('nav', 'report-controls');
    for (const source of data.merge.sources || []) {
      if (!/^[a-f0-9]{64}$/.test(source.snapshot_id || '')) continue;
      const page = portablePageUrl(`runs/${source.snapshot_id}/index.html`);
      sources.append(reportLink(page, `Source snapshot ${source.snapshot_id.slice(0, 8)}`));
    }
    content.append(sources);
  }
  if (data.publication_review) {
    const review = data.publication_review;
    content.append(reportElement('p', 'report-warning',
      `Publication review required · ${review.metadata_finding_count ?? review.metadata_findings.length} metadata concerns recorded. `
      + 'Review workbook contents, screenshots and distribution rights before sharing this export.'));
    content.append(reportLink('publication-review.json', 'Publication review inventory'));
  }
  content.append(reportElement('p', 'report-summary',
    `${rows.length.toLocaleString('en-US')} workbook pairs · Run status: ${data.status || 'unknown'}`));
  if (data.created_at || data.finished_at) {
    content.append(reportElement('p', 'report-note',
      `Started: ${data.created_at || 'not recorded'} · Finished: ${data.finished_at || 'not recorded'}`));
  }
  content.append(reportElement('p', 'report-note',
    'This view contains retained observations. Reused results follow the selected OOXML reuse policy. '
    + 'Missing outputs, failures, omitted states and unchecked regions do not establish visual equality.'));
  if (data.interrupted_index) {
    content.append(reportElement('p', 'report-warning',
      'This snapshot was recovered from an unfinished run journal. The displayed run status is its last recorded state; no validation is running in this static view.'));
  }
  if (data.error) content.append(reportElement('p', 'report-warning', `Recorded run failure: ${data.error}`));
  if (data.summary) content.append(reportDetails('Recorded run summary', data.summary));
  if (data.selection) content.append(reportDetails('Validation partition and coverage', data.selection));
  installReportReplay(content, data.replay);
  const controls = reportElement('div', 'report-controls');
  const search = reportElement('input');
  search.type = 'search';
  search.placeholder = 'Search workbook names';
  search.setAttribute('aria-label', 'Search workbook names');
  const filter = reportElement('select');
  filter.setAttribute('aria-label', 'Filter workbook outcomes');
  for (const [value, label] of [
    ['all', 'All pairs'], ['differences', 'Recorded differences'],
    ['unavailable', 'Missing or incomplete evidence'], ['reused', 'Reused observations'],
  ]) {
    const option = reportElement('option', '', label);
    option.value = value;
    filter.append(option);
  }
  controls.append(search, filter);
  const wrapper = reportElement('div', 'report-table-wrapper');
  const table = reportElement('table', 'report-table');
  const head = reportElement('thead');
  const header = reportElement('tr');
  for (const text of ['Workbook', 'Outcome', 'Differing observations', 'Provenance']) {
    header.append(reportElement('th', '', text));
  }
  head.append(header);
  const body = reportElement('tbody');
  table.append(head, body);
  wrapper.append(table);
  const pagination = reportElement('nav', 'report-pagination');
  pagination.setAttribute('aria-label', 'Workbook pages');
  const previous = reportElement('button', '', 'Previous');
  const next = reportElement('button', '', 'Next');
  const range = reportElement('span');
  pagination.append(previous, range, next);
  content.append(controls, wrapper, pagination);
  let page = 0;
  const pageSize = 50;

  function renderRows() {
    const selected = rows.filter(row => reportRowMatches(row, search.value, filter.value));
    page = Math.min(page, Math.max(0, Math.ceil(selected.length / pageSize) - 1));
    body.replaceChildren();
    for (const row of selected.slice(page * pageSize, (page + 1) * pageSize)) {
      const entry = reportElement('tr');
      const name = reportElement('td');
      const url = portablePageUrl(row.page);
      name.append(url ? reportLink(url, row.name) : reportElement('span', '', row.name));
      const error = row.error || (Array.isArray(row.errors) && row.errors.length ? row.errors.join('\n') : null);
      if (error) {
        const text = typeof error === 'string' ? error : JSON.stringify(error);
        name.append(reportElement('small', 'report-row-error', text.length > 2000
          ? `${text.slice(0, 2000)}… See run metadata for the complete record.` : text));
      }
      entry.append(name, reportElement('td', '', row.status || 'unknown'),
        reportElement('td', '', Number.isSafeInteger(row.different_regions) ? row.different_regions.toLocaleString('en-US') : 'Not recorded'));
      const provenance = reportElement('td');
      provenance.append(reportElement('span', '', row.reuse ? 'Earlier observation reused' : row.report ? 'Current observation' : 'No retained comparison'));
      if (row.merge_source) provenance.append(reportDetails('Source run identity', row.merge_source));
      if (row.reuse) provenance.append(reportDetails('Reuse decision', row.reuse));
      entry.append(provenance);
      body.append(entry);
    }
    previous.disabled = page === 0;
    next.disabled = (page + 1) * pageSize >= selected.length;
    range.textContent = selected.length
      ? `${page * pageSize + 1}–${Math.min((page + 1) * pageSize, selected.length)} of ${selected.length.toLocaleString('en-US')}`
      : 'No matching pairs';
  }
  previous.addEventListener('click', () => { page--; renderRows(); });
  next.addEventListener('click', () => { page++; renderRows(); });
  for (const input of [search, filter]) {
    input.addEventListener(input === search ? 'input' : 'change', () => { page = 0; renderRows(); });
  }
  renderRows();
}

function showComparisonReport(content, report) {
  const allRegions = atelierRegions(report, false, portableImageUrl);
  const different = allRegions.filter(region => region.different).length;
  content.append(reportElement('p', 'report-summary',
    `Recorded status: ${report.status || 'unknown'} · ${allRegions.length} observed regions/charts · ${different} differing observations`));
  content.append(reportElement('p', 'report-note',
    'Images are retained evidence from the recorded workbook states. O / OR: original before / after rebuild. '
    + 'SD / SDR: processed workbook before / after rebuild. A marked pixel difference is an observation, not by itself a programme regression.'));
  if (report.status !== 'completed_with_limitations' || report.portable_report?.failure) {
    content.append(reportElement('p', 'report-warning',
      'This comparison did not complete successfully. Available partial captures remain inspectable; absent results are unconfirmed.'));
    if (report.portable_report?.failure) content.append(reportDetails('Recorded failure', report.portable_report.failure));
  }
  const controls = reportElement('div', 'report-controls');
  const previous = reportElement('button', '', 'Previous region');
  const next = reportElement('button', '', 'Next region');
  const select = reportElement('select', 'report-region-select');
  select.setAttribute('aria-label', 'Observed region, chart and comparison phase');
  const fitButton = reportElement('button', '', 'Show 1:1');
  const include = reportElement('input');
  include.type = 'checkbox';
  const includeLabel = reportElement('label');
  includeLabel.append(include, document.createTextNode('Include checked regions without differences'));
  controls.append(previous, next, select, fitButton, includeLabel);
  const note = reportElement('p', 'report-verdict');
  const area = reportElement('section', 'report-capture-area');
  const imageScroll = reportElement('div', 'report-image-scroll');
  const diagnostic = reportElement('div');
  const formulas = reportElement('div');
  area.append(imageScroll, diagnostic, formulas);
  content.append(controls, note, area);
  let regions = [];
  let generation = 0;
  let fit = true;

  function fitImages() {
    const images = [...imageScroll.querySelectorAll('img.atelier-capture')];
    const width = images.reduce((sum, image) => sum + image.naturalWidth, 0);
    const scale = fit && width > 0 ? Math.min(1, imageScroll.clientWidth / width) : 1;
    for (const image of images) {
      if (image.naturalWidth) image.style.width = `${image.naturalWidth * scale}px`;
    }
  }

  function renderRegion() {
    const current = ++generation;
    imageScroll.replaceChildren();
    diagnostic.replaceChildren();
    formulas.replaceChildren();
    const index = select.selectedIndex;
    const region = regions[index];
    previous.disabled = index <= 0;
    next.disabled = index < 0 || index >= regions.length - 1;
    if (!region) {
      note.textContent = allRegions.length
        ? 'No differences were recorded in the observed regions. Include checked regions to inspect their images. This is not a whole-workbook identity claim.'
        : 'No rendered comparison regions are recorded. Consult the state decisions and coverage; omitted images are not evidence of pixel equality.';
      return;
    }
    const observations = [
      region.pixels_differ && 'pixel difference', region.geometry_differs && 'geometry difference',
      region.rendering_differs && 'rendering change', region.presence_differs && 'chart presence change',
      region.blank_margin_ignored && 'blank trailing margin ignored',
      region.capture_extent_ignored && 'comparison restricted to overlapping pixels',
      region.empty_geometry_ignored && 'both regions have no visible area; measured geometry retained',
      region.pixels_compared === false && 'no pixel comparison',
    ].filter(Boolean);
    note.textContent = `${index + 1} / ${regions.length} · ${observations.join(' · ') || 'No difference recorded'}`;
    const pair = reportElement('div', 'atelier-pair');
    const views = [];
    for (const side of ['left', 'right']) {
      const figure = reportElement('figure', 'atelier-figure');
      figure.append(reportElement('figcaption', '', laneLabel(region[side])));
      const url = region[`${side}Url`];
      if (url) {
        const image = reportElement('img', 'atelier-capture');
        image.alt = `${laneLabel(region[side])} · ${region.sheet} · ${region.cell || region.chart}`;
        image.addEventListener('load', fitImages);
        image.addEventListener('error', () => {
          if (current !== generation) return;
          image.replaceWith(reportElement('p', 'report-warning', 'The retained image is unavailable. No replacement was generated.'));
        }, { once: true });
        image.src = url;
        const stage = reportElement('div', 'atelier-image-stage');
        stage.append(image);
        figure.append(stage);
        views.push({ image, stage, label: laneLabel(region[side]) });
      } else {
        figure.append(reportElement('p', '',
          `No retained image for this state (${region[`${side}_rendering`] || 'see presence and geometry evidence'}).`));
      }
      pair.append(figure);
    }
    imageScroll.append(pair);
    imageScroll.scrollTop = imageScroll.scrollLeft = 0;
    fitImages();
    const retained = report.portable_report?.diagnostics?.[region.key];
    if (shouldInspectFormulas(retained?.metrics)) {
      formulas.append(createRetainedFormulaInspector(region, retainedFormulaDiagnosis(report, region.key)));
    }
    if (views.length === 2 && retained?.metrics) {
      diagnostic.append(reportElement('p', 'report-note', 'Loading retained pixel details…'));
      Promise.all(views.map(view => view.image.decode())).then(() => {
        if (current !== generation) return;
        const metrics = { ...retained.metrics, overlay_url: portableImageUrl(retained.metrics.overlay_url) };
        diagnostic.replaceChildren(createPixelInspector(views, metrics, region));
      }).catch(error => {
        if (current === generation) diagnostic.replaceChildren(reportElement('p', 'report-warning',
          `Retained pixel details unavailable: ${error.message}`));
      });
    } else if (retained?.unavailable) {
      diagnostic.append(reportElement('p', 'report-note', retained.unavailable));
    }
  }

  function populateRegions() {
    const key = regions[select.selectedIndex]?.key;
    regions = atelierRegions(report, !include.checked, portableImageUrl);
    select.replaceChildren(...regions.map(region => reportElement('option', '', region.label)));
    const index = regions.findIndex(region => region.key === key);
    if (index >= 0) select.selectedIndex = index;
    renderRegion();
  }
  previous.addEventListener('click', () => { select.selectedIndex--; renderRegion(); });
  next.addEventListener('click', () => { select.selectedIndex++; renderRegion(); });
  select.addEventListener('change', renderRegion);
  include.addEventListener('change', populateRegions);
  fitButton.addEventListener('click', () => { fit = !fit; fitButton.textContent = fit ? 'Show 1:1' : 'Fit images'; fitImages(); });
  new ResizeObserver(fitImages).observe(imageScroll);
  populateRegions();
  appendScientificContext(content, report);
}

function laneLabel(lane) {
  return ({ O: 'Original · before rebuild', OR: 'Original · rebuilt',
    SD: 'Processed · before rebuild', SDR: 'Processed · rebuilt',
    CP: 'Check Performance · before rebuild', CPR: 'Check Performance · rebuilt' })[lane] || lane;
}

function appendScientificContext(content, report) {
  const limitations = reportElement('ul');
  for (const limitation of report.limitations || []) limitations.append(reportElement('li', '', limitation));
  content.append(reportDetails('Coverage and limitations', limitations));
  content.append(reportDetails('Recorded policies and state decisions', {
    capture_policy: report.capture_policy, execution_strategy: report.execution_strategy,
    capture_partition: report.capture_partition, input_identity: report.input_identity,
    state_decisions: report.state_decisions, content_coverage: report.content_coverage,
  }));
  for (const [field, title] of [
    ['height_checks', 'Row-height observations'], ['conditional_formatting', 'Conditional-formatting decisions'],
    ['formula_diagnostics', 'Recorded formula diagnostics'], ['transitions', 'Recorded XML transitions'],
  ]) {
    if (report[field] != null) content.append(reportDetails(title, report[field]));
  }
}

if (typeof document !== 'undefined' && document.getElementById('report-data')) startReportPage();
