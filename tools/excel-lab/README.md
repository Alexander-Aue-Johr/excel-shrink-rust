# Excel Lab — shared VBA host, WPF, Rust postprocessing

This is an **experimental measurement workbench**, separate from the existing
production regression runner. The Python installer adds its files and VS Code
tasks and fixes the legacy JSON-diff COM-open/cleanup wrappers. It does not edit
the Rust shrink engine or replace the existing regression algorithms.

## Start

Run `patch_excel_lab.py` in the repository root with no arguments; `--dry-run`
only previews changes. The installer has no external Python dependencies.
Existing edited targets are guarded; backups and a standalone `restore.py` are
created before replacement. PowerShell pure tests run before source changes
when Windows PowerShell is reachable. A failed preflight retains its staged
files, exact reproduction command and log. Excel is **not** opened at install time.

1. VS Code: **Excel lab: Build macro host**. This imports reviewed `.bas` sources,
   runs VBA fixture tests, saves, closes, reopens and verifies the host before
   publishing `ExcelLabHost.xlsm` and `ExcelLabHost.manifest.json`.
2. **Excel lab: Excel and clipboard self-test** explicitly exercises real Excel,
   the in-process bitmap utilities and actual CopyPicture. It uses disposable
   fixture workbooks, not your data. It changes the clipboard contents.
3. **Excel tools: Select file or folder - Shrink - JSON diff** opens the new WPF
   window. Default: shrink + fast UsedRange metadata + visible shape geometry;
   **no exhaustive row scan, screenshots or value scan unless selected**.

Windows and WSL are both supported launch contexts. In WSL the launcher invokes
Windows `powershell.exe -STA`; host generation does not require switching to a
separate native Windows VS Code session. Native Linux without Windows/Excel is
not a supported measurement backend.

The old height tasks and regression tasks remain as they were. The added
**Excel lab: Legacy exhaustive row JSON diff** runs the previous row-only
workflow for compatibility; that workflow can still use its separate legacy
`JsonDiffProbeHost.xlsm`. The new WPF workflow uses the shared host exclusively.

## One source collection, one generated host

Editable sources are `vba/*.bas`, plus the existing
`../excel-height-check/HeightProbe.bas` and `JsonDiffProbe.bas`. The originals
are imported directly, not duplicated as a second manually maintained copy.
`LabBuild` is generated during the build and contains the exact source-set
signature. Module names must be unique; `.bas` files are UTF-8 source files and
are explicitly converted to the local VBA import code page for the build.
Unrepresentable source characters stop the build instead of being replaced.
The supplied new VBA sources are ASCII (Unicode data is escaped at runtime).

The runtime compares current source hashes, manifest signature and host SHA-256
and calls the embedded signature. Stale hosts fail early with a rebuild hint.
Editing the binary manually is not the source workflow. After a BAS edit, run
the build task again. A failed build does not publish an unvalidated host.
Previous published host files are backed up under the build run directory.

Automated BAS import needs access to `Workbook.VBProject.VBComponents`.
**No registry, Trust Center, Protected View or policy setting is changed.**
If project access is disabled, explicitly enable “Trust access to the VBA
project object model” for the build, then disable it again, or build on a
permitted developer Windows machine. Macro execution and VBA-project editing
are different permissions. The runtime needs the compiled host, not project
editing access. A generic COM error is not by itself proof of a Trust Center
restriction.

## Corrected legacy interop

`Open-LabWorkbook` checks the local OOXML package and, for a helper, the presence
of `xl/vbaProject.bin`; uses explicit method dispatch and culture (LCID 1033);
opens a helper with the minimal three arguments; and prevents pipeline
iteration of returned COM objects. The old 15-argument calls were replaced in
both legacy helper/target wrappers. File existence alone is not host validation.

An `Open` failure logs the actual **local** path, size, Excel and PowerShell
versions, nested exceptions and HRESULT values. The earlier workflow already
copied the helper locally: a WSL UNC path alone was not an established cause.
The exact cause of the reported machine-specific failure cannot be inferred
from its translated generic message. These changes harden dispatch and reveal
underlying causes; they do not claim that every policy/corruption problem is
repairable with different arguments. No automatic repair or second open is
performed behind the user's back.

Cleanup checks owner token + PID + process creation time + EXCEL process name,
allows a graceful exit, and permits at most one forced termination attempt per
identity. It never kills all Excel processes, elevates privileges, retries an
access denial, or turns cleanup failure into the primary error. A successful
`Quit()` call is logged as a request, not as proof that the process is gone.

## Modules and output

