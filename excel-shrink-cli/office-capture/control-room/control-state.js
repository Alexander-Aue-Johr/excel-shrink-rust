// Keep an older status request from undoing a newer control confirmation.
export function createControlUpdates(publish) {
  let revision = 0, writing = false;
  return {
    beginRead() { return revision; },
    acceptRead(startedRevision, controls) {
      if (writing || startedRevision !== revision) return false;
      publish(controls);
      return true;
    },
    beginWrite() { writing = true; revision++; },
    confirmWrite(controls) { publish(controls); },
    endWrite() {
      writing = false;
      // Reads begun during the write can also contain its previous value.
      // Invalidate them after failures too; the next fresh poll reconciles state.
      revision++;
    },
  };
}
