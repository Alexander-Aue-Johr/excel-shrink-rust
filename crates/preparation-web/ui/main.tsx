import { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import SceneViewport from "./components/SceneViewport";
import SheetDetails from "./components/SheetDetails";
import LinkedScenesPanel from "./components/LinkedScenesPanel";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { presentScene, type Theme } from "./lib/presentation";
import { replayAt } from "./lib/replay";
import { downloadSvg, figureSvg } from "./lib/export-scene";
import type { HitTarget } from "./lib/scene";
import { buildWorkbookScene, workbookPage } from "./lib/workbook-scene";
import { buildWorkerScene } from "./lib/worker-scene";
import { buildDistributionScene } from "./lib/chart-scene";
import { fmtBytes } from "./lib/scene";
import {
  applyUpdate,
  applyTasks,
  atlasSnapshot,
  scenes,
  type Configuration,
  type Job,
  type Update,
  type WorkTask,
  type NativeEvent,
} from "./live";
import "./app/globals.css";
import "./styles.css";

const token = document.querySelector<HTMLMetaElement>(
  'meta[name="observatory-token"]',
)!.content;

const activeStates = new Set(["starting", "running", "stopping"]);

const names: Record<string, string> = {
  idle: "Ready",
  starting: "Starting",
  running: "Running",
  stopping: "Stopping",
  stopped: "Stopped",
  completed: "Completed",
  failed: "Failed",
  prepared: "Prepared",
  interrupted: "Interrupted",
};

async function command(path: string, body: unknown = {}) {
  const response = await fetch(path, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-observatory-token": token,
    },
    body: JSON.stringify(body),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error ?? `HTTP ${response.status}`);
}

