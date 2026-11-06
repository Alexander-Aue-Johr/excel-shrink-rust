import assert from 'node:assert/strict';
import { pixelSummary, requestPixelDiagnostics } from '../pixel-diagnostics.js';

assert.equal(pixelSummary({ changed_pixels: 2, max_channel_delta: 2, total_channel_delta: 4 }),
  '2 changed px · max Δ2/255 · ΣΔ 4');
assert.match(pixelSummary({ unmatched_pixels: 100, unmatched_nonwhite_pixels: 0 }),
  /0 changed px.*100 unpaired px \(0 non-white\)/);
assert.match(pixelSummary({ unmeasured_regions: 1 }), /1 regions unavailable/);

const calls = [];
let releaseFirst;
globalThis.fetch = async url => {
  calls.push(url);
  if (calls.length === 1) await new Promise(resolve => { releaseFirst = resolve; });
  return Response.json({ summary: { changed_pixels: 8 } });
};
const first = requestPixelDiagnostics('first');
const queued = requestPixelDiagnostics('queued');
const selected = requestPixelDiagnostics('selected', { priority: true, region: '0:charts:1' });
assert.equal(calls.length, 1, 'Only one request may inspect images at a time');
releaseFirst();
await Promise.all([first, queued, selected]);
assert.match(calls[1], /selected\/pixel-diagnostics\?region=0%3Acharts%3A1/);
await requestPixelDiagnostics('first');
assert.equal(calls.length, 3, 'Cached list measurements must not decode again');
console.log('PASS pixel summaries, bounded requests, selection priority and cache reuse');

let releaseBlocked;
const longRunCalls = [];
globalThis.fetch = async url => {
  longRunCalls.push(url);
  if (longRunCalls.length === 1) await new Promise(resolve => { releaseBlocked = resolve; });
  return Response.json({ summary: { changed_pixels: 0 } });
};
const pending = Array.from({ length: 128 }, (_, index) =>
  requestPixelDiagnostics(`long-run-${index}`).then(() => 'ok', error => error.message));
await assert.rejects(requestPixelDiagnostics('excess-prefetch'), /queue is full/);
const selectedWhenFull = requestPixelDiagnostics('selected-when-full', { priority: true, region: '1:findings:0' });
releaseBlocked();
const outcomes = await Promise.all(pending);
await selectedWhenFull;
assert.equal(outcomes.filter(value => value === 'ok').length, 127);
assert.equal(outcomes.filter(value => /deferred/.test(value)).length, 1);
assert.match(longRunCalls[1], /selected-when-full/, 'Explicit selection takes precedence over bounded list prefetch');
assert.equal(longRunCalls.length, 128, 'Excess background work never enters the server queue');
console.log('PASS long-run pending limit and interactive priority under saturation');
