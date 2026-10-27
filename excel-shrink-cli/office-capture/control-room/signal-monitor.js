import { createSignalStore } from './signal-model.js';
import { capturePackages, captureDescription } from './capture-strip-model.js';
import { capturePalette, createCaptureSpatialView } from './capture-spatial-view.js';
import { createHeartbeatTrace } from './heartbeat-shader.js';

// Two data lanes (actual heartbeats and image receipts) plus a slim phase track.
// An ECG-shaped event glyph is symbolic: no physiological/estimated data exist.
export const signalHistory = createSignalStore();
const WINDOW = 60000;
const views = new Set();
const motion = matchMedia('(prefers-reduced-motion: reduce)');
const phaseColors = {
  capturing: '#67ddaf', providing_images: '#67ddaf', executing_job: '#67ddaf',
  waiting_for_job: '#6399bd', waiting_for_peers: '#648ba4', waiting_for_open_slot: '#648ba4',
  waiting_for_addin: '#bd9b5f', planning_sheet: '#d2a75a', settling: '#bd9b5f',
  ready: '#6bb7b5', paused: '#648ba4', releasing: '#a6a0b8',
  blocked_edit_mode: '#e88c66', client_error: '#e87884', failed: '#e87884',
  closed: '#506474', finished: '#506474', complete: '#506474', completed: '#506474',
};
const stateLabels = {
  working: 'Recent work',
  'heartbeat-only': 'Heartbeats only',
  quiet: 'No new signals',
  unknown: 'Waiting for telemetry',
  ended: 'Process / attempt ended',
};
const element = (tag, className, text) => {
  const node = document.createElement(tag);
  node.className = className;
  if (text != null) node.textContent = text;
  return node;
};
function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}
let frame, lastFrame = 0;
const observer = new IntersectionObserver(entries => {
  for (const entry of entries) {
    const view = [...views].find(item => item.element === entry.target);
    if (view) { view.visible = entry.isIntersecting; if (view.visible) view.draw(); }
  }
  schedule();
});
function schedule() {
  if (frame || document.hidden || motion.matches || ![...views].some(view => view.visible && view.live)) return;
  frame = requestAnimationFrame(tick);
}
function tick(time) {
  frame = null;
  if (document.hidden) return;
  if (time - lastFrame >= 1000 / 30) {
    lastFrame = time;
    for (const view of views) {
      if (!view.element.isConnected) { view.destroy(); continue; }
      if (view.visible && view.live) view.draw();
    }
  }
  schedule();
}
document.addEventListener('visibilitychange', schedule);
motion.addEventListener('change', () => { for (const view of views) view.draw(); schedule(); });