function Observatory() {
  const [liveUpdate, setUpdate] = useState<Update | null>(null);
  const [liveJobs, setJobs] = useState<Job[]>([]);
  const [liveTasks, setTasks] = useState<WorkTask[]>([]);
  const [theme, setTheme] = useState<Theme>("dark");
  const [paperMode, setPaperMode] = useState(false);
  const [hoverSheet, setHoverSheet] = useState<HitTarget | null>(null);
  const [pinnedSheet, setPinnedSheet] = useState<string | null>(null);
  const [recording, setRecording] = useState<{
    source: Update;
    events: NativeEvent[];
  } | null>(null);
  const [replayTime, setReplayTime] = useState<number | null>(null);
  const [playing, setPlaying] = useState(false);
  const [playbackRate, setPlaybackRate] = useState(0.2);
  const replay = useMemo(
    () =>
      recording && replayTime !== null
        ? replayAt(recording.source, recording.events, replayTime)
        : null,
    [recording, replayTime],
  );
  const update = replay?.update ?? liveUpdate;
  const jobs = replay?.jobs ?? liveJobs;
  const tasks = replay?.tasks ?? liveTasks;
  const focusSheet = hoverSheet?.sheetId ?? pinnedSheet;
  const focusVariant = hoverSheet?.sheetVariant ?? "optimized";
  const replaying = replayTime !== null;

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    setRecording(null);
    setReplayTime(null);
    setPlaying(false);
  }, [liveUpdate?.run_id]);

  useEffect(() => {
    if (!playing || !recording) return;
    let previous = performance.now();
    const timer = window.setInterval(() => {
      const now = performance.now();
      const step = (now - previous) * playbackRate;
      previous = now;
      setReplayTime((time) => {
        const next = Math.min(recording.source.elapsed_ms, (time ?? 0) + step);
        if (next >= recording.source.elapsed_ms) setPlaying(false);
        return next;
      });
    }, 66);
    return () => window.clearInterval(timer);
  }, [playing, recording, playbackRate]);
  const [config, setConfig] = useState<Configuration | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [stepHeight, setStepHeight] = useState(0.14);
  const [atlasView, setAtlasView] = useState<
    "three" | "seven" | "all" | "largest"
  >("three");
  const [atlasPageSize, setAtlasPageSize] = useState(7);
  const [atlasPage, setAtlasPage] = useState(0);
  const [atlasHistory, setAtlasHistory] = useState<{
    run_id: string;
    total: number;
    page: number;
    jobs: Job[];
  } | null>(null);
  const [atlasError, setAtlasError] = useState("");
  const cursor = useRef({ run: "", after: 0, status: "" });
  const initialized = useRef(false);
  const knownDefaults = useRef<Configuration | null>(null);

  useEffect(() => {
    let stopped = false;
    let timeout = 0;
    const controller = new AbortController();
    async function poll() {
      try {
        const { run, after } = cursor.current;
        const response = await fetch(
          `/api/state?run=${encodeURIComponent(run)}&after=${after}`,
          { signal: controller.signal },
        );
        if (!response.ok) throw new Error(`Status HTTP ${response.status}`);
        const wire: Omit<Update, "defaults"> & {
          defaults: Configuration | null;
          workers: number;
        } = await response.json();
        if (wire.defaults) knownDefaults.current = wire.defaults;
        if (!knownDefaults.current)
          throw new Error("Configuration missing from status response");
        const next: Update = {
          ...wire,
          defaults: { ...knownDefaults.current, workers: wire.workers },
        };
        if (stopped) return;
        const terminal =
          !activeStates.has(next.status) &&
          activeStates.has(cursor.current.status);
        cursor.current = {
          run: terminal ? "" : next.run_id,
          after: next.next,
          status: next.status,
        };
        setJobs((previous) => applyUpdate(previous, next));
        setTasks((previous) => applyTasks(previous, next));
        setUpdate(next);
        if (!initialized.current) {
          setConfig(next.defaults);
          initialized.current = true;
        }
      } catch (failure) {
        if (!stopped) setError(String(failure));
      }
      if (!stopped) timeout = window.setTimeout(poll, 750);
    }
    void poll();
    return () => {
      stopped = true;
      controller.abort();
      window.clearTimeout(timeout);
    };
  }, []);

  useEffect(() => {
    setAtlasPage(0);
    setAtlasHistory(null);
  }, [update?.run_id]);

  useEffect(() => {
    if (atlasView !== "all" || !update || replaying) return;
    const controller = new AbortController();
    let timer = 0;
    async function load() {
      try {
        const response = await fetch(
          `/api/atlas?run=${encodeURIComponent(update!.run_id)}&page=${atlasPage}&page_size=${atlasPageSize}`,
          { signal: controller.signal },
        );
        const page = await response.json();
        if (!response.ok)
          throw new Error(page.error ?? `Atlas HTTP ${response.status}`);
        if (!controller.signal.aborted) {
          setAtlasHistory(page);
          setAtlasError("");
        }
      } catch (failure) {
        if (!controller.signal.aborted) setAtlasError(String(failure));
      }
      if (!controller.signal.aborted && activeStates.has(update!.status))
        timer = window.setTimeout(load, 5000);
    }
    void load();
    return () => {
      controller.abort();
      window.clearTimeout(timer);
    };
  }, [
    atlasView,
    atlasPage,
    atlasPageSize,
    update?.run_id,
    update?.status,
    replaying,
  ]);

  const rendering = useMemo(
    () => (update ? scenes(update, jobs, tasks) : null),
    [update, jobs, tasks],
  );

  const workerScene = useMemo(
    () =>
      rendering
        ? buildWorkerScene(rendering.snapshot, rendering.run, {
            stepHeight: update?.operation === "sheet-shrink" ? stepHeight : 0,
            labelMode: "name",
            jobStyle: "blocks",
            windowSeconds:
              update?.operation === "sheet-shrink"
                ? Math.max(
                    0.05,
                    paperMode || replayTime !== null
                      ? update.elapsed_ms / 1000
                      : Math.min(70, update.elapsed_ms / 1000),
                  )
                : 120,
            overview: false,
            selectedWorkbookId: selected,
            selectedSheetId: focusSheet,
            workUnit:
              update?.operation === "sheet-shrink" ? "worksheet" : "workbook",
          })
        : null,
    [rendering, selected, stepHeight, focusSheet, paperMode, replayTime],
  );

  const distributions = useMemo(
    () =>
      rendering
        ? {
            books: buildDistributionScene(rendering.snapshot, {
              entity: "workbooks",
              metric: "size",
              labelMode: "name",
              order: "independent",
            }),
            sheets: buildDistributionScene(rendering.snapshot, {
              entity: "worksheets",
              metric: "size",
              labelMode: "xml",
              order: "independent",
            }),
          }
        : null,
    [rendering],
  );
  const observedAtlas = useMemo(
    () => (rendering ? atlasSnapshot(rendering.snapshot, jobs) : null),
    [rendering, jobs],
  );
  const replayPage = useMemo(
    () =>
      replaying && observedAtlas
        ? workbookPage(observedAtlas, atlasPage, atlasPageSize)
        : null,
    [replaying, observedAtlas, atlasPage, atlasPageSize],
  );
  const pageInfo = replayPage ?? atlasHistory;
  const displayedAtlasPage = pageInfo?.page ?? atlasPage;
  const atlas = useMemo(() => {
    if (!rendering || !observedAtlas) return null;
    let snapshot = observedAtlas;
    if (atlasView === "all") {
      if (replayPage) {
        snapshot = { ...observedAtlas, workbooks: replayPage.workbooks };
      } else {
        const pageJobs =
          atlasHistory?.run_id === update!.run_id ? atlasHistory.jobs : [];
        snapshot = atlasSnapshot(scenes(update!, pageJobs).snapshot, pageJobs);
      }
    }
    const focusedWorkbook = observedAtlas.workbooks.find((book) =>
      book.sheets.some((sheet) => sheet.id === focusSheet),
    );
    if (
      focusedWorkbook &&
      !snapshot.workbooks.some((book) => book.id === focusedWorkbook.id)
    ) {
      snapshot = {
        ...snapshot,
        workbooks: [...snapshot.workbooks, focusedWorkbook],
      };
    }
    return buildWorkbookScene(snapshot, {
      resultStyle: "split",
      labelMode: "name",
      view: atlasView === "all" || atlasView === "largest" ? "all" : "recent",
      sort: atlasView === "all" || atlasView === "largest" ? "size" : "time",
      limit:
        atlasView === "all"
          ? atlasPageSize
          : atlasView === "largest"
            ? 3
            : undefined,
      recentCount: atlasView === "seven" ? 7 : 3,
      selectedWorkbookId: selected,
      selectedSheetId: focusSheet,
      focusWorkbookId: focusedWorkbook?.id,
    });
  }, [
    rendering,
    observedAtlas,
    atlasView,
    replayPage,
    atlasHistory,
    update?.run_id,
    atlasPageSize,
    selected,
    focusSheet,
  ]);

  if (!update || !config || !rendering || !workerScene || !distributions)
    return (
      <main>
        <p>Loading Observatory…</p>
        {error && <p role="alert">{error}</p>}
      </main>
    );

  const detailed = update.operation === "sheet-shrink";
  const optimizing = update.operation !== "xlsx-convert";
  const title = detailed
    ? "SheetShrink"
    : optimizing
      ? "Excel Check Performance"
      : "XLSX Convert";
  const active = activeStates.has(update.status);
  const completed = update.counts.prepared + update.counts.failed;
  const seconds = update.elapsed_ms / 1000;
  const perHour = seconds > 0 ? (completed / seconds) * 3600 : 0;
  const remaining = Math.max(0, update.counts.total - completed);
  const remainingSeconds =
    perHour > 0 ? Math.ceil((remaining / perHour) * 3600) : null;
  const eta =
    remainingSeconds === null
      ? "—"
      : `${remainingSeconds >= 60 ? `${Math.floor(remainingSeconds / 60)} min ` : ""}${remainingSeconds % 60} s`;
  const focused = jobs.find((job) => job.id === selected) ?? jobs.at(-1);
  const activeWorkers = new Set(
    rendering.snapshot.activeJobs.map((job) => job.workerIndex),
  ).size;
  const change = <K extends keyof Configuration>(
    key: K,
    value: Configuration[K],
  ) =>
    setConfig((current) => (current ? { ...current, [key]: value } : current));

  async function act(path: string, body?: unknown) {
    setBusy(true);
    setError("");
    try {
      await command(path, body);
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  }

  async function startReplay() {
    if (!liveUpdate) return;
    setBusy(true);
    setError("");
    try {
      if (!recording) {
        const events: NativeEvent[] = [];
        let after = 0;
        for (;;) {
          const response = await fetch(
            `/api/replay?run=${encodeURIComponent(liveUpdate.run_id)}&after=${after}`,
          );
          const page = await response.json();
          if (!response.ok)
            throw new Error(page.error ?? "Replay log unavailable");
          if (page.run_id !== liveUpdate.run_id)
            throw new Error("Replay run identity changed");
          events.push(...page.events);
          if (page.complete) break;
          if (page.next <= after)
            throw new Error("Replay cursor did not advance");
          after = page.next;
        }
        if (cursor.current.run !== liveUpdate.run_id)
          throw new Error("Replay run changed while loading its history");
        setRecording({ source: liveUpdate, events });
      }
      setReplayTime(0);
      setPlaying(true);
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  }

  function selectSheet(hit: HitTarget) {
    setSelected(hit.workbookId ?? null);
    setPinnedSheet((current) =>
      current === hit.sheetId ? null : (hit.sheetId ?? null),
    );
  }

  function exportFigure() {
    const viewports = document.querySelectorAll<
      HTMLDivElement & { exportScene?: () => string }
    >(".linked-scenes .scene-viewport");
    const views = [...viewports].map((node) => node.exportScene?.());
    if (views.length !== 2 || views.some((view) => !view)) {
      setError("Both scenes must be visible before exporting");
      return;
    }
    const inputParts = config!.inputs[0]?.split(/[\\/]/).filter(Boolean) ?? [];
    const cohort =
      inputParts.at(-1) === "originals" ? inputParts.at(-2) : inputParts.at(-1);
    const subtitle = `${cohort ?? "Recorded run"} | ${update!.counts.prepared} / ${update!.counts.total} completed | ${config!.workers} workers | ${seconds.toFixed(2)} s recorded | Default`;
    const svg = figureSvg(
      views as string[],
      subtitle,
      theme === "light",
      focusSheet,
      focusVariant,
    );
    downloadSvg("sheetshrink-recorded-figure.svg", svg);
  }
  return (
    <div className={`observatory ${paperMode ? "paper-mode" : ""}`}>
      <header className="topbar">
        <a className="brand" href="#overview">
          <span className="brand-mark">
            <i />
            <i />
            <i />
          </span>
          <span>
            {title}
            <span className="brand-separator">/</span>
            <span className="brand-secondary">Observatory</span>
          </span>
        </a>
        <span className="status-pill is-live">
          <i />
          {replayTime === null
            ? names[update.status]
            : `Replay · ${playbackRate}×`}
        </span>
      </header>
      <main>
        <div className="presentation-controls">
          <button
            onClick={() => setTheme(theme === "light" ? "dark" : "light")}
          >
            {theme === "light" ? "Dark theme" : "Light theme"}
          </button>
          <button
            aria-pressed={paperMode}
            onClick={() => setPaperMode(!paperMode)}
          >
            Figure view
          </button>
          <button
            disabled={active || busy || !liveUpdate?.counts.total}
            onClick={() => void startReplay()}
          >
            Replay recorded run
          </button>
          {replayTime !== null && (
            <>
              <button onClick={() => setPlaying(!playing)}>
                {playing ? "Pause replay" : "Play replay"}
              </button>
              <select
                aria-label="Playback speed"
                value={playbackRate}
                onChange={(event) =>
                  setPlaybackRate(Number(event.target.value))
                }
              >
                {[0.05, 0.1, 0.2, 0.5, 1, 2].map((rate) => (
                  <option key={rate} value={rate}>
                    {rate}×
                  </option>
                ))}
              </select>
              <input
                aria-label="Replay position"
                type="range"
                min="0"
                max={recording?.source.elapsed_ms ?? 1}
                value={replayTime}
                onChange={(event) => {
                  setPlaying(false);
                  setReplayTime(Number(event.target.value));
                }}
              />
              <span>
                Recorded {(replayTime / 1000).toFixed(2)} /{" "}
                {((recording?.source.elapsed_ms ?? 0) / 1000).toFixed(2)} s
              </span>
              <button
                onClick={() => {
                  setReplayTime(null);
                  setPlaying(false);
                }}
              >
                Return to live
              </button>
            </>
          )}
          {focusSheet && (
            <button
              onClick={() => {
                setPinnedSheet(null);
                setHoverSheet(null);
                setSelected(null);
              }}
            >
              Clear worksheet focus
            </button>
          )}
          {detailed && (
            <select
              aria-label="Link worksheet"
              value={pinnedSheet ?? ""}
              onChange={(event) => {
                setPinnedSheet(event.target.value || null);
                setSelected(
                  rendering.snapshot.sheets.find(
                    (sheet) => sheet.id === event.target.value,
                  )?.workbookId ?? null,
                );
              }}
            >
              <option value="">Link a worksheet across views</option>
              {rendering.snapshot.sheets.map((sheet) => (
                <option key={sheet.id} value={sheet.id}>
                  {sheet.workbookId} · {sheet.indexLabel ?? `S${sheet.index}`} ·{" "}
                  {sheet.name}
                </option>
              ))}
            </select>
          )}
        </div>
        <section className="page-heading" id="overview">
          <div>
            <div className="eyebrow">
              {detailed
                ? "SHEETSHRINK / RUST PROCESSING"
                : "NATIVE EXCEL PROCESSING"}
            </div>
            <h1>{optimizing ? "Optimize. Explore." : "Convert. Explore."}</h1>
            <p>
              {detailed
                ? "Optimize workbooks with SheetShrink and explore parallel sheet and chunk processing."
                : optimizing
                  ? "Optimize workbooks in parallel with Excel Check Performance."
                  : "Save XLS and XLSX inputs as separate XLSX copies using Excel."}
            </p>
          </div>
        </section>
        <form
          className="panel preparation-form"
          onSubmit={(event) => {
            event.preventDefault();
            void act("/api/start", config);
          }}
        >
          <label className="wide">
            Input files or folders · one path per line
            <textarea
              aria-label="Input paths"
              disabled={active}
              value={config.inputs.join("\n")}
              onChange={(event) =>
                change(
                  "inputs",
                  event.target.value.split(/\r?\n/).filter(Boolean),
                )
              }
            />
          </label>
          <label className="wide">
            New output directory
            <input
              aria-label="Output directory"
              disabled={active}
              value={config.output ?? ""}
              onChange={(event) => change("output", event.target.value || null)}
            />
          </label>
          {optimizing && !detailed && (
            <>
              <label>
                Source manifest · optional
                <input
                  disabled={active}
                  value={config.source_manifest ?? ""}
                  onChange={(event) =>
                    change("source_manifest", event.target.value || null)
                  }
                />
              </label>
              <label>
                Previous preparation · optional
                <input
                  disabled={active}
                  value={config.reuse_from ?? ""}
                  onChange={(event) =>
                    change("reuse_from", event.target.value || null)
                  }
                />
              </label>
            </>
          )}
          <label>
            {detailed ? "Rayon-Threads" : "Parallel workers"}
            <input
              aria-label="Parallel workers"
              type="number"
              min="1"
              max="256"
              required
              disabled={detailed && active}
              value={config.workers}
              onChange={(event) =>
                change("workers", Number(event.target.value))
              }
            />
          </label>
          {detailed && (
            <>
              <label>
                Concurrent workbooks
                <input
                  aria-label="Active workbooks"
                  type="number"
                  min="1"
                  disabled={active}
                  value={config.max_active_workbooks ?? 2}
                  onChange={(event) =>
                    change("max_active_workbooks", Number(event.target.value))
                  }
                />
              </label>
              <label>
                Maximum chunk size · MiB
                <input
                  aria-label="Chunk size"
                  type="number"
                  step="any"
                  min="0"
                  disabled={active}
                  value={
                    (config.parallel_range_chunk_size ?? 16777216) / 1048576
                  }
                  onChange={(event) =>
                    change(
                      "parallel_range_chunk_size",
                      Math.round(Number(event.target.value) * 1048576),
                    )
                  }
                />
                <small>
                  {(
                    config.parallel_range_chunk_size ?? 16777216
                  ).toLocaleString("en-US")}{" "}
                  bytes · 0 disables chunking
                </small>
              </label>
            </>
          )}
          <label>
            RAM and commit reserve · MiB
            <input
              aria-label="Memory reserve"
              disabled={active}
              type="number"
              min="1"
              placeholder="Automatic"
              value={config.reserve_mib ?? ""}
              onChange={(event) =>
                change(
                  "reserve_mib",
                  event.target.value ? Number(event.target.value) : null,
                )
              }
            />
          </label>
          <label className="check">
            <input
              type="checkbox"
              disabled={active}
              checked={config.recursive}
              onChange={(event) => change("recursive", event.target.checked)}
            />
            Include subfolders
          </label>
          <div className="form-actions">
            <button
              className="button primary"
              type="submit"
              disabled={active || busy}
            >
              Start
            </button>
            <button
              className="button subtle"
              type="button"
              disabled={detailed || update.status !== "running" || busy}
              onClick={() =>
                void act("/api/workers", { workers: config.workers })
              }
            >
              Apply workers
            </button>
            <button
              className="button subtle"
              type="button"
              disabled={!active || busy}
              onClick={() => void act("/api/stop")}
            >
              Stop
            </button>
          </div>
          <p className="wide form-note">
            {detailed
              ? "Outputs and batch report"
              : "Outputs and source copies"}{" "}
            are saved in the output directory. The event log is saved alongside
            it as .events.jsonl. Each new run requires a new output directory.
          </p>
        </form>
        {(error || update.error) && (
          <p className="error-banner" role="alert">
            {error || update.error}
          </p>
        )}
        <section className="metrics">
          <Metric
            label="Workbooks"
            value={`${completed} / ${update.counts.total}`}
            note={`${update.counts.failed} failed · ${update.counts.reused} reused`}
          />
          <Metric
            label="Throughput"
            value={`${perHour.toFixed(1)} / h`}
            note={`${(perHour / 60).toFixed(2)} / min · since run start`}
          />
          <article className="metric">
            <div className="metric-label">Run time</div>
            <div className="metric-note">
              {replayTime !== null
                ? "Recorded position"
                : update.status === "completed"
                  ? "Total duration"
                  : "Elapsed"}
            </div>
            <div className="metric-value">{seconds.toFixed(2)} s</div>
            {active && (
              <div className="metric-note">
                Remaining · {remaining && remainingSeconds !== null ? "~" : ""}
                {remaining ? eta : "0 s"}
              </div>
            )}
          </article>
          <Metric
            label="Worker"
            value={`${activeWorkers} / ${update.defaults.workers}`}
            note="Active / configured"
          />
        </section>
        <LinkedScenesPanel
          title={
            detailed
              ? "Parallel sheets and chunks · Workbook Atlas"
              : "Parallel workbooks"
          }
          sheetId={focusSheet}
          controls={
            detailed ? (
              <>
                <label className="step-height-control">
                  Step height
                  <input
                    aria-label="Workbook step height"
                    type="range"
                    min="0"
                    max="0.3"
                    step="0.001"
                    value={stepHeight}
                    onChange={(event) =>
                      setStepHeight(Number(event.target.value))
                    }
                  />
                  <output>{stepHeight.toFixed(3)}</output>
                </label>
                <button
                  className="linked-export"
                  aria-label="Export figure SVG"
                  title="Export both diagrams as SVG"
                  onClick={exportFigure}
                >
                  Export SVG
                </button>
              </>
            ) : undefined
          }
          worker={
            <SceneViewport
              scene={presentScene(workerScene, theme)}
              renderer="auto"
              allowFullscreen={false}
              title="Worker timeline"
              className="worker-viewport"
              focusSheetId={focusSheet}
              focusSheetVariant={focusVariant}
              onHover={setHoverSheet}
              onSelect={selectSheet}
            />
          }
          atlas={
            detailed && atlas ? (
              <SceneViewport
                scene={presentScene(atlas, theme)}
                renderer="auto"
                allowFullscreen={false}
                title="Workbook-Atlas"
                className="atlas-viewport"
                focusSheetId={focusSheet}
                focusSheetVariant={focusVariant}
                onHover={setHoverSheet}
                onSelect={selectSheet}
              />
            ) : undefined
          }
          footer={
            <>
              {detailed && atlas && (
                <>
                  <div className="atlas-controls">
                    <button
                      aria-pressed={atlasView === "largest"}
                      onClick={() => setAtlasView("largest")}
                    >
                      Largest 3
                    </button>
                    <button
                      aria-pressed={atlasView === "three"}
                      onClick={() => setAtlasView("three")}
                    >
                      Latest 3
                    </button>
                    <button
                      aria-pressed={atlasView === "seven"}
                      onClick={() => setAtlasView("seven")}
                    >
                      Latest 7
                    </button>
                    <button
                      aria-pressed={atlasView === "all"}
                      onClick={() => setAtlasView("all")}
                    >
                      All by size
                    </button>
                    {atlasView === "all" && (
                      <>
                        <label>
                          Workbooks per page{" "}
                          <input
                            aria-label="Workbooks per page"
                            type="range"
                            min="1"
                            max="24"
                            step="1"
                            value={atlasPageSize}
                            onChange={(event) => {
                              setAtlasPageSize(Number(event.target.value));
                              setAtlasPage(0);
                            }}
                          />
                          <output>{atlasPageSize}</output>
                        </label>
                        <button
                          className="atlas-page-button"
                          aria-label="Previous page"
                          title="Previous page"
                          disabled={displayedAtlasPage === 0}
                          onClick={() => setAtlasPage(displayedAtlasPage - 1)}
                        >
                          <ChevronLeft size={17} aria-hidden="true" />
                        </button>
                        <span>
                          Page {displayedAtlasPage + 1} /{" "}
                          {Math.max(
                            1,
                            Math.ceil((pageInfo?.total ?? 0) / atlasPageSize),
                          )}{" "}
                          · {pageInfo?.total ?? 0} workbooks
                        </span>
                        <button
                          className="atlas-page-button"
                          aria-label="Next page"
                          title="Next page"
                          disabled={
                            !pageInfo ||
                            (displayedAtlasPage + 1) * atlasPageSize >=
                              pageInfo.total
                          }
                          onClick={() => setAtlasPage(displayedAtlasPage + 1)}
                        >
                          <ChevronRight size={17} aria-hidden="true" />
                        </button>
                      </>
                    )}
                  </div>
                  {atlasView === "all" && atlasError && (
                    <p role="alert">{atlasError}</p>
                  )}
                </>
              )}
              <p className="view-note">
                {detailed
                  ? "Time × Worker × Workbook · sheets and chunks · "
                  : "Time × Worker · one block per workbook · "}
                {replay
                  ? `${jobs.length} workbooks · ${tasks.length} recorded work items`
                  : `${update.history_limit} recent jobs retained in memory`}
              </p>
              {detailed && (
                <p className="atlas-units">
                  Atlas · uncompressed ZIP bytes · original and output
                </p>
              )}
            </>
          }
        />
        {detailed && focused && (
          <SheetDetails
            key={focused.id}
            job={focused}
            tasks={tasks}
            now={update.elapsed_ms}
          />
        )}
        {optimizing && (
          <section id="distributions">
            <div className="section-heading">
              <div>
                <div className="section-index">02 / RESULTS</div>
                <h2>Before and after optimization</h2>
                <p>
                  Workbook ZIP file bytes · uncompressed worksheet XML bytes ·
                  recent visible jobs
                </p>
              </div>
            </div>
            <p className="view-note">
              Overall size of newly optimized workbooks with size measurements ·{" "}
              {fmtBytes(update.counts.before_bytes)} →{" "}
              {fmtBytes(update.counts.after_bytes)}
            </p>
            <div className="distribution-grid">
              <section className="panel distribution-card">
                <div className="chart-heading">
                  <p className="section-index">
                    PER WORKBOOK · SORTED INDEPENDENTLY
                  </p>
                  <h3>Workbook sizes</h3>
                  <p>
                    Each layer ranks workbooks by its own size. Matching X
                    positions can represent different workbooks.
                  </p>
                </div>
                <SceneViewport
                  scene={distributions.books}
                  renderer="auto"
                  title="Workbook sizes"
                  className="distribution-viewport"
                  compact
                />
              </section>
              <section className="panel distribution-card">
                <div className="chart-heading">
                  <p className="section-index">
                    PER WORKSHEET · SORTED INDEPENDENTLY
                  </p>
                  <h3>Worksheet sizes</h3>
                  <p>
                    Each layer ranks individual worksheets by its own size.
                    Matching X positions can represent different worksheets.
                  </p>
                </div>
                <SceneViewport
                  scene={distributions.sheets}
                  renderer="auto"
                  title="Worksheet sizes"
                  className="distribution-viewport"
                  compact
                />
              </section>
            </div>
          </section>
        )}
        <section className="panel job-results">
          <div className="panel-header">
            <h2>Jobs</h2>
            <span>{jobs.length} visible</span>
          </div>
          <div className="job-table">
            <table>
              <thead>
                <tr>
                  <th>Workbook</th>
                  <th>Worker</th>
                  <th>Status</th>
                  <th>Duration</th>
                  <th>{optimizing ? "Before → After" : "XLSX output"}</th>
                </tr>
              </thead>
              <tbody>
                {[...jobs].reverse().map((job) => (
                  <tr
                    key={job.id}
                    onClick={() => setSelected(job.id)}
                    className={job.id === selected ? "selected-job" : ""}
                  >
                    <td>
                      <button type="button" onClick={() => setSelected(job.id)}>
                        {job.name}
                      </button>
                      {job.error && <p className="failure">{job.error}</p>}
                    </td>
                    <td>{job.worker === null ? "—" : job.worker + 1}</td>
                    <td>{names[job.status]}</td>
                    <td>
                      {(
                        ((job.ended_ms ?? update.elapsed_ms) - job.started_ms) /
                        1000
                      ).toFixed(1)}{" "}
                      s
                    </td>
                    <td>
                      {optimizing && (
                        <>{fmtBytes(job.before?.file_bytes ?? null)} → </>
                      )}
                      {fmtBytes(job.after?.file_bytes ?? null)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      </main>
    </div>
  );
}
function Metric({
  label,
  value,
  note,
}: {
  label: string;
  value: string;
  note: string;
}) {
  return (
    <article className="metric">
      <div className="metric-label">{label}</div>
      <div className="metric-value">{value}</div>
      <div className="metric-note">{note}</div>
    </article>
  );
}

createRoot(document.getElementById("root")!).render(<Observatory />);
