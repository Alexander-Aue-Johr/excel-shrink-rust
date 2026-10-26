import { parseA1Range, captureGroups, capturePackages, captureGeometry, captureFrames, captureDescription } from '../capture-strip-model.js';

export function runTests() {
  let checks = 0;
  function check(value, message) { if (!value) throw new Error(message); checks++; }
  const same = (actual, expected, message) => check(JSON.stringify(actual) === JSON.stringify(expected), message);
  const now = 1_800_000;
  const capture = (range, command_id, at = now - 1000, sheet_index = 0, sheet_name = 'Sheet 1') =>
    ({ at, details: { sheet_index, sheet_name, range, ...(command_id == null ? {} : { command_id }) } });

  same(parseA1Range("'Test Sheet'!$AA$12:$AB$22"), { x: 26, y: 11, width: 2, height: 11 },
    'Excel A1 ranges retain correct column and row coordinates.');
  check(['R1C1', 'Z0', 'XFE1', 'B3:A1', 'A1048577', 'A99999999999999999999'].every(value => parseA1Range(value) === null),
    'Unsupported, reversed and out-of-bounds worksheet ranges remain unavailable.');

  // The supplied design: three tiles in the first row, then two below, then
  // the final tile of that row and the first tile of the following row.
  const three = ['A1:B2', 'C1:D2', 'E1:F2'].map(range => capture(range, 'one', now - 30000));
  const two = ['A3:B4', 'C3:D4'].map(range => capture(range, 'two', now - 20000));
  const last = ['E3:F4', 'A5:B6'].map(range => capture(range, 'three', now - 10000));
  const events = [...three, ...two, ...last];
  const frames = captureFrames(events, { now });
  same(frames.map(frame => frame.count), [3, 2, 2], 'Actual capture jobs preserve the requested 3, 2, 2 packet sequence.');
  same(frames.map(frame => frame.layers.map(layer => layer.generation)), [[0], [0, 1], [0, 1, 2]],
    'Each card overlays only its current and two preceding capture generations.');
  same(frames[2].layers.map(layer => layer.events.length), [2, 2, 3], 'The third packet carries both earlier spatial generations.');
  check(frames.every(frame => JSON.stringify(frame.bounds) === JSON.stringify(frames[0].bounds)),
    'All three cards use the same worksheet coordinate basis.');
  const firstTile = frames[0].layers[0].rects[0];
  const olderTile = frames[2].layers[2].rects[0];
  same([firstTile.x, firstTile.y, firstTile.width, firstTile.height],
    [olderTile.x, olderTile.y, olderTile.width, olderTile.height], 'An older tile retains its coordinates across later packets.');
  check(firstTile.x === 0 && firstTile.y === 0 && firstTile.width === 1 / 3 && firstTile.height === 1 / 3,
    'The first tile occupies the first third of the common six-by-six source region.');
  check(frames[2].layers[0].rects[0].x === 2 / 3 && frames[2].layers[0].rects[0].y === 1 / 3
    && frames[2].layers[0].rects[1].x === 0 && frames[2].layers[0].rects[1].y === 2 / 3,
    'New tiles follow the actual worksheet row movement instead of filling every packet.');
  check(olderTile.event === three[0] && frames[2].layers[0].rects[1].event === last[1],
    'Every generation retains the exact source capture for direct tile navigation.');
  same(frames.map(frame => frame.latest), [now - 30000, now - 20000, now - 10000],
    'Frames retain real host receipt timestamps.');

  const packs = capturePackages(events, now, 60000, 600);
  check(packs.length === 3 && packs.every(pack => pack.groupCount === 1 && pack.aggregation === 'command'),
    'Separated capture jobs remain distinct timeline packages.');
  same(packs.map(pack => pack.x), [300, 400, 500], 'Timeline positions use exact receipt times, never a bin centre.');
  same(packs[2].frame.layers.map(layer => layer.generation), [0, 1, 2],
    'Timeline miniatures also include the three spatial generations.');
  check(packs[2].frame.layers[2].rects[0].x === firstTile.x && packs[2].frame.layers[2].rects[0].y === firstTile.y,
    'Timeline miniatures share the worksheet coordinate basis.');

  const burst = [capture('A1', 'job-a', now - 400), capture('B1', 'job-b', now - 300), capture('C1', 'job-c', now - 200)];
  const dense = capturePackages(burst, now, 60000, 80);
  check(dense.length === 1 && dense[0].count === 3 && dense[0].groupCount === 3 && dense[0].aggregation === 'display-bin',
    'Dense screen aggregation declares its real group count.');
  check(dense[0].groups.length === 3 && dense[0].events.every((event, index) => event === burst[index]),
    'Aggregation preserves every underlying group, capture and receipt timestamp.');
  check(captureDescription(dense[0]).includes('3 groups combined for display'), 'The tooltip distinguishes display aggregation from a capture job.');
  const wide = capturePackages(burst, now, 60000, 60000);
  check(wide.at(-1).frame.layers.length === dense[0].frame.layers.length,
    'Changing the pixel bins cannot change logical generation membership.');
  const boundary = capturePackages([capture('A1', 'left', now - 32000), capture('B1', 'right', now - 28000)], now, 60000, 600);
  check(boundary.length === 1 && boundary[0].x === 320,
    'Nearby packages on opposite nominal bin boundaries aggregate instead of overlapping or changing their time position.');
  const edge = capturePackages([capture('A1', 'edge-a', now - 25000), capture('B1', 'edge-b', now - 100)], now, 60000, 140);
  check(edge.length === 1 && edge[0].latest === now - 100 && edge[0].displayX === 117,
    'Glyphs shifted inward at the plot edge aggregate without changing their exact receipt-time anchor.');
  const longJob = captureGroups([capture('A1', 'one-job', now - 20000), capture('B1', 'one-job', now - 1000)], now);
  check(longJob.length === 1 && longJob[0].count === 2 && longJob[0].earliest === now - 20000,
    'A real command stays together even if its images arrive in different time bins.');
  const fallback = captureGroups([capture('A1', null, now - 500), capture('B1', null, now - 400)], now);
  check(fallback.length === 1 && fallback[0].grouping === 'time-window',
    'Captures without command identifiers use explicitly visual fixed receipt windows.');
  check(captureDescription(fallback[0]).includes('visual 1 s receipt group') && captureDescription(packs[0]).includes('received'),
    'Descriptions label visual grouping and host receipt time honestly.');

  const nextSheet = capture('Z99:AA100', 'four', now - 100, 1, 'Sheet 2');
  const changed = captureFrames([...events, nextSheet], { now });
  check(changed.length === 1 && changed[0].sheetName === 'Sheet 2' && changed[0].layers.length === 1,
    'A new worksheet never inherits another worksheet\'s spatial generations.');
  check(captureGroups([capture('A1', 'same-job'), capture('B1', 'same-job', now - 500, 1, 'Sheet 2')], now).length === 2,
    'One command spanning worksheets retains separate spatial groups.');
  const mixed = capturePackages([capture('A1', 'a'), nextSheet], now, 60000, 80);
  check(mixed.length === 1 && mixed[0].frame === null && mixed[0].count === 2,
    'Mixed-sheet screen aggregation has a count fallback instead of a fabricated common map.');
  check(captureGeometry([capture('A1', 'a'), capture('B1', 'a', now - 500, 0, 'Renamed Sheet')]) === null,
    'Conflicting names at the same sheet index cannot share a geometry map.');
  check(captureGeometry([{ at: now, details: { sheet_index: 0 } }]) === null
    && captureGeometry([{ at: now, details: { range: 'A1' } }]) === null,
    'Missing ranges or worksheet identity never create invented positions.');
  const noGeometry = captureFrames([{ at: now, details: { sheet_index: 0, command_id: 'no-range' } }], { now });
  check(noGeometry.length === 1 && noGeometry[0].geometryAvailable === false && noGeometry[0].layers[0].rects === null
    && noGeometry[0].unlocatedCount === 1,
    'A received capture with unknown geometry remains countable without fake tiles.');
  const unknownSheet = captureFrames([{ at: now, details: { range: 'A1', command_id: 'unknown' } }], { now });
  check(unknownSheet.length === 1 && unknownSheet[0].geometryAvailable === false,
    'An unknown worksheet also has a geometry-unavailable frame.');
  const partial = captureFrames([capture('A1', 'partial'), capture(null, 'partial', now - 900)], { now });
  check(partial[0].geometryAvailable && partial[0].unlocatedCount === 1 && partial[0].layers[0].rects.length === 1,
    'Partial geometry preserves source-backed tiles and explicitly counts only the unlocated captures.');

  const extra = ['A7:B8', 'A9:B10', 'A11:B12'].map((range, i) => capture(range, `extra-${i}`, now - 900 + i * 100));
  const bounded = captureFrames([...events, ...extra], { now });
  check(bounded.length === 3 && bounded.every(frame => frame.layers.length <= 3), 'Detail history and each overlay remain bounded to three generations.');
  check(captureFrames([...events].reverse(), { now })[2].id === frames[2].id, 'Out-of-order polling does not reverse spatial generations.');
  check(capturePackages([{ at: now + 1 }, { at: now - 100000 }], now, 60000, 150).length === 0,
    'Future and expired events do not enter the timeline.');
  check(captureFrames(events, { now: NaN }).length === 0 && capturePackages(events, now, 0, 100).length === 0
    && capturePackages(events, now, 60000, NaN).length === 0, 'Invalid time and dimensions stay empty.');
  return { checks };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) console.log(`Capture strip: ${result.checks} checks passed.`);
