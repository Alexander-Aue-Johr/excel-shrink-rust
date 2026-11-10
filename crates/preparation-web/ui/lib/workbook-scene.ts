import type {
  LabelMode,
  ResultStyle,
  SheetState,
  Snapshot,
  WorkbookState,
} from "./model";
import {
  boxEdges,
  fmtBytes,
  line,
  plane,
  shade,
  sheetColor,
  type SceneSpec,
  type Shape,
  type Vec3,
} from "./scene";

export type AtlasView = "recent" | "all";
export type AtlasSort = "time" | "reduction" | "size";
export interface AtlasSelectionOptions {
  view: AtlasView;
  sort: AtlasSort;
  recentCount?: number;
  focusWorkbookId?: string | null;
}

export interface WorkbookSceneOptions extends AtlasSelectionOptions {
  limit?: number;
  resultStyle: ResultStyle;
  labelMode: LabelMode;
  selectedWorkbookId?: string | null;
  selectedSheetId?: string | null;
}

const FRAME = "#71939a";
const MINT = "#8acdb5";
const ROSE = "#c996a4";
const FLOOR_Y = 0.025;
// One fixed square per Worksheet. Workbook width follows the observed sheet count,
// while depth stays constant: neither sheets nor workbooks ever wrap into Z rows.
const SHEET_SQUARE = 0.34;
const SHEET_PITCH = 0.44;
const INNER_PADDING = 0.28;
const BASE_D = SHEET_SQUARE + 2 * INNER_PADDING;
const BOOK_GAP = 1.05;
const UNKNOWN_WIDTH = 1.9;
const ORIGINAL_OFFSET = -0.82;
const CLEAN_OFFSET = 0.98;
const PERCENT = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

function chronological(a: WorkbookState, b: WorkbookState): number {
  return a.startAt - b.startAt || a.index - b.index || a.id.localeCompare(b.id);
}

function reportedReduction(book: WorkbookState): number | null {
  if (
    book.status !== "complete" ||
    book.completedAt === null ||
    book.originalBytes === null ||
    book.cleanedBytes === null
  )
    return null;
  return (
    (book.originalBytes - book.cleanedBytes) / Math.max(1, book.originalBytes)
  );
}

/** Select from the current historical snapshot, never from the future demo plan. */
export function getAtlasBooks(
  snapshot: Snapshot,
  options: AtlasSelectionOptions,
): WorkbookState[] {
  const books = [...snapshot.workbooks].sort(chronological);
  if (options.view === "recent") {
    const started = books.filter((book) => book.startAt <= snapshot.time);
    const count = Number.isFinite(options.recentCount)
      ? Math.max(1, Math.floor(options.recentCount!))
      : 3;
    const recent = started.length ? started.slice(-count) : books.slice(0, 1);
    const focused = books.find((book) => book.id === options.focusWorkbookId);
    if (focused && !recent.includes(focused)) {
      return [...recent.slice(1), focused].sort(chronological);
    }
    return recent;
  }
  if (options.sort === "reduction") {
    books.sort((a, b) => {
      const left = reportedReduction(a),
        right = reportedReduction(b);
      if (left === null || right === null)
        return left === right ? chronological(a, b) : left === null ? 1 : -1;
      return right - left || chronological(a, b);
    });
  }
  if (options.sort === "size") {
    books.sort(
      (a, b) =>
        (b.originalBytes ?? 0) - (a.originalBytes ?? 0) ||
        a.id.localeCompare(b.id),
    );
  }
  return books;
}

function workbookWidth(book: WorkbookState): number {
  return book.known
    ? 2 * INNER_PADDING +
        SHEET_SQUARE +
        Math.max(0, book.sheets.length - 1) * SHEET_PITCH
    : UNKNOWN_WIDTH;
}

/** Page only workbooks observed at the current replay time, largest first. */
export function workbookPage(
  snapshot: Snapshot,
  requestedPage: number,
  pageSize: number,
) {
  const books = getAtlasBooks(snapshot, { view: "all", sort: "size" });
  const size = Math.max(1, Math.min(24, Math.floor(pageSize)));
  const lastPage = Math.max(0, Math.ceil(books.length / size) - 1);
  const page = Math.max(0, Math.min(lastPage, Math.floor(requestedPage)));
  return {
    page,
    total: books.length,
    workbooks: books.slice(page * size, (page + 1) * size),
  };
}

