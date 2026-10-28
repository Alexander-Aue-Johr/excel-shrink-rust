// The supplied Shadertoy trace, adapted to observed protocol events. This is a
// symbolic heartbeat display; it does not measure physiological activity.
const PULSE_DURATION_MS = 1200;
const REVEAL_DURATION_MS = 850;
const MAX_RENDER_PIXELS = 2000000;
const MAX_POINTS = 256;
const DOMAIN = 1.3;
const R_POSITION = 0.425;

const vertexShader = `#version 300 es
layout(location = 0) in vec2 corner;
uniform vec2 resolution;
void main() {
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}`;

const backgroundShader = `#version 300 es
precision highp float;
uniform vec2 resolution;
uniform float baseline;
uniform bool showTrace;
out vec4 fragColor;

// Grid from the user's original green CRT shader. The cells remain square.
void drawGrid(inout vec3 col, vec2 coord) {
    vec2 uv = (coord - 0.5 * resolution.xy) / resolution.xx;
    uv *= 40.0;
    float pixel = dFdx(uv.x);
    uv = fract(uv - 0.5);
    float distanceToLine = min(abs(uv.x - 0.5), abs(uv.y - 0.5));
    float line = 1.0 - smoothstep(0.0, pixel * 1.5, distanceToLine);
    col = mix(col, vec3(0.2, 0.6, 0.3), line);
}

float vignette(vec2 coord) {
    vec2 uv = coord / resolution - 0.5;
    uv = max(vec2(0.00001), 1.0 - abs(uv) * 2.0);
    return 1.0 - 2.0 / (1.0 / uv.x + 1.0 / uv.y);
}

void main() {
    float v = vignette(gl_FragCoord.xy);
    vec3 color = mix(vec3(0.1, 0.4, 0.2), vec3(0.0), v);
    drawGrid(color, gl_FragCoord.xy);
    if (showTrace) {
        float y = resolution.y - gl_FragCoord.y;
        color += vec3(0.0, 0.11, 0.075) * exp(-abs(y - baseline));
    }
    color *= mix(1.0, 0.3, pow(v, 4.0));
    color *= 0.12;
    fragColor = vec4(color, 1.0);
}`;

const pulseVertexShader = `#version 300 es
layout(location = 0) in vec2 corner;
// R-peak pixel position, symbol width, revealed fraction, observed-age opacity.
layout(location = 1) in vec4 event;
uniform vec2 resolution;
uniform vec2 traceBand;
flat out vec4 pulse;
void main() {
    float left = event.x - (0.425 / 1.3) * event.y;
    float padding = 8.0;
    vec2 position = vec2(left - padding + corner.x * (event.y + padding * 2.0),
        traceBand.x + corner.y * traceBand.y);
    gl_Position = vec4(position.x / resolution.x * 2.0 - 1.0,
        1.0 - position.y / resolution.y * 2.0, 0.0, 1.0);
    pulse = event;
}`;

