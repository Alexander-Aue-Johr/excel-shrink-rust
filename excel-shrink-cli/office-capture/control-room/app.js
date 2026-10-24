import * as THREE from 'three';
import { OrbitControls } from './vendor/OrbitControls.js';
import { normalizeGroup, imageStatus, groupedPanels, visibleTiles, clippedTile, profileRectangle, activeGroupCount } from './model.js';

const $ = id => document.getElementById(id);
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
const C = { waiting: 0x3c566b, received: 0x79dce8, equal: 0x75edc0, different: 0xff7a91 };
const BOARD_W = 5.72, BOARD_H = 5.5, BOARD_STEP = 6.02, RADIUS = 18;
const AREA_W = 5.24, AREA_H = 4.06, AREA_Y = -.38;
const state = { ready: false, connected: false, groupId: null, sheetId: null, selected: null,
  pinned: false, pulse: !reducedMotion, follow: true, error: null, controls: {}, queue: null,
  view: { x: 0, y: 0, width: 1800, height: 1800 * AREA_H / AREA_W }, widthChoice: 1800 };
const boards = [], sourceTiles = [], hitTargets = [], detailPanels = [], connections = [];
const raycaster = new THREE.Raycaster(), pointer = new THREE.Vector2();
const textureCache = new Map(), flashes = new Map(), previousImages = new Map();
const dirtyControls = new Set();
const loadQueue = [];
let activeLoads = 0, lastSignature = '', detailSignature = '', groupSignature = '', sheetSignature = '';
let data, entries = [], renderer, scene, camera, controls, detailRoot, lastUi = 0;
let selectedGeneration = 0, sceneGeneration = 0, pollGeneration = 0, activeDetailSince = 0;
let hoverTimer, leaveTimer, pendingHover, pointerDown, userMovedCamera = false, cameraTween;
let latestCapture = null, pollTimer, controlBusy = false, localEvents = [];
let groupLimit = 100;

