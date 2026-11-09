import type { SceneSpec, Shape } from "./scene";
import type { RendererKind, RendererPreference } from "./model";
import { project } from "./camera";
import type * as THREE from "three";

export interface Painter {
  kind: RendererKind;
  canvas: HTMLCanvasElement;
  update: (scene: SceneSpec) => void;
  draw: (matrix: Float32Array, w: number, h: number, dpr: number) => void;
  dispose: () => void;
}

type RGB = [number, number, number];
const colourCache = new Map<string, RGB>();
const srgbToLinear = (c: number) =>
  c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
const byteLinear = Array.from({ length: 256 }, (_, n) => srgbToLinear(n / 255));
function colour(hex: string, linear: boolean): RGB {
  const key = `${linear ? "linear:" : "srgb:"}${hex}`;
  const cached = colourCache.get(key);
  if (cached) return cached;
  const h = hex.replace("#", "");
  const full = h.length === 3 ? [...h].map((c) => c + c).join("") : h;
  const bytes = [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16));
  const result = bytes.map((v) => (linear ? byteLinear[v] : v / 255)) as RGB;
  colourCache.set(key, result);
  return result;
}
const capacity = (count: number, minimum = 4) =>
  2 ** Math.ceil(Math.log2(Math.max(minimum, count)));

/** One GPU batch per primitive/opacity combination, independent of sheet colours. */
export function batchShapes(shapes: Shape[], linearColours = false): Shape[] {
  const batches = new Map<string, Shape>();
  for (const shape of shapes) {
    if (shape.positions.length < 3) continue;
    const isLine = shape.kind === "lines";
    const opacity = shape.opacity ?? 1;
    const depthWrite = !isLine && opacity > 0.9;
    const key = `${isLine ? "lines" : "triangles"}:${opacity}:${depthWrite ? 1 : 0}:${isLine ? (shape.width ?? 1) : 0}`;
    let batch = batches.get(key);
    if (!batch) {
      batch = {
        kind: isLine ? "lines" : "triangles",
        color: "#ffffff",
        opacity,
        width: shape.width,
        positions: [],
        colors: [],
        indices: isLine ? undefined : [],
      };
      batches.set(key, batch);
    }
    const baseColour = colour(shape.color, linearColours);
    const vertexColour = (index: number): RGB => {
      if (!shape.colors || shape.colors.length < index * 3 + 3)
        return baseColour;
      const rgb = [
        shape.colors[index * 3],
        shape.colors[index * 3 + 1],
        shape.colors[index * 3 + 2],
      ] as RGB;
      return linearColours ? (rgb.map(srgbToLinear) as RGB) : rgb;
    };
    const addVertex = (x: number, y: number, z: number, c: RGB) => {
      batch!.positions.push(x, y, z);
      batch!.colors!.push(c[0], c[1], c[2]);
    };
    if (isLine && shape.dashed) {
      // Explicit dashes merge with solid lines and behave identically in WebGL 1 and 2.
      for (let i = 0; i + 5 < shape.positions.length; i += 6) {
        const p = shape.positions;
        const dx = p[i + 3] - p[i],
          dy = p[i + 4] - p[i + 1],
          dz = p[i + 5] - p[i + 2];
        const length = Math.hypot(dx, dy, dz);
        if (!length) continue;
        const n = Math.max(1, Math.ceil(length / 0.18));
        const aColour = vertexColour(i / 3),
          bColour = vertexColour(i / 3 + 1);
        for (let j = 0; j < n; j++) {
          for (const t of [j / n, Math.min(1, (j + 0.55) / n)]) {
            const c: RGB = [
              aColour[0] + (bColour[0] - aColour[0]) * t,
              aColour[1] + (bColour[1] - aColour[1]) * t,
              aColour[2] + (bColour[2] - aColour[2]) * t,
            ];
            addVertex(p[i] + dx * t, p[i + 1] + dy * t, p[i + 2] + dz * t, c);
          }
        }
      }
    } else {
      const offset = batch.positions.length / 3;
      for (let i = 0; i + 2 < shape.positions.length; i += 3) {
        addVertex(
          shape.positions[i],
          shape.positions[i + 1],
          shape.positions[i + 2],
          vertexColour(i / 3),
        );
      }
      if (batch.indices) {
        if (shape.indices)
          for (const index of shape.indices) batch.indices.push(index + offset);
        else
          for (let index = 0; index < shape.positions.length / 3; index++)
            batch.indices.push(index + offset);
      }
    }
  }
  return [...batches.values()];
}

