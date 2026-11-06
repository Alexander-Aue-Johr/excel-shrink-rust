# SheetScope sources and external processing

SheetScope's input acquisition and process orchestration are independent of its
existing targeted comparison engine. They do not invoke Excel Shrink implicitly,
interpret its output as proof of visual equality, or change capture selection.
The run coordinator owns the sequence of stages, result retention and report
location. `validation_sources.rs` owns source materialisation;
`validation_process.rs` owns one external processing stage.

## Source contract

A materialised source has a root directory, a map of portable relative workbook
names to actual files, and an optional original corpus ZIP. The map is
authoritative. A directory may also contain failed outputs, so passing its path
alone to a later stage would be insufficient.

Supported inputs are local directories, individual workbooks, local ZIPs and
HTTP(S) URLs whose response is a ZIP. Workbook discovery recognises `.xlsx`,
`.xls`, `.xlsm` and `.xlsb`, case-insensitively, and ignores Excel `~$` lock files.
This is an acquisition/processing capability: the targeted visual engine still
has its own XLSX requirements. Discovering an XLS file does not establish that it
can be compared visually without a baseline-conversion adapter.

Local directories are enumerated recursively and used in place. ZIP extraction
streams workbook members into a new, uniquely named directory below the selected
work area. It does not load whole workbooks into memory, extract non-workbook
sidecars, or flatten subdirectories. Before extraction, every archive member is
checked for unsafe paths, duplicate names, case aliases, symlinks and
file/directory collisions, including members that would otherwise be ignored.
Names must also be usable on Windows. Directories containing symlinks or Windows
reparse points are rejected rather than traversed. ZIPs are currently limited to
100,000 entries and one TiB of declared expanded content; extracted lengths are
checked against each member's declaration. Other archive formats are not
implemented in this patch.

Downloads use the same existing system prerequisite as the legacy archive
runner: `curl`. The cache directory is derived from SHA-256 of the full source
URL. An exclusive per-URL file lock prevents two runs from publishing competing
partial downloads. A completed download is checked as a ZIP, hashed, flushed and
published under its content hash; an atomic `current.json` selects that version.
Reusing it verifies its stored length and SHA-256. Refreshing the URL creates a
new immutable version, so a source already materialised by another run does not
silently change. Failed partial downloads are not cache hits. The cache does not
claim a remote URL is immutable or automatically check whether its server-side
content changed; refreshing is explicit.

`curl` configuration and diagnostics remain local to the download cache. A
private redirect URL in those diagnostics is not added to a portable report by
the source layer. There is no browser/session authentication adapter in this
source implementation.

## One declared executable per stage

A processing stage is serialisable JSON. For example, Excel Shrink's actual
single-file invocation is:

```json
{
  "name": "Excel Shrink",
  "program": "C:\\tools\\excel-shrink-cli.exe",
  "input": "file",
  "output": "directory",
  "arguments": [
    "{input}",
    "{output_dir}",
    "--max-active-workbooks", "1",
    "--rayon-threads", "1",
    "--writer-workers", "1"
  ],
  "timeout_seconds": 3600
}
```

The three worker limits in this example are explicit processor arguments. They
prevent outer file-process parallelism from accidentally multiplying Excel
Shrink's internal CPU/workbook/writer concurrency. SheetScope does not infer
those limits for an arbitrary executable.

Excel Shrink currently accepts a single `.xlsx` or a directory of `.xlsx`
workbooks; a directory requires `--recursive` to include subdirectories. Its
second positional argument is an **output directory**, including for one input
file. It does not accept a corpus ZIP as its normal cleanup input. These rules
come from `collect_input_files`, `is_real_xlsx_file` and `pipeline::batch_paths`.
A directory stage therefore uses `input: "directory"`, `output: "directory"`
and arguments `["{input}", "{output}", "--recursive"]` plus the desired cleanup
settings. No archive capability is inferred from the separate regression CLI.

Supported placeholders are:

| Placeholder | Meaning |
| --- | --- |
| `{input}` | One workbook, a directory or a ZIP, according to `input` |
| `{output}` | One workbook, a directory or a ZIP, according to `output` |
| `{output_dir}` | Final workbook directory; for a file invocation, its relative parent |
| `{output_file}` | Exact expected workbook path; allowed only for file invocations |

Each array item remains exactly one command-line argument. Paths containing
spaces or command-looking characters are not split or evaluated. Literal braces
are written as `{{` and `}}`; unknown or unbalanced placeholders are configuration
errors. The same substitutions work in optional `environment` overrides.
`working_directory` can select the processor's working directory. Executables
must be explicit filesystem paths; a command name is not guessed from help text
or automatically resolved to an interpreter. To run a script, configure its
interpreter as the executable and pass the script path as a separate argument.
List scripts and external configuration files in `dependencies` as well when
processing-result reuse is enabled. This list declares content to fingerprint;
it does not implicitly pass those paths to the processor.

