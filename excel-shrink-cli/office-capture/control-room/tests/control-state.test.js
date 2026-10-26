import { createControlUpdates } from '../control-state.js';

function assert(condition, message) { if (!condition) throw new Error(message); }

function fixture() {
  let controls = { target_groups: 1 };
  const updates = createControlUpdates(value => { controls = value; });
  return { updates, target: () => controls.target_groups };
}

export function runTests() {
  const before = fixture();
  const oldRead = before.updates.beginRead();
  before.updates.beginWrite();
  before.updates.confirmWrite({ target_groups: 3 });
  before.updates.endWrite();
  before.updates.acceptRead(oldRead, { target_groups: 1 });
  assert(before.target() === 3, 'A GET begun before Apply cannot undo its confirmed target.');

  const during = fixture();
  during.updates.beginWrite();
  const overlappingRead = during.updates.beginRead();
  during.updates.acceptRead(overlappingRead, { target_groups: 2 });
  assert(during.target() === 1, 'A poll cannot change confirmed controls while Apply is pending.');
  during.updates.confirmWrite({ target_groups: 4 });
  during.updates.endWrite();
  during.updates.acceptRead(overlappingRead, { target_groups: 1 });
  assert(during.target() === 4, 'A GET begun during Apply cannot restore its previous value afterward.');

  const freshRead = during.updates.beginRead();
  during.updates.acceptRead(freshRead, { target_groups: 6 });
  assert(during.target() === 6, 'A fresh poll still accepts genuine subsequent host changes.');

  const failed = fixture();
  failed.updates.beginWrite();
  const readDuringFailure = failed.updates.beginRead();
  failed.updates.endWrite();
  failed.updates.acceptRead(readDuringFailure, { target_groups: 2 });
  assert(failed.target() === 1, 'An unconfirmed write cannot let an overlapping poll replace controls.');
  failed.updates.acceptRead(failed.updates.beginRead(), { target_groups: 5 });
  assert(failed.target() === 5, 'A failed write releases the guard so the next poll can reconcile state.');

  const zero = fixture();
  const oldPositiveRead = zero.updates.beginRead();
  zero.updates.beginWrite();
  zero.updates.confirmWrite({ target_groups: 0 });
  zero.updates.endWrite();
  zero.updates.acceptRead(oldPositiveRead, { target_groups: 1 });
  assert(zero.target() === 0, 'A confirmed drain target of zero cannot be replaced by an older positive target.');
  return { passed: 7 };
}

export const result = runTests();