function makeThreePainter(T: typeof THREE, canvas: HTMLCanvasElement): Painter {
  const renderer = new T.WebGLRenderer({
    canvas,
    antialias: true,
    alpha: true,
    powerPreference: "high-performance",
  });
  try {
    renderer.setClearColor(0x10191d, 0);
    renderer.outputColorSpace = T.SRGBColorSpace;
    const scene = new T.Scene();
    const camera = new T.Camera();
    camera.matrixAutoUpdate = false;
    camera.matrixWorld.identity();
    camera.matrixWorldInverse.identity();
    type Entry = {
      geometry: THREE.BufferGeometry;
      material: THREE.MeshBasicMaterial | THREE.LineBasicMaterial;
      object: THREE.Mesh | THREE.LineSegments;
    };
    let entries: Entry[] = [];
    let oldWidth = 0,
      oldHeight = 0,
      oldDpr = 0,
      disposed = false;
    const disposeEntry = (entry: Entry) => {
      scene.remove(entry.object);
      entry.geometry.dispose();
      entry.material.dispose();
    };
    return {
      kind: "three",
      canvas,
      update(spec) {
        if (disposed) return;
        const batches = batchShapes(spec.shapes, true);
        for (let i = batches.length; i < entries.length; i++)
          disposeEntry(entries[i]);
        entries.length = Math.min(entries.length, batches.length);
        batches.forEach((batch, i) => {
          let entry = entries[i];
          const isLine = batch.kind === "lines";
          if (entry && entry.object instanceof T.LineSegments !== isLine) {
            disposeEntry(entry);
            entry = undefined as unknown as Entry;
          }
          if (!entry) {
            const geometry = new T.BufferGeometry();
            const options = {
              color: 0xffffff,
              vertexColors: true,
              transparent: true,
              opacity: batch.opacity ?? 1,
              toneMapped: false,
            };
            const material = isLine
              ? new T.LineBasicMaterial(options)
              : new T.MeshBasicMaterial({ ...options, side: T.DoubleSide });
            const object = isLine
              ? new T.LineSegments(geometry, material)
              : new T.Mesh(geometry, material);
            object.frustumCulled = false;
            object.renderOrder = isLine ? 2 : 0;
            scene.add(object);
            entry = { geometry, material, object };
            entries[i] = entry;
          }
          entry.material.opacity = batch.opacity ?? 1;
          entry.material.depthWrite = (batch.opacity ?? 1) > 0.9 && !isLine;
          if (isLine)
            (entry.material as THREE.LineBasicMaterial).linewidth =
              batch.width ?? 1;
          const vertexCount = batch.positions.length / 3;
          const previousPosition = entry.geometry.getAttribute("position") as
            | THREE.BufferAttribute
            | undefined;
          const previousColour = entry.geometry.getAttribute("color") as
            | THREE.BufferAttribute
            | undefined;
          const previousIndex = entry.geometry.index;
          const needsIndex = !!batch.indices;
          const realloc =
            !previousPosition ||
            !previousColour ||
            previousPosition.count < vertexCount ||
            previousColour.count < vertexCount ||
            !!previousIndex !== needsIndex ||
            (needsIndex &&
              (!previousIndex || previousIndex.count < batch.indices!.length));
          if (realloc) {
            // Replacing a whole geometry frees former GL buffers instead of orphaning them.
            const geometry = new T.BufferGeometry();
            const vertices = capacity(
              Math.max(vertexCount, previousPosition?.count ?? 0),
            );
            geometry.setAttribute(
              "position",
              new T.BufferAttribute(new Float32Array(vertices * 3), 3).setUsage(
                T.DynamicDrawUsage,
              ),
            );
            geometry.setAttribute(
              "color",
              new T.BufferAttribute(new Float32Array(vertices * 3), 3).setUsage(
                T.DynamicDrawUsage,
              ),
            );
            if (needsIndex)
              geometry.setIndex(
                new T.BufferAttribute(
                  new Uint32Array(
                    capacity(
                      Math.max(
                        batch.indices!.length,
                        previousIndex?.count ?? 0,
                      ),
                      6,
                    ),
                  ),
                  1,
                ).setUsage(T.DynamicDrawUsage),
              );
            const previousGeometry = entry.geometry;
            entry.geometry = geometry;
            entry.object.geometry = geometry;
            previousGeometry.dispose();
          }
          const position = entry.geometry.getAttribute(
            "position",
          ) as THREE.BufferAttribute;
          const colours = entry.geometry.getAttribute(
            "color",
          ) as THREE.BufferAttribute;
          (position.array as Float32Array).set(batch.positions);
          (colours.array as Float32Array).set(batch.colors!);
          position.clearUpdateRanges();
          position.addUpdateRange(0, batch.positions.length);
          position.needsUpdate = true;
          colours.clearUpdateRanges();
          colours.addUpdateRange(0, batch.colors!.length);
          colours.needsUpdate = true;
          if (batch.indices) {
            const indices = entry.geometry.index!;
            indices.array.set(batch.indices);
            indices.clearUpdateRanges();
            indices.addUpdateRange(0, batch.indices.length);
            indices.needsUpdate = true;
            entry.geometry.setDrawRange(0, batch.indices.length);
          } else entry.geometry.setDrawRange(0, vertexCount);
        });
      },
      draw(matrix, w, h, dpr) {
        if (disposed) return;
        if (w !== oldWidth || h !== oldHeight || dpr !== oldDpr) {
          renderer.setPixelRatio(dpr);
          renderer.setSize(w, h, false);
          oldWidth = w;
          oldHeight = h;
          oldDpr = dpr;
        }
        camera.projectionMatrix.fromArray(matrix);
        camera.projectionMatrixInverse.copy(camera.projectionMatrix).invert();
        renderer.render(scene, camera);
      },
      dispose() {
        if (disposed) return;
        disposed = true;
        for (const entry of entries) disposeEntry(entry);
        entries = [];
        renderer.dispose();
        renderer.forceContextLoss();
      },
    };
  } catch (error) {
    renderer.dispose();
    renderer.forceContextLoss();
    throw error;
  }
}

