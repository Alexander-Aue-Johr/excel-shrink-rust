import { normalizeGroup, groupedPanels, imageStatus, visibleTiles, clippedTile, profileRectangle, activeGroupCount } from '../model.js';

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
  return { passed: 23 };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) console.log(`Control-room model: ${result.passed} checks passed.`);
