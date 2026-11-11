import { AtlasGallery } from './gallery.js';
import { ComparisonView } from './comparison.js';
import { readGzip, assetUrl, loadSheet, comparisonContext, sheetSummary, sheetEvidence, catalogCollections } from './model.js';

const $ = id => document.getElementById(id);
const state = { corpus: null, selected: new Set(), book: null, sheet: null, detail: null, compare: false, token: 0 };
let catalog;
let collections;
let gallery;
let comparison;
const names = { original: 'Original' };
const themeButton = $('theme-button');
function setTheme(light) {
  document.documentElement.dataset.theme = light ? 'light' : 'dark';
  themeButton.setAttribute('aria-pressed', String(light));
  themeButton.textContent = light ? 'Dark mode' : 'Light mode';
  gallery?.setSelected(state.selected);
  if (state.sheet && state.detail) gallery.showSheet(state.book, state.sheet, state.detail);
}
setTheme(new URLSearchParams(location.search).get('theme') === 'light');
themeButton.addEventListener('click', () => setTheme(document.documentElement.dataset.theme !== 'light'));

function element(tag, className, text) {
  const result = document.createElement(tag);
  if (className) result.className = className;
  if (text !== undefined) result.textContent = text;
  return result;
}

function button(text, action, className = 'explorer-item') {
  const result = element('button', className, text); result.type = 'button';
  result.addEventListener('click', action); return result;
}

function collection() { return collections.find(entry => entry.value === state.corpus); }
function books() { return collection()?.books || []; }
function activeVariants() { return new Set(books().flatMap(book => book.observations.map(o => o.variant))); }
function selectedVariants() {
  const active = activeVariants();
  return new Set([...state.selected].filter(id => active.has(id)));
}
function coverageText(book) {
  const selected = selectedVariants();
  const observations = book.observations.filter(o => selected.has(o.variant));
  const incomplete = observations.filter(o => !o.detail || !String(o.status).startsWith('completed')).length;
  const missing = [...selected].filter(id => !observations.some(o => o.variant === id)).length;
  return `${incomplete ? ` · ${incomplete} incomplete` : ''}${missing ? ` · ${missing} not run` : ''}`;
}
function summaries(book, sheet) {
  return book.observations.filter(o => state.selected.has(o.variant)).map(o => sheetSummary(o, sheet.name)).filter(Boolean);
}
function differenceCount(book, sheet) { return summaries(book, sheet).reduce((sum, s) => sum + s.differences, 0); }
function changedSheets(book) { return book.sheets.filter(sheet => differenceCount(book, sheet) > 0).length; }

function overview() {
  state.token++; state.book = null; state.sheet = null; state.detail = null; state.compare = false;
  gallery.overview(); render();
}

function openBook(book) {
  state.token++; state.book = book; state.sheet = null; state.detail = null; state.compare = false;
  gallery.focusBook(book); render();
}

async function openSheet(book, sheet) {
  const token = ++state.token;
  state.book = book; state.sheet = sheet; state.detail = null; state.compare = false;
  gallery.focusSheet(book, sheet); render();
  $('gallery-description').textContent = 'Loading retained capture details…';
  try {
    const detail = await loadSheet(book, sheet, selectedVariants());
    if (token !== state.token) return;
    state.detail = detail;
    gallery.showSheet(book, sheet, detail); render();
  } catch (error) {
    if (token === state.token) $('gallery-description').textContent = error.message;
  }
}

