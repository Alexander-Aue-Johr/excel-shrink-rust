import { useEffect, useRef } from "react";

export interface ProjectedSheet {
  source: string;
  points: { x: number; y: number; color?: string }[];
}

/** Connect only the focused worksheet. Other sheets remain quiet context. */
export default function SheetConnections({
  sheetId,
}: {
  sheetId: string | null;
}) {
  const root = useRef<SVGSVGElement>(null);
  useEffect(() => {
    const views = new Map<string, ProjectedSheet["points"]>();
    const update = (event: Event) => {
      const svg = root.current;
      if (!svg || !sheetId) return;
      const detail = (event as CustomEvent<ProjectedSheet>).detail;
      views.set(detail.source, detail.points);
      const rect = svg.getBoundingClientRect();
      const target = views.get("Workbook-Atlas")?.[0];
      const workers = views.get("Worker timeline") ?? [];
      svg.replaceChildren();
      if (!target) return;
      for (const point of workers) {
        const path = document.createElementNS(
          "http://www.w3.org/2000/svg",
          "path",
        );
        const x1 = point.x - rect.left,
          y1 = point.y - rect.top;
        const x2 = target.x - rect.left,
          y2 = target.y - rect.top;
        const middle = (y1 + y2) / 2;
        path.setAttribute(
          "d",
          `M${x1},${y1} C${x1},${middle} ${x2},${middle} ${x2},${y2}`,
        );
        path.setAttribute("stroke", point.color ?? "#587da0");
        path.setAttribute("stroke-width", "1.5");
        path.setAttribute("fill", "none");
        svg.append(path);
      }
    };
    window.addEventListener("sheetshrink-projection", update);
    return () => window.removeEventListener("sheetshrink-projection", update);
  }, [sheetId]);
  return (
    <svg
      key={sheetId ?? "none"}
      ref={root}
      className="sheet-connections"
      aria-hidden="true"
    />
  );
}
