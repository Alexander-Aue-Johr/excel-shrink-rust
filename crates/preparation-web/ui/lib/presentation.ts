import type { SceneSpec } from "./scene";

export type Theme = "dark" | "light";

/** Keep worksheet hues while giving neutral scaffolding contrast on paper. */
export function presentScene(scene: SceneSpec, theme: Theme): SceneSpec {
  if (theme === "dark") return scene;
  const color = (hex: string, label = false) => {
    const bytes = [1, 3, 5].map((start) =>
      parseInt(hex.slice(start, start + 2), 16),
    );
    const spread = Math.max(...bytes) - Math.min(...bytes);
    if (label) return spread < 70 ? "#415369" : hex;
    if (Math.max(...bytes) < 100 && spread < 55) return "#cbd5df";
    return hex;
  };
  return {
    ...scene,
    shapes: scene.shapes.map((shape) => ({
      ...shape,
      color: color(shape.color),
    })),
    labels: scene.labels.map((label) => ({
      ...label,
      color: color(label.color ?? "#415369", true),
    })),
  };
}
