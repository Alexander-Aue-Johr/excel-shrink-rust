import { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import SceneViewport from "./components/SceneViewport";
import { buildWorkerScene } from "./lib/worker-scene";
import { buildDistributionScene } from "./lib/chart-scene";
import { fmtBytes } from "./lib/scene";
import {
  applyUpdate,
  scenes,
  type Configuration,
  type Job,
  type Update,
} from "./live";
import "./app/globals.css";
import "./styles.css";

const token = document.querySelector<HTMLMetaElement>(
  'meta[name="observatory-token"]',
)!.content;
const activeStates = new Set(["starting", "running", "stopping"]);
const names: Record<string, string> = {
  idle: "Bereit",
  starting: "Startet",
  running: "Läuft",
  stopping: "Wird gestoppt",
  stopped: "Gestoppt",
  completed: "Abgeschlossen",
  failed: "Fehlgeschlagen",
  prepared: "Erfolgreich",
  interrupted: "Unterbrochen",
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
  const [config, setConfig] = useState<Configuration | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
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
          throw new Error("Konfiguration fehlt in der Statusantwort");
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

  const rendering = useMemo(
    () => (update ? scenes(update, jobs) : null),
    [update, jobs],
  );
  const workerScene = useMemo(
    () =>
      rendering
        ? buildWorkerScene(rendering.snapshot, rendering.run, {
            stepHeight: 0,
            labelMode: "name",
            jobStyle: "blocks",
            windowSeconds: 120,
            overview: false,
            selectedWorkbookId: selected,
            workUnit: "workbook",
          })
        : null,
    [rendering, selected],
  );
  const distributions = useMemo(
    () =>
      rendering
        ? {
            books: buildDistributionScene(rendering.snapshot, {
              entity: "workbooks",
              metric: "size",
              labelMode: "name",
              order: "paired",
            }),
            sheets: buildDistributionScene(rendering.snapshot, {
              entity: "worksheets",
              metric: "size",
              labelMode: "xml",
              order: "paired",
            }),
          }
        : null,
    [rendering],
  );

  if (!update || !config || !rendering || !workerScene || !distributions)
    return (
      <main>
        <p>Observatory wird geladen…</p>
        {error && <p role="alert">{error}</p>}
      </main>
    );
  const optimizing = update.operation === "check-performance";
  const title = optimizing ? "Excel Check Performance" : "XLSX Convert";
  const active = activeStates.has(update.status);
  const completed = update.counts.prepared + update.counts.failed;
  const seconds = update.elapsed_ms / 1000;
  const perHour = seconds > 0 ? (completed / seconds) * 3600 : 0;
  const remaining = Math.max(0, update.counts.total - completed);
  const eta =
    perHour > 0 ? `${Math.ceil((remaining / perHour) * 60)} min` : "—";
  const focused = jobs.find((job) => job.id === selected) ?? jobs.at(-1);
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
            <div className="eyebrow">NATIVE EXCEL-VERARBEITUNG</div>
            <h1>
              {optimizing
                ? "Bereinigen. Nachvollziehen."
                : "Konvertieren. Nachvollziehen."}
            </h1>
            <p>
              {optimizing
                ? "Workbooks parallel mit Excel Check Performance optimieren."
                : "XLS und XLSX als separate XLSX-Dateien mit Excel speichern."}
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
            Eingabedateien oder Ordner · ein Pfad pro Zeile
            <textarea
              aria-label="Eingabepfade"
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
            Neues Ausgabeverzeichnis
            <input
              aria-label="Ausgabeverzeichnis"
              disabled={active}
              value={config.output ?? ""}
              onChange={(event) => change("output", event.target.value || null)}
            />
          </label>
          {optimizing && (
            <>
              <label>
                Quellmanifest · optional
                <input
                  disabled={active}
                  value={config.source_manifest ?? ""}
                  onChange={(event) =>
                    change("source_manifest", event.target.value || null)
                  }
                />
              </label>
              <label>
                Frühere Vorbereitung · optional
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
            Parallele Worker
            <input
              aria-label="Parallele Worker"
              type="number"
              min="1"
              max="256"
              required
              value={config.workers}
              onChange={(event) =>
                change("workers", Number(event.target.value))
              }
            />
          </label>
          <label>
            RAM- und Commit-Reserve · MiB
            <input
              aria-label="Speicherreserve"
              disabled={active}
              type="number"
              min="512"
              placeholder="Automatisch"
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
            Unterordner einbeziehen
          </label>
          <div className="form-actions">
            <button
              className="button primary"
              type="submit"
              disabled={active || busy}
            >
              Starten
            </button>
            <button
              className="button subtle"
              type="button"
              disabled={update.status !== "running" || busy}
              onClick={() =>
                void act("/api/workers", { workers: config.workers })
              }
            >
              Worker übernehmen
            </button>
            <button
              className="button subtle"
              type="button"
              disabled={!active || busy}
              onClick={() => void act("/api/stop")}
            >
              Stoppen
            </button>
          </div>
          <p className="wide form-note">
            Ergebnisse und Quellkopien liegen im Ausgabeverzeichnis. Das
            Ereignisprotokoll liegt daneben als .events.jsonl. Ein neuer Lauf
            braucht ein neues Ausgabeverzeichnis.
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
            note={`${update.counts.failed} fehlgeschlagen · ${update.counts.reused} wiederverwendet`}
          />
          <Metric
            label="Durchsatz"
            value={`${perHour.toFixed(1)} / h`}
            note={`${(perHour / 60).toFixed(2)} / min · seit Laufbeginn`}
          />
          <Metric
            label="Restzeit"
            value={remaining ? eta : "0 min"}
            note={`${remaining} ausstehend · Hochrechnung`}
          />
          <Metric
            label="Worker"
            value={`${rendering.snapshot.activeJobs.length} / ${update.defaults.workers}`}
            note={`${Math.round(seconds)} s bisher`}
          />
        </section>
        <div className="pipeline-grid">
          <section className="panel pipeline-panel">
            <div className="panel-header">
              <div>
                <div className="section-index">01 / PROZESS</div>
                <h2>Parallele Workbooks</h2>
              </div>
            </div>
            <SceneViewport
              scene={workerScene}
              renderer="auto"
              title="Worker-Zeitleiste"
              className="worker-viewport"
              onSelect={(hit) => setSelected(hit.id)}
            />
            <p className="view-note">
              Zeit × Worker · ein Rechteck pro Workbook · {update.history_limit}{" "}
              letzte Jobs im Arbeitsspeicher
            </p>
          </section>
          <aside className="panel focus-panel">
            <div className="panel-header">
              <h3>Im Fokus</h3>
            </div>
            {focused ? (
              <div className="focus-book">
                <h4>{focused.name}</h4>
                <p className={focused.error ? "failure" : "mint"}>
                  {names[focused.status] ?? focused.status}
                </p>
                <p>
                  Worker {(focused.worker ?? 0) + 1} ·{" "}
                  {(
                    ((focused.ended_ms ?? update.elapsed_ms) -
                      focused.started_ms) /
                    1000
                  ).toFixed(1)}{" "}
                  s
                </p>
                <p>
                  {fmtBytes(focused.before?.file_bytes ?? focused.source_bytes)}{" "}
                  → {fmtBytes(focused.after?.file_bytes ?? null)}
                </p>
                {focused.error && <p className="failure">{focused.error}</p>}
              </div>
            ) : (
              <p className="view-note">
                Mit dem ersten Auftrag erscheinen die Workbook-Details.
              </p>
            )}
          </aside>
        </div>
        {optimizing && (
          <section id="distributions">
            <div className="section-heading">
              <div>
                <div className="section-index">02 / ERGEBNISSE</div>
                <h2>Vor und nach der Optimierung</h2>
                <p>
                  Workbook-Bytes im ZIP · Worksheet-Größen als unkomprimierte
                  XML-Bytes · letzte sichtbare Jobs
                </p>
              </div>
            </div>
            <p className="view-note">
              Gesamtbilanz der neu optimierten Workbooks mit Größenmessungen ·{" "}
              {fmtBytes(update.counts.before_bytes)} →{" "}
              {fmtBytes(update.counts.after_bytes)}
            </p>
            <div className="distribution-grid">
              <section className="panel distribution-card">
                <div className="chart-heading">
                  <h3>Workbook-Größen</h3>
                </div>
                <SceneViewport
                  scene={distributions.books}
                  renderer="auto"
                  title="Workbook-Größen"
                  className="distribution-viewport"
                  compact
                />
              </section>
              <section className="panel distribution-card">
                <div className="chart-heading">
                  <h3>Worksheet-Größen</h3>
                </div>
                <SceneViewport
                  scene={distributions.sheets}
                  renderer="auto"
                  title="Worksheet-Größen"
                  className="distribution-viewport"
                  compact
                />
              </section>
            </div>
          </section>
        )}
        <section className="panel job-results">
          <div className="panel-header">
            <h2>Aufträge</h2>
            <span>{jobs.length} sichtbar</span>
          </div>
          <div className="job-table">
            <table>
              <thead>
                <tr>
                  <th>Workbook</th>
                  <th>Worker</th>
                  <th>Status</th>
                  <th>Dauer</th>
                  <th>{optimizing ? "Vorher → Nachher" : "XLSX-Ausgabe"}</th>
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
                    <td>{(job.worker ?? 0) + 1}</td>
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
