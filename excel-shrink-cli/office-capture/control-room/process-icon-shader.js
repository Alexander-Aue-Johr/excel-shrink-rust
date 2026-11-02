import { iconActivity } from './targeted-model.js';

// Every icon shares this one small WebGL context. Cards never allocate their
// own contexts, and offscreen/hidden icons do not participate in animation.
const vertexSource = `#version 300 es
layout(location = 0) in vec2 corner;
out vec2 uv;
void main() {
  uv = corner;
  gl_Position = vec4(corner, 0.0, 1.0);
}`;

const fragmentSource = `#version 300 es
precision highp float;
in vec2 uv;
uniform float intensity;
uniform int symbol;
out vec4 color;

float line(vec2 p, vec2 a, vec2 b) {
  vec2 delta = b - a;
  return length(p - a - delta * clamp(dot(p - a, delta) / dot(delta, delta), 0.0, 1.0));
}
float box(vec2 p, vec2 halfSize) {
  vec2 d = abs(p) - halfSize;
  return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}
float excel(vec2 p) {
  float d = abs(box(p - vec2(0.13, 0.0), vec2(0.43, 0.53))) - 0.014;
  for (int row = 0; row < 3; row++) {
    float y = -0.26 + float(row) * 0.26;
    d = min(d, line(p, vec2(0.1, y), vec2(0.48, y)) - 0.012);
  }
  d = min(d, line(p, vec2(0.28, -0.4), vec2(0.28, 0.4)) - 0.012);
  // The workbook's front badge and X remain legible at small LED sizes.
  float badge = abs(box(p - vec2(-0.24, -0.02), vec2(0.32, 0.36))) - 0.018;
  if (box(p - vec2(-0.24, -0.02), vec2(0.33, 0.37)) < 0.0) d = badge;
  d = min(d, line(p, vec2(-0.4, -0.24), vec2(-0.08, 0.2)) - 0.026);
  return min(d, line(p, vec2(-0.4, 0.2), vec2(-0.08, -0.24)) - 0.026);
}
float xml(vec2 p) {
  float d = abs(box(p, vec2(0.5, 0.56))) - 0.014;
  d = min(d, line(p, vec2(-0.18, 0.18), vec2(-0.36, 0.0)) - 0.022);
  d = min(d, line(p, vec2(-0.36, 0.0), vec2(-0.18, -0.18)) - 0.022);
  d = min(d, line(p, vec2(0.18, 0.18), vec2(0.36, 0.0)) - 0.022);
  d = min(d, line(p, vec2(0.36, 0.0), vec2(0.18, -0.18)) - 0.022);
  return min(d, line(p, vec2(-0.07, -0.23), vec2(0.07, 0.23)) - 0.023);
}
void main() {
  float shape = symbol == 0 ? excel(uv) : xml(uv);
  if (symbol == 2) {
    shape = abs(box(uv, vec2(0.39))) - 0.018;
    for (int pin = 0; pin < 3; pin++) {
      float offset = -0.25 + float(pin) * 0.25;
      shape = min(shape, line(uv, vec2(offset, 0.43), vec2(offset, 0.67)) - 0.02);
      shape = min(shape, line(uv, vec2(offset, -0.43), vec2(offset, -0.67)) - 0.02);
      shape = min(shape, line(uv, vec2(0.43, offset), vec2(0.67, offset)) - 0.02);
      shape = min(shape, line(uv, vec2(-0.43, offset), vec2(-0.67, offset)) - 0.02);
    }
    // A stationary gear identifies the process without suggesting useful work.
    float radius = length(uv);
    float teeth = 0.21 + 0.035 * cos(atan(uv.y, uv.x) * 8.0);
    shape = min(shape, abs(radius - teeth) - 0.02);
    shape = min(shape, abs(radius - 0.075) - 0.018);
  }
  float distance = max(0.0, shape);
  float core = 1.0 - smoothstep(0.0, 0.023, distance);
  float glow = exp(-distance * 15.0) * 0.6 + exp(-distance * 4.5) * 0.28;
  vec3 neon = symbol == 0 ? vec3(0.12, 1.0, 0.48) : vec3(0.73, 0.34, 1.0);
  if (symbol == 2) neon = vec3(0.15, 0.8, 1.0);
  float alpha = clamp(core * 0.12 + intensity * (core * 0.88 + glow), 0.0, 1.0);
  // Straight RGB is intentional: Canvas compositing applies alpha once. Fading
  // both RGB and alpha would square the falloff and hide the outer neon halo.
  color = vec4(neon + vec3(0.18) * core * intensity, alpha);
}`;

