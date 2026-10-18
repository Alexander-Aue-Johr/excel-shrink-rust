# Results and provenance

## Local runs

Each execution creates one managed directory under `docs/visual-validation/results/runs`.
The suite view links to each workbook's bounds provenance, lazy-loaded screenshots,
calibration, context crops and comparison evidence. The run directory holds:

- `report.html`, `summary.json`, `case-results.csv`;
- `evaluation-state.json`, `resolved-plan.json`, `source-manifest.json`;
- `cases/` with original/final PNGs, native outputs and logs;
- `review-metadata.zip` with a checked manifest, omitting binary image artifacts.

The standard report-history task opens `results/index.html`. Pruning is explicit,
retains active and resumable runs, and never targets inputs, build caches or versioned
evidence. Regenerating a view does not alter capture timings. Old controller reports
remain read-only when their executors are no longer part of this protocol.

## Versioned evidence

The small records under `../evidence/` are intended for version control: results JSON,
CSV, independent HTML and a SHA-256 manifest. `current.json` and the evidence index
identify the current record. Screenshots, full logs, temporary captures, full source
exports and thousands of repeated receipts remain local and ignored by Git.

The included `20261017-reviewed-captures` record preserves reviewed observations from
before this packaged source, including failed observations in the selected historical
screen and the full final budget experiment. Its source identities and limitations
are explicit. It is not presented as a native run of the reconstructed source.

## Publish a new run

After inspecting a completed run:

```powershell
& .\docs\visual-validation\experiments\publish-evidence.ps1 `
    -RunDirectory '<full managed run directory>'
```

The **Visual validation: Publish evidence** task selects the latest finished evaluation
run when no directory is given. Publication verifies the entire metadata archive
against its manifest and the archived state against the current state, then writes
an allowlisted compact record in a staging directory. It validates the new manifest
before moving it to its immutable run-named directory and updating the index.
An existing published ID is not overwritten. Publishing does not run Excel, create
a Git commit, change `decisions.md` or declare an incomplete run successful.

The compact record includes exact worksheet identifiers, source/archive hashes,
settings, timings, statuses and locally computed canonical pixel hashes. It excludes
host-local absolute paths, tokens, process metadata, screenshots and cell values.
Names can still identify input data; review the publication before committing.

Results from different source versions, environments, or selected scopes are not
interchangeable. Candidate agreement is not independent visual correctness. Update
only the appropriate measurement tables and retain the scope of every observation.
