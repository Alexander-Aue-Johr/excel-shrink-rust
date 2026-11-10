export type Vec3 = [number, number, number];
export interface Shape {
  id?: string;
  positions: number[];
  indices?: number[];
  colors?: number[];
  color: string;
  opacity?: number;
  kind?: "triangles" | "lines";
  dashed?: boolean;
  width?: number;
}
export interface SceneLabel {
  id: string;
  position: Vec3;
  text: string;
  color?: string;
  size?: number;
  muted?: boolean;
  pill?: boolean;
  anchor?: "left" | "center" | "right";
  /** Size of a label attached to the X/Z surface, in scene units. */
  surface?: { width: number; height: number };
}
export interface HitTarget {
  id: string;
  workbookId?: string;
  sheetId?: string;
  sheetVariant?: "original" | "optimized";
  position: Vec3;
  /** Extend picking along the projected column body up to position. */
  pickBase?: Vec3;
  radius?: number;
  title: string;
  lines: string[];
  color?: string;
}
export interface SceneSpec {
  shapes: Shape[];
  labels: SceneLabel[];
  hits: HitTarget[];
  target: Vec3;
  span: number;
  /** Optional horizontal framing, independent of the viewport aspect ratio. */
  horizontalSpan?: number;
  yaw?: number;
  pitch?: number;
  footer?: string;
}
export const PALETTE = [
  "#81cbb6",
  "#78adce",
  "#9c94cc",
  "#d2b181",
  "#c88fa4",
  "#a5bd88",
  "#91bcc7",
  "#b1a1cb",
];
export function hsl(h: number, s: number, l: number): string {
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))));
  };
  return (
    "#" +
    [f(0), f(8), f(4)].map((n) => n.toString(16).padStart(2, "0")).join("")
  );
}
export function sheetColor(hue: number, chunk = 0, count = 1): string {
  return hsl(
    hue,
    count > 1 ? 0.26 + 0.38 * (chunk / Math.max(1, count - 1)) : 0.45,
    0.64,
  );
}
export function shade(color: string, factor: number): string {
  const c = color.replace("#", "");
  return (
    "#" +
    [0, 2, 4]
      .map((i) =>
        Math.min(255, Math.round(parseInt(c.slice(i, i + 2), 16) * factor))
          .toString(16)
          .padStart(2, "0"),
      )
      .join("")
  );
}
export function line(
  a: Vec3,
  b: Vec3,
  color = "#33464b",
  opacity = 1,
  dashed = false,
): Shape {
  return { positions: [...a, ...b], color, opacity, kind: "lines", dashed };
}
export function polyline(
  points: Vec3[],
  color: string,
  opacity = 1,
  dashed = false,
): Shape {
  const positions: number[] = [];
  for (let i = 1; i < points.length; i++)
    positions.push(...points[i - 1], ...points[i]);
  return { positions, color, opacity, kind: "lines", dashed };
}
export function plane(
  x: number,
  y: number,
  z: number,
  w: number,
  d: number,
  color: string,
  opacity = 1,
): Shape {
  return {
    positions: [x, y, z, x + w, y, z, x + w, y, z + d, x, y, z + d],
    indices: [0, 1, 2, 0, 2, 3],
    color,
    opacity,
  };
}
export function box(
  x: number,
  y: number,
  z: number,
  w: number,
  h: number,
  d: number,
  color: string,
  opacity = 1,
): Shape[] {
  const p = [
    x,
    y,
    z,
    x + w,
    y,
    z,
    x + w,
    y + h,
    z,
    x,
    y + h,
    z,
    x,
    y,
    z + d,
    x + w,
    y,
    z + d,
    x + w,
    y + h,
    z + d,
    x,
    y + h,
    z + d,
  ];
  return [
    {
      positions: p,
      indices: [0, 1, 2, 0, 2, 3, 4, 7, 6, 4, 6, 5],
      color: shade(color, 0.77),
      opacity,
    },
    {
      positions: p,
      indices: [0, 3, 7, 0, 7, 4, 1, 5, 6, 1, 6, 2],
      color: shade(color, 0.88),
      opacity,
    },
    { positions: p, indices: [3, 2, 6, 3, 6, 7], color, opacity },
  ];
}
export function boxEdges(
  x: number,
  y: number,
  z: number,
  w: number,
  h: number,
  d: number,
  color: string,
  opacity = 0.6,
  dashed = true,
): Shape {
  const p: Vec3[] = [
    [x, y, z],
    [x + w, y, z],
    [x + w, y + h, z],
    [x, y + h, z],
    [x, y, z + d],
    [x + w, y, z + d],
    [x + w, y + h, z + d],
    [x, y + h, z + d],
  ];
  const positions: number[] = [];
  for (const [a, b] of [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
  ])
    positions.push(...p[a], ...p[b]);
  return { positions, color, opacity, kind: "lines", dashed };
}
export const smooth = (x: number) => {
  const t = Math.max(0, Math.min(1, x));
  return t * t * (3 - 2 * t);
};
const BYTE_UNITS = ["B", "KiB", "MiB", "GiB", "TiB"];
const BYTE_INTEGER_FORMAT = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 0,
});
const BYTE_DECIMAL_FORMAT = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 1,
});
export const fmtBytes = (v: number | null | undefined) => {
  if (v == null || !Number.isFinite(v)) return "—";
  if (v === 0) return "0 B";
  const i = Math.max(
    0,
    Math.min(4, Math.floor(Math.log(Math.abs(v)) / Math.log(1024))),
  );
  return (
    (i > 1 ? BYTE_DECIMAL_FORMAT : BYTE_INTEGER_FORMAT).format(v / 1024 ** i) +
    " " +
    BYTE_UNITS[i]
  );
};
export const fmtTime = (v: number) => {
  const s = Math.max(0, Math.floor(v));
  return `${Math.floor(s / 60)
    .toString()
    .padStart(2, "0")}:${(s % 60).toString().padStart(2, "0")}`;
};
