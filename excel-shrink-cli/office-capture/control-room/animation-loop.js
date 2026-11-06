/** Own at most one animation callback, and none while the view is hidden.
 * Call refresh when visibility or presentation mode changes. */
export function createVisibleAnimationLoop(draw, {
  visible,
  request = callback => requestAnimationFrame(callback),
  cancel = frame => cancelAnimationFrame(frame),
}) {
  let frame = null;
  let stopped = false;

  function tick(time) {
    frame = null;
    if (stopped || !visible()) return;
    draw(time);
    refresh();
  }

  function refresh() {
    if (stopped || !visible()) {
      if (frame != null) cancel(frame);
      frame = null;
    } else if (frame == null) {
      frame = request(tick);
    }
  }

  return {
    refresh,
    destroy() { stopped = true; refresh(); },
  };
}
