import { createSignalStore } from './signal-model.js';
import { capturePackages, captureGeometry, captureDescription } from './capture-strip-model.js';

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
  if (time - lastFrame >= 66) {
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
const pulseForm = [[-1, 0], [-.7, 0], [-.52, -2], [-.38, 2], [-.12, 2], [0, -15],
  [.12, 12], [.26, -3], [.39, 0], [.62, 0], [.77, -3], [.9, -2], [1, 0]];

function drawHeart(ctx, points, now, xAt, y, plotLeft, plotRight) {
  ctx.save(); ctx.beginPath(); ctx.rect(plotLeft, y - 18, plotRight - plotLeft, 33); ctx.clip();
  ctx.strokeStyle = '#75edc04c'; ctx.lineWidth = 1;
  ctx.beginPath(); ctx.moveTo(plotLeft, y); ctx.lineTo(plotRight, y); ctx.stroke();
  const halfWidth = 9; // Symbol width, NOT a reconstructed waveform.
  for (const at of points) {
    if (at < now - WINDOW || at > now) continue;
    const x = xAt(at);
    ctx.strokeStyle = '#75edc0'; ctx.globalAlpha = .34 + .66 * Math.max(0, 1 - (now - at) / WINDOW);
    ctx.lineWidth = 1.7; ctx.lineJoin = 'round'; ctx.beginPath();
    pulseForm.forEach(([px, py], i) => i ? ctx.lineTo(x + px * halfWidth, y + py) : ctx.moveTo(x + px * halfWidth, y + py));
    ctx.stroke();
  }
  ctx.restore();
}
function drawCaptureGlyph(ctx, pack, x, y, compact) {
  const w = compact ? 19 : 28, h = compact ? 16 : 23;
  const rects = captureGeometry(pack.events);
  const alpha = pack.generation === 0 ? 1 : pack.generation === 1 ? .62 : pack.generation === 2 ? .36 : .2;
  ctx.save(); ctx.globalAlpha = alpha;
  ctx.fillStyle = '#79dce824'; ctx.strokeStyle = '#79dce8'; ctx.lineWidth = .8;
  ctx.fillRect(x-w/2, y-h/2, w, h); ctx.strokeRect(x-w/2+.5, y-h/2+.5, w-1, h-1);
  if (rects) {
    const inset = 2, iw = w - 2*inset, ih = h - 2*inset;
    for (const rect of rects.slice(-24)) {
      ctx.fillStyle = '#79dce8';
      const tx = x-w/2+inset+rect.x*iw, ty = y-h/2+inset+rect.y*ih;
      ctx.fillRect(tx, ty, Math.max(1.2, rect.width*iw), Math.max(1.2, rect.height*ih));
    }
  } else {
    // Rectangles with no valid worksheet geometry must not pretend to encode positions.
    ctx.fillStyle = '#d6e8f0'; ctx.font = '9px Consolas, monospace'; ctx.textAlign = 'center';
    ctx.fillText(String(pack.count), x, y+3);
  }
  ctx.restore();
  // Count makes high-throughput aggregation explicit even when glyphs are tiny.
  if (pack.count > 1 && rects) {
    ctx.save(); ctx.fillStyle = '#b5d7e4'; ctx.globalAlpha = Math.max(.45, alpha);
    ctx.font = '9px Consolas, monospace'; ctx.textAlign = 'center';
    ctx.fillText(String(pack.count), x, y + h/2 + 9); ctx.restore();
  }
  return { x: x-w/2-2, y:y-h/2-3, width:w+4, height:h+14, pack };
}
function drawCaptures(ctx, events, now, left, width, y, compact) {
  const packages = capturePackages(events, now, WINDOW, width, compact);
  const xAt = at => left + (at - (now - WINDOW)) / WINDOW * width;
  ctx.strokeStyle = '#79dce83b'; ctx.lineWidth = 1; ctx.beginPath();
  ctx.moveTo(left, y); ctx.lineTo(left+width, y); ctx.stroke();
  ctx.save(); ctx.beginPath(); ctx.rect(left, y-14, width, compact ? 35 : 44); ctx.clip();
  // A real timestamp marker is drawn for EVERY received event, including those
  // combined into a visual package. Pack positions are bounded screen bins.
  for (const event of events) {
    if (event.at < now-WINDOW || event.at > now) continue;
    const x = xAt(event.at);
    ctx.strokeStyle = '#79dce84c'; ctx.beginPath(); ctx.moveTo(x,y-11);ctx.lineTo(x,y+11);ctx.stroke();
  }
  const boxes = packages.map(pack => drawCaptureGlyph(ctx,pack,left+pack.x,y,compact));
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
  heading.append(state, interval);
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
      if (y < layout.capY - 17 || y > layout.capY + 23) return;
      const box = hitBoxes.find(b => x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height);
      onCaptureNavigate({ row, capture: box?.pack.events.at(-1) || null, follow: !box });
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
      canvas.style.cursor = y >= layout.capY - 17 && y <= layout.capY + 23 ? 'pointer' : 'default';
      canvas.title = box ? captureDescription(box.pack)
        : 'Select the capture timeline to view current images in the Atelier.';
    });
  }
  root.append(heading, canvas, legend);
  root.title='A 60-second timeline of observed events. Each ECG-shaped pulse represents an observed add-in heartbeat. Miniatures show received captures; the bottom track shows reported phases and completed steps. Quiet signals alone do not diagnose a failure.';
  const view={element:root,visible:false,live:false,
    update(next,options={}){
      if(!next)return;
      const same=row?.id===next.id;
      row=next;connected=options.connected!==false;liveSource=options.live!==false;stale=options.stale===true;
      const now=options.now??Date.now();
      const nextSignature=JSON.stringify([row.heartbeat,row.heartbeatCount,row.uploadedImages,row.completedCommands,
        row.lastProgress,row.lastSuccess,row.lastEvent,row.state,row.stateSince,row.active,row.alive,row.events]);
      if(!same){frozenAt=null;frozenSnapshot=null;lastSnapshot=null;observed=false;}
      if(connected&&!stale){
        lastSnapshot=signalHistory.observe(row,now);receivedAt=now;observed=true;
        if(signature!==nextSignature){frozenAt=null;frozenSnapshot=null;}
        signature=nextSignature;
      }
      view.live=connected&&!stale&&liveSource&&row.active!==false&&row.alive!==false;
      if(view.live){frozenAt=null;frozenSnapshot=null;}
      else{frozenAt??=receivedAt??now;frozenSnapshot??=lastSnapshot;}
      view.draw();schedule();
    },
    draw(){
      if(!row||!observed)return;
      const now=view.live?Date.now():frozenAt;
      const snapshot=view.live?signalHistory.snapshot(row.id,now):frozenSnapshot;
      if(!snapshot)return;
      const status=!connected?'Disconnected · last received snapshot'
        :stale||!liveSource?'Saved signal history':stateLabels[snapshot.state]||stateLabels.unknown;
      root.dataset.signalState=!connected?'disconnected':snapshot.state;
      state.textContent=status;
      interval.textContent=view.live?'LIVE · 60 s':'60 s · last snapshot';
      const totals=snapshot.totals;
      counterHeartbeat.textContent=compactNumber(totals.heartbeat);
      counterCaptures.textContent=compactNumber(totals.capture);
      counterSteps.textContent=compactNumber(totals.progress);
      const lastHeartbeat=snapshot.points.heartbeat.at(-1);
      const heartbeatAge=lastHeartbeat==null?'last heartbeat unknown':`last heartbeat ${Math.max(0,Math.floor((now-lastHeartbeat)/1000))} s ago`;
      root.setAttribute('aria-label',`${status}. ${snapshot.points.heartbeat.length} observed heartbeats; ${heartbeatAge}. ${snapshot.captureEvents.length} capture events and ${snapshot.points.progress.length} completed steps in 60 seconds. Unreported channels remain unknown.`);
      if (follow) follow.disabled = row.kind === 'Rust / Host' || !row.groupId;
      if (document.hidden || !view.visible || !root.isConnected) return;
      const width=Math.round(root.getBoundingClientRect().width);
      if(width<80)return;
      const height=compact?97:118,ratio=Math.min(devicePixelRatio||1,1.5);
      if(canvas.width!==Math.round(width*ratio)||canvas.height!==Math.round(height*ratio)){
        canvas.width=Math.round(width*ratio);canvas.height=Math.round(height*ratio);
      }
      const ctx=canvas.getContext('2d');if(!ctx)return;
      ctx.setTransform(ratio,0,0,ratio,0,0);ctx.clearRect(0,0,width,height);
      ctx.fillStyle='#08151f';ctx.fillRect(0,0,width,height);
      const left=34,graphWidth=Math.max(1,width-left-7),heartY=compact?17:23,capY=compact?50:61,stateY=compact?77:96;
      const xAt=at=>left+(at-(now-WINDOW))/WINDOW*graphWidth;
      layout={width,height,capY};
      ctx.strokeStyle='#19313e';ctx.lineWidth=.6;
      for(let i=0;i<=12;i++){
        const x=left+graphWidth*i/12;ctx.beginPath();ctx.moveTo(x,4);ctx.lineTo(x,stateY+5);ctx.stroke();
      }
      for(const y of [heartY,capY,stateY]){
        ctx.strokeStyle='#1c3440';ctx.beginPath();ctx.moveTo(left,y+14);ctx.lineTo(left+graphWidth,y+14);ctx.stroke();
      }
      const observeStart=Math.max(now-WINDOW,snapshot.since);
      if(observeStart>now-WINDOW){ctx.fillStyle='#070e18d0';ctx.fillRect(left,4,Math.max(0,xAt(observeStart)-left),stateY+6);}
      for(const [text,y,color] of [['HB',heartY,'#75edc0'],['CAP',capY,'#79dce8'],['STATE',stateY,'#9bb3c2']]){
        ctx.fillStyle=color;ctx.font='8px Consolas,monospace';ctx.fillText(text,2,y+3);
      }
      if(snapshot.supported.heartbeat) drawHeart(ctx,snapshot.points.heartbeat,now,xAt,heartY,left,left+graphWidth);
      else {ctx.strokeStyle='#485963';ctx.setLineDash([3,4]);ctx.beginPath();ctx.moveTo(left,heartY);ctx.lineTo(left+graphWidth,heartY);ctx.stroke();ctx.setLineDash([]);}
      hitBoxes=drawCaptures(ctx,snapshot.captureEvents,now,left,graphWidth,capY,compact);
      drawStates(ctx,snapshot,now,left,graphWidth,stateY);
      ctx.font='8px Segoe UI,sans-serif';ctx.fillStyle='#7998aa';ctx.textAlign='left';
      ctx.fillText('−60 s',left,height-3);ctx.fillText('−30 s',left+graphWidth/2-10,height-3);
      ctx.textAlign='right';ctx.fillText(view.live?'now':'saved',width-6,height-3);ctx.textAlign='left';
    },
    destroy(){observer.unobserve(root);views.delete(view);root.remove();},
  };
  views.add(view);observer.observe(root);
  return view;
}