function makeRawPainter(canvas: HTMLCanvasElement): Painter {
  const context = canvas.getContext("webgl", {
    alpha: true,
    antialias: true,
    powerPreference: "high-performance",
  });
  if (!context) throw new Error("WebGL ist nicht verfügbar.");
  const gl = context;
  const shaders: WebGLShader[] = [];
  let program: WebGLProgram | null = null;
  type RawEntry = {
    buffer: WebGLBuffer;
    vertices: Float32Array;
    capacityBytes: number;
    shape: Shape;
    count: number;
  };
  let buffers: RawEntry[] = [];
  let disposed = false;
  const cleanup = () => {
    if (disposed) return;
    disposed = true;
    for (const entry of buffers) gl.deleteBuffer(entry.buffer);
    buffers = [];
    if (program) gl.deleteProgram(program);
    for (const shader of shaders) gl.deleteShader(shader);
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  };
  try {
    const shader = (type: number, source: string) => {
      const result = gl.createShader(type);
      if (!result)
        throw new Error("WebGL-Shader konnte nicht angelegt werden.");
      shaders.push(result);
      gl.shaderSource(result, source);
      gl.compileShader(result);
      if (!gl.getShaderParameter(result, gl.COMPILE_STATUS))
        throw new Error(
          gl.getShaderInfoLog(result) ??
            "Shader konnte nicht kompiliert werden.",
        );
      return result;
    };
    const vert = shader(
      gl.VERTEX_SHADER,
      "attribute vec3 aPosition; attribute vec3 aColor; uniform mat4 uMatrix; varying vec3 vColor; void main(){ vColor=aColor; gl_Position=uMatrix*vec4(aPosition,1.0); }",
    );
    const frag = shader(
      gl.FRAGMENT_SHADER,
      "precision mediump float; varying vec3 vColor; uniform float uOpacity; void main(){ gl_FragColor=vec4(vColor,uOpacity); }",
    );
    program = gl.createProgram();
    if (!program)
      throw new Error("WebGL-Programm konnte nicht angelegt werden.");
    gl.attachShader(program, vert);
    gl.attachShader(program, frag);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(
        gl.getProgramInfoLog(program) ??
          "WebGL-Programm konnte nicht geladen werden.",
      );
    const pos = gl.getAttribLocation(program, "aPosition"),
      col = gl.getAttribLocation(program, "aColor");
    const mat = gl.getUniformLocation(program, "uMatrix"),
      opacity = gl.getUniformLocation(program, "uOpacity");
    const lineWidthRange = gl.getParameter(
      gl.ALIASED_LINE_WIDTH_RANGE,
    ) as Float32Array;
    return {
      kind: "webgl",
      canvas,
      update(spec) {
        if (disposed) return;
        const batches = batchShapes(spec.shapes);
        for (let i = batches.length; i < buffers.length; i++)
          gl.deleteBuffer(buffers[i].buffer);
        buffers.length = Math.min(buffers.length, batches.length);
        batches.forEach((shape, i) => {
          let entry = buffers[i];
          if (!entry) {
            const buffer = gl.createBuffer();
            if (!buffer)
              throw new Error("WebGL-Puffer konnte nicht angelegt werden.");
            entry = {
              buffer,
              vertices: new Float32Array(0),
              capacityBytes: 0,
              shape,
              count: 0,
            };
            buffers[i] = entry;
          }
          const count = shape.indices
            ? shape.indices.length
            : shape.positions.length / 3;
          const floats = count * 6;
          if (entry.vertices.length < floats)
            entry.vertices = new Float32Array(capacity(count) * 6);
          // drawArrays avoids Uint32 index extensions on WebGL 1 and still uses one draw.
          for (let vertex = 0; vertex < count; vertex++) {
            const source = (shape.indices ? shape.indices[vertex] : vertex) * 3;
            const dest = vertex * 6;
            entry.vertices[dest] = shape.positions[source];
            entry.vertices[dest + 1] = shape.positions[source + 1];
            entry.vertices[dest + 2] = shape.positions[source + 2];
            entry.vertices[dest + 3] = shape.colors![source];
            entry.vertices[dest + 4] = shape.colors![source + 1];
            entry.vertices[dest + 5] = shape.colors![source + 2];
          }
          gl.bindBuffer(gl.ARRAY_BUFFER, entry.buffer);
          if (entry.capacityBytes < entry.vertices.byteLength) {
            gl.bufferData(
              gl.ARRAY_BUFFER,
              entry.vertices.byteLength,
              gl.DYNAMIC_DRAW,
            );
            entry.capacityBytes = entry.vertices.byteLength;
          }
          gl.bufferSubData(
            gl.ARRAY_BUFFER,
            0,
            entry.vertices.subarray(0, floats),
          );
          entry.count = count;
          entry.shape = shape;
        });
      },
      draw(matrix, w, h, dpr) {
        if (disposed) return;
        const dw = Math.round(w * dpr),
          dh = Math.round(h * dpr);
        if (canvas.width !== dw || canvas.height !== dh) {
          canvas.width = dw;
          canvas.height = dh;
        }
        gl.viewport(0, 0, dw, dh);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        gl.enable(gl.BLEND);
        gl.blendFuncSeparate(
          gl.SRC_ALPHA,
          gl.ONE_MINUS_SRC_ALPHA,
          gl.ONE,
          gl.ONE_MINUS_SRC_ALPHA,
        );
        gl.enable(gl.DEPTH_TEST);
        gl.depthFunc(gl.LEQUAL);
        gl.useProgram(program);
        gl.uniformMatrix4fv(mat, false, matrix);
        gl.enableVertexAttribArray(pos);
        gl.enableVertexAttribArray(col);
        for (const isLine of [false, true])
          for (const entry of buffers) {
            if ((entry.shape.kind === "lines") !== isLine) continue;
            gl.uniform1f(opacity, entry.shape.opacity ?? 1);
            gl.depthMask((entry.shape.opacity ?? 1) > 0.9 && !isLine);
            if (isLine)
              gl.lineWidth(
                Math.max(
                  lineWidthRange[0],
                  Math.min(lineWidthRange[1], entry.shape.width ?? 1),
                ),
              );
            gl.bindBuffer(gl.ARRAY_BUFFER, entry.buffer);
            gl.vertexAttribPointer(pos, 3, gl.FLOAT, false, 24, 0);
            gl.vertexAttribPointer(col, 3, gl.FLOAT, false, 24, 12);
            gl.drawArrays(isLine ? gl.LINES : gl.TRIANGLES, 0, entry.count);
          }
        gl.depthMask(true);
      },
      dispose: cleanup,
    };
  } catch (error) {
    cleanup();
    throw error;
  }
}