const pulseShader = `#version 300 es
precision highp float;
uniform vec2 resolution;
uniform vec2 traceBand;
uniform float pixelRatio;
flat in vec4 pulse;
out vec4 fragColor;

#define AA_FALLOFF 1.2
#define CURVE_WIDTH 10.0
#define P_HEIGHT 0.15
#define Q_DEPTH 0.065
#define T_HEIGHT 0.18

// ECG waves and intervals (schematic, approximately lead II):
//
// P, Q, R, S and T are historical letter designations introduced by
// Willem Einthoven; they are NOT abbreviations for individual words.
//
// P wave:     Atrial depolarization (electrical activation).
// Q wave:     First negative deflection BEFORE R; early ventricular activation.
// R wave:     First positive deflection of QRS; ventricular activation.
// S wave:     Negative deflection AFTER R; late ventricular activation.
// T wave:     Ventricular repolarization (electrical recovery).
//
// PR/PQ interval: P-wave onset to QRS ONSET, not to the R peak
//                 (typically 120-200 ms).
// PR/PQ segment: P-wave end to QRS onset; close to the baseline.
// QRS duration:  QRS onset to QRS end (typically about 60-110 ms).
// ST segment:    QRS end (the J point) to T-wave onset; usually nearly flat.
// QT interval:   QRS onset to T-wave end; depends on heart rate.
// RR interval:   Time between consecutive R peaks.
// TP segment:    T-wave end to the next P wave; the baseline.
//
// This curve is only a plausible visual illustration. Its time and voltage
// axes are NOT medically calibrated. Here each pulse denotes a software event.
//
// The three easiest deflections to adjust:
// P_HEIGHT: previously 0.10; P is now 50% taller.
// Q_DEPTH:  previously 0.14; Q is now much shallower.
// T_HEIGHT: previously 0.32; T is now substantially lower.

float gaussPulse(float x, float center, float width) {
    float distance = (x - center) / width;
    return exp(-distance * distance);
}

// Q-R-S is ONE connected deflection rather than three separate Gaussians.
// The curve rises directly and steeply from Q to R, without a plateau.
float qrsComplex(float x) {
    const float qStart = 0.350;  // QRS onset.
    const float qBottom = 0.390; // Bottom turning point of Q.
    const float rTop = 0.425;    // R peak.
    const float sBottom = 0.470; // Bottom turning point of S.
    const float sEnd = 0.515;    // QRS end and start of the ST segment.
    const float rHeight = 1.20;  // Unchanged R height.
    const float sDepth = 0.30;   // Unchanged S depth.

    if (x < qStart || x >= sEnd) return 0.0;
    // A small Q trough with a gentle onset.
    if (x < qBottom) return -Q_DEPTH * smoothstep(qStart, qBottom, x);
    // A direct, steep rise from Q to R.
    if (x < rTop) return mix(-Q_DEPTH, rHeight, (x - qBottom) / (rTop - qBottom));
    // A steep descent from R to S.
    if (x < sBottom) return mix(rHeight, -sDepth, (x - rTop) / (sBottom - rTop));
    // A gentle return from S to the ST baseline.
    return -sDepth * (1.0 - smoothstep(sBottom, sEnd, x));
}

float heartbeat(float x) {
    // P is taller and wider, with the rounded shape of a typical P wave.
    float p = P_HEIGHT * gaussPulse(x, 0.18, 0.065);
    // QRS consists of short, connected Q-R-S deflections.
    float qrs = qrsComplex(x);
    // T is lower and wider, rising more slowly than it falls. Its left half
    // therefore has a greater width than its right half.
    float tWidth = x < 0.74 ? 0.115 : 0.085;
    float t = T_HEIGHT * gaussPulse(x, 0.74, tWidth);
    return p + qrs + t;
}

float aaStep(float a, float b, float x) {
    return (clamp(x, a, b) - a) / (b - a);
}

float baseline() { return traceBand.x + traceBand.y * (4.0 / 7.0); }

float curveY(float phase) {
    // The supplied shader uses half-height heartbeat values and graphRange 1.4.
    return baseline() - 0.5 * heartbeat(phase) * traceBand.y / 1.4;
}

float segmentDistanceSquared(vec2 point, vec2 first, vec2 last) {
    vec2 segment = last - first;
    float projection = clamp(dot(point - first, segment) / max(dot(segment, segment), 0.00001), 0.0, 1.0);
    vec2 difference = first + projection * segment - point;
    return dot(difference, difference);
}

void main() {
    vec2 point = vec2(gl_FragCoord.x, resolution.y - gl_FragCoord.y);
    float left = pulse.x - (0.425 / 1.3) * pulse.y;
    float phase = (point.x - left) / pulse.y * 1.3;
    float headPhase = pulse.z * 1.3;
    float lineWidth = max(pixelRatio, min(CURVE_WIDTH * traceBand.y / 180.0, pulse.y * 0.10));
    float halfWidth = lineWidth * 0.5;
    float samplingRadius = halfWidth + pixelRatio * AA_FALLOFF;
    float closest = 100000.0;

    // The supplied curve-distance renderer, restricted to this event's quad.
    // Distances to subpixel segments keep steep QRS strokes continuous, even
    // when a full minute of events is compressed into a small monitor.
    float firstX = max(left, point.x - samplingRadius);
    float lastX = min(left + pulse.z * pulse.y, point.x + samplingRadius);
    if (lastX >= firstX) {
        vec2 previous = vec2(firstX, curveY((firstX - left) / pulse.y * 1.3));
        for (int index = 1; index <= 32; index++) {
            float x = mix(firstX, lastX, float(index) / 32.0);
            vec2 current = vec2(x, curveY((x - left) / pulse.y * 1.3));
            closest = min(closest, segmentDistanceSquared(point, previous, current));
            previous = current;
        }
    }
    float distance = sqrt(closest);
    float core = 1.0 - aaStep(0.0, samplingRadius, distance);
    float glow = exp(-distance / max(pixelRatio, lineWidth * 1.8)) * 0.22;
    float trail = 0.25 + 0.75 * pow(clamp(1.0 - (headPhase - phase) / 1.3, 0.0, 1.0), 3.0);
    float alpha = (core * core * core * 1.5 + glow) * trail * pulse.w;

    // A bright moving phosphor head appears only while a received event is
    // being traced. No clock-driven periodic pulses are generated in silence.
    if (pulse.z < 1.0) {
        vec2 center = vec2(left + pulse.z * pulse.y, curveY(headPhase));
        float head = 1.0 - aaStep(0.0, lineWidth * 1.8 + pixelRatio * AA_FALLOFF,
            length(point - center));
        alpha += head * head * pulse.w;
    }
    vec2 vignetteUv = max(vec2(0.00001), 1.0 - abs(point / resolution - 0.5) * 2.0);
    float v = 1.0 - 2.0 / (1.0 / vignetteUv.x + 1.0 / vignetteUv.y);
    alpha *= mix(1.0, 0.3, pow(v, 4.0));
    fragColor = vec4(0.0, 1.0, 0.7, min(1.0, alpha));
}`;

