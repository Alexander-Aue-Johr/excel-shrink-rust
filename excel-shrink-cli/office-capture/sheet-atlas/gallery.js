import * as THREE from './vendor/three.module.js';
import { paintSheetCard, getSheetLayout } from './card-painter.js';
import { assetUrl, loadImage, sheetSummary } from './model.js';

const CARD_WIDTH = 1;
const CARD_HEIGHT = .7;
const GAP = .12;

export function bookLayout(books) {
  const columns = Math.max(1, Math.min(7, Math.ceil(Math.sqrt(books.length))));
  const layouts = books.map(book => {
    const sheetColumns = Math.max(1, Math.ceil(Math.sqrt(book.sheets.length * 1.5)));
    const rows = Math.max(1, Math.ceil(book.sheets.length / sheetColumns));
    const width = 6.4, height = 4.4;
    const scale = Math.min((width - .2) / (sheetColumns * (CARD_WIDTH + GAP)),
      (height - .65) / (rows * (CARD_HEIGHT + GAP)));
    return { book, sheetColumns, rows, width, height, scale };
  });
  const cellWidth = Math.max(4, ...layouts.map(item => item.width)) + 1;
  const cellHeight = Math.max(3, ...layouts.map(item => item.height)) + 1;
  layouts.forEach((item, index) => {
    item.x = (index % columns - (columns - 1) / 2) * cellWidth;
    item.y = (Math.floor((books.length - 1) / columns) / 2 - Math.floor(index / columns)) * cellHeight;
  });
  return { layouts, width: columns * cellWidth, height: Math.ceil(books.length / columns) * cellHeight };
}

