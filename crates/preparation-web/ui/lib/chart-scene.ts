import type { LabelMode, Snapshot } from "./model";
import { fmtBytes, line, smooth } from "./scene";
import type { SceneSpec, Shape, Vec3 } from "./scene";

export type DistributionOrder = "independent" | "paired";

export interface DistributionOptions {
  entity: "worksheets" | "workbooks";
  metric: "size" | "ratio";
  labelMode: LabelMode;
  /** Separate size distributions are the default; paired keeps each entity at its original rank. */
  order?: DistributionOrder;
}

interface Datum {
  id: string;
  name: string;
  workbookName: string;
  originalBytes: number;
  cleanedBytes: number | null;
  completedAt: number | null;
}
interface Point {
  x: number;
  y: number;
}

const WIDTH = 12;
const HEIGHT = 5;
const ORIGINAL_Z = -0.45;
const CLEAN_Z = 0.65;
const ORIGINAL_COLOR = "#557a91";
const ORIGINAL_EDGE = "#89adc1";
const CLEAN_COLOR = "#7ecab2";
const GROWTH_COLOR = "#cb9b9d";
const PERCENT = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

function itemData(snapshot: Snapshot, options: DistributionOptions): Datum[] {
  const books = new Map(snapshot.workbooks.map((book) => [book.id, book]));
  const visibleReportAt = (at: number | null) =>
    at !== null && at <= snapshot.time ? at : null;
  if (options.entity === "workbooks") {
    return snapshot.workbooks
      .filter((book) => book.known && book.originalBytes !== null)
      .map((book) => {
        const completedAt = visibleReportAt(book.completedAt);
        return {
          id: book.id,
          name: book.name,
          workbookName: "",
          originalBytes: book.originalBytes!,
          cleanedBytes: completedAt === null ? null : book.cleanedBytes,
          completedAt,
        };
      });
  }
  return snapshot.sheets.map((sheet) => {
    const book = books.get(sheet.workbookId);
    const completedAt = visibleReportAt(book?.completedAt ?? null);
    return {
      id: sheet.id,
      name: sheet.name,
      workbookName: book?.name ?? sheet.workbookId,
      originalBytes: sheet.originalBytes,
      cleanedBytes: completedAt === null ? null : sheet.cleanedBytes,
      completedAt,
    };
  });
}

/** Append one continuous area to a shared buffer; disconnected runs stay disconnected. */
function appendStrip(
  shape: Shape,
  outline: Shape,
  points: Point[],
  baseline: number,
  z: number,
) {
  if (points.length < 2) return;
  const indices = shape.indices!;
  const offset = shape.positions.length / 3;
  for (const point of points)
    shape.positions.push(point.x, baseline, z, point.x, point.y, z);
  for (let i = 0; i < points.length - 1; i++) {
    const a = offset + i * 2;
    indices.push(a, a + 2, a + 1, a + 1, a + 2, a + 3);
    outline.positions.push(
      points[i].x,
      points[i].y,
      z + 0.012,
      points[i + 1].x,
      points[i + 1].y,
      z + 0.012,
    );
  }
}

function filled(color: string, opacity: number): Shape {
  return { positions: [], indices: [], color, opacity, kind: "triangles" };
}
function outline(color: string, opacity = 0.95): Shape {
  return { positions: [], color, opacity, kind: "lines", width: 1.25 };
}
function xAt(index: number, count: number) {
  return count <= 1 ? WIDTH / 2 : (WIDTH * index) / (count - 1);
}
function pointsForRun(
  values: number[],
  firstIndex: number,
  totalCount: number,
): Point[] {
  if (totalCount === 1)
    return [
      { x: 0, y: values[0] },
      { x: WIDTH, y: values[0] },
    ];
  if (values.length === 1) {
    // An isolated report is a narrow rectangle at its rank, never a fake zero.
    const x = xAt(firstIndex, totalCount);
    const half = (WIDTH / Math.max(1, totalCount - 1)) * 0.34;
    return [
      { x: Math.max(0, x - half), y: values[0] },
      { x: Math.min(WIDTH, x + half), y: values[0] },
    ];
  }
  return values.map((y, i) => ({ x: xAt(firstIndex + i, totalCount), y }));
}