async function openFinding(book, sheet, id) {
  if (state.book !== book || state.sheet !== sheet || !state.detail) {
    await openSheet(book, sheet);
  }
  if (state.book !== book || state.sheet !== sheet) return;
  const detail = state.detail;
  if (!detail) return;
  const change = detail.changes.find(c => c.id === id) || detail.changes[0];
  if (!change) { renderSidebar(); return; }
  const token = ++state.token;
  state.compare = true; render();
  $('compare-title').textContent = change.label;
  $('compare-eyebrow').textContent = `${book.corpus} / ${book.name} / ${sheet.name}`;
  $('compare-description').textContent = 'Loading native captures…';
  $('comparison-host').hidden = true;
  $('comparison-host').setAttribute('aria-busy', 'true');
  try {
    const context = await comparisonContext(detail, change, names);
    if (token !== state.token) return;
    if (!context.sheet.nativeCaptures.original || context.variants.length < 2) {
      comparison?.destroy();
      $('comparison-host').replaceChildren(element('p', 'atlas-capture-note',
        'This finding has no complete rendered image pair. Its native report records the geometry, presence or rendering observation.'));
      const link = element('a', 'atlas-detail-link', 'Open retained finding report'); link.href = change.page;
      $('comparison-host').append(link); comparison = null;
    } else {
      if (!comparison) {
        $('comparison-host').replaceChildren();
        comparison = new ComparisonView($('comparison-host'), { onBack: back });
      }
      comparison.setContext(context);
    }
    const unavailable = [...selectedVariants()].filter(id => !context.variants.includes(id)).map(id => names[id]);
    $('compare-description').textContent = `${change.range} · ${change.state} · Native capture pixels. Sheet geometry is recorded separately. `
      + (unavailable.length ? `No matching retained context for ${unavailable.join(', ')}.` : '');
  } catch (error) {
    if (token === state.token) {
      comparison?.destroy(); comparison = null;
      $('comparison-host').replaceChildren(element('p', 'atlas-capture-note', error.message));
      const link = element('a', 'atlas-detail-link', 'Open detailed source report'); link.href = change.page;
      $('comparison-host').append(link);
      $('compare-description').textContent = error.message;
    }
  } finally {
    if (token === state.token) {
      $('comparison-host').hidden = false;
      $('comparison-host').setAttribute('aria-busy', 'false');
    }
  }
}

function back() {
  if (state.compare) { state.token++; state.compare = false; render(); }
  else if (state.sheet) openBook(state.book);
  else overview();
}

function render() {
  $('viewer-layout').hidden = state.compare;
  $('compare-page').hidden = !state.compare;
  $('back-button').disabled = !state.book;
  $('breadcrumbs').replaceChildren(button(collection().label, overview, 'quiet-button'));
  if (state.book) $('breadcrumbs').append(button(state.book.name, () => openBook(state.book), 'quiet-button'));
  if (state.sheet) $('breadcrumbs').append(element('span', '', state.sheet.name));
  $('gallery-eyebrow').textContent = `${state.book?.corpus || collection().label} · RETAINED VISUAL EVIDENCE`;
  $('gallery-title').textContent = state.sheet?.name || state.book?.name || 'Workbooks';
  const active = activeVariants();
  for (const label of $('pipeline-checkboxes').children) label.hidden = !active.has(label.dataset.variant);
  $('pipeline-summary').textContent = `${selectedVariants().size} variants selected`;
  if (state.detail) {
    $('gallery-description').textContent = `${state.detail.changes.length} recorded findings · ${state.detail.captures.length} retained contexts · Map shows recorded extent`;
  } else if (state.book) {
    $('gallery-description').textContent = `${state.book.sheets.length} sheets · ${changedSheets(state.book)} with recorded changes${coverageText(state.book)}`
      + (state.book.inventory_complete ? '' : ' · Sheet inventory is unavailable');
  } else {
    $('gallery-description').textContent = `${books().length} workbooks · ${books().reduce((sum, book) => sum + book.sheets.length, 0)} sheets · ${catalog.variants.length} recorded runs`;
    if (collection().missing.length) {
      $('gallery-description').textContent += ` · ${collection().missing.length} collection members unavailable`;
    }
  }
  $('gallery-hint').textContent = state.sheet ? 'Red finding → native comparison' : state.book ? 'Sheet → details' : 'Workbook → all sheets';
  $('rail-explanation').textContent = 'Δ Difference, cause unclassified. = No difference in compared captures. ? No pixel comparison. P Capture plan. I Conditional-formatting dependency candidate. F Conservative rule capture. A rule capture can cover part of a sheet. Full-sheet fallback and causal attribution require separate evidence. Dashed borders mark incomplete runs.';
  renderSidebar();
}