function percentage(original: number, cleaned: number): string {
  const saved = original > 0 ? (1 - cleaned / original) * 100 : 0;
  return `${saved < 0 ? "+" : "−"}${PERCENT.format(Math.abs(saved))} %`;
}

function stateText(book: WorkbookState): string {
  if (book.status === "complete") return "Report received";
  if (book.status === "finalizing") return "Generating report";
  if (book.status === "processing")
    return `Cleanup · ${Math.round(book.progress * 100)} %`;
  return "Waiting for metadata";
}

/** Merge the many small column faces into a handful of indexed colour batches. */
function batchShapes(shapes: Shape[]): Shape[] {
  const batches = new Map<string, Shape>();
  for (const shape of shapes) {
    const key = `${shape.kind ?? "triangles"}:${shape.color}:${shape.opacity ?? 1}:${shape.dashed ?? false}`;
    let batch = batches.get(key);
    if (!batch) {
      batch = {
        positions: [],
        color: shape.color,
        opacity: shape.opacity,
        kind: shape.kind,
        dashed: shape.dashed,
      };
      if (shape.kind !== "lines") batch.indices = [];
      batches.set(key, batch);
    }
    const offset = batch.positions.length / 3;
    batch.positions.push(...shape.positions);
    if (batch.indices) {
      const indices =
        shape.indices ??
        Array.from({ length: shape.positions.length / 3 }, (_, index) => index);
      for (const index of indices) batch.indices.push(index + offset);
    }
  }
  return [...batches.values()];
}

