import * as THREE from 'three';
import { OrbitControls } from './vendor/OrbitControls.js';
import { normalizeGroup, imageStatus, groupedPanels, visibleTiles, clippedTile, profileRectangle, activeGroupCount, sheetLayout, inspectionRegions, inspectionEntry, differenceTargets, heartbeatAge, omissionDescription, processMeasurements, tileEvidenceDescription, captureTransportSummary, groupOutcome, PRIMARY_PROFILES } from './model.js';
import { fitSheetViewport, resolveSheetViewport, zoomSheetViewport, scrollSheetViewport, sheetViewportKey, followSheetViewport } from './sheet-viewport.js';
import { initWorkerMonitor } from './worker-monitor.js';
import { workerRows } from './worker-monitor-model.js';
import { createSignalMonitor } from './signal-monitor.js';
import { findCaptureTile, profileForClient, latestTileForProfile } from './capture-navigation-model.js';
import { createControlUpdates } from './control-state.js';

const $ = id => document.getElementById(id);
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
const C = { waiting: 0x3c566b, received: 0x79dce8, equal: 0x75edc0, 'hash-equal': 0x98badf, different: 0xff7a91, omitted: 0xa5a3bb };
const CARD_W = 16, CARD_H = 9, BOARD_W = 3.67, BOARD_H = 7.75, BOARD_STEP = 3.87;
const AREA_W = 3.43, AREA_H = 7.08, AREA_Y = -.16;
let RADIUS = 24, layout = sheetLayout([]);
const state = { ready: false, connected: false, groupId: null, sheetId: null, selected: null,
  pinned: false, highlightDiff: true, follow: true, error: null, controls: {}, queue: null,
  view: { mode: 'sheet', x: 0, y: 0, width: 1800, height: 1800 * AREA_H / AREA_W },
  focus: 'overview', regionId: null, pendingDifference: false };
const controlUpdates = createControlUpdates(value => { state.controls = value; });
const boards = [], sourceTiles = [], hitTargets = [], detailPanels = [], connections = [];
const sheetCards = new Map(), processCards = new Map();
const inspectionButtons = new Map(), inspectionDownloads = new Map(), pendingInspections = new Set();
const sheetViewports = new Map(), sheetToolbars = new Map();
const raycaster = new THREE.Raycaster(), pointer = new THREE.Vector2();
const textureCache = new Map(), flashes = new Map(), previousImages = new Map();
const dirtyControls = new Set();
const loadQueue = [];
let activeLoads = 0, lastSignature = '', detailSignature = '', groupSignature = '', sheetSignature = '';
let data, entries = [], renderer, scene, camera, controls, lastUi = 0;
let selectedGeneration = 0, sceneGeneration = 0, pollGeneration = 0;
let hoverTimer, pendingHover, pointerDown, userMovedCamera = false, cameraTween;
let dockCloseTimer, compareRegion = { x: 0, y: 0, width: 1, height: 1 };
let compareZoom = 1, compareCenter = { x: .5, y: .5 };
let compareCropTimer, compareCropToken = 0, compareCropBusy = false, compareCropAbort;
const compareCropQueue = [];
let latestCapture = null, pollTimer, controlBusy = false, localEvents = [];
let pendingCaptureJump = null, followedProfile = null, lastFollowedKey = null;
let groupLimit = 100;
let miniSignature = '', lastFocusKey = '', navigationBusy = false;
let workerMonitor, omissionTooltip;
let workbookIndex = null, workbookRows = [], nextWorkbookOffset = null;
let workbookRequest = 0, lastWorkbookRefresh = 0, workbookSearchTimer;
let workbookPageLoading = false;

const WORKBOOK_FILTER_LABELS = {
  all: 'All selected workbooks',
  'needs-visual': 'Visual comparison needed',
  identical: 'Visually identical',
  differences: 'Differences found',
  'crc-unchanged': 'Unchanged by shrink',
  'workbook-only': 'Only workbook.xml cleanup',
  namespace: 'Unsupported namespace',
  'preparation-failed': 'Preparation failed',
  'other-excluded': 'Other preparation exclusions',
  deferred: 'Full scan paused',
  'recalc-drift': 'Shrink equal, recalc changed',
  'default-investigate': 'Default: investigate difference',
  screened: 'Aggressive screening complete',
  'secondary-pending': 'Full validation pending',
  'diagnosis-row_height': 'Explained: row heights',
  'diagnosis-local_whitespace': 'Explained: local whitespace',
  'diagnosis-local_hidden_content': 'Explained: local hidden content',
  'diagnosis-cross_sheet_hidden_content': 'Explained: cross-sheet hidden content',
  'diagnosis-cross_sheet_whitespace': 'Explained: cross-sheet whitespace',
  'diagnosis-recalculation': 'Explained after recalculation',
  'diagnosis-combined': 'Resolved after combined repairs',
  'diagnosis-unstable': 'Unstable / not reproducible',
  'diagnosis-unresolved': 'Unexplained differences',
};

for (const [value, label] of Object.entries(WORKBOOK_FILTER_LABELS)) {
  if (!$('groupFilter').querySelector(`option[value="${value}"]`)) {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = label;
    $('groupFilter').append(option);
  }
}
function announce(text) { $('announcement').textContent = text; }
function profileLabel(id) { return data?.profiles.find(profile => profile.id === id)?.label || id; }
function tileAt(id) { return entries.find(tile => tile.id === id); }
function selectedSheet() { return data?.sheets.find(sheet => sheet.id === state.sheetId); }
function sheetView(sheet) {
  if (!sheet) return state.view;
  const key = sheetViewportKey(state.groupId, sheet.id);
  const view = resolveSheetViewport(sheet, AREA_W / AREA_H, sheetViewports.get(key));
  sheetViewports.set(key, view);
  if (sheet.id === state.sheetId) state.view = view;
  return view;
}
function rememberView(sheet, view) {
  const resolved = resolveSheetViewport(sheet, AREA_W / AREA_H, view);
  sheetViewports.set(sheetViewportKey(state.groupId, sheet.id), resolved);
  if (sheet.id === state.sheetId) state.view = resolved;
  return resolved;
}
function setSheetFit(sheetId, mode) {
  const sheet = data?.sheets.find(sheet => sheet.id === sheetId); if (!sheet) return;
  stopFollowing(); clearDetails(); state.sheetId = sheetId;
  rememberView(sheet, fitSheetViewport(sheet, AREA_W / AREA_H, mode));
  updateSheets(); renderScene(); renderMinimap();
}
function stopFollowing() { state.follow = false; $('followCapture').checked = false; }
function labelGroup(group) { return group.ids.map(profileLabel).join(' · '); }
function safeImageUrl(url) {
  const value = new URL(url, location.href);
  if (value.origin !== location.origin || !value.pathname.startsWith('/data/')) throw new Error('The image path is outside the local output directory.');
  return value.href;
}
function setNotice(text, failure = false) { $('notice').textContent = text; $('notice').classList.toggle('failure', failure); }
function pushEvent(text) { localEvents.unshift({ at: new Date().toISOString(), message: text }); localEvents = localEvents.slice(0, 30); }

function canvasLabel(lines, width = 1280, height = 160) {
  const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
  const ctx = canvas.getContext('2d');
  for (const line of lines) {
    let size = line.size || 42;
    ctx.font = `${line.weight || 500} ${size}px "Segoe UI", sans-serif`;
    while (ctx.measureText(line.text).width > width - 36 && size > 16) ctx.font = `${line.weight || 500} ${--size}px "Segoe UI", sans-serif`;
    ctx.fillStyle = line.color || '#edf4fa'; ctx.textAlign = line.align || 'left';
    ctx.fillText(line.text, line.align === 'center' ? width / 2 : 18, line.y);
  }
  const texture = new THREE.CanvasTexture(canvas); texture.colorSpace = THREE.SRGBColorSpace; return texture;
}
function makeTextPlane(lines, width, height, canvasHeight = 160) {
  // Match the label texture's aspect to its world-space plane. A fixed
  // 1,280 px texture stretches wide worksheet titles horizontally.
  const canvasWidth = Math.max(1, Math.round(canvasHeight * width / height));
  return new THREE.Mesh(new THREE.PlaneGeometry(width, height), new THREE.MeshBasicMaterial({
    map: canvasLabel(lines, canvasWidth, canvasHeight), transparent: true, depthWrite: false, side: THREE.DoubleSide, toneMapped: false }));
}
function makeGlow(width, height, color) {
  return new THREE.Mesh(new THREE.PlaneGeometry(width + .6, height + .6), new THREE.ShaderMaterial({
    uniforms: { uColor: { value: new THREE.Color(color) }, uStrength: { value: .35 },
      uInner: { value: new THREE.Vector2(width / 2, height / 2) }, uSize: { value: new THREE.Vector2(width + .6, height + .6) } },
    vertexShader: 'varying vec2 vUv; void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0);}',
    fragmentShader: 'varying vec2 vUv; uniform vec3 uColor; uniform float uStrength; uniform vec2 uInner; uniform vec2 uSize;void main(){vec2 p=abs((vUv-.5)*uSize)-uInner;float d=max(p.x,p.y);float a=exp(-max(d,0.0)*14.0)*step(0.0,d)*uStrength;gl_FragColor=vec4(uColor,a);}',
    transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide }));
}
function curvedGeometry(width, height, uv) {
  const geometry = new THREE.PlaneGeometry(width, height, 24, 1), positions = geometry.attributes.position;
  for (let index = 0; index < positions.count; index++) {
    const x = positions.getX(index), angle = x / RADIUS;
    positions.setXYZ(index, RADIUS * Math.sin(angle), positions.getY(index), RADIUS * (1 - Math.cos(angle)));
  }
  if (uv) {
    const values = geometry.attributes.uv;
    for (let index = 0; index < values.count; index++) values.setXY(index,
      THREE.MathUtils.lerp(uv.u0, uv.u1, values.getX(index)), THREE.MathUtils.lerp(uv.v0, uv.v1, values.getY(index)));
  }
  geometry.computeVertexNormals(); return geometry;
}
function frame(width, height, color, curved = false) {
  const vertices = [], count = curved ? 32 : 1;
  const point = (x, y) => curved ? [RADIUS * Math.sin(x / RADIUS), y, RADIUS * (1 - Math.cos(x / RADIUS))] : [x, y, 0];
  for (let i = 0; i < count; i++) {
    const a = -width / 2 + width * i / count, b = -width / 2 + width * (i + 1) / count;
    vertices.push(...point(a, height / 2), ...point(b, height / 2), ...point(a, -height / 2), ...point(b, -height / 2));
  }
  vertices.push(...point(-width / 2, -height / 2), ...point(-width / 2, height / 2), ...point(width / 2, -height / 2), ...point(width / 2, height / 2));
  const geometry = new THREE.BufferGeometry(); geometry.setAttribute('position', new THREE.Float32BufferAttribute(vertices, 3));
  return new THREE.LineSegments(geometry, new THREE.LineBasicMaterial({ color, transparent: true, opacity: .8, depthWrite: false }));
}
function disposeObject(root) {
  root.traverse(object => {
    object.geometry?.dispose();
    for (const material of Array.isArray(object.material) ? object.material : object.material ? [object.material] : []) {
      if (material.map?.isCanvasTexture) material.map.dispose();
      material.dispose();
    }
  }); root.removeFromParent();
}
function sweepTextures() {
  const used = new Set(); scene.traverse(object => { if (object.material?.map) used.add(object.material.map); });
  const candidates = [...textureCache.entries()].filter(([, entry]) => entry.texture && !used.has(entry.texture)).sort((a, b) => a[1].touched - b[1].touched);
  for (const [key, entry] of candidates) {
    if (textureCache.size <= 64) break;
    entry.texture.image?.close?.(); entry.texture.dispose(); textureCache.delete(key);
  }
}
function drainLoads() {
  while (activeLoads < 2 && loadQueue.length) {
    const job = loadQueue.shift(); activeLoads++;
    job.run().finally(() => { activeLoads--; drainLoads(); setTimeout(sweepTextures, 0); });
  }
}
function loadTexture(url, maxWidth = 1024, region = null) {
  const key = `${url}#${maxWidth}#${region ? [region.x, region.y, region.width, region.height].join(',') : ''}`;
  if (textureCache.has(key)) { const cached = textureCache.get(key); cached.touched = performance.now(); return cached.promise; }
  const entry = { touched: performance.now(), texture: null };
  entry.promise = new Promise((resolve, reject) => {
    const run = async () => {
      try {
        const response = await fetch(safeImageUrl(url)); if (!response.ok) throw new Error(`Could not load image (${response.status}).`);
        let source = await createImageBitmap(await response.blob(), { imageOrientation: region ? 'none' : 'flipY', premultiplyAlpha: 'none' });
        if (region) {
          // Crop before downscaling: a 1,024 px detail must not be sampled from
          // a 1,024 px thumbnail of an 11,000 px capture.
          const crop = document.createElement('canvas'); crop.width = Math.max(1, Math.ceil(region.width)); crop.height = Math.max(1, Math.ceil(region.height));
          crop.getContext('2d').drawImage(source, 0, 0, source.width, source.height,
            -region.x, -region.y, region.sourceWidth || source.width, region.sourceHeight || source.height);
          source.close(); source = await createImageBitmap(crop, { imageOrientation: 'flipY', premultiplyAlpha: 'none' });
        }
        let bitmap = source;
        const limit = Math.min(maxWidth, renderer.capabilities.maxTextureSize);
        if (Math.max(source.width, source.height) > limit) {
          const ratio = limit / Math.max(source.width, source.height);
          bitmap = await createImageBitmap(source, { resizeWidth: Math.max(1, Math.round(source.width * ratio)), resizeHeight: Math.max(1, Math.round(source.height * ratio)), resizeQuality: 'high' });
          source.close();
        }
        const texture = new THREE.Texture(bitmap); texture.colorSpace = THREE.SRGBColorSpace; texture.needsUpdate = true;
        texture.anisotropy = Math.min(4, renderer.capabilities.getMaxAnisotropy()); entry.texture = texture; resolve(texture);
      } catch (error) { textureCache.delete(key); reject(error); }
    };
    loadQueue.push({ url, run, cancel() { textureCache.delete(key); reject(new DOMException('The visible viewport has changed.', 'AbortError')); } });
    drainLoads();
  }); textureCache.set(key, entry); return entry.promise;
}

