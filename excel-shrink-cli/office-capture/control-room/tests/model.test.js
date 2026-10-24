import { normalizeGroup, groupedPanels, imageStatus, visibleTiles, clippedTile, profileRectangle, activeGroupCount, sheetLayout, fitSheetView, inspectionEntry, inspectionRegions, differenceTargets, heartbeatAge } from '../model.js';
import { result as viewportTests } from './sheet-viewport.test.js';

function assert(condition, message) { if (!condition) throw new Error(message); }
function equal(actual, expected, message) { assert(JSON.stringify(actual) === JSON.stringify(expected), `${message}: ${JSON.stringify(actual)}`); }
const profiles = [{ id: 'original' }, { id: 'default' }, { id: 'maximum-cleaning' }, { id: 'too-aggressive-cleaning' }];
const image = pixelClass => ({ url: '/data/image.png', pixel_class: pixelClass });
const normalize = images => normalizeGroup({ profiles, sheets: [{ id: 'sheet', tiles: [{ id: 'tile', width: 800, height: 200, images }] }] }).sheets[0].tiles[0];

export function runTests() {
  const pending = normalize({ original: image(null), default: image(null), 'maximum-cleaning': image(null) });
  assert(!pending.comparisonComplete && !pending.equalAcrossAll, 'Missing peer images must remain unclassified.');
  equal(groupedPanels(pending, profiles).map(group => group.ids), [['original'], ['default'], ['maximum-cleaning']], 'Unknown classes must never merge.');
  assert(imageStatus(pending, 'default') === 'received', 'An image without a host class is received, not equal.');
  assert(imageStatus(pending, 'too-aggressive-cleaning') === 'waiting', 'Missing captures remain waiting.');

  const different = normalize({ original: image('a'), default: image('a'), 'maximum-cleaning': image('b'), 'too-aggressive-cleaning': image('b') });
  assert(different.comparisonComplete && !different.equalAcrossAll, 'A real difference cannot become an all-equal tile.');
  equal(groupedPanels(different, profiles).map(group => group.ids), [['original'], ['default'], ['maximum-cleaning', 'too-aggressive-cleaning']], 'Identical variants share a panel; the original proof remains separate.');
  assert(imageStatus(different, 'default') === 'equal' && imageStatus(different, 'maximum-cleaning') === 'different', 'Variant status follows the host pixel classes.');

  const allEqual = normalize(Object.fromEntries(profiles.map(profile => [profile.id, image('a')])));
  assert(allEqual.equalAcrossAll, 'Complete equal captures are recognized.');
  equal(groupedPanels(allEqual, profiles).map(group => group.ids), [['original'], ['default', 'maximum-cleaning', 'too-aggressive-cleaning']], 'All variants merge beside the reference.');

  const receiving = normalizeGroup({ profiles, sheets: [{ tiles: [{ status: 'receiving', width: 800, height: 200,
    images: Object.fromEntries(profiles.map(profile => [profile.id, { ...image('a'), updated_at: 1234, preview_url: '/data/preview.png' }])) }] }] }).sheets[0].tiles[0];
  assert(!receiving.comparisonComplete && imageStatus(receiving, 'default') === 'received', 'Arrival classes do not prematurely complete a host comparison.');
  assert(groupedPanels(receiving, profiles).length === 4, 'Unfinished comparisons keep variant panels separate.');
  assert(receiving.images.default.previewUrl === '/data/preview.png?v=1234', 'Replaced equal previews bypass the browser texture cache.');

  const sheet = normalizeGroup({ profiles, sheets: [{ tiles: [
    { id: 'first', width: 800, height: 200, core_rows: [0, 40] },
    { id: 'second', width: 800, height: 600, core_rows: [40, 45] },
    { id: 'third', width: 800, height: 100, core_rows: [45, 90] },
  ] }] }).sheets[0];
  equal(sheet.tiles.map(tile => tile.y), [0, 200, 800], 'Physical pixel heights determine position, not worksheet row counts.');
  equal(visibleTiles(sheet, { x: 0, y: 300, width: 300, height: 100 }).map(tile => tile.id), ['second'], 'The viewport shows only intersecting actual jobs.');
  equal(visibleTiles(sheet, { x: 900, y: 0, width: 300, height: 100 }), [], 'An empty sheet region must not fabricate tiles.');
  const clip = clippedTile(sheet.tiles[0], { x: 100, y: 50, width: 300, height: 100 });
  equal([clip.u0, clip.u1, clip.v0, clip.v1], [.125, .5, .25, .75], 'Clipping preserves the correct portion of the actual image.');

  const explicit = normalizeGroup({ sheets: [{ tiles: [{ id: 'offset', x: 120, y: 530, width: 80, height: 70 }] }] }).sheets[0];
  equal([explicit.tiles[0].x, explicit.tiles[0].y, explicit.width, explicit.height], [120, 530, 200, 600], 'Host offsets are retained exactly.');
  const heights = normalizeGroup({ sheets: [{ tiles: [{ id: 'height-change', x: 0, y: 0, width: 800, height: 200,
    images: { original: { ...image('a'), x: 0, y: 0, width: 800, height: 200 }, default: { ...image('b'), x: 0, y: 500, width: 800, height: 600 } } }] }] }).sheets[0];
  equal(profileRectangle(heights.tiles[0], 'default'), { x: 0, y: 500, width: 800, height: 600 }, 'A peer keeps its own image dimensions and cumulative offset.');
  assert(heights.height === 1100, 'Panning reaches peer content that extends beyond the original sheet.');
  equal(visibleTiles(heights, { x: 0, y: 800, width: 200, height: 100 }).map(tile => tile.id), ['height-change'], 'A job stays accessible when only a variant intersects the viewport.');
  equal(profileRectangle(heights.tiles[0], 'maximum-cleaning'), { x: 0, y: 0, width: 800, height: 200 }, 'Pending variants use the original plan until their own dimensions arrive.');
  assert(activeGroupCount({ groups: [{ status: 'unchanged' }, { status: 'differences' }, { status: 'skipped' }, { status: 'opening' }] }) === 1, 'Finished record statuses must never inflate the active count.');
  assert(activeGroupCount({ open_groups: 3, groups: [] }) === 3, 'The scheduler provides the authoritative open-group count.');
  assert(activeGroupCount({ open_groups: 3, runner_alive: false }) === 0, 'A stale snapshot from a dead runner must not claim active groups.');
  assert(activeGroupCount({ groups: [{ status: 'active', active: false }, { status: 'waiting_cleanup', active: true }] }) === 1, 'Explicit active flags take precedence over inferred status.');
  const grid = normalizeGroup({ sheets: [{ id: 'grid', width: 1600, height: 900, tiles: [
    { id: 'A1', x: 0, y: 0, width: 800, height: 450 }, { id: 'B1', x: 800, y: 0, width: 800, height: 450 },
    { id: 'A2', x: 0, y: 450, width: 800, height: 450 }, { id: 'B2', x: 800, y: 450, width: 800, height: 450 },
  ] }] }).sheets[0];
  equal(grid.tiles.map(tile => [tile.id, tile.x, tile.y]), [['A1', 0, 0], ['B1', 800, 0], ['A2', 0, 450], ['B2', 800, 450]], 'Job rectangles form a true 2D mosaic, not a cumulative strip.');
  equal(visibleTiles(grid, { x: 800, y: 450, width: 800, height: 450 }).map(tile => tile.id), ['B2'], 'A spatial viewport selects the correct job.');
  const planned = normalizeGroup({ sheets: [{ tiles: [{ width: 100, height: 100, planned_images: { default: { x: 200, y: 300, width: 400, height: 500 } } }] }] }).sheets[0];
  equal([planned.width, planned.height], [600, 800], 'Pending profile plans determine placeholder bounds before images arrive.');
  equal(profileRectangle(planned.tiles[0], 'default'), { x: 200, y: 300, width: 400, height: 500 }, 'A pending placeholder uses its profile plan.');
  const layout = sheetLayout(Array.from({ length: 9 }, (_, index) => ({ id: String(index) })));
  assert(layout.columns === 3 && layout.rows === 3 && layout.cards.every(card => card.width / card.height === 16 / 9), 'All sheet cards share 16:9 proportions in a near-square grid.');
  const fitted = fitSheetView({ width: 800, height: 45000 }, .5);
  assert(fitted.width >= 800 && fitted.height >= 45000 && fitted.width / fitted.height === .5, 'A very tall sheet fits without losing any planned rows.');
  const region = { id: 'changed', x: 1024, y: 576, width: 1024, height: 576, exact: true, changed_pixels: 3,
    pixel_classes: { original: 'a', default: 'a', 'maximum-cleaning': 'a', 'too-aggressive-cleaning': 'b' },
    changed_pixels_by_profile: { original: 0, default: 0, 'maximum-cleaning': 0, 'too-aggressive-cleaning': 3 } };
  const cropped = inspectionEntry(different, region, profiles);
  equal(groupedPanels(cropped, profiles).map(group => group.ids), [['original'], ['default', 'maximum-cleaning'], ['too-aggressive-cleaning']], 'Exact regional classes override whole-capture classes for grouping.');
  assert(cropped.images['too-aggressive-cleaning'].changedPixels === 3 && cropped.images['maximum-cleaning'].changedPixels === 0, 'Detail captions use region-specific pixel counts.');
  const unknownCrop = inspectionEntry(different, { ...region, exact: false }, profiles);
  assert(!unknownCrop.comparisonComplete && groupedPanels(unknownCrop, profiles).length === 4, 'A display crop without host proof never inherits whole-image equality.');
  const equalCrop = inspectionEntry(allEqual, { ...region, exact: false }, profiles);
  assert(equalCrop.equalAcrossAll && groupedPanels(equalCrop, profiles).length === 2, 'Proven whole-image equality remains valid for any common display crop.');
  const huge = { ...different, width: 12000, height: 1800 };
  assert(inspectionRegions(huge).every(part => part.width <= 1024 && part.height <= 576), 'Legacy huge captures remain inspectable in bounded display crops.');
  const classified = { ...huge, inspection_regions: [{ ...region, changed_pixels: 0, id: 'same' }, region] };
  equal(differenceTargets([classified]).map(target => target.region.id), ['changed'], 'Navigation skips host-proven equal regions within a differing capture.');
  assert(heartbeatAge({ last_heartbeat_at: '2026-10-24T00:00:00Z' }, Date.parse('2026-10-24T00:00:14Z')) === 14000, 'Heartbeat age is based on the received heartbeat timestamp.');
  assert(heartbeatAge({}) === null, 'Missing heartbeats are unknown, not fabricated fresh activity.');
  assert(heartbeatAge({ last_activity_at: '2026-10-24T00:00:00Z' }) === null, 'Ordinary activity must not masquerade as an actual heartbeat.');
  return { passed: 38 + viewportTests.passed };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) console.log(`Control-room model: ${result.passed} checks passed.`);
