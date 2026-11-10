import type {
  DemoRun,
  JobStyle,
  LabelMode,
  Snapshot,
  VisibleJob,
} from "./model";
import {
  fmtBytes,
  fmtTime,
  line,
  plane,
  polyline,
  shade,
  sheetColor,
  type SceneSpec,
  type Shape,
  type Vec3,
} from "./scene";

export interface WorkerSceneOptions {
  workUnit?: "worksheet" | "workbook";
  stepHeight: number;
  labelMode: LabelMode;
  jobStyle: JobStyle;
  windowSeconds: number;
  overview: boolean;
  selectedWorkbookId?: string | null;
  selectedSheetId?: string | null;
}

/** The time axis is continuous; only the view and its chronological Y origin move. */
export function buildWorkerScene(
  snapshot: Snapshot,
  run: DemoRun,
  options: WorkerSceneOptions,
): SceneSpec {
  const duration = Math.max(
    options.workUnit === "worksheet" ? 0.05 : 12,
    options.windowSeconds || 70,
  );
  const timeLabel = (seconds: number) =>
    duration < 2 ? `${(seconds * 1000).toFixed(1)} ms` : fmtTime(seconds);
  const leftTime = options.overview
    ? 0
    : Math.max(
        0,
        snapshot.time -
          duration * (options.workUnit === "worksheet" ? 1 : 0.86),
      );
  const rightTime = options.overview
    ? Math.max(snapshot.time * 1.035, 12)
    : leftTime + duration;
  const scaleX = 24 / (rightTime - leftTime);
  const leftX = leftTime * scaleX;
  const rightX = rightTime * scaleX;
  const lanePitch = 1.34;
  const laneWidth = 0.83;
  const depth = (run.workerCount - 1) * lanePitch + laneWidth;
  const step = Math.max(0, Math.min(3, options.stepHeight));
  const jobs = snapshot.jobs.filter(
    (job) => job.visibleEnd >= leftTime && job.startAt <= rightTime,
  );
  const jobBooks = new Set(jobs.map((job) => job.workbookId));
  const visibleBooks = snapshot.workbooks.filter(
    (book) =>
      book.status !== "queued" &&
      (book.completedAt === null ||
        book.completedAt >= leftTime ||
        jobBooks.has(book.id)),
  );
  const baselineIndex = visibleBooks.length
    ? Math.min(...visibleBooks.map((b) => b.index))
    : 0;
  const latestIndex = visibleBooks.length
    ? Math.max(...visibleBooks.map((b) => b.index))
    : baselineIndex;
  const maxY = Math.max(0.25, (latestIndex - baselineIndex) * step);
  const middleY = maxY * 0.38;
  const scene: SceneSpec = {
    shapes: [],
    labels: [],
    hits: [],
    target: [(leftX + rightX) / 2 - 0.15, middleY, depth / 2],
    span: Math.max(28, (depth * 0.74 + maxY * 0.7 + 3.1) * 1.8),
    // Keep the full time axis legible on wide dashboards without cropping lanes.
    horizontalSpan:
      options.workUnit === "worksheet"
        ? (24 * Math.cos(0.22) + depth * Math.sin(0.22)) * 1.08
        : undefined,
    yaw: 0.22,
    pitch: 0.82,
    footer: options.overview
      ? "Entire history so far"
      : `${timeLabel(leftTime)}–${timeLabel(rightTime)} · Camera follows the current time`,
  };
  const batches = new Map<string, Shape>();
  const add = (shape: Shape) => {
    // Job geometry is batched by appearance, rather than allocating one GPU object per job.
    const key = `${shape.kind || "triangles"}|${shape.color}|${shape.opacity ?? 1}|${!!shape.dashed}`;
    let batch = batches.get(key);
    if (!batch) {
      batch = {
        positions: [],
        indices: shape.kind === "lines" ? undefined : [],
        color: shape.color,
        opacity: shape.opacity,
        kind: shape.kind,
        dashed: shape.dashed,
      };
      batches.set(key, batch);
    }
    const offset = batch.positions.length / 3;
    batch.positions.push(...shape.positions);
    if (batch.indices) {
      if (shape.indices)
        for (const index of shape.indices) batch.indices.push(index + offset);
      else
        for (let i = 0; i < shape.positions.length / 3; i++)
          batch.indices.push(i + offset);
    }
  };

  // Quiet alternating lanes form a stable floor, independent of Workbook identity.
  for (let worker = 0; worker < run.workerCount; worker++) {
    const z = worker * lanePitch;
    add(
      plane(
        leftX,
        -0.055,
        z - 0.08,
        24,
        laneWidth + 0.16,
        worker % 2 ? "#162126" : "#18252a",
        0.88,
      ),
    );
    add(
      line(
        [leftX, -0.025, z + laneWidth + 0.19],
        [rightX, -0.025, z + laneWidth + 0.19],
        "#35464c",
        0.46,
      ),
    );
    const running = snapshot.activeJobs.some(
      (job) => job.workerIndex === worker,
    );
    scene.labels.push({
      id: `worker-${worker}`,
      position: [leftX - 0.48, 0.04, z + laneWidth / 2],
      text: `W${String(worker + 1).padStart(2, "0")}`,
      size: 10,
      color: running ? "#b1d9ce" : "#7a8c93",
      anchor: "right",
    });
    if (running)
      add(
        plane(
          leftX - 0.17,
          0.045,
          z + laneWidth / 2 - 0.05,
          0.075,
          0.1,
          "#9bd2bd",
          0.95,
        ),
      );
  }

  const ticks = niceTick((rightTime - leftTime) / 8);
  for (
    let t = Math.ceil(leftTime / ticks) * ticks;
    t <= rightTime + 1e-6;
    t += ticks
  ) {
    const x = t * scaleX;
    add(line([x, -0.012, -0.23], [x, -0.012, depth + 0.35], "#405259", 0.35));
    scene.labels.push({
      id: `time-${t}`,
      position: [x, -0.02, depth + 0.71],
      text: timeLabel(t),
      size: 9,
      muted: true,
      color: "#7a8c93",
    });
  }

  // The rear outline makes the tiny Y staircase legible without assigning a workbook colour.
  const orderedBooks =
    options.workUnit === "workbook"
      ? []
      : [...visibleBooks].sort((a, b) => a.index - b.index);
  const stair: Vec3[] = [];
  let previousY = 0;
  for (let bookNo = 0; bookNo < orderedBooks.length; bookNo++) {
    const book = orderedBooks[bookNo];
    const x = Math.max(leftX, Math.min(rightX, book.startAt * scaleX));
    const y = (book.index - baselineIndex) * step;
    if (stair.length) stair.push([x, previousY, -0.4]);
    stair.push([x, y, -0.4]);
    previousY = y;
    add(line([x, y, -0.22], [x, y, depth + 0.15], "#81969c", 0.17, true));
    if (step > 0.015)
      add(line([x, Math.max(0, y - step), -0.4], [x, y, -0.4], "#91abae", 0.7));
    // Limit book captions at overview scale; their complete names remain in job tooltips.
    const labelStride = Math.max(1, Math.ceil(orderedBooks.length / 5));
    if (bookNo % labelStride === 0 && x < rightX - 1.3) {
      scene.labels.push({
        id: `book-${book.id}`,
        position: [x + 0.12, y + 0.07, -0.77],
        text: `${String(book.index + 1).padStart(2, "0")} · ${shorten(book.name.replace(/\.xlsx$/i, ""), 18)}`,
        size: 9,
        color: "#8da0a5",
        anchor: "left",
      });
    }
  }
  if (stair.length) {
    stair.push([
      Math.min(rightX, snapshot.time * scaleX + 0.2),
      previousY,
      -0.4,
    ]);
    add(polyline(stair, "#92b6b2", 0.64));
  }

  const sortedForLabels = [...jobs].sort(
    (a, b) =>
      Number(a.done) - Number(b.done) ||
      b.visibleEnd - b.startAt - (a.visibleEnd - a.startAt),
  );
  const labelled = new Set(
    sortedForLabels.slice(0, options.overview ? 55 : 95).map((j) => j.id),
  );

  for (const job of jobs) {
    const x0 = Math.max(leftTime, job.startAt) * scaleX;
    const x1 = Math.min(rightTime, job.visibleEnd) * scaleX;
    if (x1 - x0 < 0.003) continue;
    const y = (job.workbookIndex - baselineIndex) * step + 0.045;
    const z = job.workerIndex * lanePitch;
    const colour = sheetColor(job.sheet.hue);
    const dimmed = options.selectedSheetId
      ? options.selectedSheetId !== job.sheetId
      : !!options.selectedWorkbookId &&
        options.selectedWorkbookId !== job.workbookId;
    const opacity = dimmed ? 0.15 : job.done ? 0.88 : 0.98;
    const tornLeft =
      job.chunkCount > 1 && job.chunkIndex > 0 && job.startAt >= leftTime;
    const tornRight = job.chunkCount > 1 && job.chunkIndex < job.chunkCount - 1;

    // In a corpus-wide view, subpixel teeth and braid segments only inflate buffers.
    // Keep each job's full extent, colour and hit target; restore detail in a time window.
    const overviewSubpixel = options.overview && x1 - x0 < 0.035;
    if (overviewSubpixel) {
      if (options.jobStyle === "threads")
        add(
          line(
            [x0, y + 0.07, z + laneWidth / 2],
            [x1, y + 0.07, z + laneWidth / 2],
            colour,
            opacity,
          ),
        );
      else add(plane(x0, y, z, x1 - x0, laneWidth, colour, opacity));
    } else if (options.jobStyle === "threads") {
      drawThread(
        add,
        x0,
        x1,
        y,
        z + laneWidth / 2,
        colour,
        opacity,
        job,
        tornLeft,
        tornRight,
      );
    } else {
      drawJob(
        add,
        x0,
        x1,
        y,
        z,
        laneWidth,
        colour,
        opacity,
        tornLeft,
        tornRight,
      );
    }

    if (!job.done && !dimmed) {
      const pulse = 0.7 + 0.25 * Math.sin(snapshot.time * 5);
      add(
        line(
          [x1, y + 0.045, z - 0.025],
          [x1, y + 0.045, z + laneWidth + 0.025],
          "#e3f8ee",
          pulse,
        ),
      );
      add(
        line(
          [x1 - 0.05, y + 0.06, z + laneWidth / 2],
          [x1 + 0.05, y + 0.06, z + laneWidth / 2],
          "#ffffff",
          0.95,
        ),
      );
    }

    const width = x1 - x0;
    if (
      !dimmed &&
      labelled.has(job.id) &&
      width >= (options.jobStyle === "threads" ? 1.35 : 0.93)
    ) {
      const name =
        options.labelMode === "xml"
          ? `sheet${job.sheet.index}.xml`
          : job.sheet.name;
      const phase = taskLabel(job);
      const budget = Math.max(5, Math.floor(width * 7.3));
      scene.labels.push({
        id: `job-label-${job.id}`,
        position: [
          (x0 + x1) / 2,
          y + (options.jobStyle === "threads" ? 0.18 : 0.025),
          z + laneWidth / 2,
        ],
        text:
          options.workUnit === "workbook"
            ? shorten(name, budget)
            : `${job.sheet.indexLabel ?? `S${job.sheet.index}`} · ${shorten(name, Math.max(5, budget - phase.length - 8))} · ${phase}`,
        size: options.jobStyle === "threads" ? 9 : 10,
        color: options.jobStyle === "threads" ? "#d7e6e3" : "#16292a",
        surface: { width: width * 0.92, height: laneWidth * 0.7 },
      });
    }
    scene.hits.push({
      id: job.id,
      workbookId: job.workbookId,
      sheetId: job.sheetId,
      position: [(x0 + x1) / 2, y + 0.03, z + laneWidth / 2],
      radius: Math.max(0.36, Math.min(1.3, width / 2)),
      title: job.sheet.name,
      color: colour,
      get lines() {
        return [
          options.workUnit === "workbook"
            ? job.workbookName
            : `${job.workbookName} · sheet${job.sheet.index}.xml`,
          `Worker ${job.workerIndex + 1} · ${taskLabel(job)}`,
          `${fmtTime(job.startAt)} → ${fmtTime(job.visibleEnd)} · ${(job.visibleEnd - job.startAt).toFixed(1)} s${job.done ? " · completed" : " · running"}`,
          `${options.workUnit === "workbook" ? "Workbook" : "Worksheet"} before: ${fmtBytes(job.sheet.originalBytes)}`,
          ...(job.sheet.cleanedBytes === null
            ? ["Output report pending"]
            : [`After: ${fmtBytes(job.sheet.cleanedBytes)}`]),
        ];
      },
    });
  }

  const cursorX = Math.max(leftX, Math.min(rightX, snapshot.time * scaleX));
  const curtainTop = maxY + 0.65;
  add({
    positions: [
      cursorX,
      0,
      -0.25,
      cursorX,
      curtainTop,
      -0.25,
      cursorX,
      curtainTop,
      depth + 0.25,
      cursorX,
      0,
      depth + 0.25,
    ],
    indices: [0, 1, 2, 0, 2, 3],
    color: "#a8d7c5",
    opacity: 0.045,
  });
  add(
    line(
      [cursorX, 0.012, -0.3],
      [cursorX, 0.012, depth + 0.27],
      "#b5e0cb",
      0.82,
    ),
  );
  add(
    line(
      [cursorX, 0, -0.3],
      [cursorX, curtainTop, -0.3],
      "#add5c5",
      0.55,
      true,
    ),
  );
  add(
    line(
      [cursorX, 0, depth + 0.27],
      [cursorX, curtainTop, depth + 0.27],
      "#add5c5",
      0.55,
      true,
    ),
  );
  scene.labels.push({
    id: "cursor-time",
    position: [cursorX, curtainTop + 0.2, depth + 0.3],
    text: timeLabel(snapshot.time),
    size: 10,
    color: "#c8eddb",
    pill: true,
  });

  // Explicit axis mapping stays stable when camera orbit or the rendering backend changes.
  const axisX = leftX - 0.7;
  const axisZ = depth + 1.23;
  add(line([axisX, 0, axisZ], [axisX + 2.7, 0, axisZ], "#6a858a", 0.72));
  add(line([axisX, 0, axisZ], [axisX, 0, axisZ - 1.3], "#6a858a", 0.72));
  add(line([axisX, 0, axisZ], [axisX, 1.08, axisZ], "#6a858a", 0.72));
  scene.labels.push(
    {
      id: "axis-time",
      position: [axisX + 3.05, 0, axisZ],
      text: "X · Time",
      size: 9,
      color: "#8ba0a7",
      anchor: "left",
    },
    {
      id: "axis-worker",
      position: [axisX - 0.02, 0, axisZ - 1.55],
      text: "Z · Worker",
      size: 9,
      color: "#8ba0a7",
      anchor: "right",
    },
    ...(options.workUnit === "workbook"
      ? []
      : [
          {
            id: "axis-books",
            position: [axisX, 1.35, axisZ] as Vec3,
            text: "Y · Workbook",
            size: 9,
            color: "#8ba0a7",
            anchor: "right" as const,
          },
        ]),
  );
  scene.shapes = [...batches.values()];
  return scene;
}