function initScene() {
  scene = new THREE.Scene(); scene.background = new THREE.Color('#08111c'); scene.fog = new THREE.FogExp2('#08111c', .007);
  camera = new THREE.PerspectiveCamera(43, 1, .1, 180);
  renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'low-power' });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 1.5)); renderer.outputColorSpace = THREE.SRGBColorSpace; renderer.toneMapping = THREE.NoToneMapping;
  $('viewport').append(renderer.domElement);
  omissionTooltip = document.createElement('div'); omissionTooltip.className = 'omission-tooltip';
  omissionTooltip.setAttribute('role', 'tooltip'); omissionTooltip.hidden = true; document.body.append(omissionTooltip);
  // This is the sole wheel owner over sheet content. Capture phase runs before
  // OrbitControls' canvas listener; outside a sheet, OrbitControls still owns it.
  renderer.domElement.addEventListener('wheel', handleSheetWheel, { capture: true, passive: false });
  controls = new OrbitControls(camera, renderer.domElement); controls.enableDamping = !reducedMotion; controls.dampingFactor = .075;
  controls.minDistance = 2; controls.maxDistance = 800; controls.maxPolarAngle = Math.PI * .85; controls.rotateSpeed = .5; controls.panSpeed = .7;
  controls.addEventListener('start', () => { userMovedCamera = true; cameraTween = null; cancelHover(); stopFollowing(); });
  new ResizeObserver(resize).observe($('viewport')); resize();
  new ResizeObserver(() => { renderComparePanels(); scheduleCompareCrops(); }).observe($('comparePanels'));
  renderer.domElement.addEventListener('pointermove', handlePointerMove);
  renderer.domElement.addEventListener('pointerleave', () => { omissionTooltip.hidden = true; scheduleClear(); });
  renderer.domElement.addEventListener('pointerdown', event => { pointerDown = { x: event.clientX, y: event.clientY }; });
  renderer.domElement.addEventListener('pointerup', event => {
    if (!pointerDown || Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) > 7) return;
    const hit = hitAt(event);
    if (hit?.userData.sourceTile) { const tile = hit.userData.sourceTile; focusTile(tile.entry, hit.userData.region, true); }
    else if (hit?.userData.sheetId) focusSheet(hit.userData.sheetId, true);
    pointerDown = null;
  });
  requestAnimationFrame(tick);
}
function fitCamera(animate = false) {
  const verticalTangent = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
  const distance = Math.max(22, layout.width / 2 / (verticalTangent * camera.aspect * .82), layout.height / 2 / (verticalTangent * .78)) + 4;
  const position = new THREE.Vector3(0, .4, distance), target = new THREE.Vector3(0, 0, 1);
  moveCamera(position, target, animate);
}
function moveCamera(position, target, animate = true) {
  if (animate && !reducedMotion) cameraTween = { start: performance.now(), from: camera.position.clone(), to: position, fromTarget: controls.target.clone(), target };
  else { cameraTween = null; camera.position.copy(position); controls.target.copy(target); controls.update(); }
  userMovedCamera = false;
}
function focusCardCamera(sheetId, animate = true) {
  const card = sheetCards.get(sheetId); if (!card) return;
  scene.updateMatrixWorld(true);
  const target = card.getWorldPosition(new THREE.Vector3());
  const outward = new THREE.Vector3(0, 0, 1).applyQuaternion(card.getWorldQuaternion(new THREE.Quaternion()));
  const tangent = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
  const distance = Math.max(CARD_W / 2 / (tangent * camera.aspect * .82), CARD_H / 2 / (tangent * .77))
    * ($('compareDock').closest('.room').classList.contains('comparison-open') ? 1.08 : 1);
  moveCamera(target.clone().addScaledVector(outward, distance).add(new THREE.Vector3(0, .2, 0)), target, animate);
}
function focusSheet(id, manual = false, preview = false) {
  if (!data?.sheets.some(sheet => sheet.id === id)) return;
  if (manual) stopFollowing();
  clearDetails(); state.sheetId = id; state.focus = 'sheet'; sheetView(selectedSheet());
  updateSheets(); renderScene(); focusCardCamera(id); renderMinimap();
  if (!preview) announce(`Sheet ${selectedSheet().name}`);
}
function followTile(tile) {
  if (!tile || tile.status === 'omitted') return;
  const changeCard = state.sheetId !== tile.sheetId || state.focus === 'overview';
  clearDetails(); state.sheetId = tile.sheetId; state.focus = 'sheet';
  const sheet = selectedSheet();
  rememberView(sheet, followSheetViewport(sheet, AREA_W / AREA_H, sheetView(sheet), tile));
  updateSheets(); renderScene(); renderMinimap();
  if (changeCard) focusCardCamera(tile.sheetId);
}
function focusTile(tile, region = null, manual = false) {
  if (!tile) return;
  if (tile.status === 'omitted') { if (manual) focusSheet(tile.sheetId, true); setNotice(omissionDescription(tile)); announce(omissionDescription(tile)); return; }
  if (tile.comparison_method === 'png-sha256' && tile.equalAcrossAll && !Object.values(tile.images).some(image => image.url)) {
    if (manual) focusSheet(tile.sheetId, true);
    const description = tileEvidenceDescription(tile);
    setNotice(description); announce(description); return;
  }
  if (manual) stopFollowing();
  const compare = manual && (Boolean(state.selected) || tile.comparisonComplete && !tile.equalAcrossAll);
  if (!compare) clearDetails();
  state.sheetId = tile.sheetId; state.focus = 'tile';
  const inspect = region || inspectionRegions(tile, !tile.equalAcrossAll)[0];
  const area = inspect || { x: 0, y: 0, width: tile.width, height: tile.height };
  const width = Math.max(64, Math.min(1800, Math.max(area.width, area.height * AREA_W / AREA_H)) * 1.12);
  rememberView(selectedSheet(), { mode: 'manual', width,
    x: Math.max(0, tile.x + area.x - (width - area.width) / 2),
    y: Math.max(0, tile.y + area.y - (width * AREA_H / AREA_W - area.height) / 2) });
  clampView(); updateSheets(); renderScene(); focusCardCamera(tile.sheetId);
  if (compare) showDetails(tile.id, true, inspect);
}
function resize() {
  const { width, height } = $('viewport').getBoundingClientRect(); if (!width || !height) return;
  renderer.setSize(width, height, false); camera.aspect = width / height; camera.updateProjectionMatrix();
  if (!userMovedCamera) { if (state.focus === 'overview') fitCamera(); else focusCardCamera(state.sheetId, false); }
  updateSheetToolbarPositions();
}
function setCurvedPosition(group, x, y, z = 0) {
  const angle = x / RADIUS;
  group.position.set(RADIUS * Math.sin(angle), y, RADIUS * (1 - Math.cos(angle)) + z); group.rotation.y = -angle;
}
const fitIcons = {
  width: '<path d="M8 3h7l4 4v14H5V3zM15 3v5h4M3 13h18M3 13l3-3M3 13l3 3M21 13l-3-3M21 13l-3 3"/>',
  height: '<path d="M8 3h7l4 4v14H5V3zM15 3v5h4M12 5v14M12 5l-3 3M12 5l3 3M12 19l-3-3M12 19l3-3"/>',
  sheet: '<path d="M8 3h7l4 4v14H5V3zM15 3v5h4M8 10v3h3M8 10l4 4M16 18v-3h-3M16 18l-4-4"/>',
};
const fitLabels = { width: 'Fit width', height: 'Fit height', sheet: 'Fit whole sheet' };
function createSheetToolbar(sheet) {
  let toolbar = sheetToolbars.get(sheet.id);
  if (!toolbar) {
    toolbar = document.createElement('div'); toolbar.className = 'sheet-zoom-tools'; toolbar.dataset.sheetId = sheet.id;
    toolbar.setAttribute('role', 'group');
    for (const mode of ['width', 'height', 'sheet']) {
      const button = document.createElement('button'); button.type = 'button'; button.dataset.fit = mode;
      button.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true">${fitIcons[mode]}</svg>`;
      button.addEventListener('click', () => setSheetFit(sheet.id, mode)); toolbar.append(button);
    }
    toolbar.addEventListener('pointerenter', cancelHover);
    $('sheetZoomLayer').append(toolbar); sheetToolbars.set(sheet.id, toolbar);
  }
  toolbar.setAttribute('aria-label', `${sheet.name}: shared zoom for all four profiles`);
  const mode = sheetView(sheet).mode;
  for (const button of toolbar.children) {
    button.title = `${fitLabels[button.dataset.fit]} · ${sheet.name}`;
    button.setAttribute('aria-label', button.title);
    button.setAttribute('aria-pressed', String(button.dataset.fit === mode));
  }
}
function updateSheetToolbarPositions() {
  if (!camera || !renderer) return;
  scene.updateMatrixWorld(true);
  const width = renderer.domElement.clientWidth, height = renderer.domElement.clientHeight;
  for (const [id, toolbar] of sheetToolbars) {
    const card = sheetCards.get(id); if (!card) { toolbar.hidden = true; continue; }
    const x = CARD_W / 2 - .22;
    const point = new THREE.Vector3(RADIUS * Math.sin(x / RADIUS), CARD_H / 2 - .14, RADIUS * (1 - Math.cos(x / RADIUS)) + .13);
    const anchor = card.localToWorld(point).project(camera);
    const left = card.localToWorld(new THREE.Vector3(-CARD_W / 2, CARD_H / 2, 0)).project(camera);
    const projectedWidth = Math.abs(anchor.x - left.x) * width / 2;
    toolbar.hidden = anchor.z < -1 || anchor.z > 1 || Math.abs(anchor.x) > 1.1 || Math.abs(anchor.y) > 1.1 || projectedWidth < 95;
    toolbar.style.left = `${(anchor.x + 1) * width / 2}px`;
    toolbar.style.top = `${(1 - anchor.y) * height / 2}px`;
    toolbar.style.transform = `translateX(-100%) scale(${Math.max(.65, Math.min(1, projectedWidth / 520))})`;
  }
}
function makeSheetCard(sheet, placement) {
  const card = new THREE.Group(); setCurvedPosition(card, placement.x, placement.y); scene.add(card);
  card.userData.sheetId = sheet.id;
  const backing = new THREE.Mesh(curvedGeometry(CARD_W, CARD_H), new THREE.MeshBasicMaterial({ color: sheet.id === state.sheetId ? 0x132d40 : 0x102334, side: THREE.DoubleSide }));
  backing.userData.sheetId = sheet.id; card.add(backing); hitTargets.push(backing);
  const edge = frame(CARD_W, CARD_H, sheet.id === state.sheetId ? 0x76c7cf : 0x34566c, true); edge.position.z = .012; card.add(edge);
  const differences = sheet.tiles.filter(tile => tile.comparisonComplete && !tile.equalAcrossAll).length;
  const arrived = sheet.tiles.filter(tile => Object.keys(tile.images).length).length;
  const planned = sheet.tiles.filter(tile => tile.status !== 'omitted').length;
  const omitted = Number(sheet.capture_coverage?.omitted_tiles || 0);
  const fullFallback = sheet.capture_coverage?.fallback_reason;
  const title = makeTextPlane([{ text: sheet.name, y: 68, size: 55 },
    { text: `${arrived} / ${planned} capture jobs · ${differences} with differences${omitted ? ` · ${omitted} omitted` : fullFallback ? ' · full fallback' : ''}`, y: 130, size: 29, color: fullFallback ? '#ffc987' : differences ? '#ffabbc' : '#8eadc2' }], CARD_W - 4.1, .68);
  // Leave the right side of each header for the projected accessible fit tools.
  title.geometry.dispose(); title.geometry = curvedGeometry(CARD_W - 4.1, .68);
  setCurvedPosition(title, -1.775, CARD_H / 2 - .43, .045); card.add(title);
  sheetCards.set(sheet.id, card); boards.push(card);
  createSheetToolbar(sheet);
  const view = sheetView(sheet);
  // Small cards retain every planned boundary in one inexpensive line mesh.
  // Detailed image objects are bounded; focusing a card reveals its viewport.
  const imageLimit = sheet.id === state.sheetId ? 48 : Math.floor(48 / Math.max(1, data.sheets.length - 1));
  const shown = visibleTiles(sheet, view, imageLimit);
  data.profiles.forEach((profile, index) => makeBoard(card, sheet, profile, index, shown, view));
}
function makeBoard(card, sheet, profile, profileIndex, shown, view) {
  const board = new THREE.Group(); setCurvedPosition(board, (profileIndex - (data.profiles.length - 1) / 2) * BOARD_STEP, -.39, .035); card.add(board);
  board.userData.pixelBoard = { sheet, profile, view };
  const background = new THREE.Mesh(curvedGeometry(BOARD_W, BOARD_H), new THREE.MeshBasicMaterial({ color: profileIndex === 0 ? 0x132b3c : 0x0b1e2c, side: THREE.DoubleSide }));
  background.userData.sheetId = sheet.id; background.userData.pixelBoard = { board, sheet, profile, view }; board.add(background); hitTargets.push(background);
  const title = makeTextPlane([{ text: profile.label, y: 65, size: 61, align: 'center', color: '#a9c5d6' }], BOARD_W - .16, .3, 100);
  title.geometry.dispose(); title.geometry = curvedGeometry(BOARD_W - .16, .3); title.position.set(0, BOARD_H / 2 - .22, .04); board.add(title);
  const px = AREA_W / view.width;
  const lines = [], colors = [];
  function point(x, y) { const angle = x / RADIUS; return [RADIUS * Math.sin(angle), y, RADIUS * (1 - Math.cos(angle)) + .02]; }
  for (const entry of sheet.tiles) {
    const clip = clippedTile(profileRectangle(entry, profile.id), view); if (clip.width <= 0 || clip.height <= 0) continue;
    const left = (clip.x - view.x) * px - AREA_W / 2, right = left + clip.width * px;
    const top = AREA_Y + AREA_H / 2 - (clip.y - view.y) * px, bottom = top - clip.height * px;
    const status = imageStatus(entry, profile.id);
    const color = new THREE.Color(C[status]);
    if (entry.status === 'omitted') {
      // A broken axis denotes absent evidence, never equality or pending work.
      const center = (top + bottom) / 2, amplitude = Math.min(.025, (top - bottom) / 10);
      for (const offset of [-.35, .35]) for (let index = 0; index < 18; index++) {
        const x1 = left + (right - left) * index / 18, x2 = left + (right - left) * (index + 1) / 18;
        lines.push(...point(x1, center + offset * (top - bottom) + (index % 2 ? amplitude : -amplitude)),
          ...point(x2, center + offset * (top - bottom) + (index % 2 ? -amplitude : amplitude)));
        colors.push(color.r, color.g, color.b, color.r, color.g, color.b);
      }
      if (top - bottom > .1) {
        const labelHeight = Math.min(.16, (top - bottom) * .5);
        const label = makeTextPlane([{ text: `${entry.omitted_count} omitted · not checked`, y: 66, size: 44, align: 'center', color: '#cfcbdf' }], right - left, labelHeight, 100);
        label.geometry.dispose(); label.geometry = curvedGeometry(right - left, labelHeight);
        setCurvedPosition(label, (left + right) / 2, center, .025); board.add(label);
      }
      continue;
    }
    if (status === 'hash-equal' && top - bottom > .04) {
      // A hash mark distinguishes verified equality without a preview from
      // pending captures, pixel previews and explicitly omitted regions.
      const size = Math.min(.1, (top - bottom) * .65, (right - left) * .12);
      const x = left + size * .4, y = top - size * .2;
      for (const offset of [.3, .7]) {
        lines.push(...point(x + size * offset, y), ...point(x + size * offset, y - size));
        lines.push(...point(x, y - size * offset), ...point(x + size, y - size * offset));
        colors.push(color.r, color.g, color.b, color.r, color.g, color.b,
          color.r, color.g, color.b, color.r, color.g, color.b);
      }
    }
    for (const [x1, y1, x2, y2] of [[left, top, right, top], [right, top, right, bottom], [right, bottom, left, bottom], [left, bottom, left, top]]) {
      lines.push(...point(x1, y1), ...point(x2, y2)); colors.push(color.r, color.g, color.b, color.r, color.g, color.b);
    }
  }
  const geometry = new THREE.BufferGeometry(); geometry.setAttribute('position', new THREE.Float32BufferAttribute(lines, 3)); geometry.setAttribute('color', new THREE.Float32BufferAttribute(colors, 3));
  board.add(new THREE.LineSegments(geometry, new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: .72, depthWrite: false })));
  for (const entry of shown) {
    // Pending tiles share the outline mesh and are picked mathematically from
    // the board. No per-tile textures/materials are allocated before arrival.
    if (!entry.images[profile.id]?.url) continue;
    const rectangle = profileRectangle(entry, profile.id);
    let clip = clippedTile(rectangle, view);
    const offscreen = clip.width <= 0 || clip.height <= 0;
    if (offscreen) clip = { ...rectangle, u0: 0, u1: 1, v0: 0, v1: 1 };
    const width = offscreen ? .02 : clip.width * px, height = offscreen ? .02 : clip.height * px;
    const x = THREE.MathUtils.clamp((clip.x - view.x + clip.width / 2) * px - AREA_W / 2, -AREA_W / 2, AREA_W / 2);
    const y = THREE.MathUtils.clamp(AREA_Y + AREA_H / 2 - (clip.y - view.y + clip.height / 2) * px, AREA_Y - AREA_H / 2, AREA_Y + AREA_H / 2);
    const group = new THREE.Group(); setCurvedPosition(group, x, y, .032); board.add(group);
    const backing = new THREE.Mesh(curvedGeometry(width, height), new THREE.MeshBasicMaterial({ color: 0x213d4e, transparent: true, opacity: .13, side: THREE.DoubleSide })); group.add(backing);
    const image = new THREE.Mesh(curvedGeometry(width, height, clip), new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0, depthWrite: false, side: THREE.DoubleSide, toneMapped: false, fog: false })); image.position.z = .007; group.add(image);
    const rim = frame(width, height, C.waiting, true); rim.position.z = .016; group.add(rim);
    const glow = makeGlow(width, height, C.waiting); glow.position.z = .012; group.add(glow);
    const tile = { group, image, rim, glow, clip, rectangle, offscreen, entry, sheet, view, imageData: entry.images[profile.id], p: profileIndex, profile, width, height, loaded: false };
    backing.userData.sourceTile = tile; image.userData.sourceTile = tile;
    for (const item of [backing, image, rim, glow]) item.visible = !offscreen;
    if (!offscreen) hitTargets.push(backing, image); sourceTiles.push(tile);
    // Regions are inspection crops of this exact capture, never extra jobs.
    if (entry.comparisonComplete && !entry.equalAcrossAll) {
      for (const region of inspectionRegions(entry, false)) {
        if (region.exact && !region.changed_pixels) continue;
        const regionClip = clippedTile({ x: rectangle.x + region.x, y: rectangle.y + region.y, width: region.width, height: region.height }, view);
        if (regionClip.width <= 0 || regionClip.height <= 0) continue;
        const regionW = regionClip.width * px, regionH = regionClip.height * px;
        const regionRoot = new THREE.Group();
        setCurvedPosition(regionRoot, (regionClip.x - view.x + regionClip.width / 2) * px - AREA_W / 2,
          AREA_Y + AREA_H / 2 - (regionClip.y - view.y + regionClip.height / 2) * px, .065);
        const regionColor = region.exact ? profile.id !== 'original' && region.pixel_classes?.[profile.id] === region.pixel_classes?.original ? C.equal : C.different : 0x94afbe;
        const edge = frame(regionW, regionH, regionColor, true); edge.material.opacity = region.exact ? .94 : .35; regionRoot.add(edge);
        const target = new THREE.Mesh(curvedGeometry(regionW, regionH), new THREE.MeshBasicMaterial({ color: regionColor, transparent: true, opacity: .025, depthWrite: false, side: THREE.DoubleSide }));
        target.userData.sourceTile = tile; target.userData.region = region; regionRoot.add(target); board.add(regionRoot); hitTargets.push(target);
      }
    }
  }
}
function renderScene(force = false) {
  if (!data) return;
  const views = data.sheets.map(sheet => [sheet.id, sheetView(sheet)]);
  const signature = JSON.stringify([state.groupId, state.sheetId, state.focus, views, data.profiles, data.sheets]);
  if (!force && signature === lastSignature) return;
  lastSignature = signature; const generation = ++sceneGeneration;
  const selection = state.selected, regionId = state.regionId, pinned = state.pinned;
  clearDetails(!selection); boards.splice(0).forEach(disposeObject); sourceTiles.length = 0; hitTargets.length = 0; sheetCards.clear();
  for (const [id, toolbar] of sheetToolbars) if (!data.sheets.some(sheet => sheet.id === id)) { toolbar.remove(); sheetToolbars.delete(id); }
  layout = sheetLayout(data.sheets); RADIUS = Math.max(24, layout.width / 1.6);
  camera.far = Math.max(180, layout.width * 10, layout.height * 10); camera.updateProjectionMatrix();
  data.sheets.forEach((sheet, index) => makeSheetCard(sheet, layout.cards[index]));
  const required = new Set(sourceTiles.filter(tile => tile.imageData?.url).map(tile => tile.imageData.previewUrl || tile.imageData.url));
  const selectedEntry = tileAt(selection);
  if (selectedEntry) for (const image of Object.values(selectedEntry.images)) { required.add(image.fullUrl || image.url); if (image.maskUrl) required.add(image.maskUrl); }
  for (let index = loadQueue.length - 1; index >= 0; index--) if (!required.has(loadQueue[index].url)) loadQueue.splice(index, 1)[0].cancel();
  state.ready = true; $('emptyScene').hidden = data.sheets.length > 0;
  renderTileButtons(visibleTiles(selectedSheet(), sheetView(selectedSheet()), 96));
  const loads = sourceTiles.filter(tile => tile.imageData?.url).map(async tile => {
    try {
      const texture = await loadTexture(tile.imageData.previewUrl || tile.imageData.url, tile.sheet.id === state.sheetId ? 1024 : 512);
      if (generation !== sceneGeneration) return;
      tile.image.material.map = texture; tile.image.material.needsUpdate = true; tile.loaded = true;
    } catch (error) { if (generation === sceneGeneration && error.name !== 'AbortError') setNotice(error.message, true); }
  });
  Promise.all(loads).then(() => { if (generation === sceneGeneration) sweepTextures(); });
  if (selectedEntry && sourceTiles.some(tile => tile.entry.id === selection)) {
    showDetails(selection, false, inspectionRegions(selectedEntry).find(region => region.id === regionId));
    setPinned(pinned);
  } else if (selection) clearDetails();
  updatePanControls(); renderMinimap();
  updateSheetToolbarPositions();
  if (!lastFocusKey) { fitCamera(); lastFocusKey = state.groupId; }
}