let shared;

function compile(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const message = gl.getShaderInfoLog(shader);
    gl.deleteShader(shader);
    throw new Error(`Heartbeat shader compilation failed: ${message}`);
  }
  return shader;
}

function program(gl, vertex, fragment) {
  const vertexObject = compile(gl, gl.VERTEX_SHADER, vertex);
  let fragmentObject;
  const result = gl.createProgram();
  try {
    fragmentObject = compile(gl, gl.FRAGMENT_SHADER, fragment);
    gl.attachShader(result, vertexObject);
    gl.attachShader(result, fragmentObject);
    gl.linkProgram(result);
    if (!gl.getProgramParameter(result, gl.LINK_STATUS)) {
      throw new Error(`Heartbeat shader link failed: ${gl.getProgramInfoLog(result)}`);
    }
    return result;
  } catch (error) {
    gl.deleteProgram(result);
    throw error;
  } finally {
    gl.deleteShader(vertexObject);
    if (fragmentObject) gl.deleteShader(fragmentObject);
  }
}

function initialise(renderer) {
  const gl = renderer.gl;
  renderer.background = program(gl, vertexShader, backgroundShader);
  renderer.pulse = program(gl, pulseVertexShader, pulseShader);
  renderer.vao = gl.createVertexArray();
  gl.bindVertexArray(renderer.vao);
  renderer.corners = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, renderer.corners);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  renderer.events = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, renderer.events);
  gl.bufferData(gl.ARRAY_BUFFER, MAX_POINTS * 16, gl.DYNAMIC_DRAW);
  gl.enableVertexAttribArray(1);
  gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 0, 0);
  gl.vertexAttribDivisor(1, 1);
  renderer.uniforms = new Map();
  for (const target of [renderer.background, renderer.pulse]) {
    renderer.uniforms.set(target, Object.fromEntries(['resolution', 'baseline', 'showTrace', 'traceBand', 'pixelRatio']
      .map(name => [name, gl.getUniformLocation(target, name)])));
  }
  renderer.error = null;
}