function announce(text) { $('announcement').textContent = text; }
function profileLabel(id) { return data?.profiles.find(profile => profile.id === id)?.label || id; }
function tileAt(id) { return entries.find(tile => tile.id === id); }
function selectedSheet() { return data?.sheets.find(sheet => sheet.id === state.sheetId); }
function aspectOf(entry) { return Math.max(.02, entry.width / entry.height || 1); }
function fitSize(aspect, width, height) { return aspect > width / height ? [width, width / aspect] : [height * aspect, height]; }
function labelGroup(group) { return group.ids.map(profileLabel).join(' · '); }
function safeImageUrl(url) {
  const value = new URL(url, location.href);
  if (value.origin !== location.origin || !value.pathname.startsWith('/data/')) throw new Error('Der Bildpfad liegt außerhalb des lokalen Ergebnisverzeichnisses.');
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
  return new THREE.Mesh(new THREE.PlaneGeometry(width, height), new THREE.MeshBasicMaterial({
    map: canvasLabel(lines, 1280, canvasHeight), transparent: true, depthWrite: false, side: THREE.DoubleSide, toneMapped: false }));
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
function clippedPlane(width, height, clip) {
  const geometry = new THREE.PlaneGeometry(width, height), uv = geometry.attributes.uv;
  for (let index = 0; index < uv.count; index++) uv.setXY(index,
    THREE.MathUtils.lerp(clip.u0, clip.u1, uv.getX(index)), THREE.MathUtils.lerp(clip.v0, clip.v1, uv.getY(index)));
  return geometry;
}
function frame(width, height, color, curved = false) {
  const geometry = curved ? curvedGeometry(width, height) : new THREE.PlaneGeometry(width, height);
  const edges = new THREE.EdgesGeometry(geometry); geometry.dispose();
  return new THREE.LineSegments(edges, new THREE.LineBasicMaterial({ color, transparent: true, opacity: .8, depthWrite: false }));
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
function loadTexture(url, maxWidth = 1024) {
  const key = `${url}#${maxWidth}`;
  if (textureCache.has(key)) { const cached = textureCache.get(key); cached.touched = performance.now(); return cached.promise; }
  const entry = { touched: performance.now(), texture: null };
  entry.promise = new Promise((resolve, reject) => {
    const run = async () => {
      try {
        const response = await fetch(safeImageUrl(url)); if (!response.ok) throw new Error(`Bild konnte nicht geladen werden (${response.status}).`);
        const source = await createImageBitmap(await response.blob(), { imageOrientation: 'flipY', premultiplyAlpha: 'none' });
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
    loadQueue.push({ url, run, cancel() { textureCache.delete(key); reject(new DOMException('Der sichtbare Ausschnitt hat sich geändert.', 'AbortError')); } });
    drainLoads();
  }); textureCache.set(key, entry); return entry.promise;
}

function initScene() {
  scene = new THREE.Scene(); scene.background = new THREE.Color('#08111c'); scene.fog = new THREE.FogExp2('#08111c', .007);
  camera = new THREE.PerspectiveCamera(43, 1, .1, 180);
  renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'low-power' });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 1.5)); renderer.outputColorSpace = THREE.SRGBColorSpace; renderer.toneMapping = THREE.NoToneMapping;
  $('viewport').append(renderer.domElement);
  controls = new OrbitControls(camera, renderer.domElement); controls.enableDamping = !reducedMotion; controls.dampingFactor = .075;
  controls.minDistance = 2; controls.maxDistance = 100; controls.maxPolarAngle = Math.PI * .85; controls.rotateSpeed = .5; controls.panSpeed = .7;
  controls.addEventListener('start', () => { userMovedCamera = true; cameraTween = null; cancelHover(); });
  const grid = new THREE.GridHelper(80, 80, 0x335b6e, 0x173344); grid.position.y = -3.8; grid.material.transparent = true; grid.material.opacity = .3; scene.add(grid);
  const disk = new THREE.Mesh(new THREE.CircleGeometry(36, 96), new THREE.MeshBasicMaterial({ color: 0x10293a, transparent: true, opacity: .22, depthWrite: false })); disk.rotation.x = -Math.PI / 2; disk.position.y = -3.82; scene.add(disk);
  detailRoot = new THREE.Group(); scene.add(detailRoot);
  new ResizeObserver(resize).observe($('viewport')); resize();
  renderer.domElement.addEventListener('pointermove', handlePointerMove);
  renderer.domElement.addEventListener('pointerleave', scheduleClear);
  renderer.domElement.addEventListener('pointerdown', event => { pointerDown = { x: event.clientX, y: event.clientY }; });
  renderer.domElement.addEventListener('pointerup', event => {
    if (!pointerDown || Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) > 7) return;
    const hit = hitAt(event);
    if (hit?.userData.sourceTile) { const tile = hit.userData.sourceTile; showDetails(tile.entry.id); setPinned(true); }
    else if (hit?.userData.detailPanel) setPinned(!state.pinned);
    pointerDown = null;
  });
  requestAnimationFrame(tick);
}
function fitCamera(animate = false) {
  const horizontalTangent = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)) * camera.aspect;
  const halfWidth = Math.max(11.5, (data?.profiles.length || 4) * BOARD_STEP / 2);
  const detailWidth = detailPanels.reduce((sum, panel) => sum + panel.width, 0) + Math.max(0, detailPanels.length - 1) * .3;
  const distance = Math.max(24, halfWidth / (horizontalTangent * .88) + 3.5, detailWidth / 2 / (horizontalTangent * .88) + 5);
  const position = new THREE.Vector3(0, 6, distance), target = new THREE.Vector3(0, 1, 1.7);
  if (animate && !reducedMotion) cameraTween = { start: performance.now(), from: camera.position.clone(), to: position, fromTarget: controls.target.clone(), target };
  else { camera.position.copy(position); controls.target.copy(target); controls.update(); }
  userMovedCamera = false;
}
function resize() {
  const { width, height } = $('viewport').getBoundingClientRect(); if (!width || !height) return;
  renderer.setSize(width, height, false); camera.aspect = width / height; camera.updateProjectionMatrix(); if (!userMovedCamera) fitCamera();
}
function setCurvedPosition(group, x, y, z = 0) {
  const angle = x / RADIUS;
  group.position.set(RADIUS * Math.sin(angle), y, RADIUS * (1 - Math.cos(angle)) + z); group.rotation.y = -angle;
}
function makeBoard(profile, profileIndex, shown) {
  const board = new THREE.Group(); setCurvedPosition(board, (profileIndex - (data.profiles.length - 1) / 2) * BOARD_STEP, -.7); scene.add(board);
  board.add(new THREE.Mesh(curvedGeometry(BOARD_W, BOARD_H), new THREE.MeshBasicMaterial({ color: profileIndex === 0 ? 0x163044 : 0x10283b, side: THREE.DoubleSide })));
  const edge = frame(BOARD_W, BOARD_H, profileIndex === 0 ? 0x5e94a1 : 0x3c667e, true); edge.position.z = .012; board.add(edge);
  const title = makeTextPlane([{ text: profile.label, y: 65, size: 62 }, { text: selectedSheet()?.name || 'Warte auf Sheet', y: 133, size: 26, color: '#93b3c5' }], BOARD_W - .4, .63); title.position.set(0, 2.28, .04); board.add(title);
  const px = AREA_W / state.view.width;
  for (const entry of shown) {
    const rectangle = profileRectangle(entry, profile.id);
    let clip = clippedTile(rectangle, state.view);
    const offscreen = clip.width <= 0 || clip.height <= 0;
    if (offscreen) clip = { ...rectangle, u0: 0, u1: 1, v0: 0, v1: 1 };
    const width = offscreen ? .02 : clip.width * px, height = offscreen ? .02 : clip.height * px;
    const x = THREE.MathUtils.clamp((clip.x - state.view.x + clip.width / 2) * px - AREA_W / 2, -AREA_W / 2, AREA_W / 2);
    const y = THREE.MathUtils.clamp(AREA_Y + AREA_H / 2 - (clip.y - state.view.y + clip.height / 2) * px, AREA_Y - AREA_H / 2, AREA_Y + AREA_H / 2);
    const group = new THREE.Group(); setCurvedPosition(group, x, y, .032); board.add(group);
    const backing = new THREE.Mesh(curvedGeometry(width, height), new THREE.MeshBasicMaterial({ color: 0x1b3547, side: THREE.DoubleSide })); group.add(backing);
    const image = new THREE.Mesh(curvedGeometry(width, height, clip), new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0, depthWrite: false, side: THREE.DoubleSide, toneMapped: false, fog: false })); image.position.z = .007; group.add(image);
    const rim = frame(width, height, C.waiting, true); rim.position.z = .016; group.add(rim);
    const glow = makeGlow(width, height, C.waiting); glow.position.z = .012; group.add(glow);
    const tile = { group, image, rim, glow, clip, rectangle, offscreen, entry, imageData: entry.images[profile.id], p: profileIndex, profile, width, height, loaded: false };
    backing.userData.sourceTile = tile; image.userData.sourceTile = tile;
    for (const object of [backing, image, rim, glow]) object.visible = !offscreen;
    if (!offscreen) hitTargets.push(backing, image);
    sourceTiles.push(tile);
  }
  boards.push(board);
}
function renderScene(force = false) {
  if (!data) return;
  const shown = visibleTiles(selectedSheet(), state.view);
  const signature = JSON.stringify([state.groupId, state.sheetId, state.view, data.profiles, shown.map(tile => [tile.id, tile.x, tile.y, tile.width, tile.height, tile.status, tile.comparisonComplete, tile.images])]);
  if (!force && signature === lastSignature) return;
  lastSignature = signature; const generation = ++sceneGeneration;
  const selection = state.selected, pinned = state.pinned;
  clearDetails(); boards.splice(0).forEach(disposeObject); sourceTiles.length = 0; hitTargets.length = 0;
  data.profiles.forEach((profile, index) => makeBoard(profile, index, shown));
  const required = new Set(sourceTiles.filter(tile => !tile.offscreen && tile.imageData?.url).map(tile => tile.imageData.previewUrl || tile.imageData.url));
  for (const tile of shown) if (tile.id === selection) for (const image of Object.values(tile.images)) {
    required.add(image.fullUrl || image.url); if (image.maskUrl) required.add(image.maskUrl);
  }
  for (let index = loadQueue.length - 1; index >= 0; index--) if (!required.has(loadQueue[index].url)) loadQueue.splice(index, 1)[0].cancel();
  state.ready = true; $('emptyScene').hidden = shown.length > 0;
  if (!shown.length) $('emptyScene').querySelector('strong').textContent = data.available === false ? 'Die Gruppe wartet auf Capture-Aufträge.' : 'In diesem Ausschnitt liegen noch keine aufgenommenen Kacheln.';
  renderTileButtons(shown);
  const loads = sourceTiles.filter(tile => !tile.offscreen && tile.imageData?.url).map(async tile => {
    try {
      const texture = await loadTexture(tile.imageData.previewUrl || tile.imageData.url);
      if (generation !== sceneGeneration) return;
      tile.image.material.map = texture; tile.image.material.needsUpdate = true; tile.loaded = true;
    } catch (error) { if (generation === sceneGeneration && error.name !== 'AbortError') setNotice(error.message, true); }
  });
  Promise.all(loads).then(() => { if (generation === sceneGeneration) sweepTextures(); });
  if (selection && shown.some(tile => tile.id === selection)) { showDetails(selection, false); setPinned(pinned); }
  updatePanControls();
}