let renderer;
let rendererError;
const icons = new Set();
let frame;
let lastFrame = 0;
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
const observer = new IntersectionObserver(entries => {
  for (const entry of entries) {
    const icon = [...icons].find(icon => icon.canvas === entry.target);
    if (icon) { icon.visible = entry.isIntersecting; icon.draw(); }
  }
  schedule();
});

/** Compile the shared signed-distance symbol and neon falloff shader once. */
function sharedRenderer() {
  if (rendererError) throw rendererError;
  if (renderer) return renderer;
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 128;
  const gl = canvas.getContext('webgl2', { alpha: true, antialias: false, premultipliedAlpha: false });
  if (!gl) throw new Error('WebGL2 is unavailable');
  const program = gl.createProgram();
  for (const [type, source] of [[gl.VERTEX_SHADER, vertexSource], [gl.FRAGMENT_SHADER, fragmentSource]]) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
    gl.attachShader(program, shader);
    gl.deleteShader(shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
  gl.useProgram(program);
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  renderer = { canvas, gl, intensity: gl.getUniformLocation(program, 'intensity'), symbol: gl.getUniformLocation(program, 'symbol') };
  canvas.addEventListener('webglcontextlost', event => { event.preventDefault(); renderer = null; });
  return renderer;
}

function schedule() {
  if (frame || document.hidden || ![...icons].some(icon => icon.visible && icon.animating)) return;
  frame = requestAnimationFrame(time => {
    frame = null;
    if (time - lastFrame >= 1000 / 30) {
      lastFrame = time;
      for (const icon of icons) if (icon.visible && icon.animating) icon.draw();
    }
    schedule();
  });
}
document.addEventListener('visibilitychange', () => {
  if (!document.hidden) { for (const icon of icons) if (icon.visible) icon.draw(); schedule(); }
});

/** A stable canvas follows actual lifecycle/XML timestamps without restarting fades. */
export function createProcessIcon(kind) {
  const canvas = document.createElement('canvas');
  canvas.className = 'targeted-icon';
  canvas.dataset.kind = kind;
  canvas.width = canvas.height = 128;
  canvas.setAttribute('role', 'img');
  const ctx = canvas.getContext('2d');
  let lane = {};
  const icon = {
    canvas, visible: false, animating: false,
    update(next) {
      lane = next;
      const label = kind === 'process' ? 'Windows process' : kind === 'excel' ? 'Excel' : 'XML';
      canvas.setAttribute('aria-label', `${label} · ${lane.lifecycle}`);
      icon.draw();
      schedule();
    },
    draw() {
      const activity = iconActivity(lane, kind);
      icon.animating = !reducedMotion.matches && (activity.pulse || activity.level > 0.004 && activity.level < 1);
      if (!icon.visible || document.hidden || !canvas.isConnected) return;
      const intensity = activity.level * (activity.pulse && !reducedMotion.matches
        ? 0.72 + 0.28 * Math.sin(performance.now() / 230) : 1);
      ctx.clearRect(0, 0, 128, 128);
      try {
        const { gl, canvas: source, intensity: uniform, symbol } = sharedRenderer();
        gl.uniform1f(uniform, intensity);
        gl.uniform1i(symbol, kind === 'process' ? 2 : kind === 'excel' ? 0 : 1);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        ctx.drawImage(source, 0, 0);
        canvas.dataset.renderer = 'webgl2';
      } catch (error) {
        rendererError ||= error;
        canvas.dataset.renderer = 'fallback';
        canvas.title = `Shader unavailable: ${error.message}`;
        ctx.fillStyle = kind === 'process' ? '#26ccff' : kind === 'excel' ? '#42ffa0' : '#bf74ff';
        ctx.globalAlpha = 0.15 + intensity * 0.85;
        ctx.font = 'bold 44px system-ui';
        ctx.textAlign = 'center';
        ctx.fillText(kind === 'process' ? '⚙' : kind === 'excel' ? 'X' : '</>', 64, 78);
        ctx.globalAlpha = 1;
      }
    },
    destroy() { observer.unobserve(canvas); icons.delete(icon); canvas.remove(); },
  };
  icons.add(icon);
  observer.observe(canvas);
  return icon;
}