function sharedRenderer() {
  if (shared) return shared;
  const canvas = document.createElement('canvas');
  shared = {canvas, gl: null, error: null};
  try {
    shared.gl = canvas.getContext('webgl2', {alpha: false, antialias: false, depth: false,
      stencil: false, preserveDrawingBuffer: false, premultipliedAlpha: false, powerPreference: 'low-power'});
  } catch (error) {
    shared.error = `WebGL 2 initialization failed: ${error.message}. Showing a CPU-rendered trace.`;
    return shared;
  }
  const gl = shared.gl;
  shared.error = gl ? null : 'WebGL 2 is unavailable; showing a CPU-rendered trace.';
  if (!gl) return shared;
  canvas.addEventListener('webglcontextlost', event => {
    event.preventDefault();
    shared.error = 'Graphics context lost; showing a CPU-rendered trace.';
  });
  canvas.addEventListener('webglcontextrestored', () => {
    try { initialise(shared); } catch (error) { shared.error = error.message; }
  });
  try { initialise(shared); } catch (error) { shared.error = error.message; }
  return shared;
}

function renderGpu(ctx, options, pulses) {
  const renderer = sharedRenderer();
  if (!renderer.gl || renderer.error) return renderer.error;
  const {gl, canvas} = renderer;
  const {width, height, left, top, traceTop, traceHeight, pixelRatio, showTrace} = options;
  const scale = Math.min(pixelRatio, 4096 / width, 4096 / height,
    Math.sqrt(MAX_RENDER_PIXELS / (width * height)));
  const pixelWidth = Math.max(1, Math.round(width * scale));
  const pixelHeight = Math.max(1, Math.round(height * scale));
  if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
    canvas.width = pixelWidth;
    canvas.height = pixelHeight;
  }
  gl.viewport(0, 0, pixelWidth, pixelHeight);
  gl.bindVertexArray(renderer.vao);
  gl.disable(gl.BLEND);
  gl.useProgram(renderer.background);
  let uniforms = renderer.uniforms.get(renderer.background);
  gl.uniform2f(uniforms.resolution, pixelWidth, pixelHeight);
  gl.uniform1f(uniforms.baseline, (traceTop + traceHeight * 4 / 7) * scale);
  gl.uniform1i(uniforms.showTrace, showTrace);
  gl.drawArrays(gl.TRIANGLES, 0, 6);
  if (showTrace && pulses.length) {
    const instances = new Float32Array(pulses.length * 4);
    pulses.forEach((pulse, index) => instances.set([pulse.x * scale, pulse.width * scale, pulse.reveal, pulse.alpha], index * 4));
    gl.bindBuffer(gl.ARRAY_BUFFER, renderer.events);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, instances);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
    gl.useProgram(renderer.pulse);
    uniforms = renderer.uniforms.get(renderer.pulse);
    gl.uniform2f(uniforms.resolution, pixelWidth, pixelHeight);
    gl.uniform2f(uniforms.traceBand, traceTop * scale, traceHeight * scale);
    gl.uniform1f(uniforms.pixelRatio, scale);
    gl.drawArraysInstanced(gl.TRIANGLES, 0, 6, pulses.length);
  }
  ctx.drawImage(canvas, left, top, width, height);
  return null;
}

// The fallback uses the same wave landmarks and event/reveal positions. It is
// explicitly identified to the caller; it never invents periodic heartbeats.
function heartbeat(phase) {
  const gauss = (center, width) => Math.exp(-(((phase - center) / width) ** 2));
  const smooth = (a, b) => {
    const value = Math.max(0, Math.min(1, (phase - a) / (b - a)));
    return value * value * (3 - 2 * value);
  };
  let qrs = 0;
  if (phase >= .350 && phase < .390) qrs = -.065 * smooth(.350, .390);
  else if (phase >= .390 && phase < .425) qrs = -.065 + (1.20 + .065) * (phase - .390) / .035;
  else if (phase >= .425 && phase < .470) qrs = 1.20 + (-.30 - 1.20) * (phase - .425) / .045;
  else if (phase >= .470 && phase < .515) qrs = -.30 * (1 - smooth(.470, .515));
  return .15 * gauss(.18, .065) + qrs + .18 * gauss(.74, phase < .74 ? .115 : .085);
}