function renderSidebar() {
  const list = $('explorer-list'); list.replaceChildren();
  if (state.sheet && state.detail) {
    $('rail-title').textContent = 'Recorded findings';
    $('rail-count').textContent = state.detail.changes.length;
    for (const change of state.detail.changes) {
      list.append(button(`${names[change.variant]} · ${change.label}`, () => openFinding(state.book, state.sheet, change.id)));
    }
    if (!state.detail.changes.length) list.append(element('p', 'atlas-capture-note', 'No recorded visual differences for this selection.'));
    list.append(element('p', 'atlas-capture-note', 'Uncaptured areas have no image. The map covers recorded coordinates, not a complete sheet rendering.'));
    for (const observation of state.book.observations.filter(o => state.selected.has(o.variant))) {
      if (observation.page) {
        const link = element('a', 'atlas-detail-link', `${names[observation.variant]} · detailed report`);
        link.href = assetUrl(observation.page); list.append(link);
      }
      if (!String(observation.status).startsWith('completed')) list.append(element('p', 'atlas-status', `${names[observation.variant]} · ${observation.status}`));
    }
  } else if (state.book) {
    $('rail-title').textContent = 'All sheets'; $('rail-count').textContent = state.book.sheets.length;
    for (const sheet of state.book.sheets) {
      const item = button(sheet.name, () => openSheet(state.book, sheet));
      const count = differenceCount(state.book, sheet);
      item.dataset.changed = count > 0;
      item.append(element('small', '', `${count} findings · ${sheet.state}`)); list.append(item);
      const evidence = sheetEvidence(summaries(state.book, sheet));
      item.append(element('small', '', evidence.label));
      if (evidence.indirect) item.append(element('small', '', 'I · Conditional-formatting dependency candidate'));
      if (evidence.conservative) item.append(element('small', '', 'F · Conservative rule capture'));
    }
    if (!state.book.sheets.length) list.append(element('p', 'atlas-status', 'Sheet inventory is unavailable. This view does not establish visual equality.'));
    for (const observation of state.book.observations.filter(o => !o.detail)) {
      list.append(element('p', 'atlas-status', `${names[observation.variant]} · ${observation.status} · ${observation.error || 'No comparison report retained'}`));
    }
  } else {
    $('rail-title').textContent = 'Workbooks'; $('rail-count').textContent = books().length;
    for (const book of books()) {
      const item = button('', () => openBook(book));
      item.title = book.name;
      item.append(element('strong', 'atlas-item-title', book.name));
      item.append(element('small', '', book.corpus));
      item.append(element('small', '', `${changedSheets(book)} / ${book.sheets.length} sheets with changes${coverageText(book)}${book.inventory_complete ? '' : ' · partial inventory'}`));
      list.append(item);
    }
  }
}

function corpusChanged() {
  state.corpus = $('corpus-select').value;
  state.token++; state.book = null; state.sheet = null; state.detail = null; state.compare = false;
  gallery.setBooks(books(), state.selected); render();
}

function attachControls() {
  $('corpus-select').addEventListener('change', corpusChanged);
  $('home-button').addEventListener('click', overview);
  $('back-button').addEventListener('click', back);
  $('back-to-sheet').addEventListener('click', back);
  $('reset-view').addEventListener('click', () => state.sheet ? gallery.focusSheet(state.book, state.sheet) : state.book ? gallery.focusBook(state.book) : gallery.overview());
  $('css-button').addEventListener('click', () => gallery.setMode('css'));
  $('webgl-button').addEventListener('click', () => gallery.setMode('webgl'));
  $('motion-button').addEventListener('click', () => {
    gallery.motion = !gallery.motion;
    $('motion-button').textContent = gallery.motion ? 'Animation on' : 'Animation off';
    $('motion-button').setAttribute('aria-pressed', String(gallery.motion));
    comparison?.setMotion(gallery.motion);
  });
  $('pane-toggle').addEventListener('click', () => $('viewer-layout').classList.toggle('pane-collapsed'));
  $('help-button').addEventListener('click', () => $('help-dialog').showModal());
  $('rail-items').addEventListener('click', () => state.book && !state.sheet ? renderSidebar() : state.book ? openBook(state.book) : overview());
  $('rail-findings').addEventListener('click', renderSidebar);
  document.querySelectorAll('[data-fullscreen]').forEach(control => control.addEventListener('click', async () => {
    if (document.fullscreenElement) await document.exitFullscreen();
    else try { await $('workspace').requestFullscreen(); }
    catch { $('workspace').classList.toggle('is-expanded'); }
  }));
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && !event.defaultPrevented && !$('help-dialog').open) back();
  });
}