const colorFor = state => phaseColors[state] || (state?.startsWith('waiting') ? '#648ba4' : '#526878');
const compactNumber = number => Number.isFinite(number) ? number.toLocaleString('en-US') : '—';
function drawCaptureGlyph(ctx, pack, x, y, compact) {
  const w = compact ? 34 : 46, h = compact ? 28 : 38, inset = 3;
  const boxes = [], layers = [...(pack.frame?.layers || [])].sort((a, b) => b.generation-a.generation);
  ctx.save();
  ctx.fillStyle = '#10252e'; ctx.strokeStyle = '#34677d'; ctx.lineWidth = .9;
  ctx.fillRect(x-w/2, y-h/2, w, h); ctx.strokeRect(x-w/2+.5, y-h/2+.5, w-1, h-1);
  ctx.strokeStyle = '#7595a31c'; ctx.lineWidth = .5;
  for (let i=1;i<4;i++) {
    ctx.beginPath();ctx.moveTo(x-w/2+w*i/4,y-h/2+1);ctx.lineTo(x-w/2+w*i/4,y+h/2-1);ctx.stroke();
    ctx.beginPath();ctx.moveTo(x-w/2+1,y-h/2+h*i/4);ctx.lineTo(x+w/2-1,y-h/2+h*i/4);ctx.stroke();
  }
  for (const layer of layers) {
    const style = capturePalette[layer.generation];
    if (!style) continue;
    ctx.fillStyle = style.color; ctx.globalAlpha = style.opacity;
    for (const rect of layer.rects || []) {
      const tx = x-w/2+inset+rect.x*(w-2*inset), ty = y-h/2+inset+rect.y*(h-2*inset);
      const width = Math.max(.7,rect.width*(w-2*inset)-.7), height = Math.max(.7,rect.height*(h-2*inset)-.7);
      ctx.fillRect(tx,ty,width,height);
      boxes.push({x:tx,y:ty,width,height,pack,capture:rect.event});
    }
  }
  ctx.globalAlpha = 1;
  if (!boxes.length) {
    // A count is honest when coordinates or the worksheet are not known.
    ctx.fillStyle = '#d6e8f0'; ctx.font = '10px Consolas, monospace'; ctx.textAlign = 'center';
    ctx.fillText(String(pack.count), x, y+3);
  }
  if (pack.count > 1 || pack.groupCount > 1) {
    ctx.fillStyle = '#b5d7e4'; ctx.font = '9px Consolas, monospace'; ctx.textAlign = 'center';
    ctx.fillText(pack.groupCount > 1 ? `+${pack.groupCount-1}` : String(pack.count), x, y+h/2+10);
  }
  ctx.restore();
  // Later layers are on top and win when the same range is captured again.
  return boxes.reverse();
}
function drawCaptures(ctx, events, now, left, width, y, compact) {
  const packages = capturePackages(events, now, WINDOW, width, compact);
  const xAt = at => left + (at - (now - WINDOW)) / WINDOW * width;
  ctx.strokeStyle = '#79dce83b'; ctx.lineWidth = 1; ctx.beginPath();
  ctx.moveTo(left, y); ctx.lineTo(left+width, y); ctx.stroke();
  ctx.save(); ctx.beginPath(); ctx.rect(left, y-25, width, compact ? 51 : 58); ctx.clip();
  // A real timestamp marker is drawn for EVERY received event, including those
  // combined into a visual package. Glyphs remain anchored to real receipt times.
  for (const event of events) {
    if (event.at < now-WINDOW || event.at > now) continue;
    const x = xAt(event.at);
    ctx.strokeStyle = '#79dce84c'; ctx.beginPath(); ctx.moveTo(x,y-11);ctx.lineTo(x,y+11);ctx.stroke();
  }
  const halfWidth = (compact ? 34 : 46)/2;
  const boxes = packages.map(pack => {
    const at = left+pack.x;
    // Keep a readable miniature at the edge; its tick stays at the real time.
    const center = Math.max(left+halfWidth,Math.min(left+width-halfWidth,at));
    ctx.strokeStyle='#79dce86b';ctx.beginPath();ctx.moveTo(at,y+23);ctx.lineTo(center,y+20);ctx.stroke();
    return drawCaptureGlyph(ctx,pack,center,y,compact);
  }).reverse().flat();
  ctx.restore();
  return boxes;
}
function drawStates(ctx, snapshot, now, left, width, y) {
  const first = now - WINDOW, historyStart = Math.max(first, snapshot.since);
  const items = snapshot.stateEvents.filter(item => item.state && item.at <= now);
  if (snapshot.currentState?.state && Number.isFinite(snapshot.currentState.at))
    items.push({ at: snapshot.currentState.at, state: snapshot.currentState.state });
  items.sort((a,b) => a.at-b.at);
  const xAt = at => left + (at-first)/WINDOW*width;
  let prevTime = historyStart, lastState = null;
  for (const item of items) if (item.at <= historyStart) lastState = item.state;
  const drawPart = (start,end,state) => {
    if (!state || end <= start) return;
    ctx.fillStyle = colorFor(state); ctx.fillRect(xAt(start),y-3, Math.max(.5,xAt(end)-xAt(start)),6);
  };
  for (const item of items) {
    if (item.at <= historyStart) continue;
    const at = Math.min(now,item.at);
    drawPart(prevTime,at,lastState);prevTime=at;lastState=item.state;
  }
  drawPart(prevTime,now,lastState);
  if (!items.length && snapshot.currentState?.state && snapshot.currentState.at == null) {
    // Current state is known, but its beginning is not: mark only a snapshot,
    // not a fabricated full minute of history.
    ctx.fillStyle = colorFor(snapshot.currentState.state);ctx.fillRect(xAt(now)-3,y-3,3,6);
  }
  // Successful jobs/initializations are milestones, not a mostly-empty third lane.
  for (const at of snapshot.points.progress) {
    if (at < historyStart || at > now) continue;
    const x=xAt(at);
    ctx.fillStyle='#e6b66a';ctx.beginPath();ctx.moveTo(x,y-8);ctx.lineTo(x+3,y-5);ctx.lineTo(x,y-2);ctx.lineTo(x-3,y-5);ctx.closePath();ctx.fill();
  }
}

