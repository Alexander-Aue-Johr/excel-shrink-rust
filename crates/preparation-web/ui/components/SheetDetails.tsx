import { useMemo, useState } from "react";
import type { Job, WorkTask } from "../live";
import { fmtBytes } from "../lib/scene";

export function duration(milliseconds: number): string {
  return milliseconds < 1000
    ? `${Math.max(0, milliseconds).toFixed(3)} ms`
    : `${(milliseconds / 1000).toFixed(2)} s`;
}

export default function SheetDetails({
  job,
  tasks,
  now,
}: {
  job: Job;
  tasks: WorkTask[];
  now: number;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [limit, setLimit] = useState(100);
  const grouped = useMemo(() => {
    const groups = new Map<string, WorkTask[]>();
    for (const task of tasks) {
      if (task.workbook_id !== job.id) continue;
      const group = groups.get(task.sheet) ?? [];
      group.push(task);
      groups.set(task.sheet, group);
    }
    return groups;
  }, [tasks, job.id]);
  const sheets = job.before?.worksheets ?? [];
  const after = new Map(
    job.after?.worksheets.map((sheet) => [sheet.part, sheet]) ?? [],
  );
  const part = selected ?? sheets[0]?.part;
  const phaseNames = {
    initial: "Initial cleanup",
    finalize: "Finalization",
    chunk: "Chunk",
  };
  const selectedTasks = grouped.get(part) ?? [];
  return (
    <section className="panel sheet-details">
      <div className="panel-header">
        <div>
          <div className="section-index">WORKBOOK / SHEETS / CHUNKS</div>
          <h2>{job.name}</h2>
        </div>
        <span>{sheets.length} Sheets</span>
      </div>
      <div className="sheet-detail-grid">
        <div className="job-table">
          <table>
            <thead>
              <tr>
                <th>Sheet</th>
                <th>XML before → after</th>
                <th>Chunks recorded</th>
              </tr>
            </thead>
            <tbody>
              {sheets.slice(0, limit).map((sheet) => (
                <tr
                  key={sheet.part}
                  className={sheet.part === part ? "selected-job" : ""}
                >
                  <td>
                    <button
                      type="button"
                      onClick={() => setSelected(sheet.part)}
                    >
                      {job.sheet_names?.[sheet.part] ?? sheet.part}
                    </button>
                    <small>{sheet.part}</small>
                  </td>
                  <td>
                    {fmtBytes(sheet.uncompressed_bytes)} →{" "}
                    {fmtBytes(
                      after.get(sheet.part)?.uncompressed_bytes ?? null,
                    )}
                  </td>
                  <td>
                    {
                      (grouped.get(sheet.part) ?? []).filter(
                        (task) => task.phase === "chunk",
                      ).length
                    }
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {sheets.length > limit && (
            <button
              className="button subtle"
              onClick={() => setLimit(limit + 100)}
            >
              Show more sheets
            </button>
          )}
        </div>
        <div className="job-table">
          <table>
            <thead>
              <tr>
                <th>Phase / Chunk</th>
                <th>Worker</th>
                <th>Start</th>
                <th>Duration</th>
              </tr>
            </thead>
            <tbody>
              {selectedTasks.slice(-256).map((task) => (
                <tr key={task.id}>
                  <td>
                    {phaseNames[task.phase]}
                    {task.phase === "chunk" && (
                      <>
                        {" "}
                        {(task.chunk_index ?? 0) + 1}/{task.chunk_count}
                        <small>{task.range}</small>
                      </>
                    )}
                  </td>
                  <td>{task.worker === null ? "—" : task.worker + 1}</td>
                  <td>{duration(task.started_ms)}</td>
                  <td>
                    {duration((task.ended_ms ?? now) - task.started_ms)}
                    {task.ended_ms === null && " · active"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
      <p className="view-note">
        Phases show elapsed time and may contain parallel chunks. The live view
        retains up to 4096 tasks and shows the latest 256 entries for the
        selected sheet.
      </p>
    </section>
  );
}