function renderFallback(ctx, options, pulses) {
  const {left, top, width, height, traceTop, traceHeight, showTrace} = options;
  ctx.save();
  ctx.beginPath(); ctx.rect(left, top, width, height); ctx.clip();
  const background = ctx.createRadialGradient(left + width / 2, top + height / 2, 0,
    left + width / 2, top + height / 2, Math.max(width, height) / 2);
  background.addColorStop(0, '#1a6633'); background.addColorStop(1, '#010803');
  ctx.fillStyle = background; ctx.fillRect(left, top, width, height);
  ctx.strokeStyle = '#33994d88'; ctx.lineWidth = .65;
  const pitch = width / 40;
  ctx.beginPath();
  for (let x = left; x <= left + width; x += pitch) { ctx.moveTo(x, top); ctx.lineTo(x, top + height); }
  for (let y = top + height / 2 % pitch; y <= top + height; y += pitch) { ctx.moveTo(left, y); ctx.lineTo(left + width, y); }
  ctx.stroke();
  ctx.fillStyle = '#000000e0'; ctx.fillRect(left, top, width, height);
  const baseline = top + traceTop + traceHeight * 4 / 7;
  if (showTrace) {
    ctx.strokeStyle = '#00ffb32e'; ctx.beginPath();
    ctx.moveTo(left, baseline); ctx.lineTo(left + width, baseline); ctx.stroke();
    ctx.strokeStyle = '#00ffb3'; ctx.lineJoin = 'round';
    for (const pulse of pulses) {
      const start = left + pulse.x - R_POSITION / DOMAIN * pulse.width;
      ctx.globalAlpha = pulse.alpha;
      ctx.lineWidth = Math.max(1, Math.min(CURVE_WIDTH_FALLBACK * traceHeight / 180, pulse.width * .1));
      ctx.shadowColor = '#00ffb3'; ctx.shadowBlur = 4;
      ctx.beginPath();
      for (let index = 0; index <= 128; index++) {
        const fraction = index / 128;
        if (fraction > pulse.reveal) break;
        const x = start + fraction * pulse.width;
        const y = baseline - .5 * heartbeat(fraction * DOMAIN) * traceHeight / 1.4;
        if (index) ctx.lineTo(x, y); else ctx.moveTo(x, y);
      }
      ctx.stroke();
    }
  }
  ctx.restore();
}
const CURVE_WIDTH_FALLBACK = 10;

export function createHeartbeatTrace() {
  const reveals = new Map();
  let first = true;
  return {
    draw(ctx, {width, height, left = 0, top = 0, traceTop = 0, traceHeight = height,
      points = [], now = Date.now(), windowMs = 60000, pixelRatio = 1,
      reducedMotion = false, live = true, showTrace = true} = {}) {
      if (!ctx || !(width > 0) || !(height > 0) || !(windowMs > 0)) return {renderer: 'none', error: null};
      const animationNow = performance.now();
      const visible = [...new Set(points)].filter(at => Number.isFinite(at) && at <= now && at >= now - windowMs).slice(-MAX_POINTS);
      for (const at of visible) if (!reveals.has(at)) {
        // Opening a view does not replay older history as freshly received data.
        reveals.set(at, first && now - at > 1000 ? -Infinity : animationNow);
      }
      first = false;
      const keep = new Set(visible);
      for (const at of reveals.keys()) if (!keep.has(at)) reveals.delete(at);
      const pulses = visible.map(at => ({
        x: (at - (now - windowMs)) / windowMs * width,
        width: width * PULSE_DURATION_MS / windowMs,
        reveal: reducedMotion || !live ? 1 : Math.min(1, (animationNow - reveals.get(at)) / REVEAL_DURATION_MS),
        alpha: Math.pow(Math.max(0, 1 - (now - at) / windowMs), 1.4),
      }));
      const options = {width, height, left, top, traceTop, traceHeight,
        pixelRatio: Math.max(.5, Math.min(2, pixelRatio)), showTrace};
      let error;
      try { error = renderGpu(ctx, options, pulses); }
      catch (failure) { error = failure.message; }
      if (error) {
        renderFallback(ctx, options, pulses);
        return {renderer: 'canvas2d', error};
      }
      return {renderer: 'webgl2', error: null};
    },
    reset() { reveals.clear(); first = true; },
    destroy() { reveals.clear(); first = true; },
  };
}
