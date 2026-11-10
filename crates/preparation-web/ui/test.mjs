import { build } from "esbuild";
import { test } from "node:test";
import assert from "node:assert/strict";

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

test("worksheet and chunk bars share their workbook colour and identify their phase", async () => {
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
  assert.equal(scene.hits[0].color, scene.hits[1].color);
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
});