function comparisonColor(group, entry) {
  if (group.original || group.pixelClass == null) return C.received;
  if (entry.blank_margin_normalized && Number(entry.images[group.ids[0]]?.changedPixels) === 0) return C.equal;
  return group.pixelClass === entry.images.original?.pixelClass ? C.equal : C.different;
}

function resetCompareZoom() {
  compareZoom = 1;
  compareCenter = {
    x: compareRegion.x + compareRegion.width / 2,
    y: compareRegion.y + compareRegion.height / 2,
  };
  for (const panel of detailPanels) {
    panel.fullCrop?.bitmap.close(); panel.fullCrop = null; panel.fullReady = false;
    panel.mask?.bitmap.close(); panel.mask = null;
  }
  renderComparePanels();
  scheduleCompareCrops();
}

function clampCompareCenter() {
  compareCenter.x = THREE.MathUtils.clamp(compareCenter.x, compareRegion.x,
    compareRegion.x + compareRegion.width);
  compareCenter.y = THREE.MathUtils.clamp(compareCenter.y, compareRegion.y,
    compareRegion.y + compareRegion.height);
}

function renderComparePanels() {
  for (const panel of detailPanels) {
    const { stage, canvas, preview, fullCrop, mask, representative } = panel;
    const width = stage.clientWidth, height = stage.clientHeight;
    if (!width || !height) continue;
    const ratio = Math.min(devicePixelRatio, 2);
    const pixelWidth = Math.max(1, Math.round(width * ratio));
    const pixelHeight = Math.max(1, Math.round(height * ratio));
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth;
      canvas.height = pixelHeight;
    }
    const ctx = canvas.getContext('2d');
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, width, height);
    if (!preview && !fullCrop) continue;
    const fit = Math.min(width / compareRegion.width, height / compareRegion.height);
    const scale = fit * compareZoom;
    const x = width / 2 - compareCenter.x * scale;
    const y = height / 2 - compareCenter.y * scale;
    const imageWidth = Number(representative.width) || preview?.naturalWidth || 1;
    const imageHeight = Number(representative.height) || preview?.naturalHeight || 1;
    if (preview) ctx.drawImage(preview, x, y, imageWidth * scale, imageHeight * scale);
    if (fullCrop) ctx.drawImage(fullCrop.bitmap, x + fullCrop.x * scale, y + fullCrop.y * scale,
      fullCrop.width * scale, fullCrop.height * scale);
    if (state.highlightDiff && cropHasEnoughDetail(mask, visibleComparisonCrop(panel))) {
      ctx.globalAlpha = .7;
      ctx.drawImage(mask.bitmap, x + mask.x * scale, y + mask.y * scale,
        mask.width * scale, mask.height * scale);
      ctx.globalAlpha = 1;
    }
    stage.classList.toggle('zoomed', compareZoom > 1);
  }
}

function visibleComparisonCrop(panel) {
  const width = panel.stage.clientWidth, height = panel.stage.clientHeight;
  if (!width || !height) return null;
  const fit = Math.min(width / compareRegion.width, height / compareRegion.height);
  const scale = fit * compareZoom;
  const halfWidth = width / (2 * scale), halfHeight = height / (2 * scale);
  const sourceWidth = Number(panel.representative.width) || 0;
  const sourceHeight = Number(panel.representative.height) || 0;
  const x = Math.max(0, Math.floor(compareCenter.x - halfWidth * 1.12));
  const y = Math.max(0, Math.floor(compareCenter.y - halfHeight * 1.12));
  const right = Math.min(sourceWidth, Math.ceil(compareCenter.x + halfWidth * 1.12));
  const bottom = Math.min(sourceHeight, Math.ceil(compareCenter.y + halfHeight * 1.12));
  if (right <= x || bottom <= y) return null;
  const cropWidth = right - x, cropHeight = bottom - y;
  const deviceScale = Math.min(devicePixelRatio, 2);
  const targetWidth = cropWidth * scale * deviceScale;
  const targetHeight = cropHeight * scale * deviceScale;
  const limit = Math.min(1, 2048 / targetWidth, 2048 / targetHeight,
    Math.sqrt(1000000 / (targetWidth * targetHeight)));
  return { x, y, width: cropWidth, height: cropHeight,
    outputWidth: Math.max(1, Math.round(targetWidth * limit)),
    outputHeight: Math.max(1, Math.round(targetHeight * limit)), scale: scale * deviceScale };
}

function cropHasEnoughDetail(crop, desired) {
  return crop && desired && crop.x <= desired.x && crop.y <= desired.y
    && crop.x + crop.width >= desired.x + desired.width
    && crop.y + crop.height >= desired.y + desired.height
    && crop.bitmap.width / crop.width >= desired.outputWidth / desired.width * .8;
}

function scheduleCompareCrops() {
  ++compareCropToken;
  compareCropAbort?.abort();
  compareCropQueue.length = 0;
  clearTimeout(compareCropTimer);
  compareCropTimer = setTimeout(() => {
    const token = compareCropToken;
    for (const panel of detailPanels) {
      const desired = visibleComparisonCrop(panel);
      if (!desired) continue;
      if (state.highlightDiff && panel.representative.maskUrl && !cropHasEnoughDetail(panel.mask, desired)) {
        compareCropQueue.push({ panel, field: 'mask', url: panel.representative.maskUrl, crop: desired, token });
      }
      const { preview, representative } = panel;
      const previewScale = preview && preview.naturalWidth / Number(representative.width);
      if (representative.fullUrl && (!previewScale || desired.scale > previewScale * 1.2)
          && !cropHasEnoughDetail(panel.fullCrop, desired)) {
        compareCropQueue.push({ panel, field: 'fullCrop', url: representative.fullUrl, crop: desired, token });
      }
    }
    void drainCompareCrops();
  }, 180);
}

async function drainCompareCrops() {
  if (compareCropBusy) return;
  compareCropBusy = true;
  try {
    while (compareCropQueue.length) {
      const job = compareCropQueue.shift();
      if (job.token !== compareCropToken) continue;
      const controller = new AbortController();
      compareCropAbort = controller;
      try {
        const response = await fetch(safeImageUrl(job.url), { signal: controller.signal });
        if (!response.ok) throw new Error(`Could not load comparison crop (${response.status}).`);
        const blob = await response.blob();
        const crop = job.crop;
        const bitmap = await createImageBitmap(blob, crop.x, crop.y, crop.width, crop.height, {
          resizeWidth: crop.outputWidth, resizeHeight: crop.outputHeight, resizeQuality: 'high',
          imageOrientation: 'none', premultiplyAlpha: 'none',
        });
        if (job.token !== compareCropToken) { bitmap.close(); continue; }
        job.panel[job.field]?.bitmap.close();
        job.panel[job.field] = { ...crop, bitmap };
        if (job.field === 'fullCrop') job.panel.fullReady = true;
        renderComparePanels();
      } catch (error) {
        if (job.token === compareCropToken && error.name !== 'AbortError') setNotice(error.message, true);
      } finally { if (compareCropAbort === controller) compareCropAbort = null; }
    }
  } finally { compareCropBusy = false; }
}

function zoomComparison(event) {
  event.preventDefault();
  const stage = event.currentTarget;
  const bounds = stage.getBoundingClientRect();
  const fit = Math.min(bounds.width / compareRegion.width, bounds.height / compareRegion.height);
  if (!fit) return;
  const x = event.clientX - bounds.left - bounds.width / 2;
  const y = event.clientY - bounds.top - bounds.height / 2;
  const before = { x: compareCenter.x + x / (fit * compareZoom), y: compareCenter.y + y / (fit * compareZoom) };
  const next = THREE.MathUtils.clamp(compareZoom * Math.exp(-event.deltaY * .0015), 1, 24);
  compareZoom = next;
  compareCenter = next === 1 ? {
    x: compareRegion.x + compareRegion.width / 2,
    y: compareRegion.y + compareRegion.height / 2,
  } : {
    x: before.x - x / (fit * next),
    y: before.y - y / (fit * next),
  };
  clampCompareCenter();
  renderComparePanels();
  scheduleCompareCrops();
}

function makeDetailPanel(group, entry, generation) {
  const representative = entry.images[group.ids[0]];
  if (!representative) return;
  const color = comparisonColor(group, entry);
  const sameAsOriginal = group.pixelClass != null && (
    group.pixelClass === entry.images.original?.pixelClass
    || entry.blank_margin_normalized && Number(representative.changedPixels) === 0);
  const element = document.createElement('div');
  element.className = `compare-panel compare-${group.original ? 'received' : sameAsOriginal ? 'equal' : 'different'}`;
  const title = document.createElement('strong'); title.className = 'compare-panel-title'; title.textContent = labelGroup(group);
  const stage = document.createElement('div'); stage.className = 'compare-panel-stage';
  stage.setAttribute('aria-label', `${labelGroup(group)}. Scroll to zoom all comparison images, drag to pan.`);
  const canvas = document.createElement('canvas'); stage.append(canvas);
  const status = document.createElement('span'); status.className = 'compare-panel-status';
  const blankMargin = Number(representative.blankMarginPixels ?? representative.blank_margin_pixels ?? 0);
  status.textContent = group.original ? 'Visual reference' : blankMargin > 0 && entry.blank_margin_normalized
    ? `${blankMargin.toLocaleString('en-US')} blank margin pixels · content equal`
    : group.pixelClass == null ? 'Comparison pending' : sameAsOriginal ? 'Pixels equal to original'
      : `${Number(representative.changedPixels || 0).toLocaleString('en-US')} differing pixels`;
  if (representative.url && !representative.fullUrl) status.textContent += ' · saved preview';
  element.append(title, stage, status); $('comparePanels').append(element);
  const panel = { element, stage, canvas, group, entry, representative, sameAsOriginal,
    preview: null, fullCrop: null, mask: null, fullReady: false, clip: compareRegion };
  detailPanels.push(panel);
  stage.addEventListener('wheel', zoomComparison, { passive: false });
  let drag = null;
  stage.addEventListener('pointerdown', event => {
    if (compareZoom <= 1) return;
    drag = { x: event.clientX, y: event.clientY, center: { ...compareCenter } };
    stage.setPointerCapture(event.pointerId); stage.classList.add('dragging');
  });
  stage.addEventListener('pointermove', event => {
    if (!drag) return;
    const fit = Math.min(stage.clientWidth / compareRegion.width, stage.clientHeight / compareRegion.height);
    compareCenter = { x: drag.center.x - (event.clientX - drag.x) / (fit * compareZoom),
      y: drag.center.y - (event.clientY - drag.y) / (fit * compareZoom) };
    clampCompareCenter();
    renderComparePanels();
    scheduleCompareCrops();
  });
  const endDrag = () => { drag = null; stage.classList.remove('dragging'); };
  stage.addEventListener('pointerup', endDrag); stage.addEventListener('pointercancel', endDrag);
  for (const id of group.ids) {
    const source = sourceTiles.find(tile => tile.entry.id === entry.id && tile.profile.id === id && !tile.offscreen);
    if (source) makeConnection(source, panel, color);
  }
  const previewUrl = representative.previewUrl || (representative.url === representative.fullUrl ? null : representative.url);
  if (previewUrl) {
    try {
      const image = new Image();
      image.onload = () => {
        if (generation !== selectedGeneration) return;
        panel.preview = image;
        renderComparePanels(); scheduleCompareCrops();
      };
      image.onerror = () => { if (generation === selectedGeneration) setNotice(`Could not load ${labelGroup(group)} preview.`, true); };
      image.src = safeImageUrl(previewUrl);
    } catch (error) { setNotice(error.message, true); }
  }
}

function makeConnection(source, panel, color) {
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('fill', 'none');
  path.setAttribute('stroke', `#${color.toString(16).padStart(6, '0')}`);
  path.setAttribute('stroke-width', '1.4');
  path.setAttribute('stroke-opacity', '.65');
  $('comparisonLinks').append(path);
  connections.push({ source, panel, path });
}

function showDetails(id, animate = true, region = null) {
  cancelHover();
  const originalEntry = tileAt(id);
  if (!originalEntry || !state.ready || !sourceTiles.some(tile => tile.entry.id === id)) return;
  const entry = inspectionEntry(originalEntry, region, data.profiles);
  const signature = JSON.stringify([id, region?.id, entry.comparisonComplete, entry.images]);
  if (state.selected === id && signature === detailSignature) return;
  clearDetails(false); detailSignature = signature;
  const generation = ++selectedGeneration;
  state.selected = id; state.regionId = region?.id ?? null; setPinned(true);
  compareRegion = region || { x: 0, y: 0,
    width: Math.max(1, ...Object.values(entry.images).map(image => Number(image.width) || 0)),
    height: Math.max(1, ...Object.values(entry.images).map(image => Number(image.height) || 0)) };
  const groups = groupedPanels(entry, data.profiles);
  $('comparePanels').style.setProperty('--compare-count', String(Math.max(1, groups.length)));
  groups.forEach(group => makeDetailPanel(group, entry, generation));
  resetCompareZoom();
  $('selectionBar').hidden = false;
  $('selectionText').textContent = `${entry.sheetName} · ${entry.address}`;
  const result = entry.blank_margin_normalized ? 'Blank canvas margin only'
    : entry.comparisonComplete ? entry.equalAcrossAll ? 'Identical' : 'Difference'
      : region ? 'Region without separate classification' : 'Comparison pending';
  $('selectionSummary').textContent = `${groups.length} image groups · ${result} · ${region ? `Inspection window ${Math.round(region.width)} × ${Math.round(region.height)} px` : 'Full capture'}`;
  clearTimeout(dockCloseTimer);
  $('compareDock').setAttribute('aria-hidden', 'false');
  $('compareDock').closest('.room').classList.add('comparison-open');
  renderEvidenceActions();
  announce($('selectionText').textContent + ': ' + $('selectionSummary').textContent);
  if (!animate) renderComparePanels();
}