| Module | Implemented operation | Important boundary |
|---|---|---|
| `LabRanges` | Fast UsedRange address, rows/columns, height/width in points; matched Shrunk range in both inputs | Total height can hide compensating row changes |
| `LabBounds` | Shape inventory; UsedRange + visible-shape geometric envelope; cell envelope | Not an exact painted-pixel bound |
| `LabRows` | Every row in all/used-union scope, exact merged run-length encoding | Explicitly expensive; off by default |
| `LabHash`, `LabValues` | Native Windows CNG SHA-256 of typed values/coordinates; blockwise direct VBA value comparison | Not formula-text, style, or logical calculated-equivalence comparison |
| `LabPixels`, `LabCapture` | Owned pixel/EMF buffers, tiled captures, PNG/BMP, CopyPicture matrix, VBA pixel comparisons | Clipboard/render behavior still needs the real Excel self-test |
| `LabOverflow` | Repeated baseline, pixel-probe heuristic or isolated text-removal experiment | Experimental; not production overflow certification |
| `LabMain` | Complete XML request read once, workbook pairs and all plans executed inside one VBA call | Only progress/cancellation files are polled externally |
| `LabSelfTest` | SHA vectors, typed-value fixtures, shape envelope, row batches, own bitmap tests; optional real clipboard | Run on the target Windows/Office installation |

Results are new directories `.tmp/excel-lab/<timestamp-ID>/`:
`original.json`, `shrunk.json`, `value-comparison.json`, `captures.json`,
`overflow.json`, `timings.json`, `options.json`, `manifest.json`,
`measurement-complete.json`, plus `runner.log` and `vba-progress.log`.
The Rust image stage adds `image-report.json` and converted/difference PNGs.
Timing and path information are kept out of the metadata diff.
Errors retain partial artifacts and the worker's local staging directory;
partial artifacts are not an equality result. Read `failure.json` if present.
A capture matrix can contain successful and failed variants; failures are
recorded and result in a failed task, not an empty/identical-image assumption.

### Value semantics

`Value2` is read in bounded row bands. Inner loops compare ordinary VBA Variant
arrays; there is no PowerShell/Excel call for each cell. The default block
budget is 65,536 cells, with an explicit 20-million-cell scan limit per sheet
or paired union. An inflated UsedRange can still be huge; exceeding a limit
fails visibly and requires a deliberate adjustment, never hidden sampling.

The `sha256-xlvalues1` stream is:

- fixed `XLVALUES1` domain tag as UTF-16LE;
- row-major non-Empty cells: absolute row/column as 32-bit little-endian integers;
- type tag, then canonical payload: Double IEEE754LE (negative zero normalized),
  length-prefixed UTF-16LE string, Boolean byte or 32-bit Excel error code.

Blank padding is ignored; a zero-length string, Boolean, number and error remain
distinct. Coordinates prevent the same values moved to different cells from
hashing identically. Formatting-only UsedRange changes do not alter the digest.
Excel error variants are tested against native layout. The hash is a screening
fingerprint, not a mathematical guarantee without collisions. The direct value
comparison does not rely on the hash: it reads the same absolute union rectangle
in both sheets and counts **all** differences, limiting only recorded details.

Numbers in JSON details include IEEE754 bits, so the human-readable decimal
formatter cannot hide a very small binary-value difference. Number/date display
format, workbook date-system interpretation, formula text and formatting are
not part of Value2 equality. Formula values are those Excel exposes after a
read-only open with links disabled and calculation requested manual; this is
not a promise of bit-for-bit equivalence to stored XML caches or complete
recalculation. Continue using the existing Rust regression checks for those
separate requirements.

### Range heights and shape bounds

The existing fast total-range-height idea is preserved. `LabRanges` exports
addresses as well as height/width and dimensions. The matched Shrunk rectangle
is measured in both sheets to avoid mistaking removal of unused trailing cells
for a layout change. Optional row scans locate individual differences.

Shapes are reported with coordinates, size, rotation and anchor cells. Visible
shapes enlarge the geometric envelope; grouped shapes use the group object's
geometry. Rotated rectangles are conservatively unioned with the unrotated box.
Hidden shapes are inventoried but do not extend the visible envelope. Shape
geometry is not a universal renderer: text overflow, shadows, glow, clipping,
OLE internals and objects outside worksheet coordinates are not thereby proved
covered. `painted_pixel_bounds_proven` and `full_painted_extent_proven` are
explicitly false. Range.CopyPicture inclusion of particular overlaid object
classes must be verified on representative files; a larger range alone does
not certify those objects were rendered.

### Screenshots and clipboard

A request specifies all sheets/ranges once. External JSON is validated and
converted to XML without string concatenation of XML. VBA reads that XML once,
opens staged copies and executes all captures locally. No per-image callback
into PowerShell is required. A clipboard sequence + owner-PID check rejects
stale or other-application clipboard data. Clipboard handles are borrowed only
while locked; an owned bitmap or EMF is created before the next copy.
Therefore the second `CopyPicture` does **not** erase the first saved buffer.

`appearance`: `1` screen or `2` printer. `format`: `2` bitmap or `-4147` picture.
The UI matrix tries all four combinations; printer variants may fail if the
Excel/printer environment does not support them. They are not silently mapped
back to the screen mode. Captures select a real worksheet at 100% zoom with
gridlines hidden in the disposable Excel instance. Shapes/merged areas at tile
boundaries, display scaling, fonts and printer settings can affect rendering.
Repeat-capture tests and fixed per-pair parameters are essential. Pixel equality
is equality of the captured decoded buffers, not proof of complete workbook
visual equivalence.

