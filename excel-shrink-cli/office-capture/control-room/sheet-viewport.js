// Sheet coordinates belong to the comparison, not to the 3D camera. All four
// profiles use this same viewport and the union of their measured extents.
export const FIT_MODES = ['width', 'height', 'sheet'];

const clamp = (value, low, high) => Math.max(low, Math.min(value, high));
const dimension = value => Math.max(1, Number(value) || 1);

export function resolveSheetViewport(sheet, aspect, previous = {}) {
  const ratio = Math.max(.000001, Number(aspect) || 1);
  const sheetWidth = dimension(sheet?.width), sheetHeight = dimension(sheet?.height);
  const mode = FIT_MODES.includes(previous.mode) ? previous.mode : previous.mode === 'manual' ? 'manual' : 'sheet';
  const width = mode === 'width' ? sheetWidth
    : mode === 'height' ? sheetHeight * ratio
    : mode === 'sheet' ? Math.max(sheetWidth, sheetHeight * ratio)
    : dimension(previous.width);
  const height = width / ratio;
  return { mode, width, height,
    x: clamp(Number(previous.x) || 0, 0, Math.max(0, sheetWidth - width)),
    y: clamp(Number(previous.y) || 0, 0, Math.max(0, sheetHeight - height)) };
}

export function fitSheetViewport(sheet, aspect, mode) {
  if (!FIT_MODES.includes(mode)) throw new Error(`Unknown sheet fit mode: ${mode}`);
  return resolveSheetViewport(sheet, aspect, { mode, x: 0, y: 0 });
}

// Following is navigation within the chosen view, not a request to inspect a
// tile at a different scale. Keep fit/manual zoom and reveal incoming row bands.
export function followSheetViewport(sheet, aspect, previous, tile) {
  const view = resolveSheetViewport(sheet, aspect, previous);
  if (!tile) return view;
  const top = Math.max(0, Number(tile.y) || 0), bottom = top + Math.max(0, Number(tile.height) || 0);
  if (top < view.y || bottom - top >= view.height) view.y = top;
  else if (bottom > view.y + view.height) view.y = bottom - view.height;
  return resolveSheetViewport(sheet, aspect, view);
}

export function zoomSheetViewport(sheet, aspect, previous, factor, anchor = { x: .5, y: .5 }) {
  const current = resolveSheetViewport(sheet, aspect, previous);
  const u = clamp(Number(anchor.x) || 0, 0, 1), v = clamp(Number(anchor.y) || 0, 0, 1);
  const full = fitSheetViewport(sheet, aspect, 'sheet');
  const width = clamp(current.width * factor, Math.min(16, full.width), full.width * 8);
  return resolveSheetViewport(sheet, aspect, { mode: 'manual', width,
    x: current.x + u * (current.width - width),
    y: current.y + v * (current.height - width / aspect) });
}

export function scrollSheetViewport(sheet, aspect, previous, deltaX, deltaY, shift = false) {
  const view = resolveSheetViewport(sheet, aspect, previous);
  const horizontal = dimension(sheet?.width) - view.width > .01;
  const vertical = dimension(sheet?.height) - view.height > .01;
  if (shift) view.x += deltaX || deltaY;
  else {
    // Preserve a touchpad's explicit horizontal gesture. A conventional wheel
    // follows the available axis; when both overflow, its default is vertical.
    view.x += deltaX;
    if (vertical) view.y += deltaY;
    else if (horizontal) view.x += deltaY;
  }
  return resolveSheetViewport(sheet, aspect, view);
}

export function sheetViewportKey(groupId, sheetId) {
  return JSON.stringify([String(groupId), String(sheetId)]);
}