function clearDetails(closeDock = true) {
  ++selectedGeneration; cancelHover(); state.selected = null; state.regionId = null; state.pinned = false; detailSignature = '';
  ++compareCropToken; compareCropAbort?.abort(); compareCropQueue.length = 0; clearTimeout(compareCropTimer);
  detailPanels.splice(0).forEach(panel => { panel.fullCrop?.bitmap.close(); panel.mask?.bitmap.close(); });
  connections.splice(0).forEach(link => link.path.remove());
  $('evidenceActions').hidden = true;
  if (!closeDock) {
    $('comparePanels').replaceChildren();
    return;
  }
  $('selectionBar').hidden = true;
  $('compareDock').setAttribute('aria-hidden', 'true');
  $('compareDock').closest('.room').classList.remove('comparison-open');
  clearTimeout(dockCloseTimer);
  dockCloseTimer = setTimeout(() => {
    if (!$('compareDock').closest('.room').classList.contains('comparison-open')) $('comparePanels').replaceChildren();
  }, 500);
}
function setPinned(value) { state.pinned = Boolean(value && state.selected); }
function cancelHover() { clearTimeout(hoverTimer); pendingHover = null; }
function scheduleClear() { cancelHover(); }
function intersectionAt(event) {
  const rect = renderer.domElement.getBoundingClientRect(); pointer.set((event.clientX - rect.left) / rect.width * 2 - 1, -((event.clientY - rect.top) / rect.height) * 2 + 1); raycaster.setFromCamera(pointer, camera);
  return raycaster.intersectObjects(hitTargets, false)[0];
}
function sheetPointAt(event) {
  const hit = intersectionAt(event); if (!hit) return null;
  let object = hit.object;
  while (object && !object.userData.pixelBoard) object = object.parent;
  const info = object?.userData.pixelBoard; if (!info) return null;
  const board = info.board || object, local = board.worldToLocal(hit.point.clone());
  const u = (RADIUS * Math.asin(THREE.MathUtils.clamp(local.x / RADIUS, -1, 1)) + AREA_W / 2) / AREA_W;
  const v = (AREA_Y + AREA_H / 2 - local.y) / AREA_H;
  if (u < 0 || u > 1 || v < 0 || v > 1) return null;
  const top = board.localToWorld(new THREE.Vector3(0, AREA_Y + AREA_H / 2, 0)).project(camera);
  const bottom = board.localToWorld(new THREE.Vector3(0, AREA_Y - AREA_H / 2, 0)).project(camera);
  const height = Math.max(1, Math.hypot((top.x - bottom.x) * renderer.domElement.clientWidth / 2, (top.y - bottom.y) * renderer.domElement.clientHeight / 2));
  return { sheet: info.sheet, anchor: { x: u, y: v }, screenHeight: height };
}
function handleSheetWheel(event) {
  if (!state.ready) return;
  const point = sheetPointAt(event); if (!point) return;
  event.preventDefault(); event.stopImmediatePropagation();
  stopFollowing(); clearDetails(); cameraTween = null; userMovedCamera = true;
  state.sheetId = point.sheet.id;
  const view = sheetView(point.sheet);
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? point.screenHeight : 1;
  const deltaX = event.deltaX * unit, deltaY = event.deltaY * unit;
  const next = event.ctrlKey
    ? zoomSheetViewport(point.sheet, AREA_W / AREA_H, view, Math.exp(THREE.MathUtils.clamp(deltaY, -1000, 1000) * .002), point.anchor)
    : scrollSheetViewport(point.sheet, AREA_W / AREA_H, view, deltaX * view.height / point.screenHeight, deltaY * view.height / point.screenHeight, event.shiftKey);
  rememberView(point.sheet, next); updateSheets(); renderScene(); renderMinimap();
}
function hitAt(event) {
  const hit = intersectionAt(event); if (!hit) return;
  const info = hit.object.userData.pixelBoard;
  if (info) {
    const local = info.board.worldToLocal(hit.point.clone()), px = AREA_W / info.view.width;
    const x = (RADIUS * Math.asin(THREE.MathUtils.clamp(local.x / RADIUS, -1, 1)) + AREA_W / 2) / px + info.view.x;
    const y = (AREA_Y + AREA_H / 2 - local.y) / px + info.view.y;
    const entry = info.sheet.tiles.find(tile => { const r = profileRectangle(tile, info.profile.id); return x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height; });
    if (entry) return { userData: { sourceTile: { entry, profile: info.profile, group: info.board } } };
  }
  return hit.object;
}
function handlePointerMove(event) {
  if (!state.ready || event.buttons) return;
  const hit = hitAt(event); renderer.domElement.style.cursor = hit ? 'pointer' : 'grab';
  const entry = hit?.userData.sourceTile?.entry;
  const noPreview = entry && Object.keys(entry.images).some(profile => imageStatus(entry, profile) === 'hash-equal');
  omissionTooltip.hidden = !entry || !(entry.status === 'omitted' || noPreview || entry.context_for?.length);
  if (!omissionTooltip.hidden) {
    omissionTooltip.textContent = tileEvidenceDescription(entry);
    omissionTooltip.style.left = `${Math.max(8, Math.min(event.clientX + 16, innerWidth - 380))}px`;
    omissionTooltip.style.top = `${Math.max(8, Math.min(event.clientY + 16, innerHeight - omissionTooltip.offsetHeight - 8))}px`;
    if (entry.status === 'omitted' || noPreview) { cancelHover(); return; }
  }
  if (!state.selected) { cancelHover(); return; }
  const tile = hit?.userData.sourceTile; if (!tile) { scheduleClear(); return; }
  const region = hit.userData.region, key = `${tile.entry.id}:${region?.id || ''}`;
  if ((state.selected === tile.entry.id && state.regionId === (region?.id || null)) || pendingHover === key) return;
  cancelHover(); pendingHover = key; hoverTimer = setTimeout(() => showDetails(tile.entry.id, true, region), 125);
}
function renderTileButtons(shown) {
  $('tileButtons').replaceChildren(...shown.map(entry => {
    const button = document.createElement('button'); button.textContent = entry.address || entry.id; button.dataset.tile = entry.id;
    if (entry.status === 'omitted') {
      button.textContent = `〰 ${entry.omitted_count} omitted`; button.classList.add('omitted');
      button.title = omissionDescription(entry); button.setAttribute('aria-label', button.title);
      button.addEventListener('focus', () => { setNotice(button.title); announce(button.title); });
      button.addEventListener('click', () => focusTile(entry, null, true)); return button;
    }
    if (entry.comparison_method === 'png-sha256' && entry.equalAcrossAll) {
      button.classList.add('hash-equal');
      button.textContent = `# ${button.textContent}`;
      button.title = tileEvidenceDescription(entry);
      button.setAttribute('aria-label', button.title);
      button.addEventListener('focus', () => { setNotice(button.title); announce(button.title); });
    } else if (entry.context_for?.length) {
      button.title = tileEvidenceDescription(entry);
    }
    button.classList.toggle('different', Object.keys(entry.images).some(profile => imageStatus(entry, profile) === 'different'));
    button.addEventListener('focus', () => { if (state.selected) showDetails(entry.id); });
    button.addEventListener('click', () => focusTile(entry, null, true)); return button;
  }));
}
function renderEvidenceActions() {
  $('evidenceActions').hidden = !state.selected;
  $('evidenceActions').replaceChildren(...detailPanels.map(panel => { const button = document.createElement('button'); button.textContent = `${labelGroup(panel.group)} ↗`; button.addEventListener('click', () => openEvidence(panel)); return button; }));
}
function openEvidence(panel) {
  const image = panel.representative;
  $('fullImage').src = safeImageUrl(image.fullUrl || image.url); $('imageTitle').textContent = labelGroup(panel.group);
  $('imageDescription').textContent = `${panel.entry.sheetName} · ${panel.entry.address} · ${image.fullUrl ? 'full capture' : 'saved preview'}`;
  $('imageScroll').classList.remove('native'); $('toggleImageSize').textContent = 'Show 1:1'; $('imageDialog').showModal(); setPinned(true);
}

function updateDetails() {
  if (!connections.length) return;
  const room = $('compareDock').closest('.room').getBoundingClientRect();
  const viewport = $('viewport').getBoundingClientRect();
  $('comparisonLinks').setAttribute('viewBox', `0 0 ${room.width} ${room.height}`);
  scene.updateMatrixWorld(true);
  for (const link of connections) {
    const point = link.source.group.getWorldPosition(new THREE.Vector3()).project(camera);
    if (point.z < -1 || point.z > 1 || Math.abs(point.x) > 1 || Math.abs(point.y) > 1) {
      link.path.setAttribute('visibility', 'hidden');
      continue;
    }
    link.path.removeAttribute('visibility');
    const startX = viewport.left - room.left + (point.x + 1) * viewport.width / 2;
    const startY = viewport.top - room.top + (1 - point.y) * viewport.height / 2;
    const target = link.panel.element.getBoundingClientRect();
    const endX = target.left - room.left + target.width / 2;
    const endY = target.bottom - room.top;
    const bend = Math.max(20, Math.abs(startY - endY) * .35);
    link.path.setAttribute('d', `M ${startX} ${startY} C ${startX} ${startY - bend}, ${endX} ${endY + bend}, ${endX} ${endY}`);
  }
}
function updateSourceTiles(now) {
  for (const tile of sourceTiles) {
    if (tile.placeholder) continue;
    const status = imageStatus(tile.entry, tile.profile.id), color = C[status];
    const changed = flashes.get(`${tile.entry.id}:${tile.profile.id}`);
    const flash = changed ? Math.max(0, 1 - (now - changed) / 1100) : 0;
    tile.image.material.opacity = tile.loaded ? 1 : 0; tile.rim.material.color.setHex(color);
    tile.glow.material.uniforms.uColor.value.setHex(color); tile.glow.material.uniforms.uStrength.value = status === 'waiting' ? .05 : .28 + (reducedMotion ? 0 : flash * 1.4);
    tile.rim.material.opacity = tile.entry.id === state.selected ? 1 : .75;
  }
}
function tick(now) {
  requestAnimationFrame(tick);
  if (cameraTween) {
    const t = Math.min(1, (now - cameraTween.start) / 650), ease = 1 - Math.pow(1 - t, 3);
    camera.position.lerpVectors(cameraTween.from, cameraTween.to, ease); controls.target.lerpVectors(cameraTween.fromTarget, cameraTween.target, ease); if (t === 1) cameraTween = null;
  }
  controls.update(); updateSourceTiles(now); updateDetails(now); updateSheetToolbarPositions(); renderer.render(scene, camera);
  if (now - lastUi > 500) { updateDiagnosticUi(); lastUi = now; }
}