function makeDetailPanel(group, entry, index, count) {
  const source = sourceTiles.find(tile => tile.entry.id === entry.id && tile.profile.id === group.ids[0]);
  if (!source) return;
  const representative = entry.images[group.ids[0]], sameAsOriginal = group.pixelClass != null && group.pixelClass === entry.images.original?.pixelClass;
  const width = count >= 4 ? 5.2 : count === 3 ? 6.45 : 7.5;
  // A huge job tile may extend beyond the shared sheet window. Its detail clone
  // uses the same tile-local pixel window for every peer. Different row heights
  // must neither stretch an image nor make peers show different local regions.
  const focus = sourceTiles.find(tile => tile.entry.id === entry.id && !tile.offscreen && tile.profile.id === 'original')
    || sourceTiles.find(tile => tile.entry.id === entry.id && !tile.offscreen) || source;
  let clip = clippedTile({ x: 0, y: 0, width: source.rectangle.width, height: source.rectangle.height },
    { x: focus.clip.x - focus.rectangle.x, y: focus.clip.y - focus.rectangle.y, width: focus.clip.width, height: focus.clip.height });
  if (clip.width <= 0 || clip.height <= 0) clip = { width: source.rectangle.width, height: source.rectangle.height, u0: 0, u1: 1, v0: 0, v1: 1 };
  const [imageW, imageH] = fitSize(clip.width / clip.height, width - .2, 3.65), height = imageH + .94;
  const root = new THREE.Group(); detailRoot.add(root);
  const backing = new THREE.Mesh(new THREE.BoxGeometry(width, height, .045), new THREE.MeshBasicMaterial({ color: 0x102335 })); root.add(backing);
  const color = group.original ? C.received : group.pixelClass == null ? C.received : sameAsOriginal ? C.equal : C.different;
  const image = new THREE.Mesh(clippedPlane(imageW, imageH, clip), new THREE.MeshBasicMaterial({ map: source.image.material.map, color: 0xffffff, side: THREE.DoubleSide, toneMapped: false, fog: false })); image.position.set(0, -.12, .031); root.add(image);
  const rim = frame(imageW + .03, imageH + .03, color); rim.position.copy(image.position); rim.position.z += .019; root.add(rim);
  const glow = makeGlow(imageW + .035, imageH + .035, color); glow.position.copy(image.position); glow.position.z += .014; root.add(glow);
  const title = makeTextPlane([{ text: labelGroup(group), y: 70, size: 56, align: 'center' }], width - .18, .45, 100); title.position.set(0, imageH / 2 + .2, .032); root.add(title);
  const subtitle = group.original ? 'ORIGINAL · VISUELLE REFERENZ' : group.pixelClass == null ? 'PIXELVERGLEICH AUSSTEHEND' : sameAsOriginal ? 'PIXELIDENTISCH MIT DEM ORIGINAL' : `${Number(representative.changedPixels || 0).toLocaleString('de-DE')} ABWEICHENDE PIXEL IN DER KACHEL`;
  const info = makeTextPlane([{ text: subtitle, y: 65, size: 34, color: group.original ? '#94dce8' : sameAsOriginal ? '#8ce8c4' : '#ffb0be', align: 'center' }], width - .2, .23, 100); info.position.set(0, -imageH / 2 - .37, .034); root.add(info);
  let mask = null;
  if (entry.comparisonComplete && representative.maskUrl && (group.original || !sameAsOriginal)) {
    mask = new THREE.Mesh(clippedPlane(imageW, imageH, clip), new THREE.MeshBasicMaterial({ color: 0xff315c, transparent: true, opacity: 0, depthWrite: false, side: THREE.DoubleSide, toneMapped: false, fog: false }));
    mask.position.copy(image.position); mask.position.z += .012; root.add(mask);
  }
  const start = source.group.getWorldPosition(new THREE.Vector3()); root.position.copy(start); root.quaternion.copy(source.group.getWorldQuaternion(new THREE.Quaternion())); root.scale.setScalar(source.width / width);
  const panel = { root, image, mask, rim, glow, group, entry, representative, source, clip, width, height, imageW, imageH, index, count, start, startQuaternion: root.quaternion.clone(), startScale: root.scale.x, target: new THREE.Vector3(), sameAsOriginal, fullReady: false };
  backing.userData.detailPanel = panel; image.userData.detailPanel = panel; hitTargets.push(backing, image); detailPanels.push(panel);
  group.ids.forEach(id => makeConnection(sourceTiles.find(tile => tile.entry.id === entry.id && tile.profile.id === id), panel, color));
}
function makeConnection(source, panel, color) {
  if (!source) return;
  const geometry = new THREE.BufferGeometry(), positions = new Float32Array(65 * 3); geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  const line = new THREE.Line(geometry, new THREE.LineBasicMaterial({ color, transparent: true, opacity: .55, depthWrite: false })); line.frustumCulled = false; scene.add(line);
  const dots = Array.from({ length: 3 }, () => { const dot = new THREE.Mesh(new THREE.SphereGeometry(.045, 9, 6), new THREE.MeshBasicMaterial({ color, transparent: true, opacity: .95, depthWrite: false })); scene.add(dot); return dot; });
  connections.push({ source, panel, line, dots, positions, curve: new THREE.CubicBezierCurve3(), offset: source.p * .12 });
}
async function showDetails(id, animate = true) {
  cancelHover(); clearTimeout(leaveTimer);
  const entry = tileAt(id); if (!entry || !state.ready || !sourceTiles.some(tile => tile.entry.id === id)) return;
  const signature = JSON.stringify([id, entry.comparisonComplete, entry.images]); if (state.selected === id && signature === detailSignature) return;
  clearDetails(); detailSignature = signature;
  const generation = ++selectedGeneration; state.selected = id; activeDetailSince = animate ? performance.now() : performance.now() - 1000;
  const groups = groupedPanels(entry, data.profiles); groups.forEach((group, index) => makeDetailPanel(group, entry, index, groups.length));
  $('selectionBar').hidden = false; $('selectionText').textContent = `${entry.sheetName} · ${entry.address}`;
  $('selectionSummary').textContent = `${groups.length} Bildgruppen · ${entry.comparisonComplete ? entry.equalAcrossAll ? 'identisch' : 'Abweichung' : 'Vergleich offen'} · Ausschnitt`;
  renderEvidenceActions(); announce($('selectionText').textContent + ': ' + $('selectionSummary').textContent);
  await Promise.all(detailPanels.map(async panel => {
    try {
      const image = panel.representative;
      const [texture, mask] = await Promise.all([loadTexture(image.fullUrl || image.url, image.fullUrl ? 3072 : 1536), panel.mask ? loadTexture(image.maskUrl, 3072) : null]);
      if (generation !== selectedGeneration) return;
      panel.image.material.map = texture; panel.image.material.needsUpdate = true; panel.fullReady = true;
      if (panel.mask && mask) { panel.mask.material.map = mask; panel.mask.material.needsUpdate = true; }
    } catch (error) { if (generation === selectedGeneration && error.name !== 'AbortError') setNotice(error.message, true); }
  }));
  sweepTextures();
}
function clearDetails() {
  ++selectedGeneration; clearTimeout(leaveTimer); cancelHover(); state.selected = null; state.pinned = false; detailSignature = '';
  for (let index = hitTargets.length - 1; index >= 0; index--) if (hitTargets[index].userData.detailPanel) hitTargets.splice(index, 1);
  detailPanels.splice(0).forEach(panel => disposeObject(panel.root));
  connections.splice(0).forEach(link => { disposeObject(link.line); link.dots.forEach(disposeObject); });
  $('selectionBar').hidden = true; $('evidenceActions').hidden = true; $('pinSelection').setAttribute('aria-pressed', 'false'); $('pinSelection').textContent = 'Festhalten';
}
function setPinned(value) { state.pinned = Boolean(value && state.selected); clearTimeout(leaveTimer); $('pinSelection').setAttribute('aria-pressed', String(state.pinned)); $('pinSelection').textContent = state.pinned ? 'Festgehalten' : 'Festhalten'; }
function cancelHover() { clearTimeout(hoverTimer); pendingHover = null; }
function scheduleClear() { cancelHover(); if (state.pinned) return; clearTimeout(leaveTimer); leaveTimer = setTimeout(() => { if (!state.pinned) clearDetails(); }, 480); }
function hitAt(event) { const rect = renderer.domElement.getBoundingClientRect(); pointer.set((event.clientX - rect.left) / rect.width * 2 - 1, -((event.clientY - rect.top) / rect.height) * 2 + 1); raycaster.setFromCamera(pointer, camera); return raycaster.intersectObjects(hitTargets, false)[0]?.object; }
function handlePointerMove(event) {
  if (!state.ready || event.buttons) return;
  const hit = hitAt(event); renderer.domElement.style.cursor = hit ? 'pointer' : 'grab';
  if (hit?.userData.detailPanel) { clearTimeout(leaveTimer); cancelHover(); return; }
  const tile = hit?.userData.sourceTile; if (!tile) { scheduleClear(); return; } clearTimeout(leaveTimer);
  if (state.pinned || state.selected === tile.entry.id || pendingHover === tile.entry.id) return;
  cancelHover(); pendingHover = tile.entry.id; hoverTimer = setTimeout(() => showDetails(tile.entry.id), 125);
}
function renderTileButtons(shown) {
  $('tileButtons').replaceChildren(...shown.map(entry => {
    const button = document.createElement('button'); button.textContent = entry.address || entry.id; button.dataset.tile = entry.id;
    button.classList.toggle('different', Object.keys(entry.images).some(profile => imageStatus(entry, profile) === 'different'));
    button.addEventListener('focus', () => showDetails(entry.id)); button.addEventListener('click', () => { showDetails(entry.id); setPinned(true); }); return button;
  }));
}
function renderEvidenceActions() {
  $('evidenceActions').hidden = !state.selected;
  $('evidenceActions').replaceChildren(...detailPanels.map(panel => { const button = document.createElement('button'); button.textContent = `${labelGroup(panel.group)} ↗`; button.addEventListener('click', () => openEvidence(panel)); return button; }));
}
function openEvidence(panel) {
  const image = panel.representative;
  $('fullImage').src = safeImageUrl(image.fullUrl || image.url); $('imageTitle').textContent = labelGroup(panel.group);
  $('imageDescription').textContent = `${panel.entry.sheetName} · ${panel.entry.address} · ${image.fullUrl ? 'vollständiger Capture' : 'gespeicherte Vorschau'}`;
  $('imageScroll').classList.remove('native'); $('toggleImageSize').textContent = '1:1 anzeigen'; $('imageDialog').showModal(); setPinned(true);
}

