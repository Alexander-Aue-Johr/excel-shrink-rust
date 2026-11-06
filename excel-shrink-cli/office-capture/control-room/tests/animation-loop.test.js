import assert from 'node:assert/strict';
import { createVisibleAnimationLoop } from '../animation-loop.js';

let visible = true;
let sequence = 0;
const frames = new Map();
const draws = [];
const loop = createVisibleAnimationLoop(time => draws.push(time), {
  visible: () => visible,
  request(callback) { const id = ++sequence; frames.set(id, callback); return id; },
  cancel(id) { frames.delete(id); },
});
function nextFrame(time) {
  const [id, callback] = frames.entries().next().value;
  frames.delete(id);
  callback(time);
}
loop.refresh();
loop.refresh();
assert.equal(frames.size, 1, 'Repeated status polls cannot multiply render loops');
nextFrame(10);
assert.deepEqual(draws, [10]);
assert.equal(frames.size, 1);
visible = false;
loop.refresh();
assert.equal(frames.size, 0, 'A hidden/targeted-only scene schedules no GPU frames');
loop.refresh();
assert.equal(frames.size, 0);
visible = true;
loop.refresh();
nextFrame(20);
assert.deepEqual(draws, [10, 20], 'Returning to classic captures resumes its existing scene');
visible = false;
nextFrame(30);
assert.deepEqual(draws, [10, 20], 'A callback queued before hiding cannot render once hidden');
assert.equal(frames.size, 0);
visible = true;
loop.refresh();
loop.destroy();
loop.refresh();
assert.equal(frames.size, 0, 'Disposal prevents future callbacks');
console.log('PASS rendering suspension, resumption and single animation ownership');
