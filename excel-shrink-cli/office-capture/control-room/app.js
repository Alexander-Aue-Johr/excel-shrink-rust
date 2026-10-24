import * as THREE from 'three';
import { OrbitControls } from './vendor/OrbitControls.js';
import { normalizeGroup, imageStatus, groupedPanels, visibleTiles, clippedTile, profileRectangle, activeGroupCount, sheetLayout, fitSheetView, inspectionRegions, inspectionEntry, differenceTargets, heartbeatAge } from './model.js';

const $ = id => document.getElementById(id);
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
const C = { waiting: 0x3c566b, received: 0x79dce8, equal: 0x75edc0, different: 0xff7a91 };
const CARD_W = 16, CARD_H = 9, BOARD_W = 3.67, BOARD_H = 7.75, BOARD_STEP = 3.87;
const AREA_W = 3.43, AREA_H = 7.08, AREA_Y = -.16;
let RADIUS = 24, layout = sheetLayout([]);
const state = { ready: false, connected: false, groupId: null, sheetId: null, selected: null,
  pinned: false, pulse: !reducedMotion, follow: true, error: null, controls: {}, queue: null,
  view: { x: 0, y: 0, width: 1800, height: 1800 * AREA_H / AREA_W }, widthChoice: 1800,
  focus: 'overview', regionId: null, pendingDifference: false };
const boards = [], sourceTiles = [], hitTargets = [], detailPanels = [], connections = [];
const sheetCards = new Map(), processCards = new Map();
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
let miniSignature = '', lastFocusKey = '', navigationBusy = false;

