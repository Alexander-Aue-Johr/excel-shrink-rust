import type { SceneSpec, Vec3 } from "./scene";
export interface CameraView {
  target: Vec3;
  yaw: number;
  pitch: number;
  zoom: number;
  span: number;
  width: number;
  height: number;
}
const norm = (v: Vec3): Vec3 => {
  const l = Math.hypot(...v) || 1;
  return v.map((n) => n / l) as Vec3;
};
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
export function cameraMatrix(v: CameraView): Float32Array {
  const dir: Vec3 = [
    Math.sin(v.yaw) * Math.cos(v.pitch),
    Math.sin(v.pitch),
    Math.cos(v.yaw) * Math.cos(v.pitch),
  ];
  const right = norm(cross([0, 1, 0], dir));
  const up = cross(dir, right);
  const height =
    v.span / Math.min(v.width / Math.max(v.height, 1), 1.8) / v.zoom;
  const width = (height * v.width) / Math.max(v.height, 1);
  const depth = Math.max(200, v.span * 20);
  return new Float32Array([
    (2 * right[0]) / width,
    (2 * up[0]) / height,
    (-2 * dir[0]) / depth,
    0,
    (2 * right[1]) / width,
    (2 * up[1]) / height,
    (-2 * dir[1]) / depth,
    0,
    (2 * right[2]) / width,
    (2 * up[2]) / height,
    (-2 * dir[2]) / depth,
    0,
    (-2 * dot(right, v.target)) / width,
    (-2 * dot(up, v.target)) / height,
    (2 * dot(dir, v.target)) / depth,
    1,
  ]);
}
export function project(
  p: Vec3,
  m: Float32Array,
  width: number,
  height: number,
) {
  const x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
  const y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
  const z = m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14];
  return {
    x: ((x + 1) * width) / 2,
    y: ((1 - y) * height) / 2,
    z,
    visible: x > -1.05 && x < 1.05 && y > -1.05 && y < 1.05,
  };
}
export function defaultView(
  spec: SceneSpec,
  width = 800,
  height = 400,
): CameraView {
  return {
    target: [...spec.target],
    span: spec.span,
    yaw: spec.yaw ?? 0.2,
    pitch: spec.pitch ?? 0.75,
    zoom: 1,
    width,
    height,
  };
}
