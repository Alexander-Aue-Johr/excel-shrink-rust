import { AtlasGallery } from './gallery.js';

const query = new URLSearchParams(location.search);
const run = query.get('run');
const requested = query.get('capture');
const $ = id => document.getElementById(id);
if (requested) $('follow').checked = false;
let snapshot;
let book;
let receipts = [];
let selected;
let sheetKey = '';
let revision;
let requestToken = 0;
let displayedContext;
const gallery = new AtlasGallery($('gallery'), {
  book: () => {},
  sheet: (_, sheet) => selectSheet(sheet.name),
  findings: (_, __, id) => openCapture(id),
  renderer: () => {},
});

function rectangle(receipt) {
  const g = receipt.geometry || {};
  return { x: Math.max(0, Number(g.left) || 0), y: Math.max(0, Number(g.top) || 0),
    w: Math.max(1, Number(g.width) || 1), h: Math.max(1, Number(g.height) || 1) };
}

async function selectSheet(name) {
  const sheet = book?.sheets.find(item => item.name === name);
  if (!sheet) return;
  $('sheets').value = name;
  gallery.focusSheet(book, sheet);
  const captures = receipts.filter(item => item.sheet === name).map(item => ({
    ...rectangle(item), id: item.id, range: item.range, label: item.range,
  }));
  const latest = receipts.filter(item => item.sheet === name).at(-1);
  const detail = { id: sheet.id, width: Math.max(1, ...captures.map(c => c.x + c.w)),
    height: Math.max(1, ...captures.map(c => c.y + c.h)), changes: [], captures,
    previewRegion: latest ? rectangle(latest) : { x: 0, y: 0, w: 1, h: 1 } };
  if (latest) {
    try {
      const preview = new Image();
      preview.src = latest.url;
      await preview.decode();
      detail.previewImage = preview;
    }
    catch { /* A completed run may already have moved its images into the durable report. */ }
  }
  if ($('sheets').value === name) gallery.showSheet(book, sheet, detail);
}

async function openCapture(id) {
  const capture = receipts.find(item => item.id === id);
  if (!capture) return;
  selected = id;
  const matching = receipts.filter(item => item.sheet === capture.sheet && item.range === capture.range);
  const newestByLane = new Map(matching.map(item => [item.lane, item]));
  const context = JSON.stringify([id, ...[...newestByLane.values()].map(item => item.id)]);
  if (context === displayedContext) return;
  const token = ++requestToken;
  await selectSheet(capture.sheet);
  if (token !== requestToken) return;
  $('capture-title').textContent = `${capture.sheet} · ${capture.range}`;
  const figures = [...newestByLane.values()].map(item => {
    const figure = document.createElement('figure');
    const caption = document.createElement('figcaption');
    caption.textContent = `${item.lane} · received ${new Date(item.at).toLocaleTimeString()}`;
    const image = document.createElement('img');
    image.alt = `${item.lane}: ${item.sheet}!${item.range}`;
    image.src = item.url;
    image.addEventListener('error', () => {
      caption.textContent += ' · live image expired; inspect the completed report';
    }, { once: true });
    figure.append(caption, image);
    return figure;
  });
  $('captures').replaceChildren(...figures);
  displayedContext = context;
}

async function refresh() {
  if (!run) { $('status').textContent = 'Open this view from a worker capture symbol.'; return; }
  try {
    const response = await fetch(`/api/live-atlas/${encodeURIComponent(run)}`, { cache: 'no-store' });
    if (!response.ok) throw new Error(`Live capture request failed (${response.status}).`);
    const incoming = await response.json();
    if (incoming.revision !== revision) {
      snapshot = incoming;
      revision = incoming.revision;
      receipts = snapshot.lanes.flatMap(lane => lane.recent_captures || []).sort((a, b) => a.at - b.at);
      $('workbook').textContent = snapshot.workbook;
      const sheets = [...new Set(receipts.map(item => item.sheet))];
      const nextKey = JSON.stringify(sheets);
      if (nextKey !== sheetKey) {
        sheetKey = nextKey;
        displayedContext = null;
        book = { id: run, name: snapshot.workbook, observations: [],
          sheets: sheets.map(name => ({ id: name, name })) };
        gallery.setBooks([book], new Set());
        $('sheets').replaceChildren(...sheets.map(name => {
          const option = document.createElement('option'); option.value = name; option.textContent = name; return option;
        }));
      }
      const target = !selected && receipts.some(item => item.id === requested) ? requested
        : $('follow').checked ? receipts.at(-1)?.id : selected;
      if (target && (target !== selected || $('follow').checked)) await openCapture(target);
      $('status').textContent = `${snapshot.status} · ${receipts.length} recent image receipts · up to 128 per native state`;
    }
  } catch (error) { $('status').textContent = error.message; }
  setTimeout(refresh, document.hidden ? 5000 : 1500);
}

$('sheets').addEventListener('change', () => {
  $('follow').checked = false;
  selectSheet($('sheets').value);
});
$('flat').addEventListener('click', () => gallery.setMode('css'));
$('three').addEventListener('click', () => gallery.setMode('webgl'));
refresh();