function clampView() {
  sheetView(selectedSheet());
}
function updatePanControls() {
  const sheet = selectedSheet(), current = sheetView(sheet);
  for (const [id, axis, size] of [['panX', 'x', 'width'], ['panY', 'y', 'height']]) {
    $(id).max = String(Math.max(0, Math.ceil((sheet?.[size] || 0) - current[size]))); $(id).value = String(Math.round(current[axis])); $(id).disabled = Number($(id).max) === 0;
  }
  $('sheetZoomMode').textContent = fitLabels[current.mode] || 'Manual zoom';
  $('viewRange').textContent = sheet ? `${sheet.name} · x ${Math.round(current.x).toLocaleString('en-US')}–${Math.round(current.x + current.width).toLocaleString('en-US')} · y ${Math.round(current.y).toLocaleString('en-US')}–${Math.round(current.y + current.height).toLocaleString('en-US')} px` : 'Waiting for capture geometry';
  const shown = visibleTiles(sheet, sheetView(sheet), 100000);
  $('sheetSummary').textContent = sheet ? `${data.sheets.length} Sheets · ${shown.length} / ${sheet.tiles.length} capture jobs in sheet viewport` : 'No tiles yet';
  if (sheet?.capture_coverage?.omitted_tiles) {
    $('sheetSummary').textContent = `${data.sheets.length} Sheets · ${sheet.capture_coverage.captured_tiles} / ${sheet.capture_coverage.total_tiles} capture rows selected · ${sheet.capture_coverage.omitted_tiles} not visually checked`;
    $('viewRange').textContent = `${sheet.name} · compact view; original rows are shown on tiles`;
  }
  if (sheet?.capture_coverage?.crc_length_equal_after_shrink) {
    const coverage = sheet.capture_coverage;
    const recalc = coverage.crc_length_equal_after_recalc === true
      ? 'ZIP CRC + length still equal after Excel save'
      : coverage.crc_length_equal_after_recalc === false
        ? 'ZIP CRC or length changed after Excel save'
        : 'ZIP identity after Excel save unavailable';
    const deferred = coverage.deferred_full_scan ? ' · full scan paused for later' : '';
    $('sheetSummary').textContent += ` · CRC + length equal after shrink · ${recalc}${deferred}`;
  }
  if (sheet?.capture_coverage?.fallback_reason) {
    $('sheetSummary').textContent = `${sheet.capture_coverage.total_tiles} capture rows · full capture: ${sheet.capture_coverage.fallback_reason}`;
    $('sheetSummary').title = sheet.capture_coverage.fallback_reason;
  } else { $('sheetSummary').title = ''; }
}
function updateSheets() {
  const signature = JSON.stringify(data?.sheets.map(sheet => [sheet.id, sheet.name]));
  if (signature !== sheetSignature) {
    sheetSignature = signature; $('sheetSelect').replaceChildren(...(data?.sheets || []).map(sheet => { const option = document.createElement('option'); option.value = sheet.id; option.textContent = sheet.name; return option; }));
  }
  $('sheetSelect').value = state.sheetId || '';
}
function renderMinimap() {
  const sheets = data?.sheets || [];
  const signature = JSON.stringify(sheets.map(sheet => [sheet.id, sheet.name, sheet.tiles.some(tile => tile.comparisonComplete && !tile.equalAcrossAll), sheet.id === state.sheetId]));
  if (signature === miniSignature) return; miniSignature = signature;
  $('minimapGrid').style.gridTemplateColumns = `repeat(${Math.max(1, Math.ceil(Math.sqrt(sheets.length)))}, minmax(0, 1fr))`;
  $('minimapGrid').replaceChildren(...sheets.map(sheet => {
    const button = document.createElement('button'); button.className = 'minimap-sheet'; button.textContent = sheet.name; button.title = sheet.name;
    button.classList.toggle('selected', sheet.id === state.sheetId);
    button.classList.toggle('has-difference', sheet.tiles.some(tile => tile.comparisonComplete && !tile.equalAcrossAll));
    button.setAttribute('aria-label', `View sheet ${sheet.name}`);
    let timer;
    button.addEventListener('pointerenter', () => { $('minimapLabel').textContent = sheet.name; timer = setTimeout(() => focusSheet(sheet.id, true, true), 180); });
    button.addEventListener('pointerleave', () => clearTimeout(timer));
    button.addEventListener('focus', () => { $('minimapLabel').textContent = sheet.name; });
    button.addEventListener('click', () => { clearTimeout(timer); focusSheet(sheet.id, true); }); return button;
  }));
  $('minimapLabel').textContent = selectedSheet()?.name || `${sheets.length} Sheets`;
}
function normalizeStatus(value) {
  return ({ restart_requested: 'Restart requested', restart_waiting_cleanup: 'Restart waiting for safe cleanup', preparing: 'Preparation', prepared: 'Prepared for visual comparison', requested: 'Open requested', waiting_open: 'Opening Excel', planning: 'Planning capture regions', ready: 'Add-in ready', capturing: 'Capturing', comparing: 'Pixel comparison', complete: 'Completed', completed: 'Completed', finished: 'Completed', unchanged: 'No differences reported', 'crc-identical': 'CRC + length equal to shrink input', 'workbook-only': 'Only workbook.xml cleanup available', 'paused-for-later': 'Full scan paused for later', differences: 'Differences found', skipped: 'Skipped', interrupted: 'Interrupted', waiting_cleanup: 'Closing Excel', cleanup_blocked: 'Cleanup blocked', failed: 'Error', paused: 'Paused', closed: 'Closed', exit_unconfirmed: 'Excel exit not confirmed', queued: 'Queued', running: 'Active', active: 'Active', retrying_classic: 'Classic individual comparison running' })[value] || value || 'Waiting for status';
}
function groupName(group) { return String(group.workbook || group.name || group.source || group.id || 'Workbook').split(/[\\/]/).pop(); }
function inspectionProfileLabel(profile, workbook) {
  if (profile.id !== 'original') return profile.label;
  if (workbook?.baseline_repaired === true) return 'Repaired baseline';
  if (workbook?.source_format?.toLowerCase() === 'xls') return 'Converted baseline';
  return 'Original XLSX';
}
function variantDownloadUrl(groupId, profileId) {
  return `/api/groups/${encodeURIComponent(groupId)}/variants/${encodeURIComponent(profileId)}`;
}
function hasMarkedSheets(value) {
  if (value === true) return true;
  if (Array.isArray(value)) return value.length > 0;
  if (value && typeof value === 'object') return Object.values(value).some(hasMarkedSheets);
  return false;
}
function matchesGroupFilter(group, filter) {
  if (filter === 'default-investigate') return group.default_regression_candidate === true;
  if (filter.startsWith('diagnosis-')) return group.diagnosis_categories?.includes(filter.slice(10)) || false;
  if (filter === 'screened') return group.priority_validation?.primary?.status === 'complete';
  if (filter === 'secondary-pending') return Boolean(group.priority_validation) && group.priority_validation.secondary?.status !== 'complete';
  switch (filter) {
    case 'needs-visual':
      return group.preparation_disposition === 'needs_visual';
    case 'identical':
      return group.visually_identical === true;
    case 'differences':
      return group.differences_found === true || group.status === 'differences';
    case 'crc-unchanged':
      return group.preparation_disposition === 'crc_unchanged';
    case 'workbook-only':
      return group.preparation_disposition === 'workbook_only';
    case 'namespace':
      return group.preparation_disposition === 'unsupported_namespace';
    case 'preparation-failed':
      return ['shrink_failed', 'preparation_failed'].includes(group.preparation_disposition);
    case 'other-excluded':
      return group.preparation_disposition != null
        && !['needs_visual', 'crc_unchanged', 'workbook_only', 'unsupported_namespace',
          'shrink_failed', 'preparation_failed'].includes(group.preparation_disposition);
    case 'deferred':
      return group.has_capture_deferred === true || hasMarkedSheets(group.capture_deferred);
    case 'recalc-drift':
      return group.has_recalc_identity_changed === true || hasMarkedSheets(group.recalc_identity_changed);
    default:
      return true;
  }
}
function indexedWorkbookRow(row) {
  const live = state.queue?.groups?.find(group => String(group.id) === String(row.id)
    && (!group.run_id || !state.queue?.run_id || group.run_id === state.queue.run_id));
  return live ? { ...row, ...live, available_variants: row.available_variants,
    preparation_disposition: row.preparation_disposition } : row;
}
async function loadWorkbookPage(reset = true) {
  if (!reset && workbookPageLoading) return;
  const filter = $('groupFilter').value;
  const search = $('groupSearch').value.trim();
  const offset = reset ? 0 : nextWorkbookOffset;
  if (offset == null) return;
  const request = ++workbookRequest;
  workbookPageLoading = true;
  const query = new URLSearchParams({ filter, search, offset: String(offset), limit: '100' });
  try {
    const response = await fetch(`/api/workbooks?${query}`, { cache: 'no-store' });
    if (!response.ok) throw new Error(`Workbook index unavailable (${response.status}).`);
    const page = await response.json();
    if (request !== workbookRequest) return;
    if (page.run_id && state.queue?.run_id && page.run_id !== state.queue.run_id) {
      workbookIndex = null;
      workbookRows = [];
      nextWorkbookOffset = null;
      lastWorkbookRefresh = Date.now() - 3000;
      groupSignature = '';
      renderWorkbookKpis();
      renderGroups();
      return;
    }
    workbookIndex = page;
    workbookRows = reset ? page.workbooks || [] : [...workbookRows, ...(page.workbooks || [])];
    nextWorkbookOffset = page.next_offset;
    lastWorkbookRefresh = Date.now();
    groupSignature = '';
    renderWorkbookKpis();
    renderGroups();
    renderWorkbookResultList();
  } catch (error) {
    if (request !== workbookRequest) return;
    $('workbookResultStatus').textContent = error.message;
  } finally {
    if (request === workbookRequest) workbookPageLoading = false;
  }
}
async function refreshWorkbookSummary() {
  const filter = $('groupFilter').value;
  const search = $('groupSearch').value.trim();
  const query = new URLSearchParams({ filter, search, offset: '0', limit: '1' });
  try {
    const response = await fetch(`/api/workbooks?${query}`, { cache: 'no-store' });
    if (!response.ok) return;
    const page = await response.json();
    if (filter !== $('groupFilter').value || search !== $('groupSearch').value.trim()) return;
    if (page.run_id && state.queue?.run_id && page.run_id !== state.queue.run_id) return;
    if (!page.available) return;
    if (JSON.stringify(page.counts) !== JSON.stringify(workbookIndex?.counts)) {
      void loadWorkbookPage(true);
      return;
    }
    workbookIndex = { ...workbookIndex, counts: page.counts, total: page.total,
      visual_scope_frozen: page.visual_scope_frozen };
    renderWorkbookKpis();
    renderWorkbookResultList();
  } catch {
    // Keep the last complete workbook page while the host is unavailable.
  }
}
function maybeRefreshWorkbookIndex() {
  if (Date.now() - lastWorkbookRefresh < 5000) return;
  lastWorkbookRefresh = Date.now();
  if (workbookRows.length > 100) void refreshWorkbookSummary();
  else void loadWorkbookPage(true);
}
function renderWorkbookKpis() {
  const counts = workbookIndex?.counts;
  const priorityPass = state.queue?.progress?.priority_validation?.active_pass;
  $('workbookScope').textContent = priorityPass === 'aggressive_primary'
    ? 'Aggressive screening first; full validation follows'
    : priorityPass === 'secondary_full'
      ? 'Full validation of all profiles; screening results retained'
      : workbookIndex?.visual_scope_frozen
    ? 'Preparation complete · visual scope fixed'
    : 'Preparation in progress · visual scope pending';
  if (!workbookIndex?.available || !counts) {
    $('workbookKpis').replaceChildren();
    return;
  }
  const buttons = Object.entries(WORKBOOK_FILTER_LABELS).map(([filter, label]) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'workbook-kpi';
    button.setAttribute('aria-label', `${label}: ${counts[filter] ?? 0} workbooks. Open list.`);
    const value = document.createElement('strong');
    value.textContent = Number(counts[filter] ?? 0).toLocaleString('en-US');
    const caption = document.createElement('span');
    caption.textContent = label;
    button.append(value, caption);
    button.addEventListener('click', () => {
      $('groupFilter').value = filter;
      $('workbookResultList').hidden = false;
      void loadWorkbookPage(true);
      $('workbookResultList').scrollIntoView({ behavior: reducedMotion ? 'auto' : 'smooth', block: 'nearest' });
    });
    return button;
  });
  $('workbookKpis').replaceChildren(...buttons);
}
function renderWorkbookResultList() {
  const panel = $('workbookResultList');
  if (panel.hidden) return;
  const filter = $('groupFilter').value;
  $('workbookResultTitle').textContent = WORKBOOK_FILTER_LABELS[filter] || 'Selected workbooks';
  $('workbookResultCount').textContent = workbookIndex?.available
    ? `${workbookIndex.total.toLocaleString('en-US')} matching workbooks`
    : 'Workbook index is being prepared.';
  $('moreWorkbookResults').hidden = nextWorkbookOffset == null;
  $('workbookResultRows').replaceChildren(...workbookRows.map(indexedWorkbookRow).map(row => {
    const item = document.createElement('div');
    item.className = 'workbook-result-row';
    const name = document.createElement('button');
    name.type = 'button';
    name.className = 'workbook-result-name';
    name.textContent = groupName(row);
    name.addEventListener('click', () => {
      panel.hidden = true;
      selectGroup(String(row.id));
    });
    const details = document.createElement('span');
    details.className = 'workbook-result-reason';
    details.textContent = [row.corpus, workbookReason(row), row.error].filter(Boolean).join(' · ');
    const links = document.createElement('div');
    links.className = 'workbook-result-links';
    for (const profile of PRIMARY_PROFILES) {
      if (row.available_variants?.[profile.id] !== true) continue;
      const variant = document.createElement('span');
      variant.className = 'workbook-result-variant';
      const download = document.createElement('a');
      download.href = variantDownloadUrl(String(row.id), profile.id);
      download.textContent = `Download ${inspectionProfileLabel(profile, row)}`;
      download.title = 'Download the prepared XLSX variant from the local Observatory host.';
      const button = document.createElement('button');
      button.type = 'button';
      button.textContent = 'Open in Excel';
      button.title = `Open a separate ${inspectionProfileLabel(profile, row)} copy in visible Excel.`;
      button.setAttribute('aria-label', `Open ${inspectionProfileLabel(profile, row)} in Excel`);
      button.addEventListener('click', () => void requestInspectionCopy(String(row.id), profile, $('workbookResultStatus')));
      variant.append(download, button);
      links.append(variant);
    }
    item.append(name, details, links);
    return item;
  }));
  if (workbookRows.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'workbook-result-empty';
    empty.textContent = workbookIndex?.available
      ? 'No selected workbook matches this filter.' : 'The workbook index is being prepared.';
    $('workbookResultRows').append(empty);
  }
}
function workbookReason(row) {
  const reasons = [];
  if (row.preparation_disposition === 'evidence_missing') reasons.push('Source evidence missing');
  if (row.preparation_disposition === 'unsupported_namespace') {
    reasons.push(row.reason_code ? `Unsupported namespace (${row.reason_code})` : 'Unsupported namespace');
  }
  if (row.preparation_disposition === 'crc_unchanged') reasons.push('CRC-32 and length equal after shrink');
  if (row.preparation_disposition === 'workbook_only') reasons.push('Only workbook.xml changed');
  if (['shrink_failed', 'preparation_failed'].includes(row.preparation_disposition)) {
    reasons.push('Preparation failed');
  }
  if (row.has_recalc_identity_changed) reasons.push('Shrink equal; Excel save changed CRC-32 or length');
  if (row.has_capture_deferred) reasons.push('Full scan paused for later');
  return reasons.length ? reasons.join(' · ') : normalizeStatus(row.status);
}
function renderGroups() {
  const indexed = workbookIndex?.available === true;
  const queued = state.queue?.groups || [];
  const query = $('groupSearch').value.trim().toLocaleLowerCase();
  const filter = $('groupFilter').value;
  const matching = indexed
    ? workbookRows.map(indexedWorkbookRow)
    : queued.filter(group => matchesGroupFilter(group, filter)
      && (!query || `${groupName(group)} ${group.corpus || ''}`.toLocaleLowerCase().includes(query)));
  const shown = indexed ? matching : matching.slice(0, groupLimit);
  const selected = queued.find(group => String(group.id) === state.groupId);
  if (!indexed && selected && matching.includes(selected) && !shown.includes(selected)) shown.unshift(selected);
  const signature = JSON.stringify([state.groupId, query, filter, shown]);
  if (signature === groupSignature) return;
  groupSignature = signature;
  const total = indexed ? workbookIndex.total : matching.length;
  const all = indexed ? workbookIndex.counts.all : queued.length;
  $('groupCount').textContent = `${total.toLocaleString('en-US')} / ${all.toLocaleString('en-US')}`;
  $('moreGroups').hidden = indexed ? nextWorkbookOffset == null : matching.length <= groupLimit;
  $('moreGroups').textContent = `Show ${Math.min(100, indexed ? total - shown.length : matching.length - groupLimit)} more`;
  $('groups').replaceChildren(...shown.map(group => {
    const button = document.createElement('button'); button.className = 'group'; button.classList.toggle('selected', String(group.id) === state.groupId); button.setAttribute('aria-pressed', String(String(group.id) === state.groupId));
    const head = document.createElement('span'); head.className = 'group-head';
    const number = document.createElement('span'); number.textContent = group.corpus || 'WORKBOOK';
    const status = document.createElement('span'); status.textContent = normalizeStatus(group.status); head.append(number, status);
    const name = document.createElement('span'); name.className = 'group-name'; name.textContent = groupName(group); name.title = String(group.workbook || group.source || name.textContent);
    const description = document.createElement('span'); description.className = 'group-status';
    const evidence = [
      group.preparation_disposition === 'crc_unchanged' ? 'Unchanged by shrink' : null,
      group.has_capture_deferred || hasMarkedSheets(group.capture_deferred) ? 'Full scan paused for later' : null,
      group.has_recalc_identity_changed || hasMarkedSheets(group.recalc_identity_changed)
        ? 'ZIP length/CRC changed after Excel save' : null,
    ].filter(Boolean).join(' · ');
    description.textContent = [evidence, group.error || group.stage || group.message
      || (evidence ? null : workbookReason(group))].filter(Boolean).join(' · ');
    const progress = document.createElement('span'); progress.className = 'progress'; const fill = document.createElement('i');
    const done = Number(group.completed_tiles ?? group.tiles_compared ?? 0), total = Number(group.total_tiles ?? 0);
    fill.style.width = `${total > 0 ? Math.min(100, done / total * 100) : ['complete', 'completed', 'finished', 'unchanged', 'crc-identical', 'workbook-only', 'differences', 'skipped'].includes(group.status) ? 100 : 0}%`; progress.append(fill);
    const foot = document.createElement('span'); foot.className = 'group-foot'; foot.textContent = total ? `${done} / ${total} tiles` : group.corpus || 'Status from runner';
    button.append(head, name, description, progress, foot); button.addEventListener('click', () => selectGroup(String(group.id))); return button;
  }));
}
function updateControlInputs() {
  const values = state.controls;
  const logicalCores = Math.max(1, Number(state.queue?.logical_cores || 1));
  for (const [id, key, valueId, statusId] of [
    ['sweepProcesses', 'sweep_processes', 'sweepProcessesValue', 'sweepProcessesStatus'],
    ['prepareProcesses', 'prepare_processes', 'prepareProcessesValue', 'prepareProcessesStatus'],
    ['shrinkThreads', 'shrink_threads', 'shrinkThreadsValue', 'shrinkThreadsStatus'],
  ]) {
    const slider = $(id);
    slider.max = String(key === 'sweep_processes' ? Math.max(128, Number(values[key] || 1)) : logicalCores);
    if (values[key] != null && document.activeElement !== slider && !dirtyControls.has(id)) {
      slider.value = String(values[key]);
    }
    $(valueId).textContent = slider.value;
    const running = key === 'prepare_processes' && state.queue?.phase === 'preparing'
      && Array.isArray(state.queue?.progress?.active_preparation)
      ? ` · ${state.queue.progress.active_preparation.length} active now` : '';
    $(statusId).textContent = values[key] == null
      ? 'Waiting for confirmed value.'
      : dirtyControls.has(id)
        ? `Selected: ${slider.value} · confirmed: ${values[key]} · release to apply.`
      : key === 'sweep_processes'
        ? `Confirmed: ${values[key]} · applies to newly started sweep sessions.`
        : `Confirmed: ${values[key]}${running} · applies to newly started shrink processes · maximum ${logicalCores} logical cores.`;
  }
  for (const [id, key] of [['targetGroups', 'target_groups'], ['excelProcessLimit', 'excel_process_limit'], ['systemReserve', 'system_reserve_mib'], ['openTimeout', 'open_timeout_seconds'], ['jobTimeout', 'job_timeout_seconds'], ['equalPreview', 'equal_preview_width'], ['captureMegapixels', 'capture_tile_megapixels'], ['captureBatch', 'capture_batch_tiles'], ['captureContext', 'capture_context_tiles'], ['tallSheetThreshold', 'tall_sheet_threshold'], ['tallSheetEdges', 'tall_sheet_edge_tiles'], ['crcEqualSheetThreshold', 'crc_equal_sheet_threshold_mib']]) {
    if (values[key] != null && document.activeElement !== $(id) && !dirtyControls.has(id)) $(id).value = String(values[key]);
  }
  if (document.activeElement !== $('captureTransport') && !dirtyControls.has('captureTransport')) {
    $('captureTransport').value = values.capture_transport || 'live-images';
  }
  $('captureTransportStatus').textContent = captureTransportSummary(values, data);
  const confirmedReserve = values.system_reserve_mib ?? 0;
  if (values.system_reserve_mib == null && document.activeElement !== $('systemReserve') && !dirtyControls.has('systemReserve')) $('systemReserve').value = String(confirmedReserve);
  $('systemReserveStatus').textContent = confirmedReserve === 0
    ? 'Confirmed: RAM guard off' : `Confirmed: ${confirmedReserve.toLocaleString('en-US')} MiB`;
  if (document.activeElement !== $('hideExcel')) $('hideExcel').checked = Boolean(values.hide_excel);
  $('groupRange').max = String(Math.max(24, Number(values.target_groups ?? 1), Number($('targetGroups').value || 0)));
  if (document.activeElement !== $('groupRange') && !dirtyControls.has('targetGroups')) $('groupRange').value = String(values.target_groups ?? 1);
  const draining = values.target_groups === 0;
  const active = activeGroupCount(state.queue);
  const diskGuard = state.queue?.admission_guard;
  const preparing = state.queue?.progress?.visual_scope_frozen === false;
  $('captureState').textContent = state.queue?.running === false ? 'Ended'
    : state.queue?.phase === 'awaiting_deferred_manual' ? 'Deferred scans waiting'
    : draining ? state.queue?.phase === 'paused' ? 'Run paused'
      : preparing ? 'Shrinking continues' : `${active} groups finishing`
      : diskGuard?.blocked ? 'Waiting for disk space' : values.paused ? 'Captures paused' : 'Running';
  $('pauseJobs').textContent = values.paused ? 'Resume captures' : 'Pause captures';
  $('pauseJobs').disabled = controlBusy || draining || state.queue?.running === false;
  $('pauseNote').textContent = state.queue?.running === false ? 'The run has ended. Its saved images remain available.'
    : state.queue?.phase === 'awaiting_deferred_manual'
      ? 'All regular comparisons are finished. Start each deferred full scan manually or stop the run.'
    : draining ? preparing
      ? 'Target 0: all selected workbooks continue shrinking. Visual comparisons wait when preparation finishes.'
      : 'Target 0: active comparisons finish, then the run pauses. Increase the group target to resume.'
    : diskGuard?.blocked ? `${diskGuard.message}${diskGuard.free_bytes != null ? ` Free: ${(diskGuard.free_bytes / 1073741824).toFixed(2)} GiB.` : ''}${diskGuard.error ? ` ${diskGuard.error}` : ''}`
    : values.paused ? 'Jobs already in progress will finish. New captures wait.'
      : 'Pausing captures keeps Excel open. A group target of 0 lets preparation finish, then pauses comparisons.';
  $('activeGroups').textContent = `${active} admitted · ${values.target_groups ?? '—'} capture + ${state.queue?.prefetch_groups ?? 0} preparing`;
  const demand = state.queue?.excel_opening_demand;
  $('excelProcessDemand').textContent = demand?.observed_sessions
    ? `${demand.waiting_workbooks} capture workbooks waiting to open in ${demand.waiting_sessions} sessions · ${demand.opening_workbooks} opening now. Sweep groups: ${values.sweep_processes ?? 1}; comparison groups: ${values.target_groups ?? '—'}. Excel limit: ${Number(values.excel_process_limit) === 0 ? 'unlimited' : values.excel_process_limit}.`
    : 'No preparation capture sessions waiting to open. Recalculation starts are not included in this count.';
}
async function sendControl(change) {
  if (controlBusy) return; controlBusy = true;
  controlUpdates.beginWrite();
  const wasCapturePaused = Boolean(state.controls.paused);
  for (const id of ['sweepProcesses', 'prepareProcesses', 'shrinkThreads', 'applyTarget', 'applyExcelProcessLimit', 'applyReserve', 'disableReserve', 'pauseJobs', 'applyTimeouts', 'equalPreview', 'captureTransport', 'applyCapture', 'applyTallSheet', 'applyCrcEqualSheet', 'hideExcel']) $(id).disabled = true;
  if (change.system_reserve_mib != null) $('applyReserve').textContent = 'Setting…';
  try {
    const response = await fetch('/api/control', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(change) });
    const result = await response.json(); if (!response.ok || result.ok === false) throw new Error(result.error || `Control change rejected (${response.status}).`);
    controlUpdates.confirmWrite(result.controls);
    for (const [id, key] of [['sweepProcesses', 'sweep_processes'], ['prepareProcesses', 'prepare_processes'], ['shrinkThreads', 'shrink_threads']]) {
      if (change[key] != null && Number($(id).value) === change[key]) dirtyControls.delete(id);
    }
    for (const [id, key] of [['targetGroups', 'target_groups'], ['excelProcessLimit', 'excel_process_limit'], ['systemReserve', 'system_reserve_mib'], ['openTimeout', 'open_timeout_seconds'], ['jobTimeout', 'job_timeout_seconds'], ['equalPreview', 'equal_preview_width'], ['captureMegapixels', 'capture_tile_megapixels'], ['captureBatch', 'capture_batch_tiles'], ['captureContext', 'capture_context_tiles'], ['tallSheetThreshold', 'tall_sheet_threshold'], ['tallSheetEdges', 'tall_sheet_edge_tiles'], ['crcEqualSheetThreshold', 'crc_equal_sheet_threshold_mib']]) if (change[key] != null && Number($(id).value) === change[key]) dirtyControls.delete(id);
    updateControlInputs();
    if (change.capture_transport === $('captureTransport').value) dirtyControls.delete('captureTransport');
    const text = change.target_groups === 0
      ? `Target 0 confirmed: ${state.queue?.progress?.visual_scope_frozen === false
        ? 'preparation continues; visual comparison waits at the barrier.'
        : 'active comparisons finish, then the run pauses.'}${wasCapturePaused ? ' The capture pause was lifted to allow this.' : ''}`
      : change.target_groups != null ? `Group target ${state.controls.target_groups} confirmed.`
      : change.excel_process_limit != null ? `Excel process limit ${state.controls.excel_process_limit} confirmed.`
      : change.sweep_processes != null ? `${state.controls.sweep_processes} simultaneous hash-sweep groups confirmed for future starts.`
      : change.prepare_processes != null ? `${state.controls.prepare_processes} simultaneous shrink processes confirmed for future starts.`
      : change.shrink_threads != null ? `${state.controls.shrink_threads} Rayon threads per shrink process confirmed for future starts.`
      : change.capture_transport != null ? 'Capture transport confirmed for the next primary capture phase.'
      : change.system_reserve_mib != null ? state.controls.system_reserve_mib === 0
        ? 'Free RAM guard disabled.'
        : `Free RAM reserve ${state.controls.system_reserve_mib.toLocaleString('en-US')} MiB confirmed.` : 'Controls confirmed by host.';
    pushEvent(text); setNotice(text);
  } catch (error) {
    setNotice(error.message, true);
    for (const [id, key] of [['sweepProcesses', 'sweep_processes'], ['prepareProcesses', 'prepare_processes'], ['shrinkThreads', 'shrink_threads']]) {
      if (key in change) {
        dirtyControls.delete(id);
        $(id).value = String(state.controls[key] ?? 1);
      }
    }
    if ('capture_transport' in change) {
      dirtyControls.delete('captureTransport');
      $('captureTransport').value = state.controls.capture_transport || 'live-images';
    }
    updateControlInputs();
    if ('hide_excel' in change) $('hideExcel').checked = Boolean(state.controls.hide_excel);
  }
  finally { controlUpdates.endWrite(); controlBusy = false; for (const id of ['sweepProcesses', 'prepareProcesses', 'shrinkThreads', 'applyTarget', 'applyExcelProcessLimit', 'applyReserve', 'disableReserve', 'pauseJobs', 'applyTimeouts', 'equalPreview', 'captureTransport', 'applyCapture', 'applyTallSheet', 'applyCrcEqualSheet', 'hideExcel']) $(id).disabled = false; $('applyReserve').textContent = 'Set'; updateControlInputs(); }
}
function selectGroup(id) {
  if (id === state.groupId) return;
  if (pendingCaptureJump?.groupId !== String(id)) pendingCaptureJump = null;
  state.groupId = id; state.sheetId = null; state.focus = 'overview'; lastFocusKey = ''; state.view = { mode: 'sheet', x: 0, y: 0, width: 1800, height: 1800 * AREA_H / AREA_W }; latestCapture = null; previousImages.clear(); flashes.clear();
  followedProfile = null; lastFollowedKey = null;
  data = null; entries = []; lastSignature = ''; sheetSignature = ''; clearDetails(); ++sceneGeneration;
  boards.splice(0).forEach(disposeObject); sourceTiles.length = 0; hitTargets.length = 0; sheetCards.clear();
  for (const card of processCards.values()) card.signals.destroy();
  processCards.clear(); $('processes').replaceChildren();
  for (const toolbar of sheetToolbars.values()) toolbar.remove(); sheetToolbars.clear();
  $('emptyScene').hidden = false; $('tileButtons').replaceChildren(); renderGroups();
  $('inspectionStatus').textContent = '';
  renderGroupOutcome(); renderInspectionCopies();
  ++pollGeneration; clearTimeout(pollTimer); poll();
}
// The worker list and its capture stream provide a one-click route to the
// selected physical Excel add-in. No new capture jobs are ever dispatched here.
function jumpToWorkerCaptures({ groupId, clientId, capture = null, follow = true } = {}) {
  if (!groupId || !clientId) return;
  pendingCaptureJump = { groupId: String(groupId), clientId: String(clientId), capture, follow };
  if (String(state.groupId) !== String(groupId)) selectGroup(String(groupId));
  else if (data) applyCaptureJump();
  document.querySelector('.room')?.scrollIntoView({ behavior: reducedMotion ? 'auto' : 'smooth', block: 'start' });
}
function applyCaptureJump() {
  const jump = pendingCaptureJump;
  if (!data || !jump || jump.groupId !== String(state.groupId)) return;
  pendingCaptureJump = null;
  followedProfile = profileForClient(data, jump.clientId);
  const target = findCaptureTile(data, jump.capture);
  // Historical tiles are inspected without the live follower overriding them.
  if (jump.capture && !jump.follow) {
    state.follow = false; $('followCapture').checked = false;
    if (target) {
      focusTile(target, null, true);
      setNotice(`Capture in studio: ${target.sheetName} · ${target.address}. Manual selection stays visible.`);
    } else {
      setNotice('The selected capture is unavailable in this group. Live following remains paused.', true);
    }
    return;
  }
  state.follow = true; $('followCapture').checked = true;
  $('followCapture').title = `Following captures from ${jump.clientId}`;
  clearDetails();
  const monitor = data.launcher?.clients?.[jump.clientId]?.monitor;
  const latestEvent = [...(monitor?.events || [])].reverse().find(event => event.kind === 'image_received');
  const mostRecent = target || findCaptureTile(data, latestEvent) || latestTileForProfile(data, followedProfile);
  if (mostRecent) {
    latestCapture = mostRecent.id;
    lastFollowedKey = `${followedProfile}:${mostRecent.id}`;
    followTile(mostRecent);
    setNotice(`Following captures from ${jump.clientId} · ${mostRecent.sheetName} · ${mostRecent.address}`);
  } else {
    const location = monitor?.active_job?.locations?.[0];
    const sheet = data.sheets.find(item => item.name === location?.sheet_name ||
      (location?.sheet_index != null && String(item.index ?? item.id) === String(location.sheet_index)));
    if (sheet) focusSheet(sheet.id);
    setNotice(`Following captures from ${jump.clientId}. No matching capture geometry received yet.`);
  }
}
function acceptGroup(raw) {
  data = normalizeGroup(raw); entries = data.sheets.flatMap(sheet => sheet.tiles);
  $('workbook').textContent = groupName(data); $('workbook').title = String(data.workbook || '');
  $('groupEyebrow').textContent = [data.corpus, data.capture_mode === 'classic' || data.status === 'retrying_classic' ? 'LAST RESIDENT CAPTURES · CLASSIC CROSS-CHECK' : 'LIVE STUDIO'].filter(Boolean).join(' · ');
  if (!data.sheets.some(sheet => sheet.id === state.sheetId)) state.sheetId = data.sheets[0]?.id ?? null;
  let latest = null;
  const changedByProfile = new Map();
  for (const tile of entries) for (const [profile, image] of Object.entries(tile.images)) {
    const key = `${tile.id}:${profile}`, signature = JSON.stringify([image.url, image.pixelClass, image.maskUrl]);
    if ((image.url || imageStatus(tile, profile) === 'hash-equal') && previousImages.get(key) !== signature) {
      if (previousImages.has(key)) {
        const status = imageStatus(tile, profile);
        let message = 'Pixel comparison complete';
        if (status === 'hash-equal') message = 'PNG hash equality confirmed; no preview retained';
        else if (image.pixelClass == null) message = 'Image received';
        else if (status === 'different') message = 'Pixel difference confirmed';
        pushEvent(`${tile.sheetName} \u00b7 ${tile.address} \u00b7 ${profileLabel(profile)}: ${message}`);
      }
      flashes.set(key, performance.now()); previousImages.set(key, signature); latest = tile;
      changedByProfile.set(profile, tile);
    }
  }
  const active = entries.find(tile => tile.id === String(data.active_tile) && (!data.active_sheet || tile.sheetId === String(data.active_sheet)));
  const next = followedProfile ? changedByProfile.get(followedProfile) : active || latest;
  const key = next ? `${followedProfile || 'any'}:${next.id}` : null;
  if (next && key !== lastFollowedKey) {
    lastFollowedKey = key; latestCapture = next.id;
    if (state.follow && !state.pinned && !state.selected) followTile(next);
  }
  clampView(); updateSheets(); renderScene(); updateDiagnosticUi();
  applyCaptureJump();
  if (state.pendingDifference) { state.pendingDifference = false; nextDifference(); }
}
async function poll() {
  const generation = pollGeneration;
  const controlRevision = controlUpdates.beginRead();
  try {
    const response = await fetch('/api/state', { cache: 'no-store' }); if (!response.ok) throw new Error(`Status unavailable (${response.status}).`);
    const queue = await response.json(); if (generation !== pollGeneration) return;
    const newRun = state.queue?.run_id && queue.run_id && state.queue.run_id !== queue.run_id;
    if (newRun) {
      state.groupId = null;
      data = null;
      entries = [];
      $('workbook').textContent = 'Waiting for the first current-run workbook.';
      $('groupStatus').textContent = 'Shrink preparation is running.';
      $('emptyScene').hidden = false;
      boards.splice(0).forEach(disposeObject);
      sourceTiles.length = 0;
      hitTargets.length = 0;
      for (const card of processCards.values()) card.signals.destroy();
      processCards.clear();
      $('processes').replaceChildren();
      $('sheetSelect').replaceChildren();
      $('minimapGrid').replaceChildren();
      renderGroupOutcome();
      renderInspectionCopies();
    }
    if (newRun || (workbookIndex?.run_id && queue.run_id && workbookIndex.run_id !== queue.run_id)) {
      ++workbookRequest;
      workbookPageLoading = false;
      workbookIndex = null;
      workbookRows = [];
      nextWorkbookOffset = null;
      lastWorkbookRefresh = 0;
      groupSignature = '';
      renderWorkbookKpis();
    }
    const recovered = Boolean(state.error);
    controlUpdates.acceptRead(controlRevision, queue.controls || {});
    queue.controls = state.controls;
    state.queue = queue; state.connected = true; state.error = null;
    maybeRefreshWorkbookIndex();
    workerMonitor?.update(queue);
    const runnerActive = queue.running === true && queue.runner_alive === true;
    $('stopRun').disabled = !runnerActive || queue.stop_requested === true || queue.terminate_requested === true;
    $('stopRun').textContent = queue.stop_requested && queue.running ? 'Finishing...' : 'Finish active workbooks';
    $('terminateRun').disabled = !runnerActive || queue.terminate_supported !== true || queue.terminate_requested === true;
    $('terminateRun').textContent = queue.terminate_requested && queue.running ? 'Stopping...' : 'Stop now';
    $('runLifecycle').textContent = queue.update_ready === true
      ? 'Runner ended. You can pull and rebuild; results are retained.'
      : queue.terminate_requested ? 'Cancelling active work and closing Excel sessions. The runner exits after cleanup.'
      : queue.stop_requested ? 'Finishing active workbooks and Excel cleanup. Wait before updating.'
      : queue.phase === 'awaiting_deferred_manual'
        ? 'Regular comparisons finished. Deferred full scans wait for manual start.'
      : queue.running ? 'Finish lets active workbooks complete; Stop now cancels them.'
      : 'Waiting for the runner process to exit.';

    $('connectionLight').className = 'connected';
    $('connectionText').textContent = queue.verification_fixture ? 'Saved case · UI verification'
      : queue.phase === 'owner_unverified' ? 'Connected · runner status unverified'
      : queue.phase === 'finished' ? 'Run completed · results'
      : queue.runner_alive === false || queue.phase === 'interrupted' ? 'Runner exited · last snapshot'
      : queue.running === false ? 'Run ended · results'
      : 'Live · local runner';
    if (recovered) setNotice('Connection to local host restored.');
    const memory = queue.memory;
    $('memory').textContent = memory?.available_bytes != null ? `${(memory.available_bytes / 1073741824).toFixed(1)} GiB free / ${(memory.total_bytes / 1073741824).toFixed(0)} GiB` : 'RAM unavailable';
    updateControlInputs();
    const currentGroups = queue.groups?.filter(group => !group.run_id || !queue.run_id
      || group.run_id === queue.run_id) || [];
    if (!state.groupId && currentGroups.length) {
      state.groupId = String((currentGroups.find(group => ['capturing', 'comparing', 'running', 'opening', 'active'].includes(group.status)) || currentGroups[0]).id);
    }
    renderGroups();
    if (state.groupId) {
      const detail = await fetch(`/api/groups/${encodeURIComponent(state.groupId)}`, { cache: 'no-store' });
      if (!detail.ok) throw new Error(`Group details unavailable (${detail.status}).`);
      const raw = await detail.json(); if (generation !== pollGeneration) return; acceptGroup(raw);
    }
    if (queue.diagnostics?.length) setNotice(queue.diagnostics.map(item => typeof item === 'string' ? item : item.message || JSON.stringify(item)).join(' · '), true);
    $('lastUpdate').textContent = `Updated ${new Date().toLocaleTimeString('en-US')}`;
  } catch (error) {
    if (generation !== pollGeneration) return;
    $('stopRun').disabled = true;
    $('terminateRun').disabled = true;
    state.connected = false; state.error = error.message; $('connectionLight').className = 'failed'; $('connectionText').textContent = 'Disconnected'; setNotice(`${error.message} Showing the last known state; retrying the connection shortly.`, true);
  } finally { if (generation === pollGeneration) pollTimer = setTimeout(poll, document.hidden ? 2000 : 500); }
}
function elapsedText(iso) {
  const milliseconds = Date.now() - Date.parse(iso); if (!Number.isFinite(milliseconds)) return '';
  const seconds = Math.max(0, Math.floor(milliseconds / 1000)); return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
function renderGroupOutcome() {
  const outcome = groupOutcome(data);
  const banner = $('groupOutcome');
  banner.hidden = !outcome;
  if (outcome) {
    banner.dataset.kind = outcome.kind;
    $('groupOutcomeTitle').textContent = outcome.title;
    $('groupOutcomeReason').textContent = outcome.detail;
  }
  const empty = $('emptyScene');
  empty.querySelector('strong').textContent = outcome?.title || 'The next capture will appear here.';
  empty.querySelector('span').textContent = outcome?.detail || 'Only actual jobs and images are shown.';
  renderCleanupAnalysis();
  renderVisualDiagnosis();
}
function renderVisualDiagnosis() {
  const diagnoses = data?.visual_diagnoses || [];
  $('visualDiagnosis').hidden = diagnoses.length === 0;
  const content = $('visualDiagnosisContent');
  content.replaceChildren();
  const labels = {
    row_height: 'Row heights', local_whitespace: 'Local whitespace restoration',
    local_hidden_content: 'Local hidden content restoration',
    cross_sheet_hidden_content: 'Hidden content on other sheets',
    cross_sheet_whitespace: 'Whitespace across sheets', recalculation: 'Recalculation',
    combined: 'Resolved after combined repairs', unstable: 'Unstable / not reproducible',
    unresolved: 'Unresolved', identical: 'No remaining pixel difference',
  };
  for (const { profile, diagnosis } of diagnoses) {
    const heading = document.createElement('h4');
    heading.textContent = `${profileLabel(profile)}: ${labels[diagnosis.category] || diagnosis.category || diagnosis.status}`;
    content.append(heading);
    const note = document.createElement('p');
    note.textContent = diagnosis.category === 'combined'
      ? 'Several repairs were retained together. This identifies a cumulative explanation, not an exclusive single cause.'
      : diagnosis.category === 'unstable'
        ? 'Repeated captures or recalculations changed the reference state. No shrinking cause is certified.'
        : `Scope: ${diagnosis.scope || 'recorded mismatch regions'}. Original comparison evidence is retained.`;
    content.append(note);
    for (const step of diagnosis.steps || []) {
      const item = document.createElement('p');
      item.textContent = `${labels[step.kind] || step.kind || 'Control'}: ${step.status || (step.same === true ? 'pixels equal' : 'recorded')}`;
      content.append(item);
    }
    if (diagnosis.error) {
      const error = document.createElement('p');
      error.textContent = diagnosis.error;
      content.append(error);
    }
  }
}
function renderCleanupAnalysis() {
  const group = data || state.queue?.groups?.find(item => String(item.id) === String(state.groupId));
  const profiles = Object.entries(group?.cleanup_analysis || {});
  const panel = $('cleanupAnalysis');
  panel.hidden = profiles.length === 0;
  if (panel.hidden) return;

  const content = $('cleanupAnalysisContent');
  content.replaceChildren();
  for (const [profile, result] of profiles) {
    const heading = document.createElement('h4');
    heading.textContent = profile;
    content.append(heading);
    const provenance = document.createElement('p');
    provenance.textContent = result?.method === 'measured_shrink'
      ? 'Category counts come from the measured shrink run.'
      : result?.matches_measured_output === false
        ? 'Separate analysis pass; output differs from the measured shrink run.'
        : 'Separate analysis pass; category counts describe that pass.';
    content.append(provenance);
    if (result?.status !== 'complete') {
      const note = document.createElement('p');
      note.textContent = result?.status === 'skipped_unchanged'
        ? 'No cleanup changes were applied to the published workbook.'
        : result?.error || 'Analysis unavailable.';
      content.append(note);
      continue;
    }
    const table = document.createElement('table');
    const header = document.createElement('tr');
    for (const label of ['Cause', 'Removed items', 'Removed bytes', 'Net XML reduction']) {
      const cell = document.createElement('th');
      cell.textContent = label;
      header.append(cell);
    }
    table.append(header);
    for (const row of result.rows || []) {
      const line = document.createElement('tr');
      for (const value of [row.cause, row.removed_items, row.removed_bytes_uncompressed_xml, row.reduction_bytes_uncompressed_xml]) {
        const cell = document.createElement('td');
        cell.textContent = typeof value === 'number' ? value.toLocaleString('en-US') : String(value ?? '—');
        line.append(cell);
      }
      table.append(line);
    }
    content.append(table);
  }
}
function renderInspectionCopies() {
  const availableProfiles = data?.inspection_profiles || [];
  const availability = $('inspectionAvailability');
  availability.hidden = availableProfiles.length > 0;
  availability.textContent = data
    ? 'No prepared workbook copies are available for this file.'
    : 'Waiting for workbook details.';
  for (const profile of PRIMARY_PROFILES) {
    let button = inspectionButtons.get(profile.id);
    if (!button) {
      button = document.createElement('button');
      button.type = 'button';
      button.title = `Open a separate ${profile.label} copy in visible Excel.`;
      button.addEventListener('click', () => openInspectionCopy(profile));
      inspectionButtons.set(profile.id, button);
      const download = document.createElement('a');
      download.className = 'inspection-download';
      download.textContent = 'Download';
      inspectionDownloads.set(profile.id, download);
      const variant = document.createElement('span');
      variant.className = 'inspection-variant';
      variant.append(button, download);
      $('inspectionButtons').append(variant);
    }
    const pending = pendingInspections.has(`${state.groupId}/${profile.id}`);
    const available = availableProfiles.includes(profile.id);
    button.disabled = !state.groupId || !available || pending;
    const label = inspectionProfileLabel(profile, data);
    button.textContent = pending ? `${label}: requesting...` : label;
    const download = inspectionDownloads.get(profile.id);
    download.hidden = !state.groupId || !available;
    if (download.hidden) download.removeAttribute('href');
    else download.href = variantDownloadUrl(state.groupId, profile.id);
    download.setAttribute('aria-label', `Download ${label}`);
  }
}
async function openInspectionCopy(profile) {
  const groupId = state.groupId;
  if (!groupId) return;
  await requestInspectionCopy(groupId, profile, $('inspectionStatus'));
}
async function requestInspectionCopy(groupId, profile, status) {
  const key = `${groupId}/${profile.id}`;
  if (pendingInspections.has(key)) return;
  pendingInspections.add(key);
  status.classList.remove('failure');
  status.textContent = `Preparing ${profile.label} inspection copy...`;
  renderInspectionCopies();
  try {
    const response = await fetch(`/api/groups/${encodeURIComponent(groupId)}/inspect`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ profile: profile.id }),
    });
    const result = await response.json();
    if (!response.ok || result.ok === false) {
      throw new Error(result.error || `Inspection request failed (${response.status}).`);
    }
    status.textContent = `Inspection copy requested in visible Excel: ${result.path}`;
    pushEvent(`${profile.label} inspection copy requested in visible Excel.`);
  } catch (error) {
    status.classList.add('failure');
    status.textContent = error.message;
  } finally {
    pendingInspections.delete(key);
    renderInspectionCopies();
  }
}
function updateDiagnosticUi() {
  renderGroupOutcome();
  renderInspectionCopies();
  renderThroughput();
  renderProcesses();
  $('captureTransportStatus').textContent = captureTransportSummary(state.controls, data);
  const selectedGroup = state.queue?.groups?.find(group => String(group.id) === String(state.groupId)
    && (!group.run_id || !state.queue?.run_id || group.run_id === state.queue.run_id));
  $('restartGroup').disabled = !selectedGroup || selectedGroup.restart_eligible === false
    || data?.restart_eligible === false || state.queue?.running === false
    || state.queue?.phase === 'awaiting_deferred_manual';
  const hasDeferredScan = hasMarkedSheets(selectedGroup?.capture_deferred);
  const deferredReady = state.queue?.phase === 'awaiting_deferred_manual'
    && state.queue?.running === true && state.queue?.runner_alive === true;
  $('startDeferredScan').hidden = !hasDeferredScan || !deferredReady;
  $('startDeferredScan').disabled = $('startDeferredScan').hidden
    || selectedGroup?.restart_eligible === false
    || data?.restart_eligible === false;
  $('startDeferredScan').title = 'Starts this deferred full scan after regular comparisons. If the group target is 0, it becomes 1. The 4,096-tile capture limit still applies.';
  $('deferredResume').hidden = !hasDeferredScan;
  $('deferredResume').textContent = deferredReady
    ? 'Deferred sheets have only edge captures. Start the full scan when you are ready; the 4,096-tile capture limit still applies.'
    : state.queue?.running === true
      ? 'Deferred sheets have only edge captures. The manual full scan becomes available after regular comparisons finish.'
    : `Deferred sheets have only edge captures. Resume with excel-shrink-visual-validation -Corpus "${selectedGroup?.corpus || '<NAME>'}" -Partition <N/M> -FullScanDeferred. Use the same corpus and partition as the original run. The 4,096-tile capture limit still applies.`;
  if (data) {
    const clients = Object.entries(data.launcher?.clients || {});
    const counts = clients.reduce((result, [, client]) => { result[client.status] = (result[client.status] || 0) + 1; return result; }, {});
    const opening = clients.filter(([, client]) => ['requested', 'waiting_open', 'opening'].includes(client.status));
    const terminal = ['unchanged', 'crc-identical', 'workbook-only', 'paused-for-later', 'differences', 'skipped', 'failed', 'interrupted', 'complete', 'completed', 'finished'].includes(data.status);
    $('groupStatus').textContent = data.status === 'retrying_classic'
      ? 'The runner is retrying this file with a classic individual comparison. Tiles show the last resident snapshot.'
      : data.capture_mode === 'classic'
        ? `${normalizeStatus(data.status)} · Classic comparison. Tiles show the previous resident snapshot; the final result is in the report.`
        : [normalizeStatus(data.status), clients.length ? terminal ? `${counts.closed || 0} / ${clients.length} Excel instances closed` : `${counts.ready || 0} / ${clients.length} add-ins ready` : '', ...opening.map(([id, client]) => `${profileLabel(id)}: ${normalizeStatus(client.status)} ${elapsedText(client.waiting_since || client.requested_at)}`)].filter(Boolean).join(' · ');
  }
  const events = [...(data?.events || []).map(event => typeof event === 'string' ? { message: event } : event), ...localEvents].sort((a, b) => String(b.at || b.timestamp || '').localeCompare(String(a.at || a.timestamp || ''))).slice(0, 8);
  const fragment = document.createDocumentFragment();
  for (const event of events) {
    const row = document.createElement('div'); row.className = 'event'; const time = document.createElement('time');
    const iso = event.at || event.timestamp; time.textContent = iso && Number.isFinite(Date.parse(iso)) ? new Date(iso).toLocaleTimeString('en-US') : '—';
    const text = document.createElement('span'); text.textContent = event.message || event.text || event.kind || ''; row.append(time, text); fragment.append(row);
  }
  if (!events.length) { const row = document.createElement('div'); row.className = 'event'; row.textContent = 'New capture events appear here.'; fragment.append(row); }
  $('events').replaceChildren(fragment);
  for (const button of $('tileButtons').children) { const active = button.dataset.tile === state.selected; button.classList.toggle('active', active); button.setAttribute('aria-pressed', String(active)); }
  if ($('apiState').closest('details').open) $('apiState').textContent = JSON.stringify(snapshot(), null, 2);
}
function renderProcesses() {
  const reported = data?.launcher?.clients || {};
  const profiles = data?.available === false ? [] : data?.profiles || PRIMARY_PROFILES;
  const clients = profiles.map(profile => [profile.id, reported[profile.id] || {}]);
  const ids = new Set(clients.map(([id]) => id));
  for (const [id, card] of processCards) if (!ids.has(id)) { card.signals.destroy(); card.element.remove(); processCards.delete(id); }
  if (!processCards.size) $('processes').replaceChildren();
  const terminal = ['complete', 'completed', 'finished', 'unchanged', 'crc-identical', 'workbook-only', 'paused-for-later', 'differences', 'skipped', 'failed', 'interrupted'].includes(data?.status);
  const rows = workerRows({ id: data?.id || state.groupId || 'pending', attempt_id: data?.attempt_id,
    launcher_sampled_unix_ms: data?.launcher?.sampled_unix_ms,
    active: !terminal, clients: clients.map(([id, client]) => ({ ...client, client_id: id,
      worker_id: `${state.groupId || 'pending'}/${data?.attempt_id || 'legacy'}/${id}/${client.pid || 'pending'}/${client.process_started_unix_ms || 'pending'}` })) });
  const stageLabels = { staging: 'Staging file', waiting_for_open_slot: 'Waiting to open', waiting_for_addin: 'Waiting for add-in', waiting_for_job: 'Waiting for job', executing_job: 'Capture running', blocked_edit_mode: 'Excel in edit mode', client_error: 'Add-in reported an error', closing: 'Closing Excel', exit_unconfirmed: 'Excel exit not confirmed', closed: 'Closed' };
  for (const [id, client] of clients) {
    let card = processCards.get(id);
    if (!card) {
      const element = document.createElement('div'); element.className = 'process';
      const name = document.createElement('strong'), process = document.createElement('span'), status = document.createElement('span'), error = document.createElement('small'), button = document.createElement('button');
      const signals = createSignalMonitor({ onCaptureNavigate: ({ capture, follow }) =>
        jumpToWorkerCaptures({ groupId: state.groupId, clientId: id, capture, follow }) });
      const size = document.createElement('small'), opening = document.createElement('small'), metrics = document.createElement('small');
      size.className = opening.className = metrics.className = 'process-measurement';
      process.className = 'process-id'; status.className = 'process-stage'; error.className = 'process-error'; button.textContent = 'Excel Hard Reset';
      button.title = 'Ends this Excel instance and opens a fresh copy with the current visibility setting. If comparison jobs have started, the group is compared again.';
      button.addEventListener('click', () => restartGroup(id, true));
      element.append(name, process, status, signals.element, size, opening, metrics, error, button); $('processes').append(element);
      card = { element, signals, name, process, status, size, opening, metrics, error, button }; processCards.set(id, card);
      opening.title = 'Native launch through workbook identity verification by the add-in. Includes Excel startup, workbook loading and add-in initialization. Historical COM Open measurements remain labelled separately.';
      metrics.title = 'One Excel-process memory snapshot at add-in readiness, before capture jobs. Private bytes and working set exclude separate WebView processes. Shrink duration comes from preparation.';
    }
    const alive = client.process_alive;
    card.signals.update(rows.find(row => row.clientId === id), { connected: state.connected,
      live: state.queue?.running !== false && state.queue?.runner_alive !== false });
    card.element.classList.toggle('blocked', ['blocked_edit_mode', 'exit_unconfirmed'].includes(client.stage) || Boolean(client.error));
    card.name.textContent = profileLabel(id);
    card.process.textContent = client.pid ? `PID ${client.pid} · ${alive === true ? 'Process running' : alive === false ? 'Process exited' : 'Process status unverified'} · ${client.window_mode === 'private_desktop' ? 'private desktop (hidden)' : client.hide_excel ? 'hidden' : 'visible'}` : 'No PID yet';
    card.status.textContent = reported[id] ? [stageLabels[client.stage] || normalizeStatus(client.status), client.active_command].filter(Boolean).join(' · ')
      : data?.status === 'preparing' ? 'Preparing · no add-in started yet' : 'No add-in reported yet';
    card.error.textContent = client.error || ''; card.error.hidden = !client.error;
    const measurements = processMeasurements(data, id);
    card.size.textContent = measurements.size; card.opening.textContent = measurements.opening;
    card.metrics.textContent = measurements.extra; card.metrics.hidden = !measurements.extra;
    const resetting = data?.client_reset_commands?.some(command => command.client_id === id && ['pending','running'].includes(command.status));
    card.button.disabled = !reported[id] || resetting || data?.restart_eligible === false ||
      data?.launcher?.client_reset_supported === false || state.queue?.running === false;
    card.button.textContent = resetting ? 'Hard reset running…' : 'Excel Hard Reset';
    card.element.title = 'A missing heartbeat can mean Excel is opening or busy. Process status is checked separately.';
  }
}
function renderThroughput() {
  const metric = data?.throughput, settings = data?.capture_settings;
  $('pixelRate').textContent = metric?.pixels_per_second != null ? `${(metric.pixels_per_second / 1e6).toLocaleString('en-US', { maximumFractionDigits: 2 })} MP/s` : '— MP/s';
  $('captureTotals').textContent = metric ? `${Number(metric.uploaded_images || 0).toLocaleString('en-US')} Uploads · ${(Number(metric.captured_pixels || 0) / 1e6).toLocaleString('en-US', { maximumFractionDigits: 1 })} MP · ${(Number(metric.png_bytes || 0) / 1048576).toLocaleString('en-US', { maximumFractionDigits: 1 })} MiB` : 'Waiting for measured captures';
  $('effectiveCapture').textContent = settings ? `This group: ${settings.tile_megapixels} MP · Batch ${settings.effective_batch_tiles ?? settings.batch_tiles}${settings.effective_batch_tiles != null && settings.effective_batch_tiles !== settings.batch_tiles ? ` / target ${settings.batch_tiles}` : ''}` : 'Settings take effect when a group starts.';
}
async function restartGroup(clientId = null, hardReset = false, fullScanDeferred = false) {
  if (!state.groupId) return;
  const groupId = state.groupId;
  const expectedAttempt = data?.attempt_id;
  try {
    const controlDeadline = Date.now() + 10000;
    while (hardReset && controlBusy && Date.now() < controlDeadline) await new Promise(resolve => setTimeout(resolve, 50));
    if (hardReset && controlBusy) throw new Error('The visibility setting has not been confirmed yet.');
    const response = await fetch(`/api/groups/${encodeURIComponent(groupId)}/restart`, { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...(clientId ? { client_id: clientId } : {}), ...(hardReset ? {hard_reset:true} : {}), ...(fullScanDeferred ? {full_scan_deferred:true} : {}), ...(expectedAttempt ? {expected_attempt_id:expectedAttempt} : {}), reason: clientId ? `Excel ${clientId} reload through Observatory` : fullScanDeferred ? 'Start deferred full scan through Observatory' : 'Requeue group through Observatory' }) });
    const result = await response.json(); if (!response.ok || result.ok === false) throw new Error(result.error || `Restart rejected (${response.status})`);
    setNotice(result.restart_scope === 'client'
      ? 'Hard reset requested: Excel will reopen with the current visibility setting. If comparison jobs have started, the group will be compared again.'
      : fullScanDeferred
        ? 'Deferred full scan queued. If the group target was 0, it was raised to 1. The 4,096-tile capture limit may still stop exceptionally large sheets.'
        : 'Restart requested. The entire group will be queued next with the current settings.');
    pushEvent(`Restart requested${clientId ? ` for ${profileLabel(clientId)} and its comparison group` : ' for the group'}.`);
  } catch (error) { setNotice(error.message, true); }
}
function nextDifference() {
  const targets = differenceTargets(entries);
  const index = targets.findIndex(({ tile, region }) => tile.id === state.selected && region.id === state.regionId);
  const target = targets[(index + 1) % targets.length];
  if (!target) { setNotice('No confirmed pixel difference for this group yet.'); return; }
  focusTile(target.tile, target.region, true);
}
function nextSheetDifference() {
  const sheets = data?.sheets.filter(sheet => sheet.tiles.some(tile => tile.comparisonComplete && !tile.equalAcrossAll)) || [];
  const index = sheets.findIndex(sheet => sheet.id === state.sheetId);
  const sheet = sheets[(index + 1) % sheets.length];
  if (!sheet) { setNotice('No sheet with a confirmed difference yet.'); return; }
  const target = differenceTargets(sheet.tiles)[0]; focusTile(target.tile, target.region, true);
}
async function nextGroupDifference() {
  if (navigationBusy) return;
  const groups = state.queue?.groups || [], start = groups.findIndex(group => String(group.id) === state.groupId);
  const initialGroup = state.groupId;
  const ordered = Array.from({ length: groups.length }, (_, offset) => groups[(start + offset + 1 + groups.length) % groups.length]);
  navigationBusy = true; $('nextGroupDifference').disabled = true;
  try {
    // Finished outcomes come from the queue. Active groups get an on-demand,
    // bounded scan: four requests at a time, at most sixteen active snapshots.
    const active = ordered.filter(group => group.active && String(group.id) !== initialGroup).slice(0, 16), liveDifferences = new Set();
    for (let index = 0; index < active.length; index += 4) {
      await Promise.allSettled(active.slice(index, index + 4).map(async group => {
        const response = await fetch(`/api/groups/${encodeURIComponent(group.id)}`, { cache: 'no-store', signal: AbortSignal.timeout(1500) });
        if (!response.ok) return;
        const raw = await response.json();
        if (raw.sheets?.some(sheet => sheet.tiles?.some(tile => tile.status === 'different'))) liveDifferences.add(String(group.id));
      }));
      if (state.groupId !== initialGroup) return;
    }
    for (const group of ordered) {
      if (group.status === 'differences' || Number(group.different_tiles ?? group.differences ?? 0) > 0 || liveDifferences.has(String(group.id))) {
        stopFollowing(); state.pendingDifference = true; selectGroup(String(group.id)); return;
      }
    }
    setNotice('No further group with a reported difference found.');
  } finally { navigationBusy = false; $('nextGroupDifference').disabled = false; }
}

