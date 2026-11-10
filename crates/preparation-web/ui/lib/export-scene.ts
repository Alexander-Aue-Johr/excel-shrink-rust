import { cameraMatrix, project, type CameraView } from "./camera";
import type { SceneSpec, Vec3 } from "./scene";

const escape = (text: string) =>
  text.replace(
    /[&<>"']/g,
    (c) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&apos;",
      })[c]!,
  );

/** Export the current measured scene and camera as editable vector geometry. */
export function sceneSvg(spec: SceneSpec, camera: CameraView): string {
  const { width, height } = camera;
  const matrix = cameraMatrix(camera);
  const primitives: { z: number; svg: string }[] = [];
  for (const shape of spec.shapes) {
    const points: ReturnType<typeof project>[] = [];
    for (let i = 0; i < shape.positions.length; i += 3) {
      points.push(
        project(shape.positions.slice(i, i + 3) as Vec3, matrix, width, height),
      );
    }
    const indices = shape.indices ?? points.map((_, i) => i);
    const stride = shape.kind === "lines" ? 2 : 3;
    for (let i = 0; i + stride <= indices.length; i += stride) {
      const group = indices.slice(i, i + stride).map((index) => points[index]);
      const coordinates = group
        .map((p) => `${p.x.toFixed(2)},${p.y.toFixed(2)}`)
        .join(" ");
      const opacity = shape.opacity ?? 1;
      const svg =
        stride === 2
          ? `<polyline points="${coordinates}" fill="none" stroke="${shape.color}" stroke-width="${shape.width ?? 0.8}" ${shape.dashed ? 'stroke-dasharray="5 4"' : ""} opacity="${opacity}"/>`
          : `<polygon points="${coordinates}" fill="${shape.color}" opacity="${opacity}"/>`;
      primitives.push({
        z: group.reduce((sum, p) => sum + p.z, 0) / stride,
        svg,
      });
    }
  }
  primitives.sort((a, b) => b.z - a.z);
  const workers = spec.labels.filter((label) => label.id.startsWith("worker-"));
  const labelledWorkers = new Set([
    workers[0]?.id,
    workers[Math.floor(workers.length / 2)]?.id,
    workers.at(-1)?.id,
  ]);
  const labels = spec.labels
    .filter((label) => {
      if (
        label.id.startsWith("job-label") ||
        label.id.startsWith("axis-") ||
        label.id.startsWith("book-sheet-order-")
      )
        return false;
      if (label.id.startsWith("worker-")) return labelledWorkers.has(label.id);
      return true;
    })
    .map((label) => {
      const p = project(label.position, matrix, width, height);
      if (!p.visible) return "";
      const anchor =
        label.anchor === "left"
          ? "start"
          : label.anchor === "right"
            ? "end"
            : "middle";
      const text =
        label.id.startsWith("book-name-") && label.text.length > 34
          ? label.text.slice(0, 31) + "…"
          : label.text;
      return `<text x="${p.x}" y="${p.y}" fill="${label.color ?? "#506174"}" font-size="${Math.max(12, label.size ?? 12)}" text-anchor="${anchor}"><title>${escape(label.text)}</title>${escape(text)}</text>`;
    })
    .join("");
  const anchors = spec.hits
    .filter((hit) => hit.sheetId)
    .map((hit) => ({
      sheetId: hit.sheetId,
      variant: hit.sheetVariant,
      color: hit.color,
      ...project(hit.position, matrix, width, height),
    }))
    .filter((point) => point.visible);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}"
    width="${width}" height="${height}" font-family="Arial, sans-serif">
    <title>Recorded processing scene</title>
    <metadata id="worksheet-anchors">${escape(JSON.stringify(anchors))}</metadata>
    ${primitives.map((p) => p.svg).join("")}
    ${labels}
  </svg>`;
}

/** Compose both measured views with the same worksheet links as the interactive view. */
export function figureSvg(
  views: string[],
  subtitle: string,
  light: boolean,
  sheetId: string | null,
  variant: "original" | "optimized" = "optimized",
): string {
  const background = light ? "#ffffff" : "#0c1317";
  const text = light ? "#192b40" : "#e3eae8";
  const anchors: { x: number; y: number; color: string }[][] = [];
  const positioned = views.map((view, index) => {
    const element = new DOMParser().parseFromString(
      view,
      "image/svg+xml",
    ).documentElement;
    const width = Number(element.getAttribute("width"));
    const height = Number(element.getAttribute("height"));
    const y = index === 0 ? 165 : 710;
    const frameHeight = index === 0 ? 480 : 320;
    const scale = Math.min(1840 / width, frameHeight / height);
    const points = JSON.parse(
      element.querySelector("metadata")?.textContent ?? "[]",
    );
    anchors.push(
      points
        .filter(
          (point: { sheetId: string; variant?: string }) =>
            point.sheetId === sheetId &&
            (index === 0 || (point.variant ?? "optimized") === variant),
        )
        .map((point: { x: number; y: number; color: string }) => ({
          x: 40 + (1840 - width * scale) / 2 + point.x * scale,
          y: y + (frameHeight - height * scale) / 2 + point.y * scale,
          color: point.color,
        })),
    );
    element.setAttribute("x", "40");
    element.setAttribute("y", String(y));
    element.setAttribute("width", "1840");
    element.setAttribute("height", String(frameHeight));
    return new XMLSerializer().serializeToString(element);
  });
  const target = anchors[1]?.[0];
  const links = target
    ? anchors[0]
        .map((point) => {
          const middle = (point.y + target.y) / 2;
          return `<path d="M${point.x},${point.y} C${point.x},${middle} ${target.x},${middle} ${target.x},${target.y}" fill="none" stroke="${point.color ?? text}" stroke-width="1.5" opacity="0.65"/>`;
        })
        .join("")
    : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1080" viewBox="0 0 1920 1080">
    <rect width="1920" height="1080" fill="${background}"/>
    <g fill="${text}" font-family="Arial,sans-serif">
      <text x="55" y="60" font-size="40">SheetShrink · recorded parallel cleanup</text>
      <text x="55" y="100" font-size="26">${escape(subtitle)}</text>
      <text x="55" y="145" font-size="28">A  Worksheet and chunk scheduling</text>
      <text x="55" y="690" font-size="28">B  Worksheet XML before and after cleanup</text>
      <text x="55" y="1060" font-size="20">Time runs left to right. Worker lanes share the same clock. Height in A follows workbook admission order.</text>
    </g>
    ${positioned.join("")}
    ${links}
  </svg>`;
}

export function downloadSvg(name: string, svg: string) {
  const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