function taskLabel(job: VisibleJob): string {
  if (job.phase === "initial") return "Initial scan";
  if (job.phase === "finalize") return "Finalize";
  if (job.phase === "chunk" || job.chunkCount > 1)
    return `Chunk ${job.chunkIndex + 1}/${job.chunkCount}`;
  return "Whole range";
}

function shorten(value: string, max: number) {
  return value.length <= max
    ? value
    : value.slice(0, Math.max(1, max - 1)) + "…";
}
function niceTick(value: number) {
  const p = 10 ** Math.floor(Math.log10(Math.max(0.1, value)));
  const v = value / p;
  return (v <= 1 ? 1 : v <= 2 ? 2 : v <= 5 ? 5 : 10) * p;
}

/** Paired left/right edge vertices tessellate even two torn ends without a concave fan. */
function drawJob(
  add: (shape: Shape) => void,
  x0: number,
  x1: number,
  y: number,
  z: number,
  depth: number,
  color: string,
  opacity: number,
  tornLeft: boolean,
  tornRight: boolean,
) {
  const n = tornLeft || tornRight ? 10 : 1;
  const tooth = Math.min(0.14, (x1 - x0) * 0.2);
  const positions: number[] = [];
  const indices: number[] = [];
  const left: Vec3[] = [],
    right: Vec3[] = [];
  for (let i = 0; i <= n; i++) {
    const zz = z + (i / n) * depth;
    const bite = i % 2 ? tooth : 0;
    const a: Vec3 = [x0 + (tornLeft ? bite : 0), y, zz];
    const b: Vec3 = [x1 - (tornRight ? bite : 0), y, zz];
    left.push(a);
    right.push(b);
    positions.push(...a, ...b);
    if (i < n) {
      const k = i * 2;
      indices.push(k, k + 1, k + 2, k + 1, k + 3, k + 2);
    }
  }
  add({ positions, indices, color, opacity });
  // Shallow front side gives a precise paper-strip thickness; y still encodes the workbook.
  add({
    positions: [
      x0,
      y,
      z + depth,
      x1,
      y,
      z + depth,
      x1,
      y - 0.047,
      z + depth,
      x0,
      y - 0.047,
      z + depth,
    ],
    indices: [0, 1, 2, 0, 2, 3],
    color: shade(color, 0.67),
    opacity,
  });
  add(polyline(left, shade(color, 0.75), opacity * 0.72));
  add(polyline(right, shade(color, 0.75), opacity * 0.72));
  add(
    line(
      [x0, y + 0.005, z],
      [x1, y + 0.005, z],
      shade(color, 1.12),
      opacity * 0.65,
    ),
  );
}

