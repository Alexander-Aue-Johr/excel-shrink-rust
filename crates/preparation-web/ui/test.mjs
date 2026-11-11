import { build } from "esbuild";
import { test } from "node:test";
import assert from "node:assert/strict";
import { gzipSync } from "node:zlib";
import { createHash } from "node:crypto";

const result = await build({
  entryPoints: ["live.ts"],
  bundle: true,
  write: false,
  platform: "node",
  format: "esm",
});
const { applyUpdate, applyTasks, scenes, atlasSnapshot } = await import(
  `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].contents).toString("base64")}`
);

const packageSize = (fileBytes, sheetBytes) => ({
  file_bytes: fileBytes,
  worksheets: [
    {
      part: "xl/worksheets/sheet1.xml",
      uncompressed_bytes: sheetBytes,
      compressed_bytes: 5,
    },
  ],
});
const base = {
  run_id: "one",
  operation: "check-performance",
  elapsed_ms: 2000,
  defaults: { workers: 2 },
  counts: {
    prepared: 1,
    failed: 0,
    total: 1,
    before_bytes: 30,
    after_bytes: 20,
  },
  history_limit: 256,
};

test("replay seeking uses recorded events without future metadata or reports", async () => {
  const bundle = await build({
    entryPoints: ["lib/replay.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { replayAt, createReplay } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
  );
  const event = (time, type, payload = {}) => ({
    elapsed_millis: time,
    event_type: type,
    workbook_id: "workbook-0",
    worker_index: 0,
    payload,
  });
  const original = packageSize(30, 1000),
    output = packageSize(10, 100);
  const events = [
    event(10, "job.started", { name: "test.xlsx" }),
    event(20, "workbook.metadata", { package: original }),
    event(100, "work.started", {
      task_id: 1,
      sheet: "xl/worksheets/sheet1.xml",
      phase: "chunk",
      chunk_count: 2,
    }),
    event(900, "work.finished", { task_id: 1 }),
    event(1500, "job.completed", {
      archive_sizes: {
        before_optimization: original,
        after_optimization: output,
      },
    }),
  ];
  const early = replayAt(base, events, 500);
  assert.equal(early.jobs[0].after, null);
  assert.equal(early.tasks[0].ended_ms, null);
  assert.equal(early.update.counts.prepared, 0);
  const complete = replayAt(base, events, 2000);
  assert.equal(complete.jobs[0].after.file_bytes, 10);
  assert.equal(complete.update.counts.prepared, 1);
  assert.equal(replayAt(base, events, 0).jobs.length, 0);
  assert.equal(events.length, 5);
  const seek = createReplay(base, events);
  for (const time of [0, 100, 500, 900, 2000, 500, 500, 0, 2000]) {
    const actual = seek(time);
    const expected = replayAt(base, events, time);
    assert.deepEqual(actual.jobs, expected.jobs);
    assert.deepEqual(actual.tasks, expected.tasks);
    assert.deepEqual(actual.update.counts, expected.update.counts);
  }
  const completedSnapshot = seek(2000);
  const unchanged = seek(2000);
  assert.equal(unchanged.update.events.length, 0);
  assert.equal(unchanged.jobs, completedSnapshot.jobs);
  assert.equal(unchanged.tasks, completedSnapshot.tasks);
  seek(0);
  assert.equal(completedSnapshot.jobs[0].after.file_bytes, 10);
  const manyTasks = Array.from({ length: 4200 }, (_, index) => [
    event(100, "work.started", {
      task_id: index,
      sheet: "xl/worksheets/sheet1.xml",
      phase: "chunk",
    }),
    event(200, "work.finished", { task_id: index }),
  ]).flat();
  assert.equal(replayAt(base, manyTasks, 1000).tasks.length, 4200);
});

test("worksheet search pages retain access to every sheet and the pinned selection", async () => {
  const bundle = await build({
    entryPoints: ["lib/worksheet-options.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { worksheetOptions } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
  );
  const snapshot = {
    workbooks: [{ id: "book", name: "Corpus.xlsx" }],
    sheets: Array.from({ length: 33059 }, (_, index) => ({
      id: `sheet-${index}`,
      workbookId: "book",
      name: `Data ${index}`,
    })),
  };
  const seen = new Set();
  for (let page = 0; page < 331; page += 1) {
    const result = worksheetOptions(snapshot, "", page, null);
    assert.ok(result.sheets.length <= 100);
    result.sheets.forEach((sheet) => seen.add(sheet.id));
  }
  assert.equal(seen.size, snapshot.sheets.length);
  assert.equal(worksheetOptions(snapshot, "CORPUS.XLSX", 0, null).total, 33059);
  const pinned = worksheetOptions(snapshot, "Data 33058", 0, "sheet-0");
  assert.deepEqual(
    pinned.sheets.map((sheet) => sheet.id),
    ["sheet-0", "sheet-33058"],
  );
  assert.equal(pinned.total, 1);
  const empty = worksheetOptions(snapshot, "missing", 999, "absent");
  assert.equal(empty.page, 0);
  assert.equal(empty.pages, 1);
  assert.equal(empty.sheets.length, 0);
});

test("static replay loads relative compressed events and rejects corrupted history", async () => {
  const bundle = await build({
    entryPoints: ["lib/static-replay.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { loadStaticReplay } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
  );
  const event = {
    sequence: 0,
    elapsed_millis: 10,
    event_type: "run.completed",
    payload: {},
  };
  let compressed = gzipSync(JSON.stringify(event) + "\n");
  const manifest = {
    schema: 1,
    source: { ...base, operation: "sheet-shrink" },
    events_file: "events.jsonl.gz",
    event_count: 1,
    events_bytes: compressed.length,
    events_sha256: createHash("sha256").update(compressed).digest("hex"),
  };
  const requests = [];
  const request = async (url) => {
    requests.push(String(url));
    const response = String(url).endsWith("recording.json")
      ? Response.json(manifest)
      : new Response(compressed);
    Object.defineProperty(response, "url", {
      value: "https://example.test/runs/destatis/recording.json",
    });
    return response;
  };
  const result = await loadStaticReplay(
    "./recording.json",
    new AbortController().signal,
    request,
  );
  assert.deepEqual(result.events, [event]);
  assert.deepEqual(requests, [
    "./recording.json",
    "https://example.test/runs/destatis/events.jsonl.gz",
  ]);
  manifest.events_sha256 = "0".repeat(64);
  await assert.rejects(
    loadStaticReplay("./recording.json", new AbortController().signal, request),
    /hash mismatch/,
  );
  compressed = gzipSync(JSON.stringify({ ...event, sequence: 9 }) + "\n");
  manifest.events_bytes = compressed.length;
  manifest.events_sha256 = createHash("sha256")
    .update(compressed)
    .digest("hex");
  await assert.rejects(
    loadStaticReplay("./recording.json", new AbortController().signal, request),
    /sequence or clock mismatch/,
  );
  manifest.events_bytes = 129 * 1024 * 1024;
  await assert.rejects(
    loadStaticReplay("./recording.json", new AbortController().signal, request),
    /Unsupported static replay size or source/,
  );
  manifest.events_file = "https://other.test/events.jsonl.gz";
  await assert.rejects(
    loadStaticReplay("./recording.json", new AbortController().signal, request),
    /Unsupported static replay manifest/,
  );
});

test("worker staircase supports a full 30301-workbook recording", async () => {
  const bundle = await build({
    entryPoints: ["lib/worker-scene.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { buildWorkerScene } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
  );
  const jobs = Array.from({ length: 30301 }, (_, index) => ({
    id: `workbook-${index}`,
    name: `book-${index}.xlsx`,
    worker: 0,
    started_ms: index,
    ended_ms: index + 1,
    status: "prepared",
    before: packageSize(30, 100),
    after: packageSize(20, 50),
    source_bytes: 30,
    error: null,
  }));
  const { snapshot, run } = scenes(
    { ...base, operation: "sheet-shrink", elapsed_ms: 31000 },
    jobs,
    [],
  );
  const scene = buildWorkerScene(snapshot, run, {
    workUnit: "worksheet",
    stepHeight: 0.001,
    labelMode: "name",
    jobStyle: "blocks",
    windowSeconds: 31,
    overview: false,
  });
  assert.equal(snapshot.workbooks.length, 30301);
  assert.ok(scene.shapes.some((shape) => shape.positions.length > 300000));
});

test("vector export preserves worksheet anchors and escapes source labels", async () => {
  const bundle = await build({
    stdin: {
      contents:
        'export {sceneSvg} from "./lib/export-scene"; export {defaultView} from "./lib/camera";',
      resolveDir: process.cwd(),
    },
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { sceneSvg, defaultView } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
  );
  const scene = {
    target: [0, 0, 0],
    span: 10,
    shapes: [
      {
        kind: "lines",
        positions: [0, 0, 0, 1, 0, 0],
        color: "#123456",
        dashed: true,
      },
    ],
    labels: [{ id: "name", position: [0, 0, 0], text: '<&"test>' }],
    hits: [
      {
        id: "task",
        sheetId: "workbook-1:sheet1",
        position: [0, 0, 0],
        title: "sheet",
        lines: [],
      },
    ],
  };
  const svg = sceneSvg(scene, defaultView(scene, 400, 200));
  assert.ok(svg.includes("&lt;&amp;&quot;test&gt;"));
  assert.ok(svg.includes('stroke-dasharray="5 4"'));
  assert.ok(svg.includes("workbook-1:sheet1"));
  assert.ok(svg.includes('id="worksheet-anchors"'));
});

test("worksheet identity has the same hue across chunks and filtered atlas pages", () => {
  const job = {
    id: "workbook-9",
    name: "identity.xlsx",
    worker: 0,
    started_ms: 0,
    ended_ms: 1000,
    status: "prepared",
    before: packageSize(30, 1000),
    after: packageSize(10, 100),
    source_bytes: 30,
    error: null,
  };
  const whole = scenes(base, [{ ...job, id: "workbook-1" }, job]);
  const page = scenes(base, [job]);
  assert.equal(
    whole.snapshot.workbooks[1].sheets[0].hue,
    page.snapshot.workbooks[0].sheets[0].hue,
  );
});

test("atlas original and optimized targets share worksheet identity and cover their columns", async () => {
  const bundle = await build({
    entryPoints: ["lib/workbook-scene.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { buildWorkbookScene } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
  );
  const job = {
    id: "workbook-1",
    name: "example.xlsx",
    worker: 0,
    started_ms: 0,
    ended_ms: 1000,
    status: "prepared",
    before: packageSize(100, 1000000),
    after: packageSize(10, 100),
    source_bytes: 100,
    error: null,
  };
  const snapshot = atlasSnapshot(scenes(base, [job]).snapshot, [job]);
  const scene = buildWorkbookScene(snapshot, {
    view: "all",
    sort: "size",
    resultStyle: "split",
    labelMode: "name",
  });
  const targets = scene.hits.filter((hit) => hit.sheetId);
  assert.equal(targets.length, 2);
  const original = targets.find((hit) => hit.sheetVariant === "original");
  const optimized = targets.find((hit) => hit.sheetVariant === "optimized");
  assert.equal(original.sheetId, optimized.sheetId);
  assert.equal(original.workbookId, optimized.workbookId);
  assert.notEqual(original.id, optimized.id);
  assert.equal(original.color, optimized.color);
  assert.deepEqual(original.lines, optimized.lines);
  assert.ok(original.position[1] > optimized.position[1]);
  assert.notEqual(original.position[2], optimized.position[2]);
  assert.equal(original.pickBase[0], original.position[0]);
  assert.equal(original.pickBase[2], original.position[2]);
  assert.ok(original.pickBase[1] < original.position[1]);
});

test("worksheet colours distinguish sheets and identify their phase", async () => {
  const bundled = await build({
    entryPoints: ["lib/worker-scene.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { buildWorkerScene } = await import(
    `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].contents).toString("base64")}`
  );
  const jobs = [7, 8].map((index) => ({
    id: `workbook-${index}`,
    name: `book-${index}.xlsx`,
    worker: null,
    started_ms: 0,
    ended_ms: null,
    status: "running",
    before: {
      file_bytes: 100,
      worksheets: [
        { part: "xl/worksheets/sheet1.xml", uncompressed_bytes: 1000 },
        { part: "xl/worksheets/sheet2.xml", uncompressed_bytes: 2000 },
      ],
    },
    after: null,
    sheet_names: {
      "xl/worksheets/sheet1.xml": "Power BI",
      "xl/worksheets/sheet2.xml": "Data",
    },
    sheet_order: ["xl/worksheets/sheet2.xml", "xl/worksheets/sheet1.xml"],
  }));
  const tasks = [
    {
      workbook_id: "workbook-7",
      sheet: "xl/worksheets/sheet1.xml",
      phase: "chunk",
      chunk_index: 0,
      chunk_count: 2,
    },
    {
      workbook_id: "workbook-7",
      sheet: "xl/worksheets/sheet2.xml",
      phase: "chunk",
      chunk_index: 1,
      chunk_count: 2,
    },
    {
      workbook_id: "workbook-8",
      sheet: "xl/worksheets/sheet1.xml",
      phase: "finalize",
      chunk_index: null,
      chunk_count: 1,
    },
  ].map((task, index) => ({
    ...task,
    id: index,
    worker: index,
    started_ms: 0,
    ended_ms: null,
  }));
  const update = { ...base, operation: "sheet-shrink", elapsed_ms: 4000 };
  const { snapshot, run } = scenes(update, jobs, tasks);
  const scene = buildWorkerScene(snapshot, run, {
    workUnit: "worksheet",
    stepHeight: 0.01,
    labelMode: "name",
    jobStyle: "blocks",
    windowSeconds: 4,
    overview: false,
  });
  assert.notEqual(scene.hits[0].color, scene.hits[1].color);
  assert.notEqual(scene.hits[0].color, scene.hits[2].color);
  const labels = scene.labels.filter((label) =>
    label.id.startsWith("job-label"),
  );
  assert.match(labels[0].text, /S2.*Power BI.*Chunk 1\/2/);
  assert.match(labels[1].text, /S1.*Data.*Chunk 2\/2/);
  assert.match(labels[2].text, /Finalize/);
  assert.ok(labels.every((label) => label.surface.width > 0));
  const retained = scenes(update, jobs.slice(1), tasks).snapshot;
  assert.equal(retained.sheets[0].hue, snapshot.workbooks[1].sheets[0].hue);
  const legacy = scenes(
    update,
    [{ ...jobs[0], sheet_order: undefined }],
    tasks,
  ).snapshot;
  assert.equal(legacy.sheets[0].indexLabel, "XML1");
});

test("surface nameplates rotate and foreshorten with the worker plane", async () => {
  const bundled = await build({
    entryPoints: ["lib/camera.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { cameraMatrix, defaultView, project, surfaceLabelMatrix } =
    await import(
      `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].contents).toString("base64")}`
    );
  const view = defaultView({ target: [0, 0, 0], span: 24 }, 1200, 600);
  view.yaw = 0.8;
  const matrix = cameraMatrix(view);
  const position = [1, 0, 2];
  const transform = surfaceLabelMatrix(
    position,
    { width: 10, height: 1 },
    100,
    20,
    matrix,
    1200,
    600,
  );
  const center = project(position, matrix, 1200, 600);
  assert.equal(transform.x, center.x);
  assert.equal(transform.y, center.y);
  assert.notEqual(transform.b, 0);
  assert.notEqual(transform.c, 0);
  view.yaw = 0;
  const aligned = surfaceLabelMatrix(
    position,
    { width: 10, height: 1 },
    100,
    20,
    cameraMatrix(view),
    1200,
    600,
  );
  assert.equal(aligned.b, 0);
  assert.equal(aligned.c, 0);
});

test("incremental metadata and completion use optimization controls, not saved baseline sizes", () => {
  const events = [
    {
      event_type: "job.started",
      workbook_id: "book",
      worker_index: 1,
      elapsed_millis: 10,
      payload: { name: "book.xlsx" },
    },
    {
      event_type: "workbook.metadata",
      workbook_id: "book",
      elapsed_millis: 20,
      payload: { package: packageSize(100, 1000) },
    },
    {
      event_type: "job.completed",
      workbook_id: "book",
      elapsed_millis: 200,
      payload: {
        archive_sizes: {
          before_optimization: packageSize(30, 300),
          after_optimization: packageSize(20, 100),
        },
      },
    },
  ].map((event) => ({ event }));
  const jobs = applyUpdate([], { ...base, reset: false, events });
  assert.equal(jobs[0].before.file_bytes, 30);
  assert.equal(jobs[0].after.file_bytes, 20);
  const { snapshot } = scenes(base, jobs);
  assert.equal(snapshot.jobs[0].workbookName, "book.xlsx");
  assert.equal(snapshot.jobs[0].workerIndex, 1);
  assert.equal(snapshot.sheets[0].originalBytes, 300);
  assert.equal(snapshot.sheets[0].cleanedBytes, 100);
});

test("conversion has workbook jobs without invented worksheet work", () => {
  const jobs = [
    {
      id: "legacy",
      name: "legacy.xls",
      worker: 0,
      started_ms: 0,
      ended_ms: null,
      status: "running",
      before: null,
      after: null,
      source_bytes: 900,
      error: null,
    },
  ];
  const { snapshot } = scenes({ ...base, operation: "xlsx-convert" }, jobs);
  assert.equal(snapshot.jobs.length, 1);
  assert.equal(snapshot.jobs[0].sheet.name, "legacy.xls");
  assert.equal(snapshot.jobs[0].chunkCount, 1);
  assert.equal(snapshot.sheets.length, 0);
});

test("reconnection replaces stale jobs and retains interrupted outcomes", () => {
  const interrupted = {
    id: "interrupted",
    name: "a.xlsx",
    worker: 0,
    started_ms: 0,
    ended_ms: 30,
    status: "interrupted",
    before: null,
    after: null,
    source_bytes: null,
    error: null,
  };
  const jobs = applyUpdate([{ id: "stale" }], {
    ...base,
    reset: true,
    jobs: [interrupted],
  });
  const { snapshot } = scenes(base, jobs);
  assert.equal(jobs.length, 1);
  assert.equal(snapshot.activeJobs.length, 0);
  assert.equal(snapshot.completedBooks, 0);
});

test("SheetShrink projects real chunk workers, submillisecond times and consistent atlas units", () => {
  const update = {
    ...base,
    operation: "sheet-shrink",
    reset: false,
    events: [
      {
        event: {
          event_type: "work.started",
          workbook_id: "book",
          worker_index: 1,
          elapsed_millis: 0,
          elapsed_micros: 125,
          payload: {
            task_id: 3,
            sheet: "xl/worksheets/sheet1.xml",
            phase: "chunk",
            chunk_index: 2,
            chunk_count: 4,
            range: "cells_range",
          },
        },
      },
      {
        event: {
          event_type: "work.finished",
          workbook_id: "book",
          worker_index: 1,
          elapsed_millis: 0,
          elapsed_micros: 875,
          payload: { task_id: 3 },
        },
      },
    ],
  };
  const tasks = applyTasks([], update);
  const before = { ...packageSize(20, 1000), uncompressed_payload_bytes: 2000 };
  const after = { ...packageSize(10, 500), uncompressed_payload_bytes: 1500 };
  const jobs = [
    {
      id: "book",
      name: "book.xlsx",
      worker: null,
      started_ms: 0,
      ended_ms: 2,
      status: "prepared",
      before,
      after,
      sheet_names: { "xl/worksheets/sheet1.xml": "Measurements" },
    },
  ];
  const { snapshot } = scenes(update, jobs, tasks);
  assert.equal(snapshot.jobs[0].workerIndex, 1);
  assert.equal(snapshot.jobs[0].chunkIndex, 2);
  assert.equal(snapshot.jobs[0].chunkCount, 4);
  assert.equal(snapshot.jobs[0].sheet.name, "Measurements");
  assert.equal(tasks[0].ended_ms - tasks[0].started_ms, 0.75);
  assert.equal(atlasSnapshot(snapshot, jobs).workbooks[0].originalBytes, 2000);
  assert.equal(snapshot.workbooks[0].originalBytes, 20);
});

test("atlas supports seven recent workbooks and descending size order", async () => {
  const bundled = await build({
    entryPoints: ["lib/workbook-scene.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { getAtlasBooks } = await import(
    `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].contents).toString("base64")}`
  );
  const snapshot = {
    time: 100,
    workbooks: Array.from({ length: 10 }, (_, index) => ({
      id: String(index),
      index,
      startAt: index,
      originalBytes: 10 - index,
    })),
  };
  assert.deepEqual(
    getAtlasBooks(snapshot, {
      view: "recent",
      sort: "time",
      recentCount: 7,
    }).map((book) => book.id),
    ["3", "4", "5", "6", "7", "8", "9"],
  );
  assert.deepEqual(
    getAtlasBooks(snapshot, { view: "all", sort: "size" }).map(
      (book) => book.id,
    ),
    ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"],
  );
});

test("replay atlas pages respect page size, navigation and backward seeking", async () => {
  const bundle = await build({
    entryPoints: ["lib/workbook-scene.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { workbookPage, buildWorkbookScene } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
  );
  const snapshot = {
    time: 100,
    knownBooks: 10,
    workbooks: Array.from({ length: 10 }, (_, index) => ({
      id: String(index),
      name: `book-${index}`,
      index,
      startAt: index,
      originalBytes: 1000 - index,
      cleanedBytes: 10,
      completedAt: index + 1,
      known: true,
      status: "complete",
      progress: 1,
      sheets: [],
    })),
  };
  assert.deepEqual(
    workbookPage(snapshot, 0, 3).workbooks.map((book) => book.id),
    ["0", "1", "2"],
  );
  assert.deepEqual(
    workbookPage(snapshot, 1, 3).workbooks.map((book) => book.id),
    ["3", "4", "5"],
  );
  const last = workbookPage(snapshot, 99, 3);
  assert.equal(last.page, 3);
  assert.equal(last.total, 10);
  assert.deepEqual(
    last.workbooks.map((book) => book.id),
    ["9"],
  );
  const earlier = workbookPage(
    { ...snapshot, workbooks: snapshot.workbooks.slice(0, 2) },
    3,
    3,
  );
  assert.equal(earlier.page, 0);
  assert.equal(earlier.total, 2);
  assert.deepEqual(
    earlier.workbooks.map((book) => book.id),
    ["0", "1"],
  );
  for (const pageSize of [1, 3, 7]) {
    const page = workbookPage(snapshot, 0, pageSize);
    const scene = buildWorkbookScene(
      { ...snapshot, workbooks: page.workbooks },
      {
        view: "all",
        sort: "size",
        limit: pageSize,
        resultStyle: "split",
        labelMode: "name",
      },
    );
    assert.equal(scene.hits.length, pageSize);
  }
});

test("explicit horizontal framing fills wide and narrow viewports consistently", async () => {
  const bundled = await build({
    entryPoints: ["lib/camera.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { cameraMatrix, defaultView, project } = await import(
    `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].contents).toString("base64")}`
  );
  const scene = {
    target: [12, 0, 0],
    span: 30,
    horizontalSpan: 26,
    yaw: 0,
    pitch: 0,
  };
  for (const width of [400, 1600]) {
    const matrix = cameraMatrix(defaultView(scene, width, 400));
    const left = project([0, 0, 0], matrix, width, 400);
    const right = project([24, 0, 0], matrix, width, 400);
    assert.ok(Math.abs((right.x - left.x) / width - 24 / 26) < 0.00001);
    assert.ok(left.visible && right.visible);
  }
});

test("atlas byte scale uses only displayed workbooks and includes output growth", async () => {
  const bundled = await build({
    entryPoints: ["lib/workbook-scene.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { buildWorkbookScene } = await import(
    `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].contents).toString("base64")}`
  );
  const book = (id, startAt, originalBytes, cleanedBytes) => ({
    id,
    name: id,
    index: startAt,
    startAt,
    originalBytes,
    cleanedBytes,
    completedAt: startAt + 1,
    known: true,
    status: "complete",
    sheets: [],
  });
  const snapshot = {
    time: 100,
    knownBooks: 2,
    workbooks: [
      book("hidden-large", 0, 1000000, 900000),
      book("visible-small", 1, 1000, 2000),
    ],
  };
  const options = {
    view: "recent",
    sort: "time",
    recentCount: 1,
    resultStyle: "split",
    labelMode: "name",
  };
  const scene = buildWorkbookScene(snapshot, options);
  const expected = buildWorkbookScene(
    { ...snapshot, knownBooks: 1, workbooks: [snapshot.workbooks[1]] },
    options,
  );
  assert.equal(
    scene.labels.find((label) => label.id === "atlas-scale-1").text,
    expected.labels.find((label) => label.id === "atlas-scale-1").text,
  );
  const wider = buildWorkbookScene(snapshot, { ...options, recentCount: 2 });
  assert.notEqual(
    scene.labels.find((label) => label.id === "atlas-scale-1").text,
    wider.labels.find((label) => label.id === "atlas-scale-1").text,
  );
  const focused = buildWorkbookScene(snapshot, {
    ...options,
    view: "all",
    sort: "size",
    limit: 1,
    focusWorkbookId: "visible-small",
    selectedSheetId: "visible-small:sheet1",
  });
  assert.deepEqual(
    focused.hits.map((hit) => hit.workbookId),
    ["visible-small"],
  );
  const retained = buildWorkbookScene(snapshot, {
    ...options,
    view: "all",
    sort: "size",
    limit: 2,
    focusWorkbookId: "hidden-large",
    selectedSheetId: "hidden-large:sheet1",
  });
  assert.deepEqual(
    retained.hits.map((hit) => hit.workbookId),
    ["hidden-large", "visible-small"],
  );
});