async function start() {
  catalog = await readGzip('atlas-data/catalog.json.gz');
  if (catalog.schema !== 1) throw new Error('Unsupported atlas schema.');
  if (catalog.storage) {
    $('aspect-status').textContent = `~${Math.ceil(catalog.storage.bytes_before_index / 1_000_000)} MB export package`
      + (catalog.storage.within_budget ? '' : ' · larger than one GitHub Pages site');
  }
  collections = catalogCollections(catalog);
  for (const entry of collections) {
    const option = element('option', '', entry.label);
    option.value = entry.value;
    $('corpus-select').append(option);
  }
  for (const variant of catalog.variants) {
    const duplicate = catalog.variants.filter(v => v.name === variant.name).length > 1;
    names[variant.id] = `${variant.name}${duplicate ? ` · ${variant.run_id.slice(0, 8)}` : ''}`;
    state.selected.add(variant.id);
    const label = element('label');
    label.dataset.variant = variant.id;
    const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.checked = true;
    const text = element('span', '', names[variant.id]);
    checkbox.addEventListener('change', () => {
      if (checkbox.checked) state.selected.add(variant.id); else state.selected.delete(variant.id);
      state.token++; state.compare = false;
      gallery.setSelected(state.selected);
      if (state.sheet) openSheet(state.book, state.sheet); else render();
    });
    label.append(checkbox, text); $('pipeline-checkboxes').append(label);
  }
  gallery = new AtlasGallery($('gallery-container'), {
    book: book => { state.token++; state.book = book; state.sheet = null; state.detail = null; render(); },
    sheet: openSheet, findings: openFinding,
    renderer: mode => {
      $('renderer-status').textContent = mode === 'webgl' ? 'Three.js · WebGL' : mode === 'css' ? 'Flache CSS-Ansicht' : 'WebGL unavailable · choose Flat · CSS';
      $('webgl-button').setAttribute('aria-pressed', String(mode === 'webgl'));
      $('css-button').setAttribute('aria-pressed', String(mode === 'css'));
      let note = $('webgl-note');
      if (mode === 'unavailable' && !note) {
        note = element('div', 'atlas-note', 'WebGL is unavailable in this browser.'); note.id = 'webgl-note';
        note.append(button('Open flat CSS view', () => gallery.setMode('css'), 'outline-button'));
        $('gallery-container').append(note);
      } else if (mode !== 'unavailable') note?.remove();
    }
  });
  comparison = new ComparisonView($('comparison-host'), { onBack: back });
  attachControls();
  if (!collections.length) { $('gallery-title').textContent = 'No recorded workbooks'; return; }
  const query = new URLSearchParams(location.search);
  if (collections.some(entry => entry.value === query.get('collection'))) {
    $('corpus-select').value = query.get('collection');
  }
  corpusChanged();
  const book = catalog.books.find(entry => entry.id === query.get('book'));
  if (!book) return;

  if (!books().includes(book)) {
    $('corpus-select').value = book.corpus;
    corpusChanged();
  }
  const sheet = book.sheets.find(entry => entry.name === query.get('sheet'));
  if (!sheet) {
    openBook(book);
    return;
  }

  await openSheet(book, sheet);
  if (state.book !== book || state.sheet !== sheet) return;
  const finding = state.detail?.changes.find(entry => entry.range === query.get('range'));
  if (finding) await openFinding(book, sheet, finding.id);
}

start().catch(error => {
  $('gallery-title').textContent = 'Atlas could not be loaded';
  $('gallery-description').textContent = `${error.message} Serve the exported directory over HTTP or HTTPS.`;
  console.error(error);
});
