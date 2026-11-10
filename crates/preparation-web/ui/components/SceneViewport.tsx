"use client";
import { useEffect, useRef, useState } from "react";
import { Expand, RotateCcw, Minus, Plus, Move3D } from "lucide-react";
import type { HitTarget, SceneSpec } from "@/lib/scene";
import type { RendererKind, RendererPreference } from "@/lib/model";
import {
  cameraMatrix,
  defaultView,
  project,
  surfaceLabelMatrix,
  type CameraView,
} from "@/lib/camera";
import { createPainter, type Painter } from "@/lib/renderer";

interface Props {
  scene: SceneSpec;
  renderer: RendererPreference;
  title: string;
  className?: string;
  compact?: boolean;
  maxZoom?: number;
  onSelect?: (hit: HitTarget) => void;
  onRenderer?: (kind: RendererKind) => void;
}
export default function SceneViewport({
  scene,
  renderer,
  title,
  className = "",
  compact = false,
  maxZoom = 4.5,
  onSelect,
  onRenderer,
}: Props) {
  const wrapper = useRef<HTMLDivElement>(null),
    host = useRef<HTMLDivElement>(null),
    labels = useRef<HTMLDivElement>(null);
  const latest = useRef(scene),
    painterRef = useRef<Painter | null>(null),
    view = useRef<CameraView>(defaultView(scene)),
    matrix = useRef<Float32Array>(new Float32Array(16));
  const dirty = useRef(true),
    resetKey = useRef(0),
    usedReset = useRef(-1),
    interacted = useRef(false),
    onRendererRef = useRef(onRenderer);
  const [actual, setActual] = useState<RendererKind | null>(null),
    [reason, setReason] = useState<string | null>(null),
    [failure, setFailure] = useState<string | null>(null);
  const [fallbackEpoch, setFallbackEpoch] = useState(0);
  const fallbackToCanvas = useRef(false);
  const [tooltip, setTooltip] = useState<{
      hit: HitTarget;
      x: number;
      y: number;
    } | null>(null),
    [expanded, setExpanded] = useState(false);
  const labelNodes = useRef(new Map<string, HTMLSpanElement>()),
    lastSpec = useRef<SceneSpec | null>(null);
  const drag = useRef<{
    x: number;
    y: number;
    lastX: number;
    lastY: number;
    pan: boolean;
    distance: number;
    pointer: number;
  } | null>(null);
  latest.current = scene;
  onRendererRef.current = onRenderer;

  useEffect(() => {
    fallbackToCanvas.current = false;
  }, [renderer]);
  useEffect(() => {
    let disposed = false,
      raf = 0;
    let painter: Painter | null = null;
    let shown = 0,
      lastFrame = performance.now();
    let contextLost: ((e: Event) => void) | null = null;
    lastSpec.current = null;
    setActual(null);
    setFailure(null);
    setReason(null);
    const resize = new ResizeObserver(() => {
      dirty.current = true;
    });
    if (wrapper.current) resize.observe(wrapper.current);
    const generationHost = document.createElement("div");
    createPainter(
      generationHost,
      fallbackToCanvas.current ? "canvas" : renderer,
    )
      .then((result) => {
        if (disposed) {
          result.painter.dispose();
          return;
        }
        painter = result.painter;
        host.current?.replaceChildren(painter.canvas);
        painterRef.current = painter;
        setActual(painter.kind);
        setReason(result.reason);
        onRendererRef.current?.(painter.kind);
        dirty.current = true;
        contextLost = (e: Event) => {
          e.preventDefault();
          if (!disposed && !fallbackToCanvas.current) {
            fallbackToCanvas.current = true;
            setFallbackEpoch((v) => v + 1);
          }
        };
        painter.canvas.addEventListener("webglcontextlost", contextLost);
      })
      .catch(() => {
        if (!disposed)
          setFailure("This view could not start. Please select Canvas 2D.");
      });
    function frame(now: number) {
      if (disposed) return;
      raf = requestAnimationFrame(frame);
      const frameSeconds = Math.max(0, (now - lastFrame) / 1000);
      lastFrame = now;
      if (!painter || !wrapper.current) return;
      const rect = wrapper.current.getBoundingClientRect();
      if (
        rect.bottom < -50 ||
        rect.top > window.innerHeight + 50 ||
        rect.width < 2 ||
        rect.height < 2
      )
        return;
      const spec = latest.current,
        v = view.current;
      if (usedReset.current !== resetKey.current) {
        view.current = defaultView(spec, rect.width, rect.height);
        usedReset.current = resetKey.current;
        interacted.current = false;
        dirty.current = true;
      }
      const cv = view.current;
      cv.width = rect.width;
      cv.height = rect.height;
      cv.horizontalSpan = spec.horizontalSpan;
      if (!interacted.current) {
        let change = 0;
        const distance = Math.hypot(
          ...spec.target.map((value, i) => value - cv.target[i]),
        );
        const blend =
          distance > spec.span * 0.55 ? 1 : 1 - Math.exp(-frameSeconds * 18);
        for (let i = 0; i < 3; i++) {
          const diff = spec.target[i] - cv.target[i];
          change += Math.abs(diff);
          cv.target[i] += diff * blend;
        }
        const diff = spec.span - cv.span;
        change += Math.abs(diff);
        cv.span += diff * blend;
        if (change > 0.002) dirty.current = true;
      }
      if (lastSpec.current !== spec) {
        painter.update(spec);
        lastSpec.current = spec;
        dirty.current = true;
      }
      if (!dirty.current || now - shown < 1000 / 45) return;
      shown = now;
      dirty.current = false;
      const m = cameraMatrix(cv);
      matrix.current = m;
      painter.draw(
        m,
        rect.width,
        rect.height,
        Math.min(window.devicePixelRatio || 1, 1.75),
      );
      const parent = labels.current;
      if (parent) {
        const existing = labelNodes.current,
          active = new Set<string>();
        const occupied = new Map<
          string,
          { x: number; y: number; w: number; h: number }[]
        >();
        for (const label of spec.labels) {
          const pos = project(label.position, m, rect.width, rect.height);
          if (!pos.visible) continue;
          const size = label.size ?? 10,
            anchor = label.anchor ?? "center";
          const textWidth = label.text.length * size * 0.62;
          const textHeight = size * 1.25;
          const surface = label.surface
            ? surfaceLabelMatrix(
                label.position,
                label.surface,
                textWidth,
                textHeight,
                m,
                rect.width,
                rect.height,
              )
            : null;
          const w = surface
              ? Math.abs(surface.a) * textWidth +
                Math.abs(surface.c) * textHeight
              : Math.min(230, textWidth),
            hh = surface
              ? Math.abs(surface.b) * textWidth +
                Math.abs(surface.d) * textHeight
              : size + 3;
          const xx =
            pos.x - (anchor === "left" ? 0 : anchor === "right" ? w : w / 2);
          const group = label.id.startsWith("job-label")
            ? "jobs"
            : label.id.match(
                /^(book-name|book-state|book-sheet-order|book-original|book-clean|book-wait)-/,
              )?.[1];
          // Atlas captions share a world-space row. At overview scale their vertical
          // offsets would collapse to only a few pixels; reveal detail again on zoom.
          const pixelsPerUnit = (rect.width / cv.span) * cv.zoom;
          if (
            group?.startsWith("book-") &&
            group !== "book-name" &&
            pixelsPerUnit < 28
          )
            continue;
          if (group && !label.pill) {
            const row = occupied.get(group) ?? [];
            if (
              row.some(
                (o) =>
                  Math.abs(o.y - pos.y) < Math.max(hh, o.h) &&
                  xx < o.x + o.w + 8 &&
                  xx + w + 8 > o.x,
              )
            )
              continue;
            row.push({ x: xx, y: pos.y, w, h: hh });
            occupied.set(group, row);
          }
          active.add(label.id);
          let el = existing.get(label.id);
          if (!el) {
            el = document.createElement("span");
            parent.appendChild(el);
            existing.set(label.id, el);
          }
          el.textContent = label.text;
          el.className = `world-label${label.pill ? " pill-label" : ""}${label.muted ? " muted" : ""}`;
          el.style.color = label.color ?? "";
          el.style.fontSize = `${size}px`;
          el.style.maxWidth = surface ? "none" : "";
          el.style.transformOrigin = surface ? "0 0" : "";
          el.style.transform = surface
            ? `matrix(${surface.a},${surface.b},${surface.c},${surface.d},${surface.x},${surface.y}) translate(-50%,-50%)`
            : `translate(${pos.x}px,${pos.y}px) translate(${anchor === "left" ? "0" : anchor === "right" ? "-100%" : "-50%"},-50%)`;
        }
        for (const [id, el] of existing)
          if (!active.has(id)) {
            el.remove();
            existing.delete(id);
          }
      }
    }
    raf = requestAnimationFrame(frame);
    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      resize.disconnect();
      if (painter && contextLost)
        painter.canvas.removeEventListener("webglcontextlost", contextLost);
      painter?.dispose();
      painterRef.current = null;
    };
  }, [renderer, fallbackEpoch]);

  useEffect(() => {
    const node = wrapper.current;
    if (!node) return;
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      view.current.zoom = Math.min(
        maxZoom,
        Math.max(0.4, view.current.zoom * Math.exp(-e.deltaY * 0.0015)),
      );
      dirty.current = true;
    };
    node.addEventListener("wheel", wheel, { passive: false });
    return () => node.removeEventListener("wheel", wheel);
  }, [maxZoom]);
  useEffect(() => {
    if (!expanded) return;
    const fn = (e: KeyboardEvent) => {
      if (e.key === "Escape") setExpanded(false);
    };
    document.addEventListener("keydown", fn);
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", fn);
      document.body.style.overflow = previous;
    };
  }, [expanded]);
  const locate = (x: number, y: number) => {
    const rect = wrapper.current?.getBoundingClientRect();
    if (!rect) return null;
    const px = x - rect.left,
      py = y - rect.top;
    let best: HitTarget | null = null,
      bestDistance = Infinity;
    for (const h of latest.current.hits) {
      const p = project(h.position, matrix.current, rect.width, rect.height);
      if (!p.visible) continue;
      const radius = h.radius ?? 0.5;
      const r =
        radius > 4
          ? radius
          : Math.max(12, (radius * rect.width) / view.current.span);
      const d = Math.hypot(px - p.x, py - p.y);
      if (d <= r && d < bestDistance) {
        best = h;
        bestDistance = d;
      }
    }
    return best ? { hit: best, x: px, y: py } : null;
  };
  const changeZoom = (factor: number) => {
    view.current.zoom = Math.min(
      maxZoom,
      Math.max(0.4, view.current.zoom * factor),
    );
    dirty.current = true;
  };
  return (
    <div
      ref={wrapper}
      className={`scene-viewport ${compact ? "compact" : ""} ${expanded ? "expanded" : ""} ${actual === "canvas" ? "canvas-fallback" : ""} ${className}`}
      data-renderer={actual ?? "loading"}
      aria-label={title}
      onContextMenu={(e) => e.preventDefault()}
      onPointerDown={(e) => {
        if ((e.target as HTMLElement).closest("button")) return;
        drag.current = {
          x: e.clientX,
          y: e.clientY,
          lastX: e.clientX,
          lastY: e.clientY,
          pan: e.shiftKey || e.button === 2,
          distance: 0,
          pointer: e.pointerId,
        };
        e.currentTarget.setPointerCapture(e.pointerId);
        setTooltip(null);
      }}
      onPointerMove={(e) => {
        const d = drag.current;
        if (d) {
          const dx = e.clientX - d.lastX,
            dy = e.clientY - d.lastY;
          d.distance += Math.abs(dx) + Math.abs(dy);
          d.lastX = e.clientX;
          d.lastY = e.clientY;
          if (d.distance > 4) {
            if (d.pan) {
              interacted.current = true;
              const scale =
                view.current.span / view.current.width / view.current.zoom;
              view.current.target[0] -= dx * scale;
              view.current.target[1] += dy * scale;
            } else {
              view.current.yaw -= dx * 0.006;
              view.current.pitch = Math.min(
                1.48,
                Math.max(-0.25, view.current.pitch + dy * 0.005),
              );
            }
            dirty.current = true;
          }
        } else setTooltip(locate(e.clientX, e.clientY));
      }}
      onPointerUp={(e) => {
        const d = drag.current;
        drag.current = null;
        if (d && d.distance < 5) {
          const result = locate(e.clientX, e.clientY);
          setTooltip(result);
          if (result) onSelect?.(result.hit);
        }
        if (e.currentTarget.hasPointerCapture(e.pointerId))
          e.currentTarget.releasePointerCapture(e.pointerId);
      }}
      onPointerCancel={() => {
        drag.current = null;
      }}
      onPointerLeave={() => {
        if (!drag.current) setTooltip(null);
      }}
    >
      {actual === "canvas" && (
        <div className="css-perspective-floor" aria-hidden="true">
          <div />
        </div>
      )}
      <div className="canvas-mount" ref={host} />
      <div className="scene-labels" ref={labels} aria-hidden="true" />
      {!actual && !failure && (
        <div className="scene-loading">
          <span className="loading-orbit" />
          Preparing view…
        </div>
      )}
      {failure && <div className="scene-loading">{failure}</div>}
      <div className="scene-tools">
        <button
          className="icon-button"
          title="Reset view and follow camera"
          aria-label={`${title}: Reset view`}
          onClick={() => {
            resetKey.current++;
            dirty.current = true;
          }}
        >
          <RotateCcw size={14} />
        </button>
        <button
          className="icon-button"
          title="Zoom out"
          aria-label={`${title}: Zoom out`}
          onClick={() => changeZoom(1 / 1.2)}
        >
          <Minus size={14} />
        </button>
        <button
          className="icon-button"
          title="Zoom in"
          aria-label={`${title}: Zoom in`}
          onClick={() => changeZoom(1.2)}
        >
          <Plus size={14} />
        </button>
        <span className="toolbar-divider" />
        <button
          className="icon-button"
          title={expanded ? "Exit fullscreen (Esc)" : "Fullscreen"}
          aria-label={`${title}: Fullscreen`}
          aria-pressed={expanded}
          onClick={() => {
            setExpanded(!expanded);
            dirty.current = true;
          }}
        >
          <Expand size={14} />
        </button>
      </div>
      <div className="scene-bottom">
        <span className="scene-help">
          <Move3D size={12} />
          {compact
            ? "Drag to rotate"
            : "Drag: rotate · Shift: pan · Wheel: zoom"}
        </span>
        <span
          className="renderer-indicator"
          title={reason ?? "Active renderer"}
        >
          <i />
          {actual === "three"
            ? "THREE.JS / WEBGL 2"
            : actual === "webgl"
              ? "NATIVES WEBGL"
              : actual === "canvas"
                ? "CANVAS 2D · PERSPEKTIVE"
                : "INITIALISIERUNG"}
        </span>
      </div>
      {tooltip && (
        <div
          className="scene-tooltip"
          style={{
            left: Math.max(
              8,
              Math.min(
                tooltip.x + 14,
                (wrapper.current?.clientWidth ?? 600) - 250,
              ),
            ),
            top: Math.max(
              8,
              Math.min(
                tooltip.y + 14,
                (wrapper.current?.clientHeight ?? 400) - 142,
              ),
            ),
          }}
        >
          <strong style={{ color: tooltip.hit.color }}>
            {tooltip.hit.title}
          </strong>
          {tooltip.hit.lines.map((l, i) => (
            <span key={i}>{l}</span>
          ))}
        </div>
      )}
    </div>
  );
}