function addAxes(
  scene: SceneSpec,
  items: number,
  ticks: { value: number; y: number; label: string }[],
  baseline: number,
  unit: string,
  order: DistributionOrder | "ratio" = "ratio",
) {
  const gridZ = -0.58;
  for (let i = 0; i < ticks.length; i++) {
    const tick = ticks[i];
    scene.shapes.push(
      line([0, tick.y, gridZ], [WIDTH, tick.y, gridZ], "#2c4148", 0.7),
    );
    scene.labels.push({
      id: `axis-y-${i}`,
      position: [-0.22, tick.y, gridZ],
      text: tick.label,
      size: 10,
      color: "#83979e",
      anchor: "right",
    });
  }
  scene.shapes.push(
    line([0, 0, gridZ], [0, HEIGHT + 0.12, gridZ], "#4b646b", 0.8),
  );
  scene.shapes.push(
    line([0, baseline, 0.79], [WIDTH, baseline, 0.79], "#637a80", 0.8),
  );
  // Short connectors visibly establish the two depth planes without a box cage.
  scene.shapes.push(
    line([0, baseline, ORIGINAL_Z], [0, baseline, CLEAN_Z], "#526971", 0.5),
  );
  scene.shapes.push(
    line(
      [WIDTH, baseline, ORIGINAL_Z],
      [WIDTH, baseline, CLEAN_Z],
      "#526971",
      0.5,
    ),
  );
  scene.labels.push({
    id: "axis-unit",
    position: [0, HEIGHT + 0.36, gridZ],
    text: unit,
    size: 10,
    color: "#9aafb5",
    anchor: "left",
  });
  if (items && order === "independent") {
    // The two layers may contain different numbers of known reports. Relative
    // ranks share the horizontal extent without implying matching entity IDs.
    for (const [fraction, label] of [
      [0, "largest"],
      [0.5, "median"],
      [1, "smallest"],
    ] as const) {
      const x = WIDTH * fraction;
      scene.shapes.push(
        line([x, -0.08, CLEAN_Z], [x, 0, CLEAN_Z], "#53686e", 0.8),
      );
      scene.labels.push({
        id: `axis-rank-${fraction}`,
        position: [x, -0.27, CLEAN_Z],
        text: label,
        size: 10,
        color: "#81969d",
      });
    }
  } else if (items) {
    const ranks =
      items === 1
        ? [1]
        : items > 3
          ? [1, Math.ceil(items / 2), items]
          : [1, items];
    for (const rank of ranks) {
      const x = xAt(rank - 1, items);
      scene.shapes.push(
        line([x, -0.08, CLEAN_Z], [x, 0, CLEAN_Z], "#53686e", 0.8),
      );
      scene.labels.push({
        id: `axis-rank-${rank}`,
        position: [x, -0.27, CLEAN_Z],
        text: `${rank.toLocaleString("en-US")}`,
        size: 10,
        color: "#81969d",
      });
    }
  }
  const orderText =
    order === "independent"
      ? "Size rank · sorted independently"
      : order === "paired"
        ? "Original rank · same file"
        : "Rank · sorted descending";
  scene.labels.push({
    id: "axis-order",
    position: [WIDTH / 2, -0.74, CLEAN_Z],
    text: orderText,
    size: 10,
    color: "#83979e",
  });
}

function addHits(
  scene: SceneSpec,
  data: Datum[],
  snapshot: Snapshot,
  positions: (datum: Datum, index: number) => Vec3[],
  color: string,
  layerName?: "Original" | "Optimized",
) {
  // The entire mesh is retained; only hover markers are bounded for large corpora.
  const stride = Math.max(1, Math.ceil(data.length / 850));
  for (let i = 0; i < data.length; i += stride) {
    const datum = data[i];
    positions(datum, i).forEach((position, layer) =>
      scene.hits.push({
        id: `${layerName ? `${layerName}:` : ""}${datum.id}-${layer}`,
        position,
        radius: 16,
        title: datum.name,
        color,
        get lines() {
          const savedRatio =
            datum.cleanedBytes === null
              ? null
              : (datum.originalBytes - datum.cleanedBytes) /
                Math.max(1, datum.originalBytes);
          return [
            ...(datum.workbookName ? [datum.workbookName] : []),
            `${layerName ? `${layerName} · ` : ""}Rank ${i + 1} of ${data.length.toLocaleString("en-US")}`,
            `Original: ${fmtBytes(datum.originalBytes)}`,
            datum.cleanedBytes === null
              ? "Optimized: report pending"
              : `Optimized: ${fmtBytes(datum.cleanedBytes)}`,
            ...(savedRatio === null
              ? []
              : [
                  savedRatio >= 0
                    ? `Ersparnis: ${PERCENT.format(savedRatio * 100)} %`
                    : `Zuwachs: ${PERCENT.format(-savedRatio * 100)} %`,
                ]),
            ...(datum.completedAt !== null &&
            snapshot.time - datum.completedAt < 1.8
              ? ["Revealing output"]
              : []),
          ];
        },
      }),
    );
  }
}