export function createSignalMonitor({ compact = false, onCaptureNavigate = null } = {}) {
  const root = element('span', `signal-monitor${compact ? ' signal-compact' : ''}`);
  const heading = element('span', 'signal-heading');
  const state = element('span', 'signal-state', 'Waiting for telemetry');
  const interval = element('span', 'signal-window', '60 s');
  const fullscreen = element('button', 'signal-fullscreen', 'Full screen');
  fullscreen.type = 'button';
  fullscreen.setAttribute('aria-label', 'Fullscreen signal');
  fullscreen.setAttribute('aria-pressed', 'false');
  fullscreen.title = 'Expand this signal display. Press Escape to return.';
  fullscreen.disabled = !root.requestFullscreen;
  heading.append(state, interval, fullscreen);
  const canvas = element('canvas', 'signal-canvas');
  canvas.setAttribute('aria-hidden', 'true');
  const legend = element('span', 'signal-legend');
  const counterHeartbeat = element('b', '', '—');
  const counterCaptures = element('b', '', '—');
  const counterSteps = element('b', '', '—');
  const counters = [
    ['heartbeat', 'HB', counterHeartbeat],
    ['capture', 'Images', counterCaptures],
    ['progress', 'Steps', counterSteps],
  ];
  for (const [cls, label, counter] of counters) {
    const key = element('span', `signal-key signal-${cls}`);
    key.append(element('i', ''), document.createTextNode(label + ' '), counter);
    legend.append(key);
  }
  let row, connected=true, liveSource=true, stale=false, frozenAt, receivedAt, observed=false, signature;
  let frozenSnapshot,lastSnapshot,hitBoxes=[],layout=null;
  let destroyed = false;
  const trace = createHeartbeatTrace();
  const displayNote = element('span', 'signal-display-note');
  displayNote.hidden = true;
  displayNote.setAttribute('role', 'status');

  const showDisplayNote = text => {
    setText(displayNote, text);
    if (displayNote.hidden !== !text) displayNote.hidden = !text;
  };
  fullscreen.addEventListener('click', async event => {
    event.preventDefault();
    event.stopPropagation();
    try {
      if (document.fullscreenElement === root) await document.exitFullscreen();
      else await root.requestFullscreen();
      showDisplayNote('');
    } catch (error) {
      showDisplayNote(`Fullscreen could not start: ${error.message}`);
    }
  });
  const fullscreenChanged = () => {
    if (destroyed) return;
    const expanded = document.fullscreenElement === root;
    fullscreen.textContent = expanded ? 'Exit full screen' : 'Full screen';
    fullscreen.setAttribute('aria-label', expanded ? 'Exit fullscreen signal' : 'Fullscreen signal');
    fullscreen.setAttribute('aria-pressed', String(expanded));
    if (expanded) view.visible = true;
    view.draw();
    schedule();
  };
  document.addEventListener('fullscreenchange', fullscreenChanged);
  const resize = new ResizeObserver(() => {
    if (!destroyed) view.draw();
  });
  resize.observe(root);
  const spatial = createCaptureSpatialView({ compact, onCaptureNavigate: onCaptureNavigate
    ? payload => { if (row) onCaptureNavigate({ row, ...payload }); } : null });
  const follow = onCaptureNavigate ? element('button', 'signal-follow', '↗ Follow captures') : null;
  if (follow) {
    follow.type = 'button';
    follow.addEventListener('click', event => {
      event.stopPropagation();
      if (row) onCaptureNavigate({ row, capture: null, follow: true });
    });
    legend.append(follow);
  }
  if (onCaptureNavigate) {
    canvas.setAttribute('aria-hidden', 'false');
    canvas.setAttribute('role', 'button');
    canvas.tabIndex = 0;
    canvas.setAttribute('aria-label','Capture timeline: press Enter to open and follow current images in the Atelier. Select a capture to open that region.');
    canvas.addEventListener('click', event => {
      if (!row || !layout) return;
      const bounds = canvas.getBoundingClientRect();
      const x = (event.clientX - bounds.left) * layout.width / bounds.width;
      const y = (event.clientY - bounds.top) * layout.height / bounds.height;
      if (y < layout.capY - 25 || y > layout.capY + 29) return;
      const box = hitBoxes.find(b => x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height);
      onCaptureNavigate({ row, capture: box?.capture || null, follow: !box });
    });
    canvas.addEventListener('keydown', event => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      event.preventDefault();
      event.stopPropagation();
      if (row) onCaptureNavigate({ row, capture: null, follow: true });
    });
    canvas.addEventListener('mousemove', event => {
      if (!layout) return;
      const bounds = canvas.getBoundingClientRect();
      const x = (event.clientX - bounds.left) * layout.width / bounds.width;
      const y = (event.clientY - bounds.top) * layout.height / bounds.height;
      const box = hitBoxes.find(b => x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height);
      canvas.style.cursor = y >= layout.capY - 25 && y <= layout.capY + 29 ? 'pointer' : 'default';
      canvas.title = box ? `${box.capture.details?.range || 'Capture range'} · received ${new Date(box.capture.at).toLocaleTimeString('en-US')} · displayed in ${captureDescription(box.pack)}`
        : 'Select the capture timeline to view current images in the Atelier.';
    });
  }
  root.append(heading, canvas, legend, displayNote, spatial.element);
  root.title='A 60-second timeline of observed events. Each ECG-shaped pulse represents an observed add-in heartbeat. Miniatures show received captures; the bottom track shows reported phases and completed steps. Quiet signals alone do not diagnose a failure.';
  const view={element:root,visible:false,live:false,
    get captureExpanded(){return spatial.element.open;},
    set captureExpanded(value){spatial.element.open=Boolean(value);},
    update(next,options={}){
      if(!next)return;
      const same=row?.id===next.id;
      row=next;connected=options.connected!==false;liveSource=options.live!==false;stale=options.stale===true;
      const now=options.now??Date.now();
      const nextSignature=JSON.stringify([row.heartbeat,row.heartbeatCount,row.uploadedImages,row.completedCommands,
        row.lastProgress,row.lastSuccess,row.lastEvent,row.state,row.stateSince,row.active,row.alive,row.events]);
      if(!same){frozenAt=null;frozenSnapshot=null;lastSnapshot=null;observed=false;trace.reset();}
      if(connected&&!stale){
        lastSnapshot=signalHistory.observe(row,now);receivedAt=now;observed=true;
        if(signature!==nextSignature){frozenAt=null;frozenSnapshot=null;}
        signature=nextSignature;
      }
      view.live=connected&&!stale&&liveSource&&row.active!==false&&row.alive!==false;
      if(view.live){frozenAt=null;frozenSnapshot=null;}
      else{frozenAt??=receivedAt??now;frozenSnapshot??=lastSnapshot;}
      spatial.element.hidden = row.kind === 'Rust / Host';
      const spatialSnapshot = view.live ? lastSnapshot : frozenSnapshot;
      spatial.update(spatialSnapshot?.captureHistory || spatialSnapshot?.captureEvents || [], view.live ? now : frozenAt);
      view.draw();schedule();
    },
    draw(){
      if(!row||!observed)return;
      const now=view.live?Date.now():frozenAt;
      const snapshot=view.live?signalHistory.snapshot(row.id,now):frozenSnapshot;
      if(!snapshot)return;
      const status=!connected?'Disconnected · last received snapshot'
        :stale||!liveSource?'Saved signal history':stateLabels[snapshot.state]||stateLabels.unknown;
      const signalState = !connected ? 'disconnected' : snapshot.state;
      if (root.dataset.signalState !== signalState) root.dataset.signalState = signalState;
      setText(state, status);
      setText(interval, view.live ? 'LIVE · 60 s' : '60 s · last snapshot');
      const totals=snapshot.totals;
      setText(counterHeartbeat, compactNumber(totals.heartbeat));
      setText(counterCaptures, compactNumber(totals.capture));
      setText(counterSteps, compactNumber(totals.progress));
      const lastHeartbeat=snapshot.points.heartbeat.at(-1);
      const heartbeatAge=lastHeartbeat==null?'last heartbeat unknown':`last heartbeat ${Math.max(0,Math.floor((now-lastHeartbeat)/1000))} s ago`;
      const label = `${status}. ${snapshot.points.heartbeat.length} observed heartbeats; ${heartbeatAge}. ${snapshot.captureEvents.length} capture events and ${snapshot.points.progress.length} completed steps in 60 seconds. Unreported channels remain unknown.`;
      if (root.getAttribute('aria-label') !== label) root.setAttribute('aria-label', label);
      if (follow) {
        const disabled = row.kind === 'Rust / Host' || !row.groupId;
        if (follow.disabled !== disabled) follow.disabled = disabled;
      }
      if (document.hidden || !view.visible || !root.isConnected) return;
      const bounds = canvas.getBoundingClientRect();
      const width = Math.round(bounds.width), height = Math.round(bounds.height);
      if (width < 80 || height < 40) return;
      const expanded = document.fullscreenElement === root;
      const small = compact && !expanded;
      const ratio = Math.min(devicePixelRatio || 1, 1.5);
      if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
        canvas.width = Math.round(width * ratio);
        canvas.height = Math.round(height * ratio);
      }
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
      ctx.clearRect(0, 0, width, height);
      ctx.fillStyle = '#06140e';
      ctx.fillRect(0, 0, width, height);
      const left = expanded ? 48 : 34;
      const graphWidth = Math.max(1, width - left - 7);
      const traceHeight = expanded ? height * .58 : small ? 44 : 56;
      const heartY = 4 + traceHeight * .57;
      const capY = expanded ? height * .79 : small ? 83 : 103;
      const stateY = height - (expanded ? 30 : 20);
      layout = { width, height, capY };

      const rendered = trace.draw(ctx, {
        left, top: 4, width: graphWidth, height: height - 22,
        traceTop: 0, traceHeight,
        points: snapshot.points.heartbeat, now, windowMs: WINDOW,
        pixelRatio: ratio, reducedMotion: motion.matches,
        live: view.live, showTrace: snapshot.supported.heartbeat,
        observedSince: snapshot.since,
      });
      if (root.dataset.heartbeatRenderer !== rendered.renderer) {
        root.dataset.heartbeatRenderer = rendered.renderer;
      }
      if (rendered.renderer === 'canvas2d') {
        showDisplayNote('Software signal renderer: WebGL is unavailable.');
      } else if (displayNote.textContent.startsWith('Software signal renderer:')) {
        showDisplayNote('');
      }

      for (const [text, y, color] of [
        ['HB', heartY, '#75edc0'], ['CAP', capY, '#79dce8'], ['STATE', stateY, '#9bb3c2'],
      ]) {
        ctx.fillStyle = color;
        ctx.font = `${expanded ? 11 : 8}px Consolas, monospace`;
        ctx.fillText(text, 2, y + 3);
      }
      if (!snapshot.supported.heartbeat) {
        ctx.strokeStyle = '#485963';
        ctx.setLineDash([3, 4]);
        ctx.beginPath();
        ctx.moveTo(left, heartY);
        ctx.lineTo(left + graphWidth, heartY);
        ctx.stroke();
        ctx.setLineDash([]);
      }
      hitBoxes = drawCaptures(ctx, snapshot.captureEvents, now, left, graphWidth, capY, small);
      drawStates(ctx, snapshot, now, left, graphWidth, stateY);
      ctx.font = `${expanded ? 11 : 8}px Segoe UI, sans-serif`;
      ctx.fillStyle = '#99bda9';
      ctx.textAlign = 'left';
      ctx.fillText('−60 s', left, height - 3);
      ctx.fillText('−30 s', left + graphWidth / 2 - 10, height - 3);
      ctx.textAlign = 'right';
      ctx.fillText(view.live ? 'now' : 'saved', width - 6, height - 3);
      ctx.textAlign = 'left';
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      observer.unobserve(root);
      resize.disconnect();
      document.removeEventListener('fullscreenchange', fullscreenChanged);
      trace.destroy();
      views.delete(view);
      // Removing a fullscreen element also exits browser fullscreen. Its DOM
      // position never changes merely to enter or leave the expanded display.
      root.remove();
    },
  };
  views.add(view);observer.observe(root);
  return view;
}
