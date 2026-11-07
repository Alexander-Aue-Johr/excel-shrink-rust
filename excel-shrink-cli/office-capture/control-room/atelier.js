import { atelierRegions } from './atelier-model.js';
import { requestPixelDiagnostics } from './pixel-diagnostics.js';
import { createPixelInspector } from './atelier-pixels.js';
import { createClipboardButton } from './inspection-clipboard.js';
import { adjacentDifference, loadDifferentWorkbooks } from './atelier-navigation.js';
import { createFormulaInspector } from './formula-diagnostics.js';
import { readDiagnosticResponse } from './diagnostic-response.js';

const node = (tag, text) => {
  const element = document.createElement(tag);
  if (text != null) element.textContent = text;
  return element;
};

/** View one pair of immutable captures at a time; closing releases image references. */
export function createAtelier() {
  const dialog = node('dialog');
  dialog.className = 'atelier';
  dialog.setAttribute('aria-label', 'Visual comparison atelier');
  const header = node('header');
  header.className = 'atelier-header';
  const title = node('strong');
  const close = node('button', 'Close atelier');
  const clipboardNotice = node('p');
  clipboardNotice.setAttribute('role', 'status');
  const clipboard = createClipboardButton(() => groupId, clipboardNotice);
  header.append(title, clipboard, close);
  const toolbar = node('div');
  toolbar.className = 'atelier-tools';
  const navigation = node('div');
  navigation.className = 'atelier-navigation';
  navigation.setAttribute('aria-label', 'Difference navigation');
  const options = node('div');
  options.className = 'atelier-options';
  const previous = node('button', 'Previous in file');
  const next = node('button', 'Next in file');
  const previousOverall = node('button', 'Previous difference · all files');
  const nextOverall = node('button', 'Next difference · all files');
  const select = node('select');
  select.setAttribute('aria-label', 'Comparison region and phase');
  const size = node('button', 'Show 1:1');
  const all = node('input');
  all.type = 'checkbox';
  const allLabel = node('label');
  allLabel.append(all, document.createTextNode(' Include checked regions without differences'));
  navigation.append(previous, next, previousOverall, nextOverall);
  options.append(select, size, allLabel);
  toolbar.append(navigation, options);
  const note = node('p');
  note.className = 'atelier-note';
  const images = node('div');
  images.className = 'atelier-images fit';
  dialog.append(header, clipboardNotice, toolbar, note, images);
  document.body.append(dialog);
  let ownsFullscreen = false;
  let report = null;
  let regions = [];
  let generation = 0;
  let request = null;
  let groupId = null;
  let renderGeneration = 0;
  let differentWorkbooks = [];

  function overallTarget(direction) {
    const indices = regions.flatMap((region, index) => region.different ? [index] : []);
    const next = direction > 0
      ? indices.find(index => index > select.selectedIndex)
      : indices.findLast(index => index < select.selectedIndex);
    if (next != null) return { regionIndex: next };
    return adjacentDifference(0, 0, differentWorkbooks, groupId, direction);
  }

  async function navigateOverall(direction) {
    const target = overallTarget(direction);
    if (!target) return;
    if (target.workbook) {
      const workbook = target.workbook;
      await api.open(String(workbook.id), workbook.name || workbook.workbook || String(workbook.id), target.selectLast);
    } else {
      select.selectedIndex = target.regionIndex;
      render();
    }
  }

  // Use one scale for both captures; independent fitting would invent row-height differences.
  function fitImages() {
    const captures = [...images.querySelectorAll('img.atelier-capture')];
    const width = captures.reduce((sum, image) => sum + image.naturalWidth, 0);
    const scale = images.classList.contains('fit') && width > 0
      ? Math.min(1, images.clientWidth / width) : 1;
    for (const image of captures) {
      if (image.naturalWidth) image.style.width = `${image.naturalWidth * scale}px`;
    }
  }
  new ResizeObserver(fitImages).observe(images);

  function render() {
    const rendered = ++renderGeneration;
    images.replaceChildren();
    const index = select.selectedIndex;
    const region = regions[index];
    previous.disabled = index <= 0;
    next.disabled = index < 0 || index >= regions.length - 1;
    previousOverall.disabled = !overallTarget(-1);
    nextOverall.disabled = !overallTarget(1);
    if (!region) {
      note.textContent = report
        ? 'No differences detected in the selected regions. This is not a whole-workbook visual identity claim.'
        : 'Loading retained captures...';
      return;
    }
    const outcome = region.dependencies_unchanged ? 'Skipped — unchanged chart dependencies'
      : region.different ? 'Difference found' : 'No difference detected';
    note.textContent = `${index + 1} / ${regions.length} / ${outcome} / `
      + (region.blank_margin_ignored ? 'Blank trailing margin size ignored. ' : '')
      + 'O: Original / SD: Default before rebuild / SDR: Default after rebuild. Storage decisions are recorded in run evidence.';
    const pair = node('div');
    pair.className = 'atelier-pair';
    const views = [];
    for (const side of ['left', 'right']) {
      const figure = node('figure');
      figure.className = 'atelier-figure';
      const caption = node('figcaption', region[side]);
      figure.append(caption);
      const url = region[side + 'Url'];
      if (url) {
        const image = node('img');
        image.className = 'atelier-capture';
        image.alt = `${region[side]} / ${region.sheet} / ${region.cell || region.chart}`;
        image.addEventListener('load', fitImages);
        image.src = url;
        image.addEventListener('error', () => {
          image.replaceWith(node('p', 'Retained image is unavailable. No replacement capture was generated.'));
        }, {once: true});
        const stage = node('div');
        stage.className = 'atelier-image-stage';
        stage.append(image);
        figure.append(stage);
        views.push({ image, stage, label: region[side] });
      } else {
        let message = 'No rendered image for this state; consult its geometry/presence verdict.';
        if (region[side + '_image_retained'] === false) {
          const receipt = region[side + '_image_receipt'];
          message = 'Image discarded after a verified equal comparison.';
          if (receipt) message += ` ${receipt.pixels.join(' × ')} px · SHA-256 ${receipt.sha256}`;
        } else if (region.dependencies_unchanged) {
          message = 'Capture skipped — unchanged chart dependencies.';
        }
        figure.append(node('p', message));
      }
      pair.append(figure);
    }
    images.append(pair);
    fitImages();
    images.scrollTop = 0;
    images.scrollLeft = 0;
    if (views.length === 2 && region.different) {
      const measuring = node('p', 'Measuring retained pixels…');
      const formulas = createFormulaInspector(groupId, region, report?._evidence_id);
      images.append(measuring, formulas);
      Promise.all(views.map(view => view.image.decode())).then(async () => {
        if (rendered !== renderGeneration || !dialog.open) return;
        const diagnostic = await requestPixelDiagnostics(groupId, { priority: true, region: region.key });
        if (rendered !== renderGeneration || !dialog.open) return;
        if (diagnostic.report_id !== report?._evidence_id) {
          throw new Error('The retained comparison changed. Reopen the atelier to inspect its latest evidence.');
        }
        const entry = diagnostic.regions.find(item =>
          `${item.pair_index}:${item.kind}:${item.finding_index}` === region.key);
        if (!entry) throw new Error('No rendered image pair is available for this diagnostic.');
        measuring.remove();
        images.insertBefore(createPixelInspector(views, entry.metrics, region), formulas);
      }).catch(error => {
        if (rendered === renderGeneration) measuring.textContent = `Pixel inspection unavailable: ${error.message}`;
      });
    }
  }
  function populate() {
    regions = atelierRegions(report || {}, !all.checked);
    select.replaceChildren(...regions.map(region => node('option', region.label)));
    render();
  }
  select.addEventListener('change', render);
  all.addEventListener('change', populate);
  previous.addEventListener('click', () => { select.selectedIndex--; render(); });
  next.addEventListener('click', () => { select.selectedIndex++; render(); });
  previousOverall.addEventListener('click', () => void navigateOverall(-1));
  nextOverall.addEventListener('click', () => void navigateOverall(1));
  size.addEventListener('click', () => {
    const fit = images.classList.toggle('fit');
    size.textContent = fit ? 'Show 1:1' : 'Fit images';
    fitImages();
  });
  close.addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => {
    generation++;
    renderGeneration++;
    request?.abort();
    request = null;
    images.replaceChildren();
    report = null;
    regions = [];
    if (ownsFullscreen && document.fullscreenElement === document.documentElement) void document.exitFullscreen();
  });
  const api = {
    async open(id, workbook, selectLast = false) {
      const current = ++generation;
      groupId = id;
      differentWorkbooks = [];
      clipboardNotice.textContent = '';
      request?.abort();
      request = new AbortController();
      const signal = request.signal;
      report = null;
      all.checked = false;
      title.textContent = workbook;
      populate();
      if (!dialog.open) dialog.showModal();
      void loadDifferentWorkbooks(signal).then(workbooks => {
        if (current !== generation) return;
        differentWorkbooks = workbooks;
        previousOverall.disabled = !overallTarget(-1);
        nextOverall.disabled = !overallTarget(1);
      }).catch(error => {
        if (current === generation && error.name !== 'AbortError') {
          previousOverall.title = nextOverall.title = `Cross-file navigation unavailable: ${error.message}`;
        }
      });
      // Request in the click handler, before awaiting I/O, to retain user activation.
      if (!document.fullscreenElement && document.documentElement.requestFullscreen) {
        ownsFullscreen = true;
        void document.documentElement.requestFullscreen().catch(() => { ownsFullscreen = false; }); // The viewport-sized dialog remains usable.
      }
      try {
        const response = await fetch(`/api/groups/${encodeURIComponent(id)}/targeted-report`, {
          cache: 'no-store', signal: request.signal,
        });
        const result = await readDiagnosticResponse(response, 'Retained comparison');
        if (current !== generation) return;
        report = result;
        populate();
        if (selectLast && regions.length) {
          select.selectedIndex = regions.length - 1;
          render();
        }
      } catch (error) {
        if (current === generation && error.name !== 'AbortError') {
          note.textContent = `Cannot open retained comparison: ${error.message}`;
        }
      }
    },
  };
  return api;
}