function announce(text) { $('announcement').textContent = text; }
function profileLabel(id) { return data?.profiles.find(profile => profile.id === id)?.label || id; }
function tileAt(id) { return entries.find(tile => tile.id === id); }
function selectedSheet() { return data?.sheets.find(sheet => sheet.id === state.sheetId); }
function sheetView(sheet) { if (!sheet) return state.view; return state.focus === 'tile' && sheet.id === state.sheetId ? state.view : fitSheetView(sheet, AREA_W / AREA_H); }
function stopFollowing() { state.follow = false; $('followCapture').checked = false; }
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
function clippedPlane(width, height, clip) {
  const geometry = new THREE.PlaneGeometry(width, height), uv = geometry.attributes.uv;
  for (let index = 0; index < uv.count; index++) uv.setXY(index,
    THREE.MathUtils.lerp(clip.u0, clip.u1, uv.getX(index)), THREE.MathUtils.lerp(clip.v0, clip.v1, uv.getY(index)));
  return geometry;
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
        const response = await fetch(safeImageUrl(url)); if (!response.ok) throw new Error(`Bild konnte nicht geladen werden (${response.status}).`);
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
  controls.minDistance = 2; controls.maxDistance = 800; controls.maxPolarAngle = Math.PI * .85; controls.rotateSpeed = .5; controls.panSpeed = .7;
  controls.addEventListener('start', () => { userMovedCamera = true; cameraTween = null; cancelHover(); stopFollowing(); });
  detailRoot = new THREE.Group(); scene.add(detailRoot);
  new ResizeObserver(resize).observe($('viewport')); resize();
  renderer.domElement.addEventListener('pointermove', handlePointerMove);
  renderer.domElement.addEventListener('pointerleave', scheduleClear);
  renderer.domElement.addEventListener('pointerdown', event => { pointerDown = { x: event.clientX, y: event.clientY }; });
  renderer.domElement.addEventListener('pointerup', event => {
    if (!pointerDown || Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) > 7) return;
    const hit = hitAt(event);
    if (hit?.userData.sourceTile) { const tile = hit.userData.sourceTile; focusTile(tile.entry, hit.userData.region, true); }
    else if (hit?.userData.detailPanel) setPinned(!state.pinned);
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
  else { camera.position.copy(position); controls.target.copy(target); controls.update(); }
  userMovedCamera = false;
}
function focusCardCamera(sheetId, animate = true) {
  const card = sheetCards.get(sheetId); if (!card) return;
  scene.updateMatrixWorld(true);
  const target = card.getWorldPosition(new THREE.Vector3());
  const outward = new THREE.Vector3(0, 0, 1).applyQuaternion(card.getWorldQuaternion(new THREE.Quaternion()));
  const tangent = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
  const distance = Math.max(CARD_W / 2 / (tangent * camera.aspect * .82), CARD_H / 2 / (tangent * .77));
  moveCamera(target.clone().addScaledVector(outward, distance).add(new THREE.Vector3(0, .2, 0)), target, animate);
}
function focusSheet(id, manual = false, preview = false) {
  if (!data?.sheets.some(sheet => sheet.id === id)) return;
  if (manual) stopFollowing();
  clearDetails(); state.sheetId = id; state.focus = 'sheet'; state.view.x = state.view.y = 0;
  updateSheets(); renderScene(); focusCardCamera(id); renderMinimap();
  if (!preview) announce(`Sheet ${selectedSheet().name}`);
}
function focusTile(tile, region = null, manual = false) {
  if (!tile) return;
  if (manual) stopFollowing();
  clearDetails(); state.sheetId = tile.sheetId; state.focus = 'tile';
  const inspect = region || inspectionRegions(tile, !tile.equalAcrossAll)[0];
  const area = inspect || { x: 0, y: 0, width: tile.width, height: tile.height };
  state.widthChoice = Math.max(64, Math.min(1800, Math.max(area.width, area.height * AREA_W / AREA_H)) * 1.12);
  state.view.x = Math.max(0, tile.x + area.x - (state.widthChoice - area.width) / 2);
  state.view.y = Math.max(0, tile.y + area.y - (state.widthChoice * AREA_H / AREA_W - area.height) / 2);
  clampView(); updateSheets(); renderScene(); focusCardCamera(tile.sheetId);
  if (manual) { showDetails(tile.id, true, inspect); setPinned(true); }
}
function resize() {
  const { width, height } = $('viewport').getBoundingClientRect(); if (!width || !height) return;
  renderer.setSize(width, height, false); camera.aspect = width / height; camera.updateProjectionMatrix();
  if (!userMovedCamera) { if (state.focus === 'overview') fitCamera(); else focusCardCamera(state.sheetId, false); }
}
function setCurvedPosition(group, x, y, z = 0) {
  const angle = x / RADIUS;
  group.position.set(RADIUS * Math.sin(angle), y, RADIUS * (1 - Math.cos(angle)) + z); group.rotation.y = -angle;
}
function makeSheetCard(sheet, placement) {
  const card = new THREE.Group(); setCurvedPosition(card, placement.x, placement.y); scene.add(card);
  card.userData.sheetId = sheet.id;
  const backing = new THREE.Mesh(curvedGeometry(CARD_W, CARD_H), new THREE.MeshBasicMaterial({ color: sheet.id === state.sheetId ? 0x132d40 : 0x102334, side: THREE.DoubleSide }));
  backing.userData.sheetId = sheet.id; card.add(backing); hitTargets.push(backing);
  const edge = frame(CARD_W, CARD_H, sheet.id === state.sheetId ? 0x76c7cf : 0x34566c, true); edge.position.z = .012; card.add(edge);
  const differences = sheet.tiles.filter(tile => tile.comparisonComplete && !tile.equalAcrossAll).length;
  const arrived = sheet.tiles.filter(tile => Object.keys(tile.images).length).length;
  const title = makeTextPlane([{ text: sheet.name, y: 68, size: 55 },
    { text: `${arrived} / ${sheet.tiles.length} Capture-Aufträge · ${differences} mit Abweichung`, y: 130, size: 29, color: differences ? '#ffabbc' : '#8eadc2' }], CARD_W - .55, .68);
  title.geometry.dispose(); title.geometry = curvedGeometry(CARD_W - .55, .68);
  title.position.set(0, CARD_H / 2 - .43, .045); card.add(title);
  sheetCards.set(sheet.id, card); boards.push(card);
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
    const color = new THREE.Color(C[imageStatus(entry, profile.id)]);
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
  const signature = JSON.stringify([state.groupId, state.sheetId, state.focus, state.view, data.profiles, data.sheets]);
  if (!force && signature === lastSignature) return;
  lastSignature = signature; const generation = ++sceneGeneration;
  const selection = state.selected, regionId = state.regionId, pinned = state.pinned;
  clearDetails(); boards.splice(0).forEach(disposeObject); sourceTiles.length = 0; hitTargets.length = 0; sheetCards.clear();
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
  if (selectedEntry && sourceTiles.some(tile => tile.entry.id === selection)) { showDetails(selection, false, inspectionRegions(selectedEntry).find(region => region.id === regionId)); setPinned(pinned); }
  updatePanControls(); renderMinimap();
  if (!lastFocusKey) { fitCamera(); lastFocusKey = state.groupId; }
}

function makeDetailPanel(group, entry, index, count) {
  const source = sourceTiles.find(tile => tile.entry.id === entry.id && tile.profile.id === group.ids[0]);
  if (!source) return;
  const representative = entry.images[group.ids[0]], sameAsOriginal = group.pixelClass != null && group.pixelClass === entry.images.original?.pixelClass;
  const width = count >= 4 ? 3.65 : count === 3 ? 4.7 : 6.2;
  // A huge job tile may extend beyond the shared sheet window. Its detail clone
  // uses the same tile-local pixel window for every peer. Different row heights
  // must neither stretch an image nor make peers show different local regions.
  const focus = sourceTiles.find(tile => tile.entry.id === entry.id && !tile.offscreen && tile.profile.id === 'original')
    || sourceTiles.find(tile => tile.entry.id === entry.id && !tile.offscreen) || source;
  const textureRegion = entry.inspection || { x: Math.max(0, focus.clip.x - focus.rectangle.x), y: Math.max(0, focus.clip.y - focus.rectangle.y), width: focus.clip.width, height: focus.clip.height };
  const clip = { ...textureRegion, u0: textureRegion.x / source.rectangle.width, u1: (textureRegion.x + textureRegion.width) / source.rectangle.width,
    v0: 1 - (textureRegion.y + textureRegion.height) / source.rectangle.height, v1: 1 - textureRegion.y / source.rectangle.height };
  const [imageW, imageH] = fitSize(clip.width / clip.height, width - .2, 3.1), height = imageH + .94;
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
  if ((entry.comparisonComplete || entry.inspection) && representative.maskUrl && (group.original || !sameAsOriginal)) {
    mask = new THREE.Mesh(clippedPlane(imageW, imageH, clip), new THREE.MeshBasicMaterial({ color: 0xff315c, transparent: true, opacity: 0, depthWrite: false, side: THREE.DoubleSide, toneMapped: false, fog: false }));
    mask.position.copy(image.position); mask.position.z += .012; root.add(mask);
  }
  const start = source.group.getWorldPosition(new THREE.Vector3()); root.position.copy(start); root.quaternion.copy(source.group.getWorldQuaternion(new THREE.Quaternion())); root.scale.setScalar(source.width / width);
  const panel = { root, image, mask, rim, glow, group, entry, representative, source, clip, textureRegion, width, height, imageW, imageH, index, count, start, startQuaternion: root.quaternion.clone(), startScale: root.scale.x, target: new THREE.Vector3(), sameAsOriginal, fullReady: false };
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
async function showDetails(id, animate = true, region = null) {
  cancelHover(); clearTimeout(leaveTimer);
  const originalEntry = tileAt(id); if (!originalEntry || !state.ready || !sourceTiles.some(tile => tile.entry.id === id)) return;
  const entry = inspectionEntry(originalEntry, region, data.profiles);
  const signature = JSON.stringify([id, region?.id, entry.comparisonComplete, entry.images]); if (state.selected === id && signature === detailSignature) return;
  clearDetails(); detailSignature = signature;
  const generation = ++selectedGeneration; state.selected = id; state.regionId = region?.id ?? null; activeDetailSince = animate ? performance.now() : performance.now() - 1000;
  const groups = groupedPanels(entry, data.profiles); groups.forEach((group, index) => makeDetailPanel(group, entry, index, groups.length));
  $('selectionBar').hidden = false; $('selectionText').textContent = `${entry.sheetName} · ${entry.address}`;
  $('selectionSummary').textContent = `${groups.length} Bildgruppen · ${entry.comparisonComplete ? entry.equalAcrossAll ? 'identisch' : 'Abweichung' : region ? 'Ausschnitt ohne eigene Klassifizierung' : 'Vergleich offen'} · ${region ? `${Math.round(region.width)} × ${Math.round(region.height)} px` : 'Capture'}`;
  renderEvidenceActions(); announce($('selectionText').textContent + ': ' + $('selectionSummary').textContent);
  await Promise.all(detailPanels.map(async panel => {
    try {
      const image = panel.representative;
      const rect = panel.source.rectangle;
      // Preview files have fewer pixels; crop coordinates follow the retained
      // file dimensions rather than pretending that preview pixels are 1:1.
      const textureRegion = { ...panel.textureRegion, sourceWidth: rect.width, sourceHeight: rect.height };
      const [texture, mask] = await Promise.all([loadTexture(image.fullUrl || image.url, 2048, textureRegion), panel.mask ? loadTexture(image.maskUrl, 2048, panel.textureRegion) : null]);
      if (generation !== selectedGeneration) return;
      panel.image.geometry.dispose(); panel.image.geometry = new THREE.PlaneGeometry(panel.imageW, panel.imageH);
      panel.image.material.map = texture; panel.image.material.needsUpdate = true; panel.fullReady = true;
      if (panel.mask && mask) { panel.mask.geometry.dispose(); panel.mask.geometry = new THREE.PlaneGeometry(panel.imageW, panel.imageH); panel.mask.material.map = mask; panel.mask.material.needsUpdate = true; }
    } catch (error) { if (generation === selectedGeneration && error.name !== 'AbortError') setNotice(error.message, true); }
  }));
  sweepTextures();
}
function clearDetails() {
  ++selectedGeneration; clearTimeout(leaveTimer); cancelHover(); state.selected = null; state.regionId = null; state.pinned = false; detailSignature = '';
  for (let index = hitTargets.length - 1; index >= 0; index--) if (hitTargets[index].userData.detailPanel) hitTargets.splice(index, 1);
  detailPanels.splice(0).forEach(panel => disposeObject(panel.root));
  connections.splice(0).forEach(link => { disposeObject(link.line); link.dots.forEach(disposeObject); });
  $('selectionBar').hidden = true; $('evidenceActions').hidden = true; $('pinSelection').setAttribute('aria-pressed', 'false'); $('pinSelection').textContent = 'Festhalten';
}
function setPinned(value) { state.pinned = Boolean(value && state.selected); clearTimeout(leaveTimer); $('pinSelection').setAttribute('aria-pressed', String(state.pinned)); $('pinSelection').textContent = state.pinned ? 'Festgehalten' : 'Festhalten'; }
function cancelHover() { clearTimeout(hoverTimer); pendingHover = null; }
function scheduleClear() { cancelHover(); if (state.pinned) return; clearTimeout(leaveTimer); leaveTimer = setTimeout(() => { if (!state.pinned) clearDetails(); }, 480); }
function hitAt(event) {
  const rect = renderer.domElement.getBoundingClientRect(); pointer.set((event.clientX - rect.left) / rect.width * 2 - 1, -((event.clientY - rect.top) / rect.height) * 2 + 1); raycaster.setFromCamera(pointer, camera);
  const hit = raycaster.intersectObjects(hitTargets, false)[0]; if (!hit) return;
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
  if (hit?.userData.detailPanel) { clearTimeout(leaveTimer); cancelHover(); return; }
  const tile = hit?.userData.sourceTile; if (!tile) { scheduleClear(); return; } clearTimeout(leaveTimer);
  const region = hit.userData.region, key = `${tile.entry.id}:${region?.id || ''}`;
  if (state.pinned || (state.selected === tile.entry.id && state.regionId === (region?.id || null)) || pendingHover === key) return;
  cancelHover(); pendingHover = key; hoverTimer = setTimeout(() => showDetails(tile.entry.id, true, region), 125);
}
function renderTileButtons(shown) {
  $('tileButtons').replaceChildren(...shown.map(entry => {
    const button = document.createElement('button'); button.textContent = entry.address || entry.id; button.dataset.tile = entry.id;
    button.classList.toggle('different', Object.keys(entry.images).some(profile => imageStatus(entry, profile) === 'different'));
    button.addEventListener('focus', () => showDetails(entry.id)); button.addEventListener('click', () => focusTile(entry, null, true)); return button;
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
  const anchor = controls.target.clone().addScaledVector(up, 1.9).addScaledVector(toward, 2.7);
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
function updatePanControls() {
  const sheet = selectedSheet();
  for (const [id, axis, size] of [['panX', 'x', 'width'], ['panY', 'y', 'height']]) {
    $(id).max = String(Math.max(0, Math.ceil((sheet?.[size] || 0) - state.view[size]))); $(id).value = String(Math.round(state.view[axis])); $(id).disabled = Number($(id).max) === 0;
  }
  const current = sheetView(sheet);
  $('viewRange').textContent = sheet ? `${sheet.name} · x ${Math.round(current.x).toLocaleString('de-DE')}–${Math.round(current.x + current.width).toLocaleString('de-DE')} · y ${Math.round(current.y).toLocaleString('de-DE')}–${Math.round(current.y + current.height).toLocaleString('de-DE')} px` : 'Warte auf Capture-Geometrie';
  const shown = visibleTiles(sheet, sheetView(sheet), 100000);
  $('sheetSummary').textContent = sheet ? `${data.sheets.length} Sheets · ${shown.length} / ${sheet.tiles.length} Capture-Aufträge im Sheet-Ausschnitt` : 'Noch keine Kacheln';
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
    button.setAttribute('aria-label', `Sheet ${sheet.name} ansehen`);
    let timer;
    button.addEventListener('pointerenter', () => { $('minimapLabel').textContent = sheet.name; timer = setTimeout(() => focusSheet(sheet.id, true, true), 180); });
    button.addEventListener('pointerleave', () => clearTimeout(timer));
    button.addEventListener('focus', () => { $('minimapLabel').textContent = sheet.name; });
    button.addEventListener('click', () => { clearTimeout(timer); focusSheet(sheet.id, true); }); return button;
  }));
  $('minimapLabel').textContent = selectedSheet()?.name || `${sheets.length} Sheets`;
}
function normalizeStatus(value) {
  return ({ restart_requested: 'Neustart angefragt', restart_waiting_cleanup: 'Neustart wartet auf sicheres Schließen', preparing: 'Vorbereitung', requested: 'Öffnen angefragt', waiting_open: 'Wartet auf Öffnen', opening: 'Excel öffnet', planning: 'Capture-Bereiche werden geplant', ready: 'Add-in bereit', capturing: 'Captures laufen', comparing: 'Pixelvergleich', complete: 'Abgeschlossen', completed: 'Abgeschlossen', finished: 'Abgeschlossen', unchanged: 'Identisch', differences: 'Unterschiede gefunden', skipped: 'Übersprungen', interrupted: 'Unterbrochen', waiting_cleanup: 'Schließt Excel', failed: 'Fehler', paused: 'Pausiert', closed: 'Geschlossen', queued: 'Eingeplant', running: 'Aktiv', active: 'Aktiv', retrying_classic: 'Klassischer Einzelvergleich läuft' })[value] || value || 'Warte auf Status';
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
  for (const [id, key] of [['targetGroups', 'target_groups'], ['openTimeout', 'open_timeout_seconds'], ['jobTimeout', 'job_timeout_seconds'], ['equalPreview', 'equal_preview_width'], ['captureMegapixels', 'capture_tile_megapixels'], ['captureBatch', 'capture_batch_tiles']]) {
    if (values[key] != null && document.activeElement !== $(id) && !dirtyControls.has(id)) $(id).value = String(values[key]);
  }
  if (document.activeElement !== $('hideExcel')) $('hideExcel').checked = Boolean(values.hide_excel);
  $('groupRange').max = String(Math.max(24, Number(values.target_groups || 1), Number($('targetGroups').value || 1)));
  if (document.activeElement !== $('groupRange') && !dirtyControls.has('targetGroups')) $('groupRange').value = String(values.target_groups || 1);
  $('captureState').textContent = state.queue?.running === false ? 'Beendet' : values.paused ? 'Pausiert' : 'Laufen'; $('pauseJobs').textContent = values.paused ? 'Fortsetzen' : 'Pausieren';
  $('pauseNote').textContent = state.queue?.running === false ? 'Der Lauf ist beendet. Seine gespeicherten Bilder bleiben verfügbar.' : values.paused ? 'Bereits laufende Aufträge werden fertig. Neue Captures warten.' : 'Pause greift vor dem nächsten Auftrag. Excel darf weiter öffnen.';
  const active = activeGroupCount(state.queue);
  $('activeGroups').textContent = `${active} aktiv · Ziel ${values.target_groups ?? '—'}`;
}
async function sendControl(change) {
  if (controlBusy) return; controlBusy = true;
  for (const id of ['applyTarget', 'pauseJobs', 'applyTimeouts', 'equalPreview', 'applyCapture', 'hideExcel']) $(id).disabled = true;
  try {
    const response = await fetch('/api/control', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(change) });
    const result = await response.json(); if (!response.ok || result.ok === false) throw new Error(result.error || `Steuerung abgelehnt (${response.status}).`);
    state.controls = result.controls;
    for (const [id, key] of [['targetGroups', 'target_groups'], ['openTimeout', 'open_timeout_seconds'], ['jobTimeout', 'job_timeout_seconds'], ['equalPreview', 'equal_preview_width'], ['captureMegapixels', 'capture_tile_megapixels'], ['captureBatch', 'capture_batch_tiles']]) if (change[key] != null && Number($(id).value) === change[key]) dirtyControls.delete(id);
    updateControlInputs();
    const text = change.target_groups ? `Gruppenziel ${state.controls.target_groups} bestätigt.` : 'Steuerung vom Host bestätigt.';
    pushEvent(text); setNotice(text);
  } catch (error) { setNotice(error.message, true); updateControlInputs(); if ('hide_excel' in change) $('hideExcel').checked = Boolean(state.controls.hide_excel); }
  finally { controlBusy = false; for (const id of ['applyTarget', 'pauseJobs', 'applyTimeouts', 'equalPreview', 'applyCapture', 'hideExcel']) $(id).disabled = false; }
}
function selectGroup(id) {
  if (id === state.groupId) return;
  state.groupId = id; state.sheetId = null; state.focus = 'overview'; lastFocusKey = ''; state.view.x = 0; state.view.y = 0; latestCapture = null; previousImages.clear(); flashes.clear();
  data = null; entries = []; lastSignature = ''; sheetSignature = ''; clearDetails(); ++sceneGeneration;
  boards.splice(0).forEach(disposeObject); sourceTiles.length = 0; hitTargets.length = 0; sheetCards.clear(); processCards.clear(); $('processes').replaceChildren();
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
    latestCapture = latest.id; if (state.follow && !state.pinned && !state.selected) focusTile(latest);
  }
  clampView(); updateSheets(); renderScene(); updateDiagnosticUi();
  if (state.pendingDifference) { state.pendingDifference = false; nextDifference(); }
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
  renderThroughput();
  renderProcesses();
  $('restartGroup').disabled = !state.groupId || data?.restart_eligible === false || state.queue?.running === false;
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
function renderProcesses() {
  const clients = Object.entries(data?.launcher?.clients || {});
  const ids = new Set(clients.map(([id]) => id));
  for (const [id, card] of processCards) if (!ids.has(id)) { card.element.remove(); processCards.delete(id); }
  if (!clients.length && !processCards.size) { $('processes').textContent = 'Noch keine Excel-Prozesse gemeldet.'; return; }
  if (!processCards.size) $('processes').replaceChildren();
  const stageLabels = { staging: 'Datei wird bereitgestellt', waiting_for_open_slot: 'Wartet auf Öffnen', waiting_for_addin: 'Wartet auf Add-in', waiting_for_job: 'Wartet auf Auftrag', executing_job: 'Capture läuft', blocked_edit_mode: 'Excel im Bearbeitungsmodus', client_error: 'Add-in meldet Fehler', closed: 'Geschlossen' };
  for (const [id, client] of clients) {
    let card = processCards.get(id);
    if (!card) {
      const element = document.createElement('div'); element.className = 'process';
      const heart = document.createElement('span'); heart.className = 'heartbeat'; heart.textContent = '♥'; heart.setAttribute('aria-hidden', 'true');
      const name = document.createElement('strong'), process = document.createElement('span'), status = document.createElement('span'), beat = document.createElement('small'), error = document.createElement('small'), button = document.createElement('button');
      process.className = 'process-id'; status.className = 'process-stage'; error.className = 'process-error'; button.textContent = 'Mit Gruppe neu laden';
      button.addEventListener('click', () => restartGroup(id));
      element.append(heart, name, process, status, beat, error, button); $('processes').append(element);
      card = { element, heart, name, process, status, beat, error, button, count: null }; processCards.set(id, card);
    }
    const count = Number(client.heartbeat_count || 0), age = heartbeatAge(client);
    const alive = client.process_alive;
    if (card.count != null && count > card.count && !reducedMotion) {
      card.heart.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.4)', offset: .2 }, { transform: 'scale(1)' }], { duration: 520, easing: 'ease-out' });
    }
    card.count = count;
    const glow = age == null || client.status === 'closed' ? 0 : Math.max(0, Math.exp(-age / 14000));
    card.heart.style.setProperty('--beat-glow', String(glow));
    card.heart.style.opacity = String(.18 + glow * .82);
    card.element.classList.toggle('blocked', client.stage === 'blocked_edit_mode' || Boolean(client.error));
    card.name.textContent = profileLabel(id);
    card.process.textContent = client.pid ? `PID ${client.pid} · ${alive === true ? 'Prozess läuft' : alive === false ? 'Prozess beendet' : 'Prozessstatus unbestätigt'} · ${client.hide_excel ? 'verborgen' : 'sichtbar'}` : 'Noch keine PID';
    card.status.textContent = [stageLabels[client.stage] || normalizeStatus(client.status), client.active_command].filter(Boolean).join(' · ');
    card.beat.textContent = count ? `${count.toLocaleString('de-DE')} Heartbeats · letzter vor ${age == null ? '?' : Math.floor(age / 1000)} s` : 'Noch kein Add-in-Heartbeat';
    card.error.textContent = client.error || ''; card.error.hidden = !client.error;
    card.button.disabled = data?.restart_eligible === false || state.queue?.running === false;
    card.element.title = 'Ein ausbleibender Heartbeat kann bedeuten, dass Excel gerade öffnet oder beschäftigt ist. Der Prozessstatus wird separat geprüft.';
  }
}
function renderThroughput() {
  const metric = data?.throughput, settings = data?.capture_settings;
  $('pixelRate').textContent = metric?.pixels_per_second != null ? `${(metric.pixels_per_second / 1e6).toLocaleString('de-DE', { maximumFractionDigits: 2 })} MP/s` : '— MP/s';
  $('captureTotals').textContent = metric ? `${Number(metric.uploaded_images || 0).toLocaleString('de-DE')} Uploads · ${(Number(metric.captured_pixels || 0) / 1e6).toLocaleString('de-DE', { maximumFractionDigits: 1 })} MP · ${(Number(metric.png_bytes || 0) / 1048576).toLocaleString('de-DE', { maximumFractionDigits: 1 })} MiB` : 'Warte auf gemessene Captures';
  $('effectiveCapture').textContent = settings ? `Diese Gruppe: ${settings.tile_megapixels} MP · Batch ${settings.effective_batch_tiles ?? settings.batch_tiles}${settings.effective_batch_tiles != null && settings.effective_batch_tiles !== settings.batch_tiles ? ` / Ziel ${settings.batch_tiles}` : ''}` : 'Einstellungen werden beim Gruppenstart übernommen.';
}
async function restartGroup(clientId = null) {
  if (!state.groupId) return;
  const groupId = state.groupId;
  try {
    const response = await fetch(`/api/groups/${encodeURIComponent(groupId)}/restart`, { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...(clientId ? { client_id: clientId } : {}), reason: clientId ? `Add-in ${clientId} über Observatory neu laden` : 'Gruppe über Observatory neu einreihen' }) });
    const result = await response.json(); if (!response.ok || result.ok === false) throw new Error(result.error || `Neustart abgelehnt (${response.status})`);
    setNotice('Neustart angefragt. Die gesamte Gruppe wird nach dem sicheren Schließen als Nächstes eingereiht.');
    pushEvent(`Neustart angefragt${clientId ? ` für ${profileLabel(clientId)} und seine Vergleichsgruppe` : ' für die Gruppe'}.`);
  } catch (error) { setNotice(error.message, true); }
}
function nextDifference() {
  const targets = differenceTargets(entries);
  const index = targets.findIndex(({ tile, region }) => tile.id === state.selected && region.id === state.regionId);
  const target = targets[(index + 1) % targets.length];
  if (!target) { setNotice('Für diese Gruppe liegt bisher keine bestätigte Pixelabweichung vor.'); return; }
  focusTile(target.tile, target.region, true);
}
function nextSheetDifference() {
  const sheets = data?.sheets.filter(sheet => sheet.tiles.some(tile => tile.comparisonComplete && !tile.equalAcrossAll)) || [];
  const index = sheets.findIndex(sheet => sheet.id === state.sheetId);
  const sheet = sheets[(index + 1) % sheets.length];
  if (!sheet) { setNotice('Bisher kein Sheet mit bestätigter Abweichung.'); return; }
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
    setNotice('Keine weitere Gruppe mit gemeldeter Abweichung gefunden.');
  } finally { navigationBusy = false; $('nextGroupDifference').disabled = false; }
}

