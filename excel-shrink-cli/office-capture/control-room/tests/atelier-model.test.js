const assert = {
  equal(actual, expected, message = '') {
    if (actual !== expected) throw new Error(`${message}: ${actual} !== ${expected}`);
  },
};
const test = (name, run) => { run(); console.log(`PASS ${name}`); };
import { atelierRegions, evidenceImageUrl, shouldInspectFormulas } from '../atelier-model.js';

test('formula explanations require changed pixels in this capture', () => {
  assert.equal(shouldInspectFormulas({ changed_pixels: 1 }), true);
  for (const metrics of [undefined, {}, { changed_pixels: 0 },
    { changed_pixels: 0, unmatched_nonwhite_pixels: 30 }, { changed_pixels: NaN }]) {
    assert.equal(shouldInspectFormulas(metrics), false);
  }
  const report = { comparisons: [{ findings: [{ sheet: 'A', cell: 'A1',
    pixels_differ: false, formula_diagnostics: { volatile_paths: 1 } }] }] };
  assert.equal(atelierRegions(report).length, 0);
});

test('atelier retains phase-specific differences and chart evidence', () => {
  const report = {comparisons: [{left:'O', right:'SD', findings:[
    {sheet:'Deals',cell:'A1:I7',geometry_differs:false,blank_margin_ignored:true,
      left_png:'evidence/one/O.png',right_png:'evidence/one/SD.png'},
    {sheet:'Table',cell:'A1:D4',pixels_differ:true,left_png:'evidence/one/a.png'},
  ], charts:[{sheet:'Chart',chart:'Revenue',presence_differs:true}]}]};
  const differences = atelierRegions(report);
  assert.equal(differences.length, 2);
  assert.equal(differences[0].left, 'O');
  assert.equal(differences[0].right, 'SD');
  assert.equal(differences[0].rightUrl, null);
  assert.equal(atelierRegions(report,false).length, 3);
});

test('image paths are confined and encoded without accepting traversal or file URLs', () => {
  assert.equal(evidenceImageUrl('evidence/id/a b.png'), '/api/evidence/id/a%20b.png');
  for (const path of ['C:/secret.png','file:///secret.png','evidence/../secret.png',
    'evidence/id/./a.png','evidence/id/a.json','evidence//a.png']) {
    assert.equal(evidenceImageUrl(path), null, path);
  }
});
