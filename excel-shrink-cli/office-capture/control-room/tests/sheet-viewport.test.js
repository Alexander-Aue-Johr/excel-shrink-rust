import { fitSheetViewport, resolveSheetViewport, zoomSheetViewport, scrollSheetViewport, sheetViewportKey } from '../sheet-viewport.js';

export function runViewportTests() {
  let passed = 0;
  function check(condition, message) { if (!condition) throw new Error(message); passed++; }
  const tall = { width: 800, height: 12000 }, wide = { width: 12000, height: 800 }, aspect = .5;
  const width = fitSheetViewport(tall, aspect, 'width');
  check(width.width === 800 && width.height === 1600, 'Fit width uses the common worksheet width.');
  const height = fitSheetViewport(wide, aspect, 'height');
  check(height.height === 800 && height.width === 400, 'Fit height retains horizontal overflow.');
  const page = fitSheetViewport(tall, aspect, 'sheet');
  check(page.height === 12000 && page.width === 6000, 'Fit sheet contains both dimensions.');
  const scrolled = scrollSheetViewport(tall, aspect, width, 0, 200);
  check(scrolled.y === 200 && scrolled.x === 0 && scrolled.mode === 'width', 'Wheel scroll preserves fit width.');
  const sideways = scrollSheetViewport(wide, aspect, height, 0, 200);
  check(sideways.x === 200 && sideways.y === 0, 'Wheel follows sole horizontal overflow.');
  const both = { mode: 'manual', width: 200, x: 40, y: 50 };
  const vertical = scrollSheetViewport(tall, aspect, both, 0, 100);
  check(vertical.x === 40 && vertical.y === 150, 'Manual mode with both axes uses vertical wheel scrolling.');
  const horizontal = scrollSheetViewport(tall, aspect, both, 0, 100, true);
  check(horizontal.x === 140 && horizontal.y === 50, 'Shift-wheel always scrolls horizontally.');
  const touchpad = scrollSheetViewport(tall, aspect, both, 35, 60);
  check(touchpad.x === 75 && touchpad.y === 110, 'Two-axis touchpad gestures preserve both deltas.');
  const unchanged = scrollSheetViewport(tall, aspect, page, 70, 90);
  check(unchanged.x === 0 && unchanged.y === 0, 'Fit sheet cannot scroll beyond its extents.');
  const edge = scrollSheetViewport(tall, aspect, width, 0, 1e9);
  check(edge.y === 10400, 'Scrolling is clamped to actual content.');
  const current = resolveSheetViewport(tall, aspect, both), anchor = { x: .7, y: .3 };
  const zoomed = zoomSheetViewport(tall, aspect, current, .5, anchor);
  check(zoomed.mode === 'manual' && zoomed.width === 100, 'Ctrl-wheel leaves fit mode.');
  check(zoomed.x + zoomed.width * anchor.x === current.x + current.width * anchor.x
    && zoomed.y + zoomed.height * anchor.y === current.y + current.height * anchor.y, 'The pointer retains the same sheet coordinate during zoom.');
  const reset = fitSheetViewport(tall, aspect, 'width');
  check(reset.width === 800 && reset.x === 0 && reset.y === 0, 'Selecting a fit mode overwrites prior manual zoom and offsets.');
  const resizedFit = resolveSheetViewport(tall, 1, scrolled);
  check(resizedFit.width === 800 && resizedFit.height === 800 && resizedFit.y === 200, 'Fit width recomputes height after panel resize.');
  const resizedHeight = resolveSheetViewport(wide, 1, sideways);
  check(resizedHeight.width === 800 && resizedHeight.height === 800 && resizedHeight.x === 200, 'Fit height recomputes width after panel resize.');
  const resizedManual = resolveSheetViewport(tall, 1, current);
  check(resizedManual.width === current.width && resizedManual.x === current.x && resizedManual.y === current.y, 'Manual resize retains scale and source origin.');
  const expanded = resolveSheetViewport({ width: 1600, height: 24000 }, aspect, width);
  check(expanded.width === 1600 && expanded.mode === 'width', 'Fit follows newly discovered geometry.');
  const shrunk = resolveSheetViewport({ width: 100, height: 100 }, aspect, { ...both, x: 900, y: 900 });
  check(shrunk.x === 0 && shrunk.y === 0, 'Shrinking geometry clamps stale offsets.');
  check(sheetViewportKey('one', 'sheet') !== sheetViewportKey('two', 'sheet'), 'Identical sheet IDs in different workbooks do not share zoom state.');
  check(sheetViewportKey('a:b', 'c') !== sheetViewportKey('a', 'b:c'), 'Viewport keys cannot collide through delimiters.');
  return { passed };
}

export const result = runViewportTests();
if (typeof process !== 'undefined' && process.versions?.node) console.log(`Sheet viewports: ${result.passed} checks passed.`);
