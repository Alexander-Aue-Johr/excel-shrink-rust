export type DatasetSize = "small" | "research" | "stress";
export type EventType =
  | "workbook.started"
  | "workbook.metadata"
  | "job.started"
  | "job.completed"
  | "workbook.completed";
export interface WorksheetSpec {
  id: string;
  workbookId: string;
  index: number;
  name: string;
  originalBytes: number;
  cleanedBytes: number;
  chunkCount: number;
  hue: number;
}
export interface WorkbookSpec {
  id: string;
  index: number;
  name: string;
  sheets: WorksheetSpec[];
  originalBytes: number;
  cleanedBytes: number;
  startAt: number;
  metadataAt: number;
  completedAt: number;
}
export interface JobSpec {
  id: string;
  workbookId: string;
  sheetId: string;
  workerIndex: number;
  startAt: number;
  endAt: number;
  chunkIndex: number;
  chunkCount: number;
}
export interface DemoEvent {
  id: string;
  at: number;
  type: EventType;
  workbookId: string;
  sheetId?: string;
  jobId?: string;
  workerIndex?: number;
  payload?: Record<string, unknown>;
}
export interface DemoRun {
  id: string;
  size: DatasetSize;
  workerCount: number;
  workbooks: WorkbookSpec[];
  jobs: JobSpec[];
  events: DemoEvent[];
  duration: number;
  sheetCount: number;
}
export interface SheetState {
  id: string;
  workbookId: string;
  index: number;
  indexLabel?: string;
  name: string;
  originalBytes: number;
  cleanedBytes: number | null;
  chunkCount: number;
  hue: number;
  done: boolean;
}
export interface WorkbookState {
  id: string;
  index: number;
  name: string;
  known: boolean;
  status: "queued" | "processing" | "finalizing" | "complete";
  originalBytes: number | null;
  cleanedBytes: number | null;
  sheets: SheetState[];
  startAt: number;
  completedAt: number | null;
  progress: number;
}
export interface VisibleJob extends JobSpec {
  phase?: "initial" | "finalize" | "chunk";
  visibleEnd: number;
  done: boolean;
  sheet: SheetState;
  workbookIndex: number;
  workbookName: string;
}
export interface Snapshot {
  time: number;
  workbooks: WorkbookState[];
  sheets: SheetState[];
  jobs: VisibleJob[];
  events: DemoEvent[];
  activeJobs: VisibleJob[];
  completedBooks: number;
  knownBooks: number;
  processingBooks: number;
  completedSheets: number;
  beforeBytes: number;
  afterBytes: number;
  savedBytes: number;
  reduction: number;
}
export type PlaybackMode = "live" | "paused" | "replay";
export interface PlaybackState {
  liveTime: number;
  cursor: number;
  mode: PlaybackMode;
  playbackRate: number;
  simulationRate: number;
  running: boolean;
}
export type LabelMode = "name" | "xml";
export type JobStyle = "blocks" | "threads";
export type ResultStyle = "shrink" | "split";
export type RendererPreference = "auto" | "three" | "webgl" | "canvas";
export type RendererKind = "three" | "webgl" | "canvas";
