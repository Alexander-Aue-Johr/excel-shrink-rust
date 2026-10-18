# Validation architecture

Excel-backed validation is separated into stored-state, recalculation, and rendering
checks. Excel Shrink itself remains an OOXML transformation pipeline.

## Canonical baseline

When Excel-backed validation is enabled, the corpus workbook is copied to an immutable
baseline after the narrowly scoped `xl/workbook.xml` defined-name repair. That repaired
baseline is the input to Excel Shrink and to the stored-state screenshot comparison.
Package auditing records unexpected member changes instead of hiding them behind cache
misses.

## Borrowed value comparison

`value_views.rs` scans worksheet XML with borrowed byte views. The normal ordered path
does not allocate one owned object per cell. Shared strings are indexed once and
referenced through compact ranges/digests. Per-sheet value/formula digests provide the
fast path; a digest mismatch falls through to the detailed comparator.

Stored `<v>` bytes are cached Excel results, not proof that a formula is current.
Preservation checks therefore also compare formula elements. Shared strings and inline
strings are resolved as text. Malformed or unsupported structures fail conservatively.

## Recalculation

Recalculation is shrunk-first:

1. Compare repaired baseline and direct shrink output without Excel.
2. Open only the shrunk workbook in Excel, call `CalculateFullRebuild()` once, save a
   separate copy, and compare pre/post-recalculation stored values.
3. If no stored value changed, validation is complete and the large baseline is never
   opened in Excel.
4. If values changed, recalculate the paired baseline and compare the two freshly
   recalculated workbooks on the affected sheets. This distinguishes a shrink-induced
   change from stale/volatile source results.

A content-addressed recalculated baseline may still be reused when the conservative
calculation certificate allows it. Unverified/volatile/external calculation contexts
are not persisted as timeless references.

The PowerShell recalculation script is intentionally linear. It does not poll
`CalculationState`, sleep, or emit heartbeat loops. Rust measures the blocking
recalculation interval, renders the one-line terminal spinner, and enforces
`EXCEL_SHRINK_RECALC_TIMEOUT_SECONDS`. Non-recalculation automation uses
`EXCEL_SHRINK_AUTOMATION_STALL_SECONDS`.

## Screenshot validation

Screenshots compare the repaired baseline with the direct shrink output and never use
recalculation-saved workbooks. Workbooks are opened read-only with manual calculation,
events/macros disabled, and external-link updates disabled.

Screenshot reuse is guarded by stored render inputs rather than calculation-cache
eligibility. Ordinary external workbook-link metadata is therefore cacheable when link
updates are disabled. Volatile/refresh-on-open inputs and external visual relationships
(linked images, OLE objects, controls) remain runtime-sensitive and disable reuse.
Contextual sheets such as charts/conditional-formatting sheets retain broader package
dependencies.

Screenshot cache version 10 intentionally invalidates older identities after these
rules changed.

### Capture failure and drawing-bound policy

Rust-generated JSON consumed by Windows PowerShell 5.1 is read explicitly as UTF-8.
Known DrawingML metadata extensions (`a16:creationId`, `a14:hiddenFill`,
`a14:hiddenLine`) stay on the OOXML fast path; unknown extensions and unsupported
geometry remain COM fallbacks. Duplicate shape names are treated as ambiguous rather
than silently deduplicated.

Repeated `Range.CopyPicture` failures and memory-like capture errors trigger recursive
bisection along the physically longer axis. Clipboard-provenance and file-system/PNG
failures do not trigger bisection.

Real-world cases are recorded in `tools/visual-tests/REAL_WORLD_VISUAL_CASES.md`.

## Generated state and logs

Repository-local transient state has one root:

```text
.tmp/
  regression/
  regression-cache/
  screenshots/
```

The old `regression/tmp/` path is retained only as a cleanup target.

Each process writes a unique run log:

```text
logs/runs/excel-shrink-run-<unix-ms>-<pid>.log
```

Visual timing/slow JSONL files remain independent diagnostics. `tools/clean_generated.py`
and the matching VS Code tasks clear caches or reset all local generated state without
deleting corpus files, baselines, reports, or measurements.

## Verification

```console
cargo fmt --all -- --check
cargo check -p excel-shrink-cli --bin excel-shrink-cli
cargo test -p excel-shrink-cli --bin excel-shrink-cli regression::excel_automation -- --nocapture
cargo test -p excel-shrink-cli --bin excel-shrink-cli visual_ -- --nocapture
python tools/visual-tests/benchmark_value_compare_v9.py --rows 20000
```