function makeCanvasPainter(canvas: HTMLCanvasElement): Painter {
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas 2D konnte nicht gestartet werden.");
  let shapes: Shape[] = [];
  type Primitive = {
    shapeIndex: number;
    indices: number[];
    vertices: number[];
    center: [number, number, number];
    line: boolean;
  };
  let primitives: Primitive[] = [];
  let parallelXYPlanes = false;

  const flatXY = (shape: Shape) => {
    const z = shape.positions[2];
    for (let i = 5; i < shape.positions.length; i += 3)
      if (Math.abs(shape.positions[i] - z) > 1e-7) return false;
    return true;
  };
  function prepare(spec: SceneSpec) {
    shapes = spec.shapes;
    primitives = [];
    const fills = shapes.filter(
      (shape) => shape.kind !== "lines" && shape.positions.length >= 3,
    );
    // Dense distribution areas are parallel XY planes: sort whole planes by Z.
    // This is exact for their occlusion and avoids sorting thousands of chart triangles.
    parallelXYPlanes = fills.length > 0 && fills.every(flatXY);
    const add = (shapeIndex: number, indices: number[], isLine: boolean) => {
      const p = shapes[shapeIndex].positions;
      const center: [number, number, number] = [0, 0, 0];
      const vertices = parallelXYPlanes ? indices : [...new Set(indices)];
      for (const index of vertices) {
        center[0] += p[index * 3];
        center[1] += p[index * 3 + 1];
        center[2] += p[index * 3 + 2];
      }
      center[0] /= vertices.length;
      center[1] /= vertices.length;
      center[2] /= vertices.length;
      primitives.push({ shapeIndex, indices, vertices, center, line: isLine });
    };
    shapes.forEach((shape, shapeIndex) => {
      if (shape.kind === "lines") {
        for (let i = 0; i + 5 < shape.positions.length; i += 6)
          add(shapeIndex, [i / 3, i / 3 + 1], true);
        return;
      }
      const indices =
        shape.indices ??
        Array.from({ length: shape.positions.length / 3 }, (_, i) => i);
      if (indices.length < 3) return;
      if (parallelXYPlanes) {
        add(shapeIndex, indices, false);
        return;
      }
      for (let i = 0; i + 2 < indices.length; i += 3) {
        let count = 3;
        // Keep two adjacent coplanar triangles as one face path, avoiding a diagonal seam.
        if (i + 5 < indices.length) {
          const first = indices.slice(i, i + 3),
            second = indices.slice(i + 3, i + 6);
          const shared = second.filter((index) => first.includes(index));
          const fourth = second.find((index) => !first.includes(index));
          if (shared.length === 2 && fourth !== undefined) {
            const p = shape.positions,
              a = first[0] * 3,
              b = first[1] * 3,
              c = first[2] * 3,
              d = fourth * 3;
            const ux = p[b] - p[a],
              uy = p[b + 1] - p[a + 1],
              uz = p[b + 2] - p[a + 2];
            const vx = p[c] - p[a],
              vy = p[c + 1] - p[a + 1],
              vz = p[c + 2] - p[a + 2];
            const nx = uy * vz - uz * vy,
              ny = uz * vx - ux * vz,
              nz = ux * vy - uy * vx;
            const distance =
              nx * (p[d] - p[a]) +
              ny * (p[d + 1] - p[a + 1]) +
              nz * (p[d + 2] - p[a + 2]);
            if (
              Math.abs(distance) <=
              1e-7 * Math.max(1, Math.hypot(nx, ny, nz))
            )
              count = 6;
          }
        }
        add(shapeIndex, indices.slice(i, i + count), false);
        if (count === 6) i += 3;
      }
    });
  }
  return {
    kind: "canvas",
    canvas,
    update: prepare,
    draw(m, w, h, dpr) {
      if (
        canvas.width !== Math.round(w * dpr) ||
        canvas.height !== Math.round(h * dpr)
      ) {
        canvas.width = Math.round(w * dpr);
        canvas.height = Math.round(h * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const projected = shapes.map((shape) => {
        const points: ReturnType<typeof project>[] = [];
        for (let i = 0; i < shape.positions.length; i += 3)
          points.push(
            project(
              [
                shape.positions[i],
                shape.positions[i + 1],
                shape.positions[i + 2],
              ],
              m,
              w,
              h,
            ),
          );
        return points;
      });
      const ordered = primitives
        .map((primitive) => {
          const p = primitive.center;
          const depth = parallelXYPlanes
            ? m[10] * p[2]
            : m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14];
          return { primitive, depth: depth - (primitive.line ? 1e-7 : 0) };
        })
        .sort((a, b) => b.depth - a.depth);
      let fill = "",
        stroke = "",
        alpha = -1,
        dash: boolean | null = null,
        width = -1;
      type FaceBounds = {
        primitive: Primitive;
        left: number;
        right: number;
        top: number;
        bottom: number;
      };
      let pending: FaceBounds[] = [],
        pendingColor = "",
        pendingOpacity = 1;
      const flushFill = () => {
        if (!pending.length) return;
        if (fill !== pendingColor) {
          ctx.fillStyle = pendingColor;
          fill = pendingColor;
        }
        if (alpha !== pendingOpacity) {
          ctx.globalAlpha = pendingOpacity;
          alpha = pendingOpacity;
        }
        ctx.fill();
        pending = [];
      };
      const compatible = (a: FaceBounds, b: FaceBounds) => {
        if (
          a.right <= b.left + 0.001 ||
          b.right <= a.left + 0.001 ||
          a.bottom <= b.top + 0.001 ||
          b.bottom <= a.top + 0.001
        )
          return true;
        if (a.primitive.shapeIndex !== b.primitive.shapeIndex) return false;
        const shared = a.primitive.vertices.filter((index) =>
          b.primitive.vertices.includes(index),
        );
        if (shared.length !== 2) return false;
        const pts = projected[a.primitive.shapeIndex],
          p = pts[shared[0]],
          q = pts[shared[1]];
        const side = (index: number) =>
          (q.x - p.x) * (pts[index].y - p.y) -
          (q.y - p.y) * (pts[index].x - p.x);
        const first = a.primitive.vertices
          .filter((index) => !shared.includes(index))
          .map(side);
        const second = b.primitive.vertices
          .filter((index) => !shared.includes(index))
          .map(side);
        // Adjacent ribbon faces may share a boundary, but their interiors must not overlap.
        return (
          (first.every((v) => v >= -0.001) &&
            second.every((v) => v <= 0.001)) ||
          (first.every((v) => v <= 0.001) && second.every((v) => v >= -0.001))
        );
      };
      for (const { primitive } of ordered) {
        const shape = shapes[primitive.shapeIndex],
          points = projected[primitive.shapeIndex];
        const opacity = shape.opacity ?? 1;
        if (primitive.line) {
          flushFill();
          if (alpha !== opacity) {
            ctx.globalAlpha = opacity;
            alpha = opacity;
          }
          ctx.beginPath();
          if (stroke !== shape.color) {
            ctx.strokeStyle = shape.color;
            stroke = shape.color;
          }
          if (width !== (shape.width ?? 0.8)) {
            ctx.lineWidth = shape.width ?? 0.8;
            width = shape.width ?? 0.8;
          }
          if (dash !== !!shape.dashed) {
            ctx.setLineDash(shape.dashed ? [4, 3] : []);
            dash = !!shape.dashed;
          }
          const a = points[primitive.indices[0]],
            b = points[primitive.indices[1]];
          ctx.moveTo(a.x, a.y);
          ctx.lineTo(b.x, b.y);
          ctx.stroke();
        } else {
          const bounds: FaceBounds = {
            primitive,
            left: Infinity,
            right: -Infinity,
            top: Infinity,
            bottom: -Infinity,
          };
          if (!parallelXYPlanes)
            for (const index of primitive.vertices) {
              const point = points[index];
              bounds.left = Math.min(bounds.left, point.x);
              bounds.right = Math.max(bounds.right, point.x);
              bounds.top = Math.min(bounds.top, point.y);
              bounds.bottom = Math.max(bounds.bottom, point.y);
            }
          if (
            pending.length &&
            (parallelXYPlanes ||
              pending.length >= 32 ||
              pendingColor !== shape.color ||
              pendingOpacity !== opacity ||
              !pending.every((previous) => compatible(previous, bounds)))
          )
            flushFill();
          if (!pending.length) {
            ctx.beginPath();
            pendingColor = shape.color;
            pendingOpacity = opacity;
          }
          for (let i = 0; i + 2 < primitive.indices.length; i += 3) {
            const a = points[primitive.indices[i]],
              b = points[primitive.indices[i + 1]],
              c = points[primitive.indices[i + 2]];
            ctx.moveTo(a.x, a.y);
            ctx.lineTo(b.x, b.y);
            ctx.lineTo(c.x, c.y);
            ctx.closePath();
          }
          pending.push(bounds);
        }
      }
      flushFill();
      ctx.globalAlpha = 1;
    },
    dispose() {
      shapes = [];
      primitives = [];
    },
  };
}

export async function createPainter(
  host: HTMLElement,
  preference: RendererPreference,
): Promise<{ painter: Painter; reason: string | null }> {
  let reason: string | null = null;
  const canvas = () => {
    const c = document.createElement("canvas");
    c.setAttribute("aria-hidden", "true");
    c.className = "scene-canvas";
    return c;
  };
  if (preference === "auto" || preference === "three") {
    try {
      const T = await import("three");
      const c = canvas();
      const painter = makeThreePainter(T, c);
      host.replaceChildren(c);
      return { painter, reason };
    } catch {
      reason = "Three.js/WebGL 2 nicht verfügbar";
    }
  }
  if (preference !== "canvas") {
    try {
      const c = canvas();
      const painter = makeRawPainter(c);
      host.replaceChildren(c);
      return { painter, reason };
    } catch {
      reason = (reason ? reason + " · " : "") + "WebGL nicht verfügbar";
    }
  }
  const c = canvas();
  const painter = makeCanvasPainter(c);
  host.replaceChildren(c);
  return { painter, reason };
}