/**
 * Build all four distributions from one historical snapshot. Each area uses a
 * linear indexed strip with two vertices per sample; no per-sample mesh objects.
 */
export function buildDistributionScene(
  snapshot: Snapshot,
  options: DistributionOptions,
): SceneSpec {
  const scene: SceneSpec = {
    shapes: [],
    labels: [],
    hits: [],
    target: [6, 2.3, 0],
    span: 15.6,
    yaw: 0.08,
    pitch: 0.12,
  };
  const data = itemData(snapshot, options);
  const entityLabel =
    options.entity === "worksheets" ? "Worksheets" : "Workbooks";

  if (options.metric === "size") {
    const order = options.order ?? "independent";
    data.sort(
      (a, b) => b.originalBytes - a.originalBytes || a.id.localeCompare(b.id),
    );
    const hasReport = (
      datum: Datum,
    ): datum is Datum & { cleanedBytes: number; completedAt: number } =>
      datum.cleanedBytes !== null &&
      datum.completedAt !== null &&
      datum.completedAt <= snapshot.time;
    // Reports can occasionally be larger than their source file. Both layers
    // keep one truthful byte scale, including those reported increases.
    const maximum = data.reduce(
      (max, datum) =>
        Math.max(
          max,
          datum.originalBytes,
          hasReport(datum) ? datum.cleanedBytes : 0,
        ),
      1,
    );
    const heightOf = (bytes: number) => (bytes / maximum) * HEIGHT;
    const animatedClean = (datum: Datum) => {
      if (!hasReport(datum)) return null;
      const t = smooth((snapshot.time - datum.completedAt) / 1.8);
      return heightOf(
        datum.originalBytes + (datum.cleanedBytes - datum.originalBytes) * t,
      );
    };
    addAxes(
      scene,
      data.length,
      [0, 0.25, 0.5, 0.75, 1].map((fraction) => ({
        value: maximum * fraction,
        y: fraction * HEIGHT,
        label: fmtBytes(maximum * fraction),
      })),
      0,
      "File size · linear",
      order,
    );
    if (!data.length) {
      scene.labels.push({
        id: "empty",
        position: [6, 2.35, 0],
        text: "Waiting for size measurements",
        size: 12,
        color: "#82999f",
      });
      scene.footer = "The distribution updates as metadata arrives.";
      return scene;
    }
    const original = filled(ORIGINAL_COLOR, 0.52);
    const originalOutline = outline(ORIGINAL_EDGE, 0.85);
    appendStrip(
      original,
      originalOutline,
      pointsForRun(
        data.map((item) => heightOf(item.originalBytes)),
        0,
        data.length,
      ),
      0,
      ORIGINAL_Z,
    );
    scene.shapes.push(original, originalOutline);

    const clean = filled(CLEAN_COLOR, 0.72);
    const cleanOutline = outline("#a3e4ca");
    const reported = data.filter(hasReport);
    if (order === "independent") {
      // Sort the current animated heights, not just their final values. Newly
      // arriving reports therefore never create an unsorted edge mid-transition.
      const cleanHeights = new Map(
        reported.map((datum) => [datum.id, animatedClean(datum)!]),
      );
      reported.sort(
        (a, b) =>
          cleanHeights.get(b.id)! - cleanHeights.get(a.id)! ||
          a.id.localeCompare(b.id),
      );
      if (reported.length)
        appendStrip(
          clean,
          cleanOutline,
          pointsForRun(
            reported.map((datum) => cleanHeights.get(datum.id)!),
            0,
            reported.length,
          ),
          0,
          CLEAN_Z,
        );
      addHits(
        scene,
        data,
        snapshot,
        (datum, index) => [
          [xAt(index, data.length), heightOf(datum.originalBytes), ORIGINAL_Z],
        ],
        ORIGINAL_EDGE,
        "Original",
      );
      addHits(
        scene,
        reported,
        snapshot,
        (datum, index) => [
          [xAt(index, reported.length), cleanHeights.get(datum.id)!, CLEAN_Z],
        ],
        CLEAN_COLOR,
        "Optimized",
      );
    } else {
      // Preserving correspondence intentionally leaves gaps for outstanding
      // reports; a missing result must never look like a zero-byte file.
      let runStart = -1;
      let values: number[] = [];
      const flush = () => {
        if (runStart >= 0)
          appendStrip(
            clean,
            cleanOutline,
            pointsForRun(values, runStart, data.length),
            0,
            CLEAN_Z,
          );
        runStart = -1;
        values = [];
      };
      for (let i = 0; i < data.length; i++) {
        const value = animatedClean(data[i]);
        if (value === null) {
          flush();
          continue;
        }
        if (runStart < 0) runStart = i;
        values.push(value);
      }
      flush();
      addHits(
        scene,
        data,
        snapshot,
        (datum, index) => {
          const positions: Vec3[] = [
            [
              xAt(index, data.length),
              heightOf(datum.originalBytes),
              ORIGINAL_Z,
            ],
          ];
          const cleanY = animatedClean(datum);
          if (cleanY !== null)
            positions.push([xAt(index, data.length), cleanY, CLEAN_Z]);
          return positions;
        },
        ORIGINAL_EDGE,
      );
    }
    if (clean.positions.length) scene.shapes.push(clean, cleanOutline);
    scene.footer = `${data.length.toLocaleString("en-US")} ${entityLabel} · ${reported.length.toLocaleString("en-US")} Results · ${order === "independent" ? "layers sorted independently · no individual pairing" : "same file at the same X position"}`;
    return scene;
  }

  const reported = data.filter(
    (datum): datum is Datum & { cleanedBytes: number } =>
      datum.cleanedBytes !== null,
  );
  const ratio = (datum: Datum & { cleanedBytes: number }) =>
    (datum.originalBytes - datum.cleanedBytes) /
    Math.max(1, datum.originalBytes);
  reported.sort((a, b) => ratio(b) - ratio(a) || a.id.localeCompare(b.id));
  const minimum = Math.min(0, ...reported.map(ratio));
  const lower = minimum < 0 ? -Math.ceil(Math.abs(minimum) * 10) / 10 : 0;
  const y = (value: number) => ((value - lower) / (1 - lower)) * HEIGHT;
  const baseline = y(0);
  const ticks = [...(lower < 0 ? [lower] : []), 0, 0.25, 0.5, 0.75, 1].map(
    (value) => ({
      value,
      y: y(value),
      label: `${PERCENT.format(value * 100)} %`,
    }),
  );
  addAxes(scene, reported.length, ticks, baseline, "File size reduction");
  if (!reported.length) {
    scene.labels.push({
      id: "empty",
      position: [6, 2.35, 0],
      text: "Waiting for completed workbooks",
      size: 12,
      color: "#82999f",
    });
    scene.footer = "Relative reductions appear after a completion report.";
    return scene;
  }

  const animatedRatio = (datum: Datum & { cleanedBytes: number }) =>
    ratio(datum) *
    smooth((snapshot.time - (datum.completedAt ?? snapshot.time)) / 1.8);
  const allPoints = pointsForRun(
    reported.map((datum) => y(animatedRatio(datum))),
    0,
    reported.length,
  );
  const expanded: Point[] = [];
  // Insert the exact zero crossing. A segment that straddles the baseline must
  // not form a self-crossing quadrilateral when some sheets grow after cleanup.
  for (let i = 0; i < allPoints.length; i++) {
    const point = allPoints[i];
    if (i) {
      const previous = allPoints[i - 1];
      if ((previous.y - baseline) * (point.y - baseline) < 0) {
        const fraction = (baseline - previous.y) / (point.y - previous.y);
        expanded.push({
          x: previous.x + (point.x - previous.x) * fraction,
          y: baseline,
        });
      }
    }
    expanded.push(point);
  }
  const savings = filled(CLEAN_COLOR, 0.65);
  const growth = filled(GROWTH_COLOR, 0.68);
  const savingsOutline = outline("#a3dfc8");
  const growthOutline = outline("#ddb5b7");
  let run: Point[] = [];
  let sign = expanded[0].y >= baseline ? 1 : -1;
  const flushRatio = () =>
    appendStrip(
      sign >= 0 ? savings : growth,
      sign >= 0 ? savingsOutline : growthOutline,
      run,
      baseline,
      0.25,
    );
  for (let i = 0; i < expanded.length; i++) {
    const point = expanded[i];
    const nextSign = point.y === baseline ? sign : point.y > baseline ? 1 : -1;
    if (nextSign !== sign && run.length) {
      const previous = run[run.length - 1];
      flushRatio();
      run = [previous];
      sign = nextSign;
    }
    run.push(point);
  }
  flushRatio();
  if (savings.positions.length) scene.shapes.push(savings, savingsOutline);
  if (growth.positions.length) scene.shapes.push(growth, growthOutline);
  addHits(
    scene,
    reported,
    snapshot,
    (datum, index) => [
      [
        xAt(index, reported.length),
        y(animatedRatio(datum as Datum & { cleanedBytes: number })),
        0.25,
      ],
    ],
    CLEAN_COLOR,
  );
  scene.footer = `${reported.length.toLocaleString("en-US")} ${entityLabel} · sorted by relative reduction${minimum < 0 ? " · negative values = growth" : ""}`;
  return scene;
}
