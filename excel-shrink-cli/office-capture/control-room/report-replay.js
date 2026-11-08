import { ReplayClock, ReplayReader } from './replay-engine.js';

function replayElement(tag, text) {
  const element = document.createElement(tag);
  if (text != null) element.textContent = text;
  return element;
}

export function replayTime(milliseconds) {
  const seconds = Math.floor(milliseconds / 1000);
  return [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60]
    .map(value => String(value).padStart(2, '0')).join(':');
}

/** Playback changes this historical progress panel only; final result rows stay final. */
export function installReportReplay(content, descriptor) {
  const panel = replayElement('section');
  panel.className = 'report-replay report-details';
  panel.append(replayElement('h2', 'Recorded run replay'));
  const message = replayElement('p');
  panel.append(message);
  content.append(panel);
  if (!descriptor?.available || descriptor.manifest !== 'replay/manifest.json') {
    message.textContent = descriptor?.reason || 'Replay history was not recorded for this run.';
    return;
  }
  message.textContent = 'Run-level checkpoints. Detailed process, sheet, capture and heartbeat history is unavailable in this channel.';
  const load = replayElement('button', 'Load recorded replay');
  const controls = replayElement('fieldset');
  controls.className = 'report-replay-controls';
  controls.disabled = true;
  const play = replayElement('button', 'Play');
  const back = replayElement('button', '−10 s');
  const forward = replayElement('button', '+10 s');
  const restart = replayElement('button', 'Restart');
  const speed = replayElement('select');
  speed.setAttribute('aria-label', 'Replay speed');
  for (const value of [0.1, 1, 10, 100, 1000]) {
    const option = replayElement('option', `${value}×`);
    option.value = String(value);
    option.selected = value === 1;
    speed.append(option);
  }
  const seek = replayElement('input');
  seek.type = 'range';
  seek.min = '0';
  seek.step = '1';
  seek.value = '0';
  seek.setAttribute('aria-label', 'Recorded replay time');
  const time = replayElement('output', '00:00:00');
  const observation = replayElement('p', 'Load history to inspect a recorded checkpoint.');
  observation.setAttribute('aria-live', 'polite');
  const progress = replayElement('progress');
  progress.max = 1;
  progress.value = 0;
  progress.setAttribute('aria-label', 'Historical completed workbook pairs');
  controls.append(play, back, forward, restart, speed, seek, time);
  panel.append(load, controls, progress, observation);

  let reader;
  let clock;
  let pending;
  let generation = 0;
  let busy = false;
  let frame;
  let lastFrame = 0;
  let disposed = false;
  let displayedSequence;

  function displayTime() {
    const position = clock.position();
    seek.value = String(position);
    time.textContent = `${replayTime(position)} / ${replayTime(clock.duration)}`;
    play.textContent = clock.playing ? 'Pause' : 'Play';
  }

  async function showAt(position) {
    pending?.abort();
    pending = new AbortController();
    const current = ++generation;
    busy = true;
    try {
      const event = await reader.stateAt(position, pending.signal);
      if (current !== generation) return;
      if (!event) {
        displayedSequence = undefined;
        observation.textContent = 'No checkpoint had been recorded at this time.';
        progress.value = 0;
        return;
      }
      if (displayedSequence === event.sequence) return;
      displayedSequence = event.sequence;
      const state = event.state;
      const total = Number.isSafeInteger(state.total) && state.total >= 0 ? state.total : 0;
      const completed = Number.isSafeInteger(state.completed) && state.completed >= 0 ? state.completed : 0;
      progress.max = Math.max(1, total);
      progress.value = Math.max(0, Math.min(completed, total));
      observation.textContent = [
        `Checkpoint ${replayTime(event.at_ms)}`, `Phase ${state.state || 'unavailable'}`,
        `${completed} / ${total} completed`, `${state.failed ?? '?'} failed`,
        `${state.reused ?? '?'} reused`, state.current, state.error,
      ].filter(value => value != null && value !== '').join(' · ');
    } catch (error) {
      if (current === generation && error.name !== 'AbortError') {
        displayedSequence = undefined;
        clock.pause();
        observation.textContent = `Replay unavailable: ${error.message}`;
        displayTime();
      }
    } finally {
      if (current === generation) busy = false;
    }
  }

  function tick(now) {
    if (disposed || !clock.playing) return;
    displayTime();
    if (clock.playing && now - lastFrame >= 100) {
      lastFrame = now;
      if (clock.position() >= clock.duration) {
        clock.pause();
        void showAt(clock.duration);
        displayTime();
      } else if (!busy) void showAt(clock.position());
    }
    if (!disposed && clock.playing) frame = requestAnimationFrame(tick);
  }

  function jump(position) {
    clock.seek(position);
    displayTime();
    void showAt(clock.position());
  }

  load.addEventListener('click', async () => {
    load.disabled = true;
    pending?.abort();
    pending = new AbortController();
    try {
      reader = new ReplayReader(descriptor.manifest);
      const manifest = await reader.load(pending.signal);
      clock = new ReplayClock(manifest.duration_ms);
      seek.max = String(clock.duration);
      controls.disabled = false;
      load.hidden = true;
      await showAt(0);
      displayTime();
    } catch (error) {
      observation.textContent = `Replay unavailable: ${error.message}`;
    } finally {
      load.disabled = false;
    }
  });
  play.addEventListener('click', () => {
    if (clock.playing) {
      clock.pause();
      cancelAnimationFrame(frame);
    } else {
      clock.play();
      frame = requestAnimationFrame(tick);
    }
    displayTime();
    void showAt(clock.position());
  });
  back.addEventListener('click', () => jump(clock.position() - 10000));
  forward.addEventListener('click', () => jump(clock.position() + 10000));
  restart.addEventListener('click', () => jump(0));
  seek.addEventListener('input', () => jump(Number(seek.value)));
  speed.addEventListener('change', () => clock.setSpeed(Number(speed.value)));
  window.addEventListener('pagehide', () => {
    disposed = true;
    generation++;
    pending?.abort();
    clock?.pause();
    cancelAnimationFrame(frame);
  }, { once: true });
}