`output_extension` optionally changes the expected extension, for example
`"xlsx"` or `".xlsx"`. All other components of the relative filename are retained.
An input pair such as `sample.xls` and `sample.xlsx` is rejected before execution
if conversion would make both outputs `sample.xlsx`. Case-insensitive output
collisions are likewise rejected. Renaming arbitrary stems, dropping directory
structure or creating one-to-many outputs is outside this stage contract.

## Granularity, ownership and file movement

For `input: "file"`, SheetScope owns a bounded set of child processes and starts
one invocation per accepted input. For directory or ZIP input, the entire stage
starts one process; the configured outer process limit does not subdivide that
batch. The processor remains responsible for its own internal concurrency.

`output: "file"` requires file invocation. Directory and ZIP output work with all
three input granularities. Expected files must not already exist, and an output
root cannot be the input root. Batch output directories must initially be empty;
a directory processor cannot write beneath the directory it reads.

A complete directory input is passed in place, avoiding a corpus copy. After a
partial or failed earlier stage, a directory processor receives a temporary view
containing only accepted workbooks if the underlying directory also contains
other workbooks. Those files are copied, not hard-linked: a processor that edits
its inputs must not mutate the previous stage through a staging hard link.
Programs are still required to honour the separate input/output contract; this
is process orchestration, not a filesystem sandbox for arbitrary executable code.
Non-workbook sidecars in a directly supplied local directory remain visible to
that processor; declared dependency/version settings must account for them when
persistent processing reuse is enabled by the coordinator. The coordinator also
fingerprints complete local directory contents for directory invocation and the
raw original archive for archive invocation, so visible sidecars are not silently
left out of the cache identity. A nonempty user-controlled processing cache key
records assumptions about dependencies that cannot be captured as regular files,
such as service/model versions, plugins and inherited environment. Changing such
an assumption requires a new key; executable and argv identity alone do not
establish it.

A complete original ZIP is passed directly to an archive processor. Other
accepted input sets are assembled into a temporary ZIP. ZIP outputs are checked
and unpacked into owned scratch space, and expected workbooks are renamed into
their final destinations on the same filesystem. For a batch, archive members
must retain their relative names. For a single-file invocation, the expected
member is the resulting workbook's basename. ZIP packaging uses Store because
modern Excel files are already ZIP-compressed. Only the selected workbook map is
packaged, excluding stale outputs, logs and unrelated files.

The stage's scratch directory is removed after collection, and does not become
part of its report. Direct directory/file output goes to the caller's final
workbook paths without a redundant intermediate copy. Whether the coordinator
chooses a persistent output directory or temporary outputs is independent of the
report directory.

## Outcomes and failures

Each stage writes `stage.json`; each invocation writes an atomic
`invocations/NNNNNN/outcome.json` and separate `stdout.log` and `stderr.log` files.
Output streams go directly to disk, not into an unbounded in-memory buffer or
repeatedly rewritten event array. Receipts are persisted at pending, running and
terminal boundaries. Terminal logs are flushed before the completed receipt.
A run that crashes can therefore leave an explicit running attempt, without
certifying its output. These files are process provenance; they are not visual
validation results.

Receipts include the affected input names, expected/produced/accepted/missing
output names, expanded argv, configured environment overrides, working directory,
exit code/description, timeout or other error, and timestamps. The final stage
snapshot is written even when fail-fast returns an error. If a higher-level I/O
failure prevents that final snapshot, already-written invocation receipts remain
available independently.

With continuation disabled, the first observed failed invocation stops further
dispatch. Already-owned processes finish and their outcomes are retained; queued
invocations are marked skipped. With continuation enabled, successful file
invocations continue. A zero-exit batch that omits some expected files contributes
its present verified files and records the missing ones. A nonzero or timed-out
invocation contributes no accepted files, even if it left apparently complete
workbooks: presence alone cannot establish that its processing succeeded. Those
paths remain listed as produced outputs where applicable. File-format validity
and visual evidence are established by later stages, not by an exit code.

Each invocation has a configurable timeout, defaulting to one hour. A whole-
corpus directory or archive stage may need a longer explicit limit. Timeout
cleanup targets only the invocation's owned tree: a private process group on
Unix, and `taskkill /PID /T /F` while retaining the live child handle on Windows.
Unconfirmed termination stops further work even when continuation was requested.
It propagates as `UnconfirmedTermination`, including through error context, so the
coordinator retains working inputs. Owned stage scratch is also retained and its
path is included in the error. No global Excel/process-name kill is used. Processors must synchronously await
their workers rather than detach background services; arbitrary detached-process
ownership is not inferred by this adapter.

## Verification included with this patch

The Rust tests cover path traversal/case collisions, preserving existing files,
source and archive round trips, corrupted download cache entries, literal argv
substitution, extension collisions, bounded file subprocesses, batch granularity,
nonzero exits, missing outputs, timeouts and propagating only accepted inputs.
The process tests launch the current test executable as a controlled fixture;
they require neither Excel nor a third-party converter. They were provided for
the local agent and were not executed in the SCE editing environment.
