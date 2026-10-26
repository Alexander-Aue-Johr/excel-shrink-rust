import { findCaptureTile, profileForClient, latestTileForProfile } from '../capture-navigation-model.js';

export function runTests() {
  let checks = 0;
  function check(value, message) {
    if (!value) throw new Error(message);
    checks++;
  }

  const group = {
    profiles: [{ id: 'original' }, { id: 'default' }],
    aliases: { default: 'physicalA' },
    sheets: [
      {
        id: '0', index: 0, name: 'First',
        tiles: [{
          id: 'tile0', tile_index: 0, address: 'A1:Z100',
          images: { original: { url: '/data/a', updated_at: 100 } },
        }],
      },
      {
        id: '1', index: 1, name: 'Second',
        tiles: [{
          id: 'tile1', tile_index: 0, address: 'A1:Z100',
          images: { default: { url: '/data/b', updated_at: 200 } },
        }],
      },
    ],
  };
  check(findCaptureTile(group, { details: { sheet_index: 1, tile_index: 0, range: 'A1:Z100' } })?.id === 'tile1',
    'Exact sheet and range resolve the observed tile.');
  check(findCaptureTile(group, { details: { range: 'A1:Z100' } }) === null,
    'An ambiguous range across sheets never opens the wrong tile.');
  check(findCaptureTile(group, { details: { range: 'A1:Z100', tile_index: 0 } }) === null,
    'Matching tile indices on different sheets cannot disambiguate an event.');
  check(findCaptureTile(group, { details: { sheet_name: 'First', range: '$A$1:$Z$100' } })?.id === 'tile0',
    'Sheet name and absolute A1 notation resolve a tile.');
  check(findCaptureTile(group, { details: { sheet_index: 1, sheet_name: 'Second', tile_index: 0 } })?.id === 'tile1',
    'Consistent sheet index and name allow navigation by tile index.');
  check(findCaptureTile(group, { details: { sheet_index: 1, sheet_name: 'First', range: 'A1:Z100' } }) === null,
    'Contradictory sheet index and name never choose either worksheet.');

  const oneSheet = { ...group, sheets: [group.sheets[0]] };
  check(findCaptureTile(oneSheet, { details: { sheet_index: 99, range: 'A1:Z100' } }) === null,
    'An unknown sheet index cannot fall back to an otherwise unique range.');
  check(findCaptureTile(oneSheet, { details: { sheet_name: 'Missing', range: 'A1:Z100' } }) === null,
    'An unknown sheet name cannot fall back to an otherwise unique range.');
  check(findCaptureTile(oneSheet, { details: { sheet_index: 99, sheet_name: 'First', tile_index: 0 } }) === null,
    'A matching name cannot override an unknown explicit sheet index.');
  check(findCaptureTile(oneSheet, { details: { sheet_index: 0, sheet_name: 'Missing', tile_index: 0 } }) === null,
    'A matching index cannot override an unknown explicit sheet name.');
  check(findCaptureTile(oneSheet, { details: { sheet_index: 0, tile_index: 1, range: 'A1:Z100' } }) === null,
    'A matching range cannot override a conflicting tile index.');
  check(findCaptureTile(oneSheet, { details: { sheet_index: 0, tile_index: 0, range: 'B2:C3' } }) === null,
    'A matching tile index cannot override a conflicting range.');
  check(findCaptureTile(oneSheet, { details: { sheet_index: 0 } }) === null,
    'A worksheet alone does not identify a capture tile.');
  check(findCaptureTile(oneSheet, { details: { sheet_index: 0, range: '' } }) === null,
    'An empty range does not identify a capture tile.');

  check(profileForClient(group, 'physicalA') === 'default',
    'A physical Office.js client maps to its logical profile.');
  check(latestTileForProfile(group, 'default')?.id === 'tile1'
    && latestTileForProfile(group, 'original')?.id === 'tile0',
    'Latest images use their actual update times.');
  const withoutTimes = {
    sheets: group.sheets.map(sheet => ({
      ...sheet,
      tiles: sheet.tiles.map(tile => ({ ...tile, images: { original: { url: '/data/a' } } })),
    })),
  };
  check(latestTileForProfile(withoutTimes, 'original') === null,
    'An image without a timestamp is not asserted to be the newest.');

  return { checks };
}

export const result = runTests();
if (typeof process !== 'undefined' && process.versions?.node) {
  console.log(`Capture navigation: ${result.checks} checks passed.`);
}