PNG-direct uses GDI+ in the VBA process. BMP uses an explicit opaque RGB24/white
policy, then Rust converts the completed batch in parallel with fast PNG
encoding. Original BMPs remain. “Both” performs separate acquisitions, so timing
also includes each acquisition; see `capture_seconds` and conversion timings.
No speedup claim is made until measured on actual workbooks.

There is **no transparent argument to `Range.CopyPicture`**. The extra variant
reads `xlPicture`/EMF and rasterizes to a transparent ARGB canvas at a fixed DPI.
It can retain unpainted transparency; it cannot make a white-filled Excel object
transparent. No white pixels are deleted. The bitmap route explicitly loses
source alpha; BMP is opaque. The same two transparency/CopyPicture settings are
used on both sides of a comparison.

Size limits: 32 million pixels/image, image axes below 32,766, default tile
size 1,200 points, default maximum 1,024 tiles/variant. Both workbook geometries
are considered when dividing ranges. A single oversized row/column or inflated
range can still fail the safety guard. Supply a smaller explicit plan rather
than raising every limit blindly. Equal dimensions alone are not a proof that
Excel copied every expected painted pixel.

### Text overflow experiment

Select an explicit JSON plan. `capture` must include both the source text cell
and the probe rectangle; capturing only the neighboring cells can omit incoming
overflow. `counterfactual` copies the sheet into another disposable workbook,
requires two identical baseline captures, and removes **one constant, unmerged,
unprotected text cell only on that extra copy**. It then compares the specified
neighboring pixel crop. No original file is saved. Scratch copying isolates
rich-text formatting from subsequent experiments.

It rejects formula/merged/protected sources and changed range geometry;
unstable baselines produce an inconclusive result. The crop uses point-to-pixel
proportions. Conditional formatting, sheet-copy effects and paint behavior can
also explain changes: this is an experiment, not definitive text attribution.
The `nonuniform` method checks pixel variation against an interior reference;
uniform nonempty regions exist, so it is neither an emptiness certificate nor
a text detector. The existing production overflow algorithm remains untouched.

## Plans and examples

`examples/capture-plan.json` demonstrates two acquisitions of one range.
`examples/overflow-plan.json` demonstrates a controlled constant-text probe.
Replace `Tabelle1` and the cell references with actual sheet names/regions.
Pair IDs are six digits (`000000` for the first selected workbook). A nonempty
capture plan **replaces** automatic whole-envelope capture and the automatic UI
matrix; repeat plan entries with different parameters for an explicit matrix.
One global plan can name multiple sheets and workbook pairs. Every entry still
needs an actual counterpart sheet in two-workbook mode.

The default operation is intentionally bounded: fast metadata first, narrower
captures second, exhaustive values/rows only when needed. To preserve review
clarity, the patch does not wire every experimental option into existing
regression tests or change their baselines.

## Rust image tool and reproducibility

`native/` is a separate Cargo workspace; the root Cargo.toml/Cargo.lock is not
modified. The image crate is pinned to `0.25.8`; other dependencies are resolved
on the first build. This creates **its own** `native/Cargo.lock`; review/commit
that lockfile for reproducible experiments. Later UI runs use `--locked` when
it exists. The first native build may require registry/network access.

Task **Excel lab: Rust image tests** runs its unit tests with a bounded target
directory under `.tmp/excel-lab-rust-target`. The runtime uses a configurable
Rayon pool (default four workers), per-image limits and decoded RGBA comparisons,
not PNG compressed bytes. A global machine-memory bound is not inferred from
the per-image limit: reducing workers reduces simultaneous decoded buffers.
Dimension mismatches are differences; alpha participates; no resize/alignment
or transparent-pixel masking is silently applied.

## Validation status

The deliverable includes installer/structure tests, pure Windows PowerShell
checks, eight Rust image tests and real VBA/clipboard fixture tests. Only tests
actually executed by the installer/task on your machine can validate that
machine's COM, WPF, Office bitness, native DLL and clipboard behavior.
The development environment that produced this patch did not contain Windows
Excel, Windows PowerShell or a Rust compiler. Static/source checks and Python/
Bash tests are not described as an end-to-end Windows test.

## Primary API references

- https://learn.microsoft.com/en-us/office/vba/api/excel.workbooks.open
- https://learn.microsoft.com/en-us/office/vba/api/excel.range.copypicture
- https://learn.microsoft.com/en-us/office/vba/api/excel.range.value2
- https://learn.microsoft.com/en-us/office/vba/api/excel.shape
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboarddata
- https://learn.microsoft.com/en-us/windows/win32/api/bcrypt/nf-bcrypt-bcryptcreatehash
- https://learn.microsoft.com/en-us/windows/win32/api/bcrypt/nf-bcrypt-bcrypthashdata
- https://learn.microsoft.com/en-us/windows/apps/get-started/
- https://docs.rs/image/0.25.8/image/struct.ImageReader.html
