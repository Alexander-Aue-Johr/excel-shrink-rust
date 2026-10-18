# Results and provenance

## Local execution

Each run lives under `docs/visual-validation/results/runs/<id>/`. The run-history
and workbook pages link to bounds, calibration, context crops, final screenshots
and individual outcomes. Core records are `evaluation-state.json`, `summary.json`,
`case-results.csv`, `resolved-plan.json` and `source-manifest.json`. `cases/` retains
full producer data, logs, original/final images and the standalone workbook inspectors.

A completed execution is not necessarily a successful capture. The summary groups
unresolved worksheet outcomes by workbook, original worksheet index, exact name and
error code. Repetitions remain individual cases but are not counted as new defects.
Missing canonical digests, incomplete scopes and changed repeats cannot qualify a
configuration. Unchanged-input agreement is not original/shrunk equivalence.

## Review bundle

`review-metadata.zip` is a **compact projection by default**. It contains the complete
core records, complete case metrics/specifications, final lifecycle and supervisor
receipts, preflight results, and a clearly labelled metadata page for each capture
case. These pages are created inside the archive; local screenshot inspectors are
not replaced. Full plans, producer journals, repeated snapshots and report assets
remain local. Long textual logs receive an explicitly labelled head/tail excerpt.
Native/renderer diagnostic measurement files are retained conservatively.

`bundle-manifest.json` identifies the profile, included files, transformations and
omissions. Excerpts and omitted textual support files record original sizes and
SHA-256 values; image receipts remain in the case metrics. Nothing is silently
presented as a complete log. The writer validates entry membership, sizes and hashes
before publication. Compact does not mean anonymized: case data and excerpts can
still contain local paths and input identifiers.

To rebuild a review bundle without opening Excel:

```powershell
& .\docs\visual-validation\experiments\reproduce.ps1 `
    -ResumeRun '<managed run directory>' -ReportOnly
```

Add `-FullMetadata` only when all original local text evidence is needed. This is a
report-only option, not a new experiment. It still omits image bytes. The compact
projection and full detail use the same archive filename; all local originals remain
available to generate either view again. Completed measurement times are unchanged.

## Version control

| Keep in Git | Keep local |
|---|---|
| Source, tests, protocol and pinned input registry | Downloaded corpus files, `.tmp`, caches and build outputs |
| Cargo lockfiles for the executable workspaces | Generated producer snapshots and full execution logs |
| Seven named synthetic XLSX fixtures and their manifests | Other generated fixture variants |
| Required prebuilt height/metadata helper XLSM files plus BAS source | The rebuildable `tools/excel-lab/ExcelLabHost.xlsm` |
| Small immutable records under `../evidence/` | Complete run directories and review ZIPs |

The height/metadata helpers are input dependencies of a separate metadata-comparison
workflow, not capture results. Editing their BAS source does not rewrite the embedded
macro. Do not remove the helpers without replacing their build/distribution contract.

Use the existing validation task to check code and record a **read-only Git audit**.
For only membership/ignore checks:

```text
python tools/validate-visual-experiments.py --audit-only
```

The result is `.tmp/visual-validation/repository-audit.json`. It distinguishes tracked
paths, non-ignored new paths, ignored tracked files and missing required inputs. It
never changes the index or deletes files. A source export without `.git` cannot
establish tracked status and is reported as unavailable, not clean. Audit-only exits
with 2 when review is needed or Git evidence is unavailable. Code validation remains
separate from membership findings. An ignore rule does not untrack an existing file;
review exact paths before explicitly removing any generated item from the index.
Keep the existing `evidence/** -text` Git attribute so manifests survive checkout
without newline rewriting. The auditor checks working files, not staged bytes.

## Published records

After inspecting a finished evaluation, run **Visual validation: Publish evidence**
or `publish-evidence.ps1 -RunDirectory '<managed run directory>'`. The publisher
verifies the source archive and its state, then writes an allowlisted JSON, CSV,
self-contained HTML and SHA-256 manifest under an immutable run-named directory.
Existing published IDs are not overwritten. `current.json` and the index link to
the latest published record, including explicit exceptions when present.

Published records exclude screenshots, machine paths, process metadata, tokens and
cell values. Exact input and worksheet names remain for reproducibility. Input
hashes, effective capture settings, phase limits, statuses and canonical image hashes
are retained. Repeated pixels are labelled equal only when **all** recorded repeats
are qualified and their recorded environments and canonical outputs agree.

Historical observations remain in `20261017-reviewed-captures`. Later records are
separate replications, not retroactive edits to those measurements. Publication does
not run Excel, commit files or automatically change `decisions.md`. Pruning local
run history is explicit, retains active/resumable runs and never targets inputs,
build caches or published evidence.