/** Three lightly braided ribbons evoke actual concurrent strands and remain low-poly. */
function drawThread(
  add: (shape: Shape) => void,
  x0: number,
  x1: number,
  y: number,
  z: number,
  color: string,
  opacity: number,
  job: VisibleJob,
  tornLeft: boolean,
  tornRight: boolean,
) {
  const length = x1 - x0;
  const steps = Math.max(4, Math.min(34, Math.ceil(length * 7)));
  for (let strand = 0; strand < 3; strand++) {
    const positions: number[] = [],
      indices: number[] = [];
    const strandColor =
      strand === 0
        ? shade(color, 0.78)
        : strand === 2
          ? shade(color, 1.08)
          : color;
    for (let i = 0; i <= steps; i++) {
      const x = x0 + (length * i) / steps;
      const phase =
        x * 2.8 + (strand * Math.PI * 2) / 3 + job.workerIndex * 0.55;
      const zz = z + Math.sin(phase) * 0.16;
      const yy = y + 0.07 + Math.cos(phase) * 0.065;
      positions.push(x, yy, zz - 0.057, x, yy, zz + 0.057);
      if (i < steps) {
        const k = i * 2;
        indices.push(k, k + 1, k + 2, k + 1, k + 3, k + 2);
      }
    }
    add({ positions, indices, color: strandColor, opacity });
  }
  // A solid collar denotes the worksheet boundary. Fine open ends denote continuing chunks.
  for (const [x, torn] of [
    [x0, tornLeft],
    [x1, tornRight],
  ] as [number, boolean][]) {
    if (!torn) {
      add({
        positions: [
          x - 0.045,
          y + 0.08,
          z - 0.23,
          x + 0.045,
          y + 0.08,
          z - 0.23,
          x + 0.045,
          y + 0.08,
          z + 0.23,
          x - 0.045,
          y + 0.08,
          z + 0.23,
        ],
        indices: [0, 1, 2, 0, 2, 3],
        color: shade(color, 1.18),
        opacity,
      });
    } else {
      for (let f = -1; f <= 1; f++)
        add(
          line(
            [x, y + 0.09, z + f * 0.13],
            [x + (x === x0 ? -0.11 : 0.11), y + 0.09, z + f * 0.21],
            color,
            opacity * 0.75,
          ),
        );
    }
  }
}