function wireControls() {
  $('stopRun').addEventListener('click', async () => {
    $('stopRun').disabled = true;
    try {
      const response = await fetch('/api/run/stop', { method: 'POST' });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || `Stop failed (${response.status}).`);
      setNotice(result.message);
    } catch (error) { setNotice(error.message, true); $('stopRun').disabled = false; }
  });
  $('terminateRun').addEventListener('click', async () => {
    if (!window.confirm('Stop this run now? Active workbook workers will be cancelled and their Excel sessions closed. Completed results remain saved.')) return;
    $('terminateRun').disabled = true;
    try {
      const response = await fetch('/api/run/terminate', { method: 'POST' });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || `Immediate stop failed (${response.status}).`);
      $('stopRun').disabled = true;
      setNotice(result.message);
    } catch (error) {
      setNotice(error.message, true);
      $('terminateRun').disabled = false;
    }
  });
  $('groupSearch').addEventListener('input', () => {
    clearTimeout(workbookSearchTimer);
    workbookSearchTimer = setTimeout(() => {
      groupLimit = 100;
      void loadWorkbookPage(true);
    }, 250);
  });
  $('groupFilter').addEventListener('change', () => {
    groupLimit = 100;
    void loadWorkbookPage(true);
  });
  $('moreGroups').addEventListener('click', () => {
    if (workbookIndex?.available) void loadWorkbookPage(false);
    else { groupLimit += 100; renderGroups(); }
  });
  $('moreWorkbookResults').addEventListener('click', () => void loadWorkbookPage(false));
  $('closeWorkbookResults').addEventListener('click', () => {
    $('workbookResultList').hidden = true;
  });
  function adjustTarget(delta) { const value = Math.max(0, Number($('targetGroups').value || 0) + delta); dirtyControls.add('targetGroups'); $('targetGroups').value = String(value); $('groupRange').max = String(Math.max(24, value)); $('groupRange').value = String(value); }
  $('minus').addEventListener('click', () => adjustTarget(-1)); $('plus').addEventListener('click', () => adjustTarget(1));
  for (const [id, key, valueId, statusId] of [
    ['sweepProcesses', 'sweep_processes', 'sweepProcessesValue', 'sweepProcessesStatus'],
    ['prepareProcesses', 'prepare_processes', 'prepareProcessesValue', 'prepareProcessesStatus'],
    ['shrinkThreads', 'shrink_threads', 'shrinkThreadsValue', 'shrinkThreadsStatus'],
  ]) {
    $(id).addEventListener('input', () => {
      dirtyControls.add(id);
      $(valueId).textContent = $(id).value;
      $(statusId).textContent = `Selected: ${$(id).value} · release to apply to new processes.`;
    });
    $(id).addEventListener('change', () => sendControl({ [key]: Number($(id).value) }));
  }
  for (const id of ['targetGroups', 'systemReserve', 'openTimeout', 'jobTimeout', 'equalPreview', 'captureMegapixels', 'captureBatch', 'captureContext', 'tallSheetThreshold', 'tallSheetEdges', 'crcEqualSheetThreshold']) $(id).addEventListener('input', () => dirtyControls.add(id));
  $('applyReserve').addEventListener('click', () => {
    const reserve = Number($('systemReserve').value);
    if (!Number.isSafeInteger(reserve) || reserve < 0 || reserve > 65536) return setNotice('Free RAM reserve must be a whole number from 0 to 65,536 MiB.', true);
    sendControl({ system_reserve_mib: reserve });
  });
  $('disableReserve').addEventListener('click', () => {
    $('systemReserve').value = '0';
    dirtyControls.delete('systemReserve');
    sendControl({ system_reserve_mib: 0 });
  });
  $('targetGroups').addEventListener('input', () => { const value = Number($('targetGroups').value); if (Number.isSafeInteger(value) && value >= 0) { $('groupRange').max = String(Math.max(24, value)); $('groupRange').value = String(value); } });
  $('groupRange').addEventListener('input', event => { dirtyControls.add('targetGroups'); $('targetGroups').value = event.target.value; });
  $('groupRange').addEventListener('change', event => sendControl({ target_groups: Number(event.target.value) }));
  $('applyTarget').addEventListener('click', () => { const value = Number($('targetGroups').value); if (!Number.isSafeInteger(value) || value < 0) return setNotice('Enter a whole number of groups, at least 0.', true); sendControl({ target_groups: value }); });
  $('targetGroups').addEventListener('keydown', event => { if (event.key === 'Enter') $('applyTarget').click(); });
  $('applyExcelProcessLimit').addEventListener('click', () => {
    const value = Number($('excelProcessLimit').value);
    if (!Number.isSafeInteger(value) || value < 0 || value > 2147483647) return setNotice('Enter a nonnegative Excel process limit; 0 means unlimited.', true);
    sendControl({ excel_process_limit: value });
  });
  $('pauseJobs').addEventListener('click', () => sendControl({ paused: !state.controls.paused }));
  $('applyTimeouts').addEventListener('click', () => {
    const open = Number($('openTimeout').value), job = Number($('jobTimeout').value);
    if (![open, job].every(value => Number.isSafeInteger(value) && value > 0)) return setNotice('Timeouts must be positive whole seconds.', true);
    sendControl({ open_timeout_seconds: open, job_timeout_seconds: job });
  });
  $('equalPreview').addEventListener('change', event => sendControl({ equal_preview_width: Number(event.target.value) }));
  $('captureTransport').addEventListener('change', event => {
    dirtyControls.add('captureTransport');
    sendControl({ capture_transport: event.target.value });
  });

  $('applyCapture').addEventListener('click', () => {

    const size = Number($('captureMegapixels').value), batch = Number($('captureBatch').value);
    if (!Number.isInteger(size) || size < 1 || size > 64 || !Number.isInteger(batch) || batch < 1 || batch > 16) return setNotice('Target capture size: 1–64 MP. Batch: 1–16 jobs.', true);
    const context = Number($('captureContext').value);
    if (!Number.isInteger(context) || context < 0 || context > 16) return setNotice('Difference context must be 0 to 16 capture bands.', true);
    sendControl({ capture_tile_megapixels: size, capture_batch_tiles: batch, capture_context_tiles: context });
  });
  $('applyTallSheet').addEventListener('click', () => {
    const threshold = Number($('tallSheetThreshold').value), edges = Number($('tallSheetEdges').value);
    if (![threshold, edges].every(value => Number.isInteger(value) && value >= 1 && value <= 2147483647)) return setNotice('Capture rows must be positive integers (at most 2,147,483,647).', true);
    sendControl({ tall_sheet_threshold: threshold, tall_sheet_edge_tiles: edges });
  });
  $('applyCrcEqualSheet').addEventListener('click', () => {
    const threshold = Number($('crcEqualSheetThreshold').value);
    if (!Number.isInteger(threshold) || threshold < 1 || threshold > 2147483647) {
      return setNotice('CRC-equal sheet threshold must be a positive integer in MiB.', true);
    }
    sendControl({ crc_equal_sheet_threshold_mib: threshold });
  });
  $('hideExcel').addEventListener('change', event => sendControl({ hide_excel: event.target.checked }));
  $('restartGroup').addEventListener('click', () => restartGroup());
  $('startDeferredScan').addEventListener('click', () => restartGroup(null, false, true));
  $('followCapture').addEventListener('change', event => {
    state.follow = event.target.checked;
    if (state.follow) {
      clearDetails();
      followTile(followedProfile ? latestTileForProfile(data, followedProfile) : tileAt(latestCapture));
    }
  });
  $('sheetSelect').addEventListener('change', event => focusSheet(event.target.value, true));
  for (const [id, axis] of [['panX', 'x'], ['panY', 'y']]) $(id).addEventListener('input', event => {
    const sheet = selectedSheet(); if (!sheet) return;
    stopFollowing(); rememberView(sheet, { ...sheetView(sheet), [axis]: Number(event.target.value) });
    clearDetails(); renderScene();
  });
  $('highlightDiff').checked = state.highlightDiff;
  $('highlightDiff').addEventListener('change', event => { state.highlightDiff = event.target.checked; renderComparePanels(); });
  $('nextDifference').addEventListener('click', nextDifference); $('nextSheetDifference').addEventListener('click', nextSheetDifference); $('nextGroupDifference').addEventListener('click', nextGroupDifference);
  $('resetCamera').addEventListener('click', () => { stopFollowing(); clearDetails(); state.focus = 'overview'; renderScene(); fitCamera(true); });
  $('stageMode').addEventListener('click', () => { document.body.classList.toggle('presenting'); $('stageMode').textContent = document.body.classList.contains('presenting') ? '⛶ Back' : '⛶ Present'; });
  $('resetCompareZoom').addEventListener('click', resetCompareZoom);
  $('closeSelection').addEventListener('click', () => clearDetails());
  $('closeImage').addEventListener('click', () => $('imageDialog').close());
  $('toggleImageSize').addEventListener('click', () => { const native = $('imageScroll').classList.toggle('native'); $('toggleImageSize').textContent = native ? 'Fit to window' : 'Show 1:1'; });
  document.addEventListener('keydown', event => { if (event.key === 'Escape' && !$('imageDialog').open) clearDetails(); });
}
function snapshot() {
  return { ready: state.ready, live: true, connected: state.connected, error: state.error, controls: state.controls,
    running: state.queue?.running, runnerAlive: state.queue?.runner_alive, phase: state.queue?.phase,
    groupId: state.groupId, sheetId: state.sheetId, selected: state.selected, regionId: state.regionId, focus: state.focus, sheetCards: sheetCards.size, pinned: state.pinned, follow: state.follow,
    view: { ...sheetView(selectedSheet()) }, sheetViews: data?.sheets.map(sheet => ({ id: sheet.id, view: sheetView(sheet) })), profiles: data?.profiles, status: data?.status, launcher: data?.launcher,
    groupCount: state.queue?.groups?.length || 0, totalTiles: entries.length, visibleTiles: [...new Set(sourceTiles.map(tile => tile.entry.id))],
    tiles: sourceTiles.map(tile => ({ id: tile.entry.id, address: tile.entry.address, profile: tile.profile.id, status: imageStatus(tile.entry, tile.profile.id), loaded: tile.loaded, rectangle: tile.rectangle, clip: tile.clip, offscreen: tile.offscreen })),
    details: detailPanels.map(panel => ({ profiles: panel.group.ids, pixelClass: panel.group.pixelClass,
      sameAsOriginal: panel.sameAsOriginal, fullReady: panel.fullReady, maskPresent: Boolean(panel.mask),
      maskOpacity: state.highlightDiff && panel.mask ? .7 : 0, clip: panel.clip, aspect: panel.clip.width / panel.clip.height })),
    compareZoom, compareOpen: $('compareDock').closest('.room').classList.contains('comparison-open'),
    connections: connections.length, textureCount: renderer?.info.memory.textures || 0, textureCache: textureCache.size,
    viewport: { width: renderer?.domElement.clientWidth, height: renderer?.domElement.clientHeight } };
}
function geometrySnapshot() {
  scene.updateMatrixWorld(true); const rect = renderer.domElement.getBoundingClientRect();
  const project = value => { const ndc = value.clone().project(camera); return { world: value.toArray(), screen: [rect.left + (ndc.x + 1) * rect.width / 2, rect.top + (1 - ndc.y) * rect.height / 2] }; };
  const planned = [], profileWindows = [];
  sheetCards.get(state.sheetId)?.traverse(board => {
    const info = board.userData.pixelBoard; if (!info || !info.profile || info.board) return;
    profileWindows.push({ sheet: info.sheet.id, profile: info.profile.id, view: { ...info.view },
      center: project(board.localToWorld(new THREE.Vector3(0, AREA_Y, .02))) });
    for (const entry of visibleTiles(info.sheet, info.view, 1024)) {
      const r = profileRectangle(entry, info.profile.id), px = AREA_W / info.view.width;
      const x = (r.x - info.view.x + r.width / 2) * px - AREA_W / 2;
      const y = AREA_Y + AREA_H / 2 - (r.y - info.view.y + r.height / 2) * px;
      const point = new THREE.Vector3(RADIUS * Math.sin(x / RADIUS), y, RADIUS * (1 - Math.cos(x / RADIUS)));
      planned.push({ id: entry.id, profile: info.profile.id, center: project(board.localToWorld(point)) });
    }
  });
  return { sources: sourceTiles.map(tile => ({ id: tile.entry.id, profile: tile.profile.id, center: project(tile.group.getWorldPosition(new THREE.Vector3())) })),
    planned, profileWindows, camera: { position: camera.position.toArray(), target: controls.target.toArray() },
    boards: boards.map(board => ({ sheet: board.userData.sheetId, position: board.position.toArray(), rotationY: board.rotation.y, center: project(board.getWorldPosition(new THREE.Vector3())) })),
    details: detailPanels.map(panel => { const bounds = panel.element.getBoundingClientRect(); return {
      profiles: panel.group.ids, center: { screen: [bounds.left + bounds.width / 2, bounds.top + bounds.height / 2] } }; }),
    connections: connections.map(link => ({ sourceProfile: link.source.profile.id, profiles: link.panel.group.ids,
      path: link.path.getAttribute('d') })) };
}

window.__observatory = { snapshot, geometry: geometrySnapshot, selectGroup, jumpToWorkerCaptures, nextDifference, nextSheetDifference, nextGroupDifference, focusSheet,
  setSheetFit,
  hoverTile(id, regionId) { const tile = tileAt(typeof id === 'number' ? entries[id]?.id : id); return showDetails(tile?.id, true, inspectionRegions(tile).find(region => region.id === regionId)); }, clearHover: clearDetails, pin: setPinned,
  setView(x, y, width = state.view.width) { const sheet = selectedSheet(); if (!sheet) return; stopFollowing(); state.focus = 'tile'; rememberView(sheet, { mode: 'manual', x, y, width }); renderScene(); },
  openEvidence(index = 0) { if (detailPanels[index]) openEvidence(detailPanels[index]); } };
try { initScene(); wireControls(); workerMonitor = initWorkerMonitor({ selectGroup, navigateToCaptures: jumpToWorkerCaptures }); poll(); }
catch (error) { state.error = error.message; $('error').textContent = error.message; $('error').hidden = false; console.error(error); }