function updateDetails(now) {
  if (!detailPanels.length) return;
  const t = reducedMotion ? 1 : Math.min(1, (now - activeDetailSince) / 720), ease = 1 - Math.pow(1 - t, 3);
  const right = new THREE.Vector3(1, 0, 0).applyQuaternion(camera.quaternion);
  const up = new THREE.Vector3(0, 1, 0).applyQuaternion(camera.quaternion);
  const toward = new THREE.Vector3(0, 0, 1).applyQuaternion(camera.quaternion);
  const anchor = new THREE.Vector3(0, 3.55, 2).addScaledVector(toward, 2.5);
  const width = detailPanels.reduce((sum, panel) => sum + panel.width, 0) + (detailPanels.length - 1) * .33;
  let cursor = -width / 2;
  detailPanels.forEach(panel => {
    panel.target.copy(anchor).addScaledVector(right, cursor + panel.width / 2); cursor += panel.width + .33;
    panel.root.position.lerpVectors(panel.start, panel.target, ease);
    panel.root.quaternion.slerpQuaternions(panel.startQuaternion, camera.quaternion, ease);
    panel.root.scale.setScalar(THREE.MathUtils.lerp(panel.startScale, 1, ease)); panel.root.position.addScaledVector(up, Math.sin(t * Math.PI) * .4);
    const phase = (now / 1000) % 4.8;
    const pulse = phase < 2.35 ? Math.pow(Math.sin(phase / 2.35 * Math.PI), 2) * .88 : 0;
    if (panel.mask) panel.mask.material.opacity = state.pulse && panel.mask.material.map ? (reducedMotion ? .6 : pulse) : 0;
    panel.glow.material.uniforms.uStrength.value = .5 + (state.pulse && !panel.sameAsOriginal ? pulse * .4 : 0);
  });
  scene.updateMatrixWorld(true);
  connections.forEach(link => {
    const start = link.source.group.getWorldPosition(new THREE.Vector3());
    const end = link.panel.root.getWorldPosition(new THREE.Vector3()).addScaledVector(up, -link.panel.height * .48);
    link.curve.v0.copy(start); link.curve.v1.copy(start).addScaledVector(toward, 1.3).addScaledVector(up, 1.2);
    link.curve.v2.copy(end).addScaledVector(up, -1.1).addScaledVector(toward, .5); link.curve.v3.copy(end);
    for (let index = 0; index <= 64; index++) link.curve.getPoint(index / 64).toArray(link.positions, index * 3);
    link.line.geometry.attributes.position.needsUpdate = true; link.line.material.opacity = .55 * ease;
    link.dots.forEach((dot, index) => { const progress = reducedMotion ? (index + 1) / 4 : (now / 4200 + index / 3 + link.offset) % 1; dot.position.copy(link.curve.getPoint(progress)); dot.material.opacity = ease * .92; });
  });
}
function updateSourceTiles(now) {
  for (const tile of sourceTiles) {
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
  controls.update(); updateSourceTiles(now); updateDetails(now); renderer.render(scene, camera);
  if (now - lastUi > 500) { updateDiagnosticUi(); lastUi = now; }
}

function clampView() {
  const sheet = selectedSheet();
  state.view.width = state.widthChoice || Math.max(1, sheet?.width || 1800);
  state.view.height = state.view.width * AREA_H / AREA_W;
  state.view.x = Math.max(0, Math.min(state.view.x, Math.max(0, (sheet?.width || 0) - state.view.width)));
  state.view.y = Math.max(0, Math.min(state.view.y, Math.max(0, (sheet?.height || 0) - state.view.height)));
}
function bringIntoView(tile, center = false) {
  state.sheetId = tile.sheetId; clampView();
  if (center || tile.x < state.view.x || tile.x >= state.view.x + state.view.width) state.view.x = Math.max(0, tile.x - (state.view.width - Math.min(tile.width, state.view.width)) / 2);
  if (center || tile.y < state.view.y || tile.y + Math.min(tile.height, state.view.height) > state.view.y + state.view.height) state.view.y = Math.max(0, tile.y - (state.view.height - Math.min(tile.height, state.view.height)) / 2);
  clampView(); updateSheets();
}
function updatePanControls() {
  const sheet = selectedSheet();
  for (const [id, axis, size] of [['panX', 'x', 'width'], ['panY', 'y', 'height']]) {
    $(id).max = String(Math.max(0, Math.ceil((sheet?.[size] || 0) - state.view[size]))); $(id).value = String(Math.round(state.view[axis])); $(id).disabled = Number($(id).max) === 0;
  }
  $('viewRange').textContent = sheet ? `${sheet.name} · x ${Math.round(state.view.x).toLocaleString('de-DE')}–${Math.round(state.view.x + state.view.width).toLocaleString('de-DE')} · y ${Math.round(state.view.y).toLocaleString('de-DE')}–${Math.round(state.view.y + state.view.height).toLocaleString('de-DE')} px` : 'Warte auf Capture-Geometrie';
  const shown = visibleTiles(sheet, state.view);
  $('sheetSummary').textContent = sheet ? `${shown.length} sichtbar / ${sheet.tiles.length} Capture-Kacheln` : 'Noch keine Kacheln';
}
function updateSheets() {
  const signature = JSON.stringify(data?.sheets.map(sheet => [sheet.id, sheet.name]));
  if (signature !== sheetSignature) {
    sheetSignature = signature; $('sheetSelect').replaceChildren(...(data?.sheets || []).map(sheet => { const option = document.createElement('option'); option.value = sheet.id; option.textContent = sheet.name; return option; }));
  }
  $('sheetSelect').value = state.sheetId || '';
}
function normalizeStatus(value) {
  return ({ preparing: 'Vorbereitung', requested: 'Öffnen angefragt', waiting_open: 'Wartet auf Öffnen', opening: 'Excel öffnet', planning: 'Capture-Bereiche werden geplant', ready: 'Add-in bereit', capturing: 'Captures laufen', comparing: 'Pixelvergleich', complete: 'Abgeschlossen', completed: 'Abgeschlossen', finished: 'Abgeschlossen', unchanged: 'Identisch', differences: 'Unterschiede gefunden', skipped: 'Übersprungen', interrupted: 'Unterbrochen', waiting_cleanup: 'Schließt Excel', failed: 'Fehler', paused: 'Pausiert', closed: 'Geschlossen', queued: 'Eingeplant', running: 'Aktiv', active: 'Aktiv', retrying_classic: 'Klassischer Einzelvergleich läuft' })[value] || value || 'Warte auf Status';
}
function groupName(group) { return String(group.workbook || group.name || group.source || group.id || 'Arbeitsmappe').split(/[\\/]/).pop(); }
function renderGroups() {
  const groups = state.queue?.groups || [];
  const query = $('groupSearch').value.trim().toLocaleLowerCase();
  const matching = query ? groups.filter(group => `${groupName(group)} ${group.corpus || ''}`.toLocaleLowerCase().includes(query)) : groups;
  const shown = matching.slice(0, groupLimit);
  // Keep the selected active workbook reachable even when thousands precede it.
  const selected = groups.find(group => String(group.id) === state.groupId);
  if (selected && !shown.includes(selected)) shown.unshift(selected);
  const signature = JSON.stringify([state.groupId, query, shown]); if (signature === groupSignature) return; groupSignature = signature;
  $('groupCount').textContent = groups.length.toLocaleString('de-DE');
  $('moreGroups').hidden = matching.length <= groupLimit; $('moreGroups').textContent = `Weitere ${Math.min(100, matching.length - groupLimit)} anzeigen`;
  $('groups').replaceChildren(...shown.map(group => {
    const button = document.createElement('button'); button.className = 'group'; button.classList.toggle('selected', String(group.id) === state.groupId); button.setAttribute('aria-pressed', String(String(group.id) === state.groupId));
    const head = document.createElement('span'); head.className = 'group-head';
    const number = document.createElement('span'); number.textContent = `GRUPPE ${String(groups.indexOf(group) + 1).padStart(2, '0')}`;
    const status = document.createElement('span'); status.textContent = normalizeStatus(group.status); head.append(number, status);
    const name = document.createElement('span'); name.className = 'group-name'; name.textContent = groupName(group); name.title = String(group.workbook || group.source || name.textContent);
    const description = document.createElement('span'); description.className = 'group-status'; description.textContent = group.stage || group.message || group.corpus || '';
    const progress = document.createElement('span'); progress.className = 'progress'; const fill = document.createElement('i');
    const done = Number(group.completed_tiles ?? group.tiles_compared ?? 0), total = Number(group.total_tiles ?? 0);
    fill.style.width = `${total > 0 ? Math.min(100, done / total * 100) : ['complete', 'completed', 'finished', 'unchanged', 'differences', 'skipped'].includes(group.status) ? 100 : 0}%`; progress.append(fill);
    const foot = document.createElement('span'); foot.className = 'group-foot'; foot.textContent = total ? `${done} / ${total} Kacheln` : group.corpus || 'Status vom Runner';
    button.append(head, name, description, progress, foot); button.addEventListener('click', () => selectGroup(String(group.id))); return button;
  }));
}
function updateControlInputs() {
  const values = state.controls;
  for (const [id, key] of [['targetGroups', 'target_groups'], ['openTimeout', 'open_timeout_seconds'], ['jobTimeout', 'job_timeout_seconds'], ['equalPreview', 'equal_preview_width']]) {
    if (values[key] != null && document.activeElement !== $(id) && !dirtyControls.has(id)) $(id).value = String(values[key]);
  }
  $('groupRange').max = String(Math.max(24, Number(values.target_groups || 1), Number($('targetGroups').value || 1)));
  if (document.activeElement !== $('groupRange') && !dirtyControls.has('targetGroups')) $('groupRange').value = String(values.target_groups || 1);
  $('captureState').textContent = state.queue?.running === false ? 'Beendet' : values.paused ? 'Pausiert' : 'Laufen'; $('pauseJobs').textContent = values.paused ? 'Fortsetzen' : 'Pausieren';
  $('pauseNote').textContent = state.queue?.running === false ? 'Der Lauf ist beendet. Seine gespeicherten Bilder bleiben verfügbar.' : values.paused ? 'Bereits laufende Aufträge werden fertig. Neue Captures warten.' : 'Pause greift vor dem nächsten Auftrag. Excel darf weiter öffnen.';
  const active = activeGroupCount(state.queue);
  $('activeGroups').textContent = `${active} aktiv · Ziel ${values.target_groups ?? '—'}`;
}
async function sendControl(change) {
  if (controlBusy) return; controlBusy = true;
  for (const id of ['applyTarget', 'pauseJobs', 'applyTimeouts', 'equalPreview']) $(id).disabled = true;
  try {
    const response = await fetch('/api/control', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(change) });
    const result = await response.json(); if (!response.ok || result.ok === false) throw new Error(result.error || `Steuerung abgelehnt (${response.status}).`);
    state.controls = result.controls;
    for (const [id, key] of [['targetGroups', 'target_groups'], ['openTimeout', 'open_timeout_seconds'], ['jobTimeout', 'job_timeout_seconds'], ['equalPreview', 'equal_preview_width']]) if (change[key] != null && Number($(id).value) === change[key]) dirtyControls.delete(id);
    updateControlInputs();
    const text = change.target_groups ? `Gruppenziel ${state.controls.target_groups} bestätigt.` : 'Steuerung vom Host bestätigt.';
    pushEvent(text); setNotice(text);
  } catch (error) { setNotice(error.message, true); updateControlInputs(); }
  finally { controlBusy = false; for (const id of ['applyTarget', 'pauseJobs', 'applyTimeouts', 'equalPreview']) $(id).disabled = false; }
}
function selectGroup(id) {
  if (id === state.groupId) return;
  state.groupId = id; state.sheetId = null; state.view.x = 0; state.view.y = 0; latestCapture = null; previousImages.clear(); flashes.clear();
  data = null; entries = []; lastSignature = ''; sheetSignature = ''; clearDetails(); ++sceneGeneration;
  boards.splice(0).forEach(disposeObject); sourceTiles.length = 0; hitTargets.length = 0;
  $('emptyScene').hidden = false; $('tileButtons').replaceChildren(); renderGroups();
  ++pollGeneration; clearTimeout(pollTimer); poll();
}
function acceptGroup(raw) {
  data = normalizeGroup(raw); entries = data.sheets.flatMap(sheet => sheet.tiles);
  $('workbook').textContent = groupName(data); $('workbook').title = String(data.workbook || '');
  $('groupEyebrow').textContent = [data.corpus, data.capture_mode === 'classic' || data.status === 'retrying_classic' ? 'LETZTE RESIDENT-CAPTURES · KLASSISCHE GEGENPRÜFUNG' : 'LIVE ATELIER'].filter(Boolean).join(' · ');
  if (!data.sheets.some(sheet => sheet.id === state.sheetId)) state.sheetId = data.sheets[0]?.id ?? null;
  let latest = null;
  for (const tile of entries) for (const [profile, image] of Object.entries(tile.images)) {
    const key = `${tile.id}:${profile}`, signature = JSON.stringify([image.url, image.pixelClass, image.maskUrl]);
    if (image.url && previousImages.get(key) !== signature) {
      if (previousImages.has(key)) pushEvent(`${tile.sheetName} · ${tile.address} · ${profileLabel(profile)}: ${image.pixelClass == null ? 'Bild empfangen' : imageStatus(tile, profile) === 'different' ? 'Pixelabweichung bestätigt' : 'Pixelvergleich fertig'}`);
      flashes.set(key, performance.now()); previousImages.set(key, signature); latest = tile;
    }
  }
  const active = entries.find(tile => tile.id === String(data.active_tile) && (!data.active_sheet || tile.sheetId === String(data.active_sheet)));
  latest = active || latest;
  if (latest && latest.id !== latestCapture) {
    latestCapture = latest.id; if (state.follow && !state.pinned && !state.selected) bringIntoView(latest);
  }
  clampView(); updateSheets(); renderScene(); updateDiagnosticUi();
}
async function poll() {
  const generation = pollGeneration;
  try {
    const response = await fetch('/api/state', { cache: 'no-store' }); if (!response.ok) throw new Error(`Status nicht verfügbar (${response.status}).`);
    const queue = await response.json(); if (generation !== pollGeneration) return;
    const recovered = Boolean(state.error);
    state.queue = queue; state.controls = queue.controls || {}; state.connected = true; state.error = null;
    $('connectionLight').className = 'connected';
    $('connectionText').textContent = queue.verification_fixture ? 'Gespeicherter Fall · UI-Prüfung'
      : queue.phase === 'owner_unverified' ? 'Verbunden · Runnerstatus unbestätigt'
      : queue.phase === 'finished' ? 'Lauf abgeschlossen · Ergebnisse'
      : queue.runner_alive === false || queue.phase === 'interrupted' ? 'Runner beendet · letzter Stand'
      : queue.running === false ? 'Lauf beendet · Ergebnisse'
      : 'Live · lokaler Runner';
    if (recovered) setNotice('Verbindung zum lokalen Host wiederhergestellt.');
    const memory = queue.memory;
    $('memory').textContent = memory?.available_bytes != null ? `${(memory.available_bytes / 1073741824).toFixed(1)} GiB frei / ${(memory.total_bytes / 1073741824).toFixed(0)} GiB` : 'RAM nicht verfügbar';
    updateControlInputs();
    if (!state.groupId && queue.groups?.length) state.groupId = String((queue.groups.find(group => ['capturing', 'comparing', 'running', 'opening', 'active'].includes(group.status)) || queue.groups[0]).id);
    renderGroups();
    if (state.groupId) {
      const detail = await fetch(`/api/groups/${encodeURIComponent(state.groupId)}`, { cache: 'no-store' });
      if (!detail.ok) throw new Error(`Gruppendetails nicht verfügbar (${detail.status}).`);
      const raw = await detail.json(); if (generation !== pollGeneration) return; acceptGroup(raw);
    }
    if (queue.diagnostics?.length) setNotice(queue.diagnostics.map(item => typeof item === 'string' ? item : item.message || JSON.stringify(item)).join(' · '), true);
    $('lastUpdate').textContent = `Aktualisiert ${new Date().toLocaleTimeString('de-DE')}`;
  } catch (error) {
    if (generation !== pollGeneration) return;
    state.connected = false; state.error = error.message; $('connectionLight').className = 'failed'; $('connectionText').textContent = 'Verbindung unterbrochen'; setNotice(`${error.message} Letzten bekannten Stand anzeigen; erneuter Verbindungsversuch folgt.`, true);
  } finally { if (generation === pollGeneration) pollTimer = setTimeout(poll, document.hidden ? 2000 : 500); }
}
function elapsedText(iso) {
  const milliseconds = Date.now() - Date.parse(iso); if (!Number.isFinite(milliseconds)) return '';
  const seconds = Math.max(0, Math.floor(milliseconds / 1000)); return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
function updateDiagnosticUi() {
  if (data) {
    const clients = Object.entries(data.launcher?.clients || {});
    const counts = clients.reduce((result, [, client]) => { result[client.status] = (result[client.status] || 0) + 1; return result; }, {});
    const opening = clients.filter(([, client]) => ['requested', 'waiting_open', 'opening'].includes(client.status));
    const terminal = ['unchanged', 'differences', 'skipped', 'failed', 'interrupted', 'complete', 'completed', 'finished'].includes(data.status);
    $('groupStatus').textContent = data.status === 'retrying_classic'
      ? 'Der Runner wiederholt diese Datei im klassischen Einzelvergleich. Die Kacheln zeigen den letzten Resident-Stand.'
      : data.capture_mode === 'classic'
        ? `${normalizeStatus(data.status)} · Klassischer Vergleich. Die Kacheln zeigen den vorherigen Resident-Stand; das endgültige Ergebnis steht im Bericht.`
        : [normalizeStatus(data.status), clients.length ? terminal ? `${counts.closed || 0} / ${clients.length} Excel-Instanzen geschlossen` : `${counts.ready || 0} / ${clients.length} Add-ins bereit` : '', ...opening.map(([id, client]) => `${profileLabel(id)}: ${normalizeStatus(client.status)} ${elapsedText(client.waiting_since || client.requested_at)}`)].filter(Boolean).join(' · ');
  }
  const events = [...(data?.events || []).map(event => typeof event === 'string' ? { message: event } : event), ...localEvents].sort((a, b) => String(b.at || b.timestamp || '').localeCompare(String(a.at || a.timestamp || ''))).slice(0, 8);
  const fragment = document.createDocumentFragment();
  for (const event of events) {
    const row = document.createElement('div'); row.className = 'event'; const time = document.createElement('time');
    const iso = event.at || event.timestamp; time.textContent = iso && Number.isFinite(Date.parse(iso)) ? new Date(iso).toLocaleTimeString('de-DE') : '—';
    const text = document.createElement('span'); text.textContent = event.message || event.text || event.kind || ''; row.append(time, text); fragment.append(row);
  }
  if (!events.length) { const row = document.createElement('div'); row.className = 'event'; row.textContent = 'Neue Capture-Ereignisse erscheinen hier.'; fragment.append(row); }
  $('events').replaceChildren(fragment);
  for (const button of $('tileButtons').children) { const active = button.dataset.tile === state.selected; button.classList.toggle('active', active); button.setAttribute('aria-pressed', String(active)); }
  if ($('apiState').closest('details').open) $('apiState').textContent = JSON.stringify(snapshot(), null, 2);
}
function nextDifference() {
  const differences = entries.filter(entry => Object.keys(entry.images).some(profile => imageStatus(entry, profile) === 'different'));
  const current = differences.findIndex(entry => entry.id === state.selected);
  const tile = differences[(current + 1) % differences.length];
  if (!tile) { setNotice('Für diese Gruppe liegt bisher keine bestätigte Pixelabweichung vor.'); return; }
  state.follow = false; $('followCapture').checked = false; bringIntoView(tile, true); renderScene(); showDetails(tile.id); setPinned(true); fitCamera(true);
}
function wireControls() {
  $('groupSearch').addEventListener('input', () => { groupLimit = 100; renderGroups(); });
  $('moreGroups').addEventListener('click', () => { groupLimit += 100; renderGroups(); });
  function adjustTarget(delta) { const value = Math.max(1, Number($('targetGroups').value || 1) + delta); dirtyControls.add('targetGroups'); $('targetGroups').value = String(value); $('groupRange').max = String(Math.max(24, value)); $('groupRange').value = String(value); }
  $('minus').addEventListener('click', () => adjustTarget(-1)); $('plus').addEventListener('click', () => adjustTarget(1));
  for (const id of ['targetGroups', 'openTimeout', 'jobTimeout', 'equalPreview']) $(id).addEventListener('input', () => dirtyControls.add(id));
  $('targetGroups').addEventListener('input', () => { const value = Number($('targetGroups').value); if (Number.isSafeInteger(value) && value > 0) { $('groupRange').max = String(Math.max(24, value)); $('groupRange').value = String(value); } });
  $('groupRange').addEventListener('input', event => { dirtyControls.add('targetGroups'); $('targetGroups').value = event.target.value; });
  $('groupRange').addEventListener('change', event => sendControl({ target_groups: Number(event.target.value) }));
  $('applyTarget').addEventListener('click', () => { const value = Number($('targetGroups').value); if (!Number.isSafeInteger(value) || value < 1) return setNotice('Bitte eine positive ganze Gruppenzahl eingeben.', true); sendControl({ target_groups: value }); });
  $('targetGroups').addEventListener('keydown', event => { if (event.key === 'Enter') $('applyTarget').click(); });
  $('pauseJobs').addEventListener('click', () => sendControl({ paused: !state.controls.paused }));
  $('applyTimeouts').addEventListener('click', () => {
    const open = Number($('openTimeout').value), job = Number($('jobTimeout').value);
    if (![open, job].every(value => Number.isSafeInteger(value) && value > 0)) return setNotice('Zeitlimits müssen positive ganze Sekunden sein.', true);
    sendControl({ open_timeout_seconds: open, job_timeout_seconds: job });
  });
  $('equalPreview').addEventListener('change', event => sendControl({ equal_preview_width: Number(event.target.value) }));
  $('followCapture').addEventListener('change', event => { state.follow = event.target.checked; if (state.follow) { clearDetails(); const tile = tileAt(latestCapture); if (tile) { bringIntoView(tile); renderScene(); } } });
  $('sheetSelect').addEventListener('change', event => { state.sheetId = event.target.value; state.follow = false; $('followCapture').checked = false; state.view.x = state.view.y = 0; clampView(); clearDetails(); renderScene(); });
  for (const [id, axis] of [['panX', 'x'], ['panY', 'y']]) $(id).addEventListener('input', event => { state.follow = false; $('followCapture').checked = false; state.view[axis] = Number(event.target.value); clampView(); clearDetails(); renderScene(); });
  $('viewScale').addEventListener('change', event => { state.widthChoice = Number(event.target.value); clampView(); renderScene(); });
  $('pulseDiff').checked = state.pulse; $('pulseDiff').addEventListener('change', event => { state.pulse = event.target.checked; });
  $('nextDifference').addEventListener('click', nextDifference); $('resetCamera').addEventListener('click', () => { clearDetails(); fitCamera(true); });
  $('stageMode').addEventListener('click', () => { document.body.classList.toggle('presenting'); $('stageMode').textContent = document.body.classList.contains('presenting') ? '⛶ Zurück' : '⛶ Bühne'; });
  $('pinSelection').addEventListener('click', () => setPinned(!state.pinned)); $('closeSelection').addEventListener('click', clearDetails);
  for (const id of ['selectionBar', 'evidenceActions', 'tileButtons']) { $(id).addEventListener('pointerenter', () => clearTimeout(leaveTimer)); $(id).addEventListener('pointerleave', scheduleClear); }
  $('closeImage').addEventListener('click', () => $('imageDialog').close());
  $('toggleImageSize').addEventListener('click', () => { const native = $('imageScroll').classList.toggle('native'); $('toggleImageSize').textContent = native ? 'An Fenster anpassen' : '1:1 anzeigen'; });
  document.addEventListener('keydown', event => { if (event.key === 'Escape' && !$('imageDialog').open) clearDetails(); });
}
function snapshot() {
  return { ready: state.ready, live: true, connected: state.connected, error: state.error, controls: state.controls,
    running: state.queue?.running, runnerAlive: state.queue?.runner_alive, phase: state.queue?.phase,
    groupId: state.groupId, sheetId: state.sheetId, selected: state.selected, pinned: state.pinned, follow: state.follow,
    view: { ...state.view }, profiles: data?.profiles, status: data?.status, launcher: data?.launcher,
    groupCount: state.queue?.groups?.length || 0, totalTiles: entries.length, visibleTiles: [...new Set(sourceTiles.map(tile => tile.entry.id))],
    tiles: sourceTiles.map(tile => ({ id: tile.entry.id, address: tile.entry.address, profile: tile.profile.id, status: imageStatus(tile.entry, tile.profile.id), loaded: tile.loaded, rectangle: tile.rectangle, clip: tile.clip, offscreen: tile.offscreen })),
    details: detailPanels.map(panel => ({ profiles: panel.group.ids, pixelClass: panel.group.pixelClass, sameAsOriginal: panel.sameAsOriginal, fullReady: panel.fullReady, maskPresent: Boolean(panel.mask), maskOpacity: panel.mask?.material.opacity || 0, clip: panel.clip, aspect: panel.imageW / panel.imageH })),
    connections: connections.length, textureCount: renderer?.info.memory.textures || 0, textureCache: textureCache.size,
    viewport: { width: renderer?.domElement.clientWidth, height: renderer?.domElement.clientHeight } };
}
function geometrySnapshot() {
  scene.updateMatrixWorld(true); const rect = renderer.domElement.getBoundingClientRect();
  const project = value => { const ndc = value.clone().project(camera); return { world: value.toArray(), screen: [rect.left + (ndc.x + 1) * rect.width / 2, rect.top + (1 - ndc.y) * rect.height / 2] }; };
  const up = new THREE.Vector3(0, 1, 0).applyQuaternion(camera.quaternion);
  return { sources: sourceTiles.map(tile => ({ id: tile.entry.id, profile: tile.profile.id, center: project(tile.group.getWorldPosition(new THREE.Vector3())) })),
    boards: boards.map((board, index) => ({ profile: data.profiles[index].id, position: board.position.toArray(), rotationY: board.rotation.y })),
    details: detailPanels.map(panel => ({ profiles: panel.group.ids, center: project(panel.root.getWorldPosition(new THREE.Vector3())) })),
    connections: connections.map(link => ({ sourceProfile: link.source.profile.id, profiles: link.panel.group.ids,
      startError: link.curve.v0.distanceTo(link.source.group.getWorldPosition(new THREE.Vector3())),
      endError: link.curve.v3.distanceTo(link.panel.root.getWorldPosition(new THREE.Vector3()).addScaledVector(up, -link.panel.height * .48)) })) };
}

window.__observatory = { snapshot, geometry: geometrySnapshot, selectGroup, nextDifference,
  hoverTile(id) { return showDetails(typeof id === 'number' ? entries[id]?.id : id); }, clearHover: clearDetails, pin: setPinned,
  setView(x, y, width = state.widthChoice) { state.follow = false; $('followCapture').checked = false; state.view.x = x; state.view.y = y; state.widthChoice = width; clampView(); renderScene(); },
  openEvidence(index = 0) { if (detailPanels[index]) openEvidence(detailPanels[index]); } };
try { initScene(); wireControls(); poll(); }
catch (error) { state.error = error.message; $('error').textContent = error.message; $('error').hidden = false; console.error(error); }