function wireControls() {
  $('groupSearch').addEventListener('input', () => { groupLimit = 100; renderGroups(); });
  $('moreGroups').addEventListener('click', () => { groupLimit += 100; renderGroups(); });
  function adjustTarget(delta) { const value = Math.max(1, Number($('targetGroups').value || 1) + delta); dirtyControls.add('targetGroups'); $('targetGroups').value = String(value); $('groupRange').max = String(Math.max(24, value)); $('groupRange').value = String(value); }
  $('minus').addEventListener('click', () => adjustTarget(-1)); $('plus').addEventListener('click', () => adjustTarget(1));
  for (const id of ['targetGroups', 'openTimeout', 'jobTimeout', 'equalPreview', 'captureMegapixels', 'captureBatch']) $(id).addEventListener('input', () => dirtyControls.add(id));
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
  $('applyCapture').addEventListener('click', () => {
    const size = Number($('captureMegapixels').value), batch = Number($('captureBatch').value);
    if (!Number.isInteger(size) || size < 1 || size > 32 || !Number.isInteger(batch) || batch < 1 || batch > 16) return setNotice('Capture-Zielgröße: 1–32 MP. Batch: 1–16 Aufträge.', true);
    sendControl({ capture_tile_megapixels: size, capture_batch_tiles: batch });
  });
  $('hideExcel').addEventListener('change', event => sendControl({ hide_excel: event.target.checked }));
  $('restartGroup').addEventListener('click', () => restartGroup());
  $('followCapture').addEventListener('change', event => { state.follow = event.target.checked; if (state.follow) { clearDetails(); focusTile(tileAt(latestCapture)); } });
  $('sheetSelect').addEventListener('change', event => focusSheet(event.target.value, true));
  for (const [id, axis] of [['panX', 'x'], ['panY', 'y']]) $(id).addEventListener('input', event => { stopFollowing(); state.focus = 'tile'; state.view[axis] = Number(event.target.value); clampView(); clearDetails(); renderScene(); focusCardCamera(state.sheetId); });
  $('viewScale').addEventListener('change', event => { stopFollowing(); state.focus = 'tile'; state.widthChoice = Number(event.target.value); clampView(); clearDetails(); renderScene(); focusCardCamera(state.sheetId); });
  $('pulseDiff').checked = state.pulse; $('pulseDiff').addEventListener('change', event => { state.pulse = event.target.checked; });
  $('nextDifference').addEventListener('click', nextDifference); $('nextSheetDifference').addEventListener('click', nextSheetDifference); $('nextGroupDifference').addEventListener('click', nextGroupDifference);
  $('resetCamera').addEventListener('click', () => { stopFollowing(); clearDetails(); state.focus = 'overview'; renderScene(); fitCamera(true); });
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
    groupId: state.groupId, sheetId: state.sheetId, selected: state.selected, regionId: state.regionId, focus: state.focus, sheetCards: sheetCards.size, pinned: state.pinned, follow: state.follow,
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
  const planned = [];
  sheetCards.get(state.sheetId)?.traverse(board => {
    const info = board.userData.pixelBoard; if (!info || !info.profile || info.board) return;
    for (const entry of visibleTiles(info.sheet, info.view, 1024)) {
      const r = profileRectangle(entry, info.profile.id), px = AREA_W / info.view.width;
      const x = (r.x - info.view.x + r.width / 2) * px - AREA_W / 2;
      const y = AREA_Y + AREA_H / 2 - (r.y - info.view.y + r.height / 2) * px;
      const point = new THREE.Vector3(RADIUS * Math.sin(x / RADIUS), y, RADIUS * (1 - Math.cos(x / RADIUS)));
      planned.push({ id: entry.id, profile: info.profile.id, center: project(board.localToWorld(point)) });
    }
  });
  return { sources: sourceTiles.map(tile => ({ id: tile.entry.id, profile: tile.profile.id, center: project(tile.group.getWorldPosition(new THREE.Vector3())) })),
    planned,
    boards: boards.map(board => ({ sheet: board.userData.sheetId, position: board.position.toArray(), rotationY: board.rotation.y, center: project(board.getWorldPosition(new THREE.Vector3())) })),
    details: detailPanels.map(panel => ({ profiles: panel.group.ids, center: project(panel.root.getWorldPosition(new THREE.Vector3())) })),
    connections: connections.map(link => ({ sourceProfile: link.source.profile.id, profiles: link.panel.group.ids,
      startError: link.curve.v0.distanceTo(link.source.group.getWorldPosition(new THREE.Vector3())),
      endError: link.curve.v3.distanceTo(link.panel.root.getWorldPosition(new THREE.Vector3()).addScaledVector(up, -link.panel.height * .48)) })) };
}

window.__observatory = { snapshot, geometry: geometrySnapshot, selectGroup, nextDifference, nextSheetDifference, nextGroupDifference, focusSheet,
  hoverTile(id, regionId) { const tile = tileAt(typeof id === 'number' ? entries[id]?.id : id); return showDetails(tile?.id, true, inspectionRegions(tile).find(region => region.id === regionId)); }, clearHover: clearDetails, pin: setPinned,
  setView(x, y, width = state.widthChoice) { stopFollowing(); state.focus = 'tile'; state.view.x = x; state.view.y = y; state.widthChoice = width; clampView(); renderScene(); },
  openEvidence(index = 0) { if (detailPanels[index]) openEvidence(detailPanels[index]); } };
try { initScene(); wireControls(); poll(); }
catch (error) { state.error = error.message; $('error').textContent = error.message; $('error').hidden = false; console.error(error); }
