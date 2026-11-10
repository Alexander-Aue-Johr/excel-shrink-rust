import { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import SceneViewport from "./components/SceneViewport";
import SheetDetails from "./components/SheetDetails";
import { buildWorkbookScene } from "./lib/workbook-scene";
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
  const [update, setUpdate] = useState<Update | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [tasks, setTasks] = useState<WorkTask[]>([]);
  const [config, setConfig] = useState<Configuration | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [stepHeight, setStepHeight] = useState(0.14);
  const [atlasView, setAtlasView] = useState<"three" | "seven" | "all">(
    "three",
  );
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
    if (atlasView !== "all" || !update) return;
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
  }, [atlasView, atlasPage, atlasPageSize, update?.run_id, update?.status]);

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
                ? Math.max(0.05, Math.min(70, update.elapsed_ms / 1000))
                : 120,
            overview: false,
            selectedWorkbookId: selected,
            workUnit:
              update?.operation === "sheet-shrink" ? "worksheet" : "workbook",
          })
        : null,
    [rendering, selected, stepHeight],
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
  const atlas = useMemo(
    () =>
      rendering
        ? buildWorkbookScene(
            atlasView === "all"
              ? atlasSnapshot(
                  scenes(
                    update!,
                    atlasHistory?.run_id === update!.run_id
                      ? atlasHistory.jobs
                      : [],
                  ).snapshot,
                  atlasHistory?.jobs ?? [],
                )
              : atlasSnapshot(rendering.snapshot, jobs),
            {
              resultStyle: "split",
              labelMode: "name",
              view: atlasView === "all" ? "all" : "recent",
              sort: atlasView === "all" ? "size" : "time",
              recentCount: atlasView === "seven" ? 7 : 3,
              selectedWorkbookId: selected,
            },
          )
        : null,
    [rendering, selected, jobs, atlasView, atlasHistory],
  );

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
  const eta =
    perHour > 0 ? `${Math.ceil((remaining / perHour) * 60)} min` : "—";
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

  return (
    <div className="observatory">
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
          {names[update.status]}
        </span>
      </header>
      <main>
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
                Maximum chunk size · bytes
                <input
                  aria-label="Chunk size"
                  type="number"
                  min="0"
                  disabled={active}
                  value={config.parallel_range_chunk_size ?? 16777216}
                  onChange={(event) =>
                    change(
                      "parallel_range_chunk_size",
                      Number(event.target.value),
                    )
                  }
                />
              </label>
            </>
          )}
          <label>
            RAM and commit reserve · MiB
            <input
              aria-label="Memory reserve"
              disabled={active}
              type="number"
              min="512"
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
          <Metric
            label="Time remaining"
            value={remaining ? eta : "0 min"}
            note={`${remaining} pending · estimate`}
          />
          <Metric
            label="Worker"
            value={`${activeWorkers} / ${update.defaults.workers}`}
            note={`${Math.round(seconds)} s elapsed`}
          />
        </section>
        <div className="pipeline-grid">
          <section className="panel pipeline-panel">
            <div className="panel-header">
              <div>
                <div className="section-index">01 / PROCESS</div>
                <h2>
                  {detailed
                    ? "Parallel sheets and chunks"
                    : "Parallel workbooks"}
                </h2>
              </div>
              {detailed && (
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
              )}
            </div>
            <SceneViewport
              scene={workerScene}
              renderer="auto"
              title="Worker timeline"
              className="worker-viewport"
              onSelect={(hit) =>
                setSelected(
                  detailed
                    ? (tasks.find((task) => `task-${task.id}` === hit.id)
                        ?.workbook_id ?? null)
                    : hit.id,
                )
              }
            />
            <p className="view-note">
              {detailed
                ? "Time × Worker × Workbook · sheets and chunks · "
                : "Time × Worker · one block per workbook · "}
              {update.history_limit} recent jobs retained in memory
            </p>
          </section>
        </div>
        {detailed && atlas && (
          <section className="panel atlas-panel">
            <div className="panel-header">
              <h2>Workbook-Atlas</h2>
              <span>
                Uncompressed ZIP bytes · original and output
                {atlasView === "all" && " · largest original workbooks first"}
              </span>
            </div>
            <div className="atlas-controls">
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
                    disabled={atlasPage === 0}
                    onClick={() => setAtlasPage((page) => page - 1)}
                  >
                    Previous page
                  </button>
                  <span>
                    Page {(atlasHistory?.page ?? atlasPage) + 1} /{" "}
                    {Math.max(
                      1,
                      Math.ceil((atlasHistory?.total ?? 0) / atlasPageSize),
                    )}{" "}
                    · {atlasHistory?.total ?? 0} workbooks
                  </span>
                  <button
                    disabled={
                      !atlasHistory ||
                      (atlasPage + 1) * atlasPageSize >= atlasHistory.total
                    }
                    onClick={() => setAtlasPage((page) => page + 1)}
                  >
                    Next page
                  </button>
                </>
              )}
            </div>
            {atlasView === "all" && atlasError && (
              <p role="alert">{atlasError}</p>
            )}
            <SceneViewport
              scene={atlas}
              renderer="auto"
              title="Workbook-Atlas"
              className="atlas-viewport"
              onSelect={(hit) =>
                setSelected(
                  jobs.find(
                    (job) =>
                      hit.id === `book:${job.id}` ||
                      hit.id.startsWith(`sheet:${job.id}:`),
                  )?.id ?? null,
                )
              }
            />
          </section>
        )}
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
