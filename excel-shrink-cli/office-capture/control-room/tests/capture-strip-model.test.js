import { parseA1Range, capturePackages, captureGeometry } from '../capture-strip-model.js';

export function runTests() {
  let checks = 0;
  function check(value, message) {
    if (!value) throw new Error(message);
    checks++;
  }

  const rect = parseA1Range("'Test Sheet'!$AA$12:$AB$22");
  check(JSON.stringify(rect) === JSON.stringify({ x: 26, y: 11, width: 2, height: 11 }),
    'Excel A1 ranges retain correct column and row coordinates.');
  check(parseA1Range('R1C1') === null && parseA1Range('Z0') === null && parseA1Range('XFE1') === null,
    'Unsupported or invalid geometry stays unavailable.');

  const timestamp = 1_800_000;
  const events = [
    { at: timestamp - 50000, details: { sheet_index: 0, range: 'A1:B2' } },
    { at: timestamp - 500, details: { sheet_index: 0, range: 'C1:D2' } },
    { at: timestamp - 400, details: { sheet_index: 0, range: 'A3:B4' } },
  ];
  const packs = capturePackages(events, timestamp, 60000, 300);
  check(packs.length === 2 && packs[1].count === 2 && packs[1].generation === 0 && packs[0].generation === 1,
    'A dense burst is grouped without moving the 60-second time axis.');
  const geometry = captureGeometry(packs[1].events);
  check(geometry.length === 2 && geometry[0].x > geometry[1].x && geometry[1].y > geometry[0].y,
    'Tile glyphs preserve relative worksheet positions.');
  check(captureGeometry([{ at: timestamp, details: { sheet_index: 0 } }]) === null,
    'Do not invent positions without a valid range.');
  check(captureGeometry([
    { at: timestamp, details: { sheet_index: 0, range: 'A1' } },
    { at: timestamp, details: { sheet_index: 1, range: 'B3' } },
  ]) === null, 'Do not collapse locations from different sheets into a fake common map.');
  check(capturePackages([{ at: timestamp + 1 }, { at: timestamp - 100000 }], timestamp, 60000, 150).length === 0,
    'Future and expired events do not enter the strip.');

  return { checks };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) {
  console.log(`Capture strip: ${result.checks} checks passed.`);
}
