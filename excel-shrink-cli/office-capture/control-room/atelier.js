import { atelierRegions } from './atelier-model.js';

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
  header.append(title, close);
  const toolbar = node('div');
  toolbar.className = 'atelier-tools';
  const previous = node('button', 'Previous');
  const next = node('button', 'Next');
  const select = node('select');
  select.setAttribute('aria-label', 'Comparison region and phase');
  const size = node('button', 'Show 1:1');
  const all = node('input');
  all.type = 'checkbox';
  const allLabel = node('label');
  allLabel.append(all, document.createTextNode(' Include checked regions without differences'));
  toolbar.append(previous, select, next, size, allLabel);
  const note = node('p');
  note.className = 'atelier-note';
  const images = node('div');
  images.className = 'atelier-images fit';
  dialog.append(header, toolbar, note, images);
  document.body.append(dialog);
  let ownsFullscreen = false;
  let report = null;
  let regions = [];
  let generation = 0;
  let request = null;

  // Use one scale for both captures; independent fitting would invent row-height differences.
  function fitImages() {
    const captures = [...images.querySelectorAll('img')];
    const width = captures.reduce((sum, image) => sum + image.naturalWidth, 0);
    const scale = images.classList.contains('fit') && width > 0
      ? Math.min(1, images.clientWidth / width) : 1;
    for (const image of captures) {
      if (image.naturalWidth) image.style.width = `${image.naturalWidth * scale}px`;
    }
  }
  new ResizeObserver(fitImages).observe(images);

  function render() {
    images.replaceChildren();
    const index = select.selectedIndex;
    const region = regions[index];
    previous.disabled = index <= 0;
    next.disabled = index < 0 || index >= regions.length - 1;
    if (!region) {
      note.textContent = report
        ? 'No differences detected in the selected regions. This is not a whole-workbook visual identity claim.'
        : 'Loading retained captures...';
      return;
    }
    note.textContent = `${index + 1} / ${regions.length} / ${region.different ? 'Difference found' : 'No difference detected'} / `
      + (region.blank_margin_ignored ? 'Blank trailing margin size ignored. ' : '')
      + 'O: Original / SD: Default before rebuild / SDR: Default after rebuild. Images are retained run evidence.';
    const pair = node('div');
    pair.className = 'atelier-pair';
    for (const side of ['left', 'right']) {
      const figure = node('figure');
      figure.className = 'atelier-figure';
      const caption = node('figcaption', region[side]);
      figure.append(caption);
      const url = region[side + 'Url'];
      if (url) {
        const image = node('img');
        image.alt = `${region[side]} / ${region.sheet} / ${region.cell || region.chart}`;
        image.addEventListener('load', fitImages);
        image.src = url;
        image.addEventListener('error', () => {
          image.replaceWith(node('p', 'Retained image is unavailable. No replacement capture was generated.'));
        }, {once: true});
        figure.append(image);
      } else {
        figure.append(node('p', 'No rendered image for this state; consult its geometry/presence verdict.'));
      }
      pair.append(figure);
    }
    images.append(pair);
    fitImages();
    images.scrollTop = 0;
    images.scrollLeft = 0;
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
  size.addEventListener('click', () => {
    const fit = images.classList.toggle('fit');
    size.textContent = fit ? 'Show 1:1' : 'Fit images';
    fitImages();
  });
  close.addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => {
    generation++;
    request?.abort();
    request = null;
    images.replaceChildren();
    report = null;
    regions = [];
    if (ownsFullscreen && document.fullscreenElement === document.documentElement) void document.exitFullscreen();
  });
  return {
    async open(id, workbook) {
      const current = ++generation;
      request?.abort();
      request = new AbortController();
      report = null;
      all.checked = false;
      title.textContent = workbook;
      populate();
      if (!dialog.open) dialog.showModal();
      // Request in the click handler, before awaiting I/O, to retain user activation.
      if (!document.fullscreenElement && document.documentElement.requestFullscreen) {
        ownsFullscreen = true;
        void document.documentElement.requestFullscreen().catch(() => { ownsFullscreen = false; }); // The viewport-sized dialog remains usable.
      }
      try {
        const response = await fetch(`/api/groups/${encodeURIComponent(id)}/targeted-report`, {
          cache: 'no-store', signal: request.signal,
        });
        const result = await response.json();
        if (current !== generation) return;
        if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`);
        report = result;
        populate();
      } catch (error) {
        if (current === generation && error.name !== 'AbortError') {
          note.textContent = `Cannot open retained comparison: ${error.message}`;
        }
      }
    },
  };
}