export function buildWorkbookScene(
  snapshot: Snapshot,
  options: WorkbookSceneOptions,
): SceneSpec {
  const split = options.resultStyle === "split";
  const books = getAtlasBooks(snapshot, options).slice(
    0,
    options.limit ?? Infinity,
  );
  const focused = snapshot.workbooks.find(
    (book) => book.id === options.focusWorkbookId,
  );
  if (focused && !books.includes(focused) && books.length)
    books.splice(books.length - 1, 1, focused);
  const visible = new Set(books.map((book) => book.id));
  // Recent view travels along the same absolute X row used by the time overview.
  // Hidden/future workbooks contribute only their already-public placeholder width.
  const ordered = options.selectedSheetId
    ? books
    : getAtlasBooks(snapshot, {
        view: "all",
        sort: options.view === "recent" ? "time" : options.sort,
      });
  let nextX = 0;
  const layout = ordered
    .map((book) => {
      const width = workbookWidth(book);
      const entry = { book, x: nextX, width };
      nextX += width + BOOK_GAP;
      return entry;
    })
    .filter((entry) => visible.has(entry.book.id));
  const left = layout[0]?.x ?? 0;
  const right = layout.length
    ? layout[layout.length - 1].x + layout[layout.length - 1].width
    : UNKNOWN_WIDTH;
  const width = right - left;
  const largestKnown = Math.max(
    1,
    ...books.map((book) =>
      Math.max(book.originalBytes ?? 0, book.cleanedBytes ?? 0),
    ),
  );
  const byteScale = 3.35 / largestKnown;
  const shapes: Shape[] = [];
  const result: SceneSpec = {
    shapes,
    labels: [],
    hits: [],
    target: [
      (left + right) / 2,
      split ? 0.9 : 1.25,
      BASE_D / 2 + (split ? 0.12 : 0),
    ],
    // Wide pages also need room for the vertical component of the tilted X axis.
    span: Math.max(10.8, width * 0.2 + 6),
    horizontalSpan: Math.max(10.8, width + 3.6) * 1.06,
    yaw: 0.13,
    pitch: 0.74,
    footer: `One row · worksheets by XML index · ${options.view === "recent" ? "most recently started workbooks at the current time" : options.sort === "reduction" ? "largest relative reduction first; pending reports last" : "Workbooks by start time"}`,
  };

  // The grid follows the visible interval and stays bounded even for 800 books.
  const zMin = split ? ORIGINAL_OFFSET - 0.35 : -0.35;
  const zMax = BASE_D + (split ? CLEAN_OFFSET + 0.4 : 0.4);
  const gridStep = Math.max(0.65, Math.ceil(width / (160 * 0.65)) * 0.65);
  for (
    let x = Math.floor((left - 0.7) / gridStep) * gridStep;
    x <= right + 0.7;
    x += gridStep
  ) {
    shapes.push(line([x, -0.015, zMin], [x, -0.015, zMax], "#25383e", 0.34));
  }
  for (let z = zMin; z <= zMax; z += 0.65) {
    shapes.push(
      line([left - 0.7, -0.015, z], [right + 0.7, -0.015, z], "#25383e", 0.34),
    );
  }

  if (snapshot.knownBooks > 0) {
    const axisX = left - 0.8;
    const axis: Vec3 = [axisX, FLOOR_Y, -0.1];
    shapes.push(line(axis, [axisX, FLOOR_Y + 3.35, -0.1], "#536b74", 0.65));
    for (const fraction of [0, 0.5, 1]) {
      const y = FLOOR_Y + 3.35 * fraction;
      shapes.push(
        line([axisX - 0.08, y, -0.1], [axisX + 0.08, y, -0.1], "#718993", 0.8),
      );
      result.labels.push({
        id: `atlas-scale-${fraction}`,
        position: [axisX - 0.23, y, -0.1],
        text: fmtBytes(largestKnown * fraction),
        muted: true,
        size: 9,
        anchor: "right",
      });
    }
  }

  function addWorkbookFrame(
    x: number,
    z: number,
    width: number,
    height: number,
    color: string,
    opacity: number,
    dashed: boolean,
  ) {
    const h = Math.max(0.012, height);
    shapes.push(
      boxEdges(x, FLOOR_Y, z, width, h, BASE_D, color, opacity, dashed),
    );
    shapes.push(
      plane(x, FLOOR_Y + h, z, width, BASE_D, color, opacity * 0.045),
    );
  }

  function addSheetColumn(
    sheet: SheetState,
    x: number,
    z: number,
    square: number,
    height: number,
    opacity: number,
    outline = false,
  ) {
    if (options.selectedSheetId && options.selectedSheetId !== sheet.id)
      opacity *= 0.18;
    const segments = Math.max(1, sheet.chunkCount);
    if (outline) {
      shapes.push(
        boxEdges(
          x,
          FLOOR_Y,
          z,
          square,
          Math.max(0.002, height),
          square,
          sheetColor(sheet.hue),
          opacity,
          true,
        ),
      );
      if (segments > 1) {
        for (let segment = 1; segment < segments; segment++) {
          const y = FLOOR_Y + (height * segment) / segments;
          const color = sheetColor(sheet.hue);
          shapes.push(
            line([x, y, z], [x + square, y, z], color, opacity * 0.65, true),
          );
          shapes.push(
            line(
              [x + square, y, z],
              [x + square, y, z + square],
              color,
              opacity * 0.65,
              true,
            ),
          );
        }
      }
      return;
    }
    if (height <= 0.00001) {
      shapes.push(
        plane(x, FLOOR_Y, z, square, square, sheetColor(sheet.hue), opacity),
      );
      return;
    }
    for (let segment = 0; segment < segments; segment++) {
      const y = FLOOR_Y + (height * segment) / segments;
      const top = FLOOR_Y + (height * (segment + 1)) / segments;
      const color = sheetColor(sheet.hue);
      // Two pairs of vertical faces; inner chunk ceilings are invisible and omitted.
      // Each face pair uses eight corners; the final top closes the complete
      // column once with four vertices instead of a box's redundant eight.
      shapes.push({
        positions: [
          x,
          y,
          z,
          x + square,
          y,
          z,
          x + square,
          top,
          z,
          x,
          top,
          z,
          x,
          y,
          z + square,
          x + square,
          y,
          z + square,
          x + square,
          top,
          z + square,
          x,
          top,
          z + square,
        ],
        indices: [0, 1, 2, 0, 2, 3, 4, 7, 6, 4, 6, 5],
        color: shade(color, 0.77),
        opacity,
      });
      shapes.push({
        positions: [
          x,
          y,
          z,
          x,
          top,
          z,
          x,
          top,
          z + square,
          x,
          y,
          z + square,
          x + square,
          y,
          z,
          x + square,
          y,
          z + square,
          x + square,
          top,
          z + square,
          x + square,
          top,
          z,
        ],
        indices: [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
        color: shade(color, 0.88),
        opacity,
      });
      if (segment === segments - 1)
        shapes.push(plane(x, top, z, square, square, color, opacity));
    }
  }

  layout.forEach(({ book, x, width }) => {
    const z = 0;
    const selected = book.id === options.selectedWorkbookId;
    const completed =
      book.status === "complete" &&
      book.completedAt !== null &&
      book.cleanedBytes !== null;
    const transition = completed ? 1 : 0;
    const reveal = book.known ? 1 : 0;
    const originalBytes = book.originalBytes ?? 0;
    const cleanedBytes = completed ? book.cleanedBytes! : originalBytes;
    const originalH = originalBytes * byteScale;
    const cleanedH = cleanedBytes * byteScale;
    const currentH = (originalH + (cleanedH - originalH) * transition) * reveal;
    const beforeZ = z + (split ? ORIGINAL_OFFSET * transition : 0);
    const afterZ = z + (split ? CLEAN_OFFSET * transition : 0);
    const currentColor = completed
      ? cleanedBytes > originalBytes
        ? ROSE
        : MINT
      : FRAME;
    const titleZ = z + BASE_D + (split ? CLEAN_OFFSET + 0.4 : 0.39);

    shapes.push(
      plane(
        x - 0.07,
        0,
        z - 0.07,
        width + 0.14,
        BASE_D + 0.14,
        selected ? "#29484b" : "#1c2c32",
        0.92,
      ),
    );
    shapes.push(
      boxEdges(
        x - 0.07,
        0,
        z - 0.07,
        width + 0.14,
        0.018,
        BASE_D + 0.14,
        selected ? MINT : "#365058",
        selected ? 0.75 : 0.5,
        !selected,
      ),
    );

    result.labels.push({
      id: `book-name-${book.id}`,
      position: [x + width / 2, FLOOR_Y, titleZ],
      text: book.name,
      color: selected ? "#d9eee7" : "#c6d6d8",
      size: 11,
    });
    const reportText = !book.known
      ? "Metadaten ausstehend"
      : completed
        ? `${fmtBytes(originalBytes)} → ${fmtBytes(cleanedBytes)} · ${percentage(originalBytes, cleanedBytes)}`
        : `${fmtBytes(originalBytes)} · ${stateText(book)}`;
    result.labels.push({
      id: `book-state-${book.id}`,
      position: [x + width / 2, FLOOR_Y - 0.38, titleZ],
      text: reportText,
      color: completed ? currentColor : "#7e979f",
      size: 9,
    });
    result.hits.push({
      id: `book:${book.id}`,
      workbookId: book.id,
      position: [x + width / 2, FLOOR_Y + currentH + 0.08, afterZ + BASE_D / 2],
      radius: 32,
      title: book.name,
      get lines() {
        return book.known
          ? [
              stateText(book),
              `Workbook original: ${fmtBytes(originalBytes)}`,
              ...(completed
                ? [
                    `Optimized workbook: ${fmtBytes(cleanedBytes)}`,
                    `Change: ${percentage(originalBytes, cleanedBytes)}`,
                  ]
                : ["Output available after the complete workbook report"]),
              `${book.sheets.length} Worksheets · shared linear scale`,
            ]
          : ["Queued", "File and worksheet sizes are not yet available."];
      },
      color: currentColor,
    });

    if (!book.known) {
      result.labels.push({
        id: `book-wait-${book.id}`,
        position: [x + width / 2, FLOOR_Y + 0.06, z + BASE_D / 2],
        text: `#${book.index + 1} · wartet`,
        color: "#526e78",
        size: 10,
      });
      return;
    }

    // Wire envelopes encode aggregate Workbook bytes; the contained columns retain exactly
    // the same byte scale. No per-card height normalization or log scaling is used.
    if (split && completed) {
      shapes.push(
        plane(
          x,
          FLOOR_Y - 0.008,
          beforeZ,
          width,
          BASE_D,
          "#203038",
          0.8 * transition,
        ),
      );
      shapes.push(
        plane(
          x,
          FLOOR_Y - 0.004,
          afterZ,
          width,
          BASE_D,
          "#213b37",
          0.65 * transition,
        ),
      );
      addWorkbookFrame(
        x,
        beforeZ,
        width,
        originalH * reveal,
        FRAME,
        0.25 + transition * 0.15,
        true,
      );
      addWorkbookFrame(x, afterZ, width, currentH, currentColor, 0.62, false);
      if (transition > 0.12) {
        shapes.push(
          line(
            [x - 0.05, FLOOR_Y, beforeZ + BASE_D / 2],
            [x - 0.05, FLOOR_Y, afterZ + BASE_D / 2],
            "#6e9c92",
            0.36,
            true,
          ),
        );
        result.labels.push({
          id: `book-original-${book.id}`,
          position: [x + width / 2, originalH + 0.19, beforeZ],
          text: "ORIGINAL",
          color: "#6e8b97",
          size: 8,
        });
        result.labels.push({
          id: `book-clean-${book.id}`,
          position: [x + width / 2, currentH + 0.19, afterZ + BASE_D],
          text: "OPTIMIZED",
          color: currentColor,
          size: 8,
        });
      }
    } else {
      addWorkbookFrame(
        x,
        z,
        width,
        originalH * reveal,
        FRAME,
        completed ? 0.33 : 0.5,
        true,
      );
      if (completed)
        addWorkbookFrame(x, z, width, currentH, currentColor, 0.62, false);
    }

    const sortedSheets = [...book.sheets].sort((a, b) => a.index - b.index);
    const square = SHEET_SQUARE;
    const xPadding = INNER_PADDING;
    const zPadding = INNER_PADDING;

    sortedSheets.forEach((sheet, sheetIndex) => {
      const sx = x + xPadding + sheetIndex * SHEET_PITCH;
      const sz = zPadding;
      const sh = sheet.originalBytes * byteScale * reveal;
      const finalSheetBytes =
        completed && sheet.cleanedBytes !== null
          ? sheet.cleanedBytes
          : sheet.originalBytes;
      const ch =
        (sheet.originalBytes +
          (finalSheetBytes - sheet.originalBytes) * transition) *
        byteScale *
        reveal;
      const color = sheetColor(sheet.hue);

      if (split && completed) {
        addSheetColumn(
          sheet,
          sx,
          beforeZ + sz,
          square,
          sh,
          0.16 + transition * 0.2,
        );
        addSheetColumn(sheet, sx, afterZ + sz, square, ch, 0.9);
      } else {
        if (completed)
          addSheetColumn(
            sheet,
            sx,
            z + sz,
            square,
            sh,
            0.38 * transition,
            true,
          );
        addSheetColumn(sheet, sx, z + sz, square, ch, 0.94);
      }

      const label =
        options.labelMode === "xml" ? `sheet${sheet.index}.xml` : sheet.name;
      result.hits.push({
        id: `sheet:${sheet.id}`,
        workbookId: book.id,
        sheetId: sheet.id,
        position: [
          sx + square / 2,
          FLOOR_Y + ch + 0.03,
          afterZ + sz + square / 2,
        ],
        radius: Math.max(10, Math.min(23, square * 24)),
        title: label,
        get lines() {
          return [
            book.name,
            `${sheet.name} · xl/worksheets/sheet${sheet.index}.xml`,
            `Original: ${fmtBytes(sheet.originalBytes)}`,
            ...(completed && sheet.cleanedBytes !== null
              ? [
                  `Optimized: ${fmtBytes(sheet.cleanedBytes)} · ${percentage(sheet.originalBytes, sheet.cleanedBytes)}`,
                ]
              : ["Optimized size: report pending"]),
            sheet.chunkCount > 1
              ? `${sheet.chunkCount} equal color segments · chunks ordered bottom to top`
              : "Unsplit worksheet",
          ];
        },
        color,
      });

      // Detailed labels are reserved for the selected Workbook, keeping the overview legible.
      if (
        selected &&
        sortedSheets.length <= 18 &&
        (!options.selectedSheetId || options.selectedSheetId === sheet.id)
      ) {
        result.labels.push({
          id: `sheet-label-${sheet.id}`,
          position: [
            sx + square / 2,
            FLOOR_Y + ch + 0.13,
            afterZ + sz + square / 2,
          ],
          text: label,
          color,
          size: 8,
        });
      }
    });

    if (sortedSheets.length > 0) {
      result.labels.push({
        id: `book-sheet-order-${book.id}`,
        position: [x + width / 2, FLOOR_Y + 0.01, beforeZ - 0.22],
        text: `${sortedSheets.length} Worksheets · ${options.labelMode === "xml" ? `sheet${sortedSheets[0].index}.xml → sheet${sortedSheets[sortedSheets.length - 1].index}.xml` : "by XML index"}`,
        color: "#718a93",
        size: 8,
      });
    }
  });

  result.shapes = batchShapes(shapes);
  // Small worksheet columns stay selectable even when a strongly shrunken envelope overlaps.
  result.hits.sort(
    (a, b) =>
      Number(a.id.startsWith("book:")) - Number(b.id.startsWith("book:")),
  );
  return result;
}