export class AtlasGallery {
  constructor(host, callbacks) {
    this.host = host;
    this.callbacks = callbacks;
    this.entries = new Map();
    this.bookEntries = new Map();
    this.selected = new Set();
    this.view = { x: 0, y: 0, span: 10 };
    this.target = { ...this.view };
    this.home = { ...this.view };
    this.motion = !matchMedia('(prefers-reduced-motion: reduce)').matches;
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(38, 1, .01, 10000);
    this.world = document.createElement('div');
    this.world.className = 'atlas-world';
    this.host.append(this.world);
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'atlas-webgl';
    this.host.append(this.canvas);
    try {
      this.renderer = new THREE.WebGLRenderer({ canvas: this.canvas, antialias: true, alpha: true });
      this.renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
      this.mode = 'webgl';
    } catch {
      this.mode = 'unavailable';
    }
    this.canvas.addEventListener('click', event => this.pick(event));
    this.host.addEventListener('wheel', event => {
      if (event.ctrlKey) return;
      event.preventDefault();
      this.target.span = Math.max(.7, Math.min(this.home.span * 2, this.target.span * Math.exp(event.deltaY * .001)));
    }, { passive: false });
    let drag;
    this.host.addEventListener('pointerdown', event => {
      this.dragged = false;
      drag = { x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY };
    });
    this.host.addEventListener('pointermove', event => {
      if (!drag || !event.buttons) return;
      if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) > 4) this.dragged = true;
      const scale = this.view.span / Math.max(1, this.host.clientHeight);
      this.target.x -= (event.clientX - drag.x) * scale;
      this.target.y += (event.clientY - drag.y) * scale;
      drag.x = event.clientX; drag.y = event.clientY;
    });
    this.host.addEventListener('pointerup', () => { drag = null; });
    this.host.addEventListener('click', event => {
      if (this.dragged && event.detail > 0) { event.preventDefault(); event.stopImmediatePropagation(); }
    }, true);
    this.observer = new ResizeObserver(() => this.resize());
    this.observer.observe(host);
    this.resize();
    this.animate();
  }

  setMode(mode) {
    this.mode = mode === 'css' ? 'css' : this.renderer ? 'webgl' : 'unavailable';
    this.host.dataset.renderer = this.mode;
    this.callbacks.renderer(this.mode);
  }

  setBooks(books, selected) {
    this.clear();
    this.selected = selected;
    const layout = bookLayout(books);
    for (const item of layout.layouts) this.addBook(item);
    const aspect = this.host.clientWidth / Math.max(1, this.host.clientHeight);
    this.home = { x: 0, y: 0, span: Math.max(layout.height, layout.width / aspect) * 1.12 };
    this.focus = null;
    this.target = { ...this.home };
    this.view = { ...this.home };
    this.setMode(this.mode === 'unavailable' ? 'webgl' : this.mode);
  }

  addBook(item) {
    const group = new THREE.Group();
    group.position.set(item.x, item.y, 0);
    this.scene.add(group);
    const label = document.createElement('div');
    label.className = 'atlas-book-label';
    label.style.left = `${(item.x - item.width / 2) * 100}px`;
    label.style.top = `${(-item.y - item.height / 2) * 100}px`;
    label.style.width = `${item.width * 100}px`;
    label.textContent = item.book.name;
    this.world.append(label);
    const headingCanvas = document.createElement('canvas'); headingCanvas.width = 1800; headingCanvas.height = 160;
    const headingContext = headingCanvas.getContext('2d');
    headingContext.fillStyle = '#d9e3e7'; headingContext.font = '100px Segoe UI';
    const title = item.book.name.length > 35 ? item.book.name.slice(0, 32) + '…' : item.book.name;
    headingContext.fillText(title, 0, 108, 1800);
    const headingTexture = new THREE.CanvasTexture(headingCanvas); headingTexture.colorSpace = THREE.SRGBColorSpace;
    const heading = new THREE.Mesh(new THREE.PlaneGeometry(item.width, item.width * 160 / 1800),
      new THREE.MeshBasicMaterial({ map: headingTexture, transparent: true }));
    heading.position.set(0, item.height / 2 - .2, .002); group.add(heading);
    this.bookEntries.set(item.book.id, { ...item, group, label, heading });
    item.book.sheets.forEach((sheet, index) => {
      const x = (index % item.sheetColumns - (item.sheetColumns - 1) / 2) * (CARD_WIDTH + GAP) * item.scale;
      const y = ((item.rows - 1) / 2 - Math.floor(index / item.sheetColumns)) * (CARD_HEIGHT + GAP) * item.scale - .18;
      const canvas = document.createElement('canvas');
      canvas.width = 256; canvas.height = 179;
      const texture = new THREE.CanvasTexture(canvas);
      texture.colorSpace = THREE.SRGBColorSpace;
      const material = new THREE.MeshBasicMaterial({ map: texture });
      const body = new THREE.Mesh(new THREE.PlaneGeometry(CARD_WIDTH * item.scale, CARD_HEIGHT * item.scale), material);
      body.position.set(x, y, .001);
      group.add(body);
      const button = document.createElement('button');
      button.className = 'atlas-sheet';
      button.dataset.sheet = sheet.name;
      button.dataset.book = item.book.id;
      button.dataset.instance = crypto.randomUUID();
      button.style.left = `${(item.x + x - CARD_WIDTH * item.scale / 2) * 100}px`;
      button.style.top = `${(-item.y - y - CARD_HEIGHT * item.scale / 2) * 100}px`;
      button.style.width = `${CARD_WIDTH * item.scale * 100}px`;
      button.style.height = `${CARD_HEIGHT * item.scale * 100}px`;
      button.style.setProperty('--tile-scale', item.scale);
      const name = document.createElement('span');
      name.className = 'atlas-sheet-name'; name.textContent = sheet.name;
      const preview = document.createElement('img');
      preview.className = 'atlas-preview'; preview.alt = ''; preview.decoding = 'async';
      const metadata = document.createElement('small');
      metadata.className = 'atlas-sheet-meta';
      button.append(preview, name, metadata);
      this.world.append(button);
      const entry = { book: item.book, sheet, body, canvas, texture, button, preview, metadata,
        x: item.x + x, y: item.y + y, scale: item.scale, group, opacity: 0, desiredOpacity: 0, attachments: [] };
      body.userData.entry = entry;
      button.addEventListener('click', event => {
        event.stopPropagation();
        if (entry.hits) {
          const bounds = button.getBoundingClientRect();
          this.hit(entry, entry.hits, (event.clientX - bounds.left) / bounds.width, (event.clientY - bounds.top) / bounds.height);
        } else this.open(entry);
      });
      this.entries.set(`${item.book.id}:${sheet.id}`, entry);
      this.paintBase(entry);
    });
  }

  paintBase(entry) {
    const observations = entry.book.observations.filter(o => this.selected.has(o.variant));
    const summaries = observations.map(o => sheetSummary(o, entry.sheet.name)).filter(Boolean);
    const count = summaries.reduce((sum, s) => sum + s.differences, 0);
    const incomplete = observations.some(o => !o.detail || !String(o.status).startsWith('completed'));
    const ctx = entry.canvas.getContext('2d');
    ctx.setTransform(entry.canvas.width / 420, 0, 0, entry.canvas.height / 294, 0, 0);
    ctx.fillStyle = count ? '#3a3039' : '#233741'; ctx.fillRect(0, 0, 420, 294);
    ctx.strokeStyle = count ? '#a4646d' : '#405763'; ctx.lineWidth = 3; ctx.strokeRect(1, 1, 418, 292);
    ctx.fillStyle = count ? '#df8f96' : '#93a8b4';
    ctx.font = '18px Segoe UI'; ctx.fillText(String(entry.sheet.order + 1).padStart(2, '0'), 15, 27);
    ctx.font = '20px Segoe UI'; ctx.fillText(entry.sheet.name.slice(0, 32), 15, 150);
    if (count) ctx.fillText(`${count} findings`, 15, 267);
    if (incomplete) ctx.fillText('Incomplete', 15, 58);
    entry.texture.needsUpdate = true;
    entry.button.classList.toggle('has-differences', count > 0);
    entry.button.classList.toggle('is-incomplete', incomplete);
    entry.button.setAttribute('aria-label', `${entry.sheet.name}, ${count} recorded differences${incomplete ? ', incomplete' : ''}`);
    entry.metadata.textContent = summaries.find(s => s.rows)?.rows
      ? `Envelope · ${summaries.find(s => s.rows).rows} rows · ${summaries.find(s => s.columns)?.columns || '?'} columns`
      : entry.sheet.state;
  }

  setSelected(selected) {
    this.selected = selected;
    for (const entry of this.entries.values()) {
      this.removeDetail(entry);
      this.paintBase(entry);
    }
    if (this.focus?.book) this.enrichBook(this.focus.book);
  }

  open(entry) {
    if (this.focus?.sheet === entry.sheet.id && this.focus?.book === entry.book.id) {
      this.callbacks.findings(entry.book, entry.sheet);
    } else if (this.focus?.book === entry.book.id) {
      this.focusSheet(entry.book, entry.sheet);
      this.callbacks.sheet(entry.book, entry.sheet);
    } else {
      this.focusBook(entry.book);
      this.callbacks.book(entry.book);
    }
  }

  focusBook(book) {
    const item = this.bookEntries.get(book.id);
    if (!item) return;
    this.focus = { book: book.id };
    const aspect = this.host.clientWidth / Math.max(1, this.host.clientHeight);
    this.target = { x: item.x, y: item.y, span: Math.max(item.height, item.width / aspect) * 1.16 };
    this.enrichBook(book.id);
  }

  enrichBook(id) {
    this.host.classList.add('atlas-enriched');
    for (const entry of this.entries.values()) {
      entry.button.classList.toggle('is-enriched', entry.book.id === id);
      if (entry.book.id !== id) { this.removeDetail(entry); continue; }
      const summary = entry.book.observations.filter(o => this.selected.has(o.variant))
        .map(o => sheetSummary(o, entry.sheet.name)).find(s => s?.thumbnail);
      if (summary) this.loadPreview(entry, assetUrl(summary.thumbnail));
    }
  }

  async loadPreview(entry, url) {
    const token = entry.previewToken = Symbol();
    try {
      const image = await loadImage(url);
      if (entry.previewToken !== token) return;
      const texture = new THREE.Texture(image); texture.needsUpdate = true; texture.colorSpace = THREE.SRGBColorSpace;
      const scale = entry.scale * Math.min(.94 / image.naturalWidth, .49 / image.naturalHeight);
      const layer = new THREE.Mesh(new THREE.PlaneGeometry(image.naturalWidth * scale, image.naturalHeight * scale),
        new THREE.MeshBasicMaterial({ map: texture, transparent: true, opacity: 0 }));
      layer.position.copy(entry.body.position); layer.position.y -= .035 * entry.scale; layer.position.z += .002;
      entry.group.add(layer);
      entry.layer?.geometry.dispose(); entry.layer?.material.map.dispose(); entry.layer?.material.dispose();
      if (entry.layer) entry.group.remove(entry.layer);
      entry.layer = layer; entry.opacity = 0; entry.desiredOpacity = 1;
      entry.preview.src = url;
      entry.preview.onload = () => { if (entry.previewToken === token) entry.preview.classList.add('is-ready'); };
      if (entry.preview.complete) entry.preview.classList.add('is-ready');
    } catch (error) {
      if (entry.previewToken === token) entry.metadata.textContent = error.message;
    }
  }

  focusSheet(book, sheet) {
    const entry = this.entries.get(`${book.id}:${sheet.id}`);
    if (!entry) return;
    this.focus = { book: book.id, sheet: sheet.id };
    this.target = { x: entry.x, y: entry.y, span: 1.05 * entry.scale };
    this.enrichBook(book.id);
  }

  showSheet(book, sheet, detail) {
    const entry = this.entries.get(`${book.id}:${sheet.id}`);
    if (!entry || this.focus?.sheet !== sheet.id) return;
    this.removeDetail(entry);
    const canvas = document.createElement('canvas'); canvas.width = 960; canvas.height = 672;
    const layout = getSheetLayout(detail, canvas.width, canvas.height);
    const previewRegion = { ...layout.previewRegion,
      x: Math.min(detail.previewRegion.x, detail.width - layout.previewRegion.w),
      y: Math.min(detail.previewRegion.y, detail.height - layout.previewRegion.h) };
    const result = paintSheetCard(canvas, detail, { previewRegion,
      changes: detail.changes, capturedRegions: detail.captures });
    const texture = new THREE.CanvasTexture(canvas); texture.colorSpace = THREE.SRGBColorSpace;
    const layer = new THREE.Mesh(new THREE.PlaneGeometry(CARD_WIDTH * entry.scale, CARD_HEIGHT * entry.scale),
      new THREE.MeshBasicMaterial({ map: texture, transparent: true, opacity: 0 }));
    layer.position.copy(entry.body.position); layer.position.z += .004;
    layer.userData = { entry, hits: result.hits };
    entry.group.add(layer); entry.layer = layer; entry.desiredOpacity = 1;
    entry.preview.src = canvas.toDataURL(); entry.preview.classList.add('is-ready', 'atlas-full-preview');
    entry.hits = result.hits;
    for (const attachment of result.attachments) this.addMap(entry, attachment);
    const extent = result.attachments[0];
    const width = entry.scale * (1 + (extent?.placement === 'right' ? extent.widthRatio : 0));
    const height = CARD_HEIGHT * entry.scale * (1 + (extent?.placement === 'bottom' ? extent.heightRatio : 0));
    const aspect = this.host.clientWidth / Math.max(1, this.host.clientHeight);
    this.target = { x: entry.x + (width - entry.scale) / 2, y: entry.y - (height - CARD_HEIGHT * entry.scale) / 2,
      span: Math.max(height, width / aspect) * 1.35 };
  }

  addMap(entry, attachment) {
    const width = attachment.widthRatio * entry.scale;
    const height = CARD_HEIGHT * attachment.heightRatio * entry.scale;
    const x = attachment.placement === 'right' ? entry.body.position.x + .5 * entry.scale + width / 2 + .03 * entry.scale : entry.body.position.x;
    const y = attachment.placement === 'bottom' ? entry.body.position.y - CARD_HEIGHT / 2 * entry.scale - height / 2 - .03 * entry.scale : entry.body.position.y;
    const texture = new THREE.CanvasTexture(attachment.canvas); texture.colorSpace = THREE.SRGBColorSpace;
    const mesh = new THREE.Mesh(new THREE.PlaneGeometry(width, height),
      new THREE.MeshBasicMaterial({ map: texture, transparent: true, opacity: 0 }));
    mesh.position.set(x, y, .006); mesh.userData = { entry, hits: attachment.hits };
    entry.group.add(mesh);
    const button = document.createElement('button'); button.className = 'atlas-map';
    button.setAttribute('aria-label', `Recorded extent of ${entry.sheet.name}`);
    button.style.cssText = `left:${(entry.x - entry.body.position.x + x - width / 2) * 100}px;top:${(-entry.y + entry.body.position.y - y - height / 2) * 100}px;width:${width * 100}px;height:${height * 100}px`;
    button.append(attachment.canvas); this.world.append(button);
    button.addEventListener('click', event => {
      const bounds = button.getBoundingClientRect();
      this.hit(entry, attachment.hits, (event.clientX - bounds.left) / bounds.width, (event.clientY - bounds.top) / bounds.height);
    });
    entry.attachments.push({ mesh, button });
  }

  hit(entry, hits, x, y) {
    const hit = hits.find(h => x >= h.x && x <= h.x + h.w && y >= h.y && y <= h.y + h.h && h.kind === 'change');
    this.callbacks.findings(entry.book, entry.sheet, hit?.changeId);
  }

  overview() {
    this.focus = null; this.target = { ...this.home };
    for (const entry of this.entries.values()) {
      entry.button.classList.remove('is-enriched');
      entry.desiredOpacity = 0;
      entry.preview.classList.remove('is-ready');
      for (const attachment of entry.attachments) attachment.button.style.opacity = '0';
    }
  }

  pick(event) {
    const bounds = this.canvas.getBoundingClientRect();
    const pointer = new THREE.Vector2((event.clientX - bounds.left) / bounds.width * 2 - 1, 1 - (event.clientY - bounds.top) / bounds.height * 2);
    const ray = new THREE.Raycaster(); ray.setFromCamera(pointer, this.camera);
    const hits = ray.intersectObjects([...this.entries.values()].flatMap(e => [e.body, e.layer, ...e.attachments.map(a => a.mesh)].filter(Boolean)));
    const hit = hits[0];
    if (!hit) return;
    const entry = hit.object.userData.entry;
    if (hit.object.userData.hits) this.hit(entry, hit.object.userData.hits, hit.uv.x, 1 - hit.uv.y);
    else this.open(entry);
  }

  resize() {
    const width = this.host.clientWidth, height = Math.max(1, this.host.clientHeight);
    this.camera.aspect = width / height; this.camera.updateProjectionMatrix();
    this.renderer?.setSize(width, height);
  }

  animate() {
    this.frame = requestAnimationFrame(() => this.animate());
    const blend = this.motion ? .13 : 1;
    for (const key of ['x', 'y', 'span']) this.view[key] += (this.target[key] - this.view[key]) * blend;
    const scale = this.host.clientHeight / this.view.span;
    this.world.style.transform = `translate(${this.host.clientWidth / 2 - this.view.x * scale}px,${this.host.clientHeight / 2 + this.view.y * scale}px) scale(${scale / 100})`;
    this.camera.position.set(this.view.x, this.view.y, this.view.span / (2 * Math.tan(THREE.MathUtils.degToRad(19))));
    this.camera.lookAt(this.view.x, this.view.y, 0);
    for (const entry of this.entries.values()) {
      entry.opacity += (entry.desiredOpacity - entry.opacity) * blend;
      if (entry.layer) entry.layer.material.opacity = entry.opacity;
      for (const attachment of entry.attachments) attachment.mesh.material.opacity = entry.opacity;
    }
    if (this.mode === 'webgl') this.renderer.render(this.scene, this.camera);
  }

  removeDetail(entry) {
    entry.previewToken = null;
    entry.preview.classList.remove('is-ready', 'atlas-full-preview');
    entry.preview.removeAttribute('src');
    entry.button.onclick = null; entry.hits = null;
    for (const object of [entry.layer, ...entry.attachments.map(a => a.mesh)].filter(Boolean)) {
      entry.group.remove(object); object.geometry.dispose(); object.material.map?.dispose(); object.material.dispose();
    }
    for (const attachment of entry.attachments) attachment.button.remove();
    entry.layer = null; entry.attachments = []; entry.opacity = 0;
  }

  clear() {
    for (const entry of this.entries.values()) {
      this.removeDetail(entry); entry.body.geometry.dispose(); entry.texture.dispose(); entry.body.material.dispose();
    }
    for (const entry of this.bookEntries.values()) {
      entry.heading.geometry.dispose(); entry.heading.material.map.dispose(); entry.heading.material.dispose();
    }
    this.scene.clear(); this.entries.clear(); this.bookEntries.clear(); this.world.replaceChildren();
  }
}
