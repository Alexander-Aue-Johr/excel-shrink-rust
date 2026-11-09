# Explicit native reference comparisons

`regression reference-compare` renders an externally specified plan after rebuilding
and saving both working copies in their resident Excel processes. It bypasses the
targeted selector, its dependency exclusions and content-envelope clipping. Both
copies are opened even for byte-identical input, enabling same-input controls.

The plan is a JSON array. Freeze its areas before inspecting candidate coverage.

`--verify-objects`, or the launcher's `-VerifyObjects`, additionally inventories
native worksheet shapes and charts after rebuilding both copies. Only unrotated
images and charts whose reported rectangles fit inside one supplied reference
area qualify. Excel can expose a chart container twice, including as an
unsupported shape. An unsupported entry is accepted as a chart alias only when
its name and full native rectangle exactly match a native chart. Other object
types, missing inventory, rotated objects and objects outside the supplied area
fail explicitly before reference images are captured. The native inventory stays
in `reference.json`, including on qualification failure.

This qualification concerns reported object rectangles. Shadows, glows and other
ink outside that model require a separate envelope. It does not certify that a
supplied plan covers every rendering effect. Native image geometry uses the
documented shape model and its `Image` type. [Microsoft Shape API](https://learn.microsoft.com/en-us/javascript/api/excel/excel.shape?view=excel-js-1.9), [Microsoft shape types](https://learn.microsoft.com/en-us/javascript/api/excel/excel.shapetype?view=excel-js-1.9)

```json
[
  {"sheet": "Rows", "range": "A1:C128"}
]
```

Run one bounded private-desktop experiment using the shared application binary.
Set TEMP and TMP to a working drive before launching.

```powershell
& tools/run-reference-comparison.ps1 `
    -Binary 'V:\deltasieve-build\debug\deltasieve.exe' `
    -Original 'G:\reference-inputs\original.xlsx' `
    -Default 'G:\reference-inputs\variant.xlsx' `
    -Plan 'G:\reference-inputs\plan.json' `
    -Output 'G:\reference-results\trial-01' `
    -OperationTimeoutSeconds 90 -TotalTimeoutSeconds 240
```

The output directory and its `.runtime` sibling must be new. The launcher sets
child `TEMP`, `TMP` and run-log paths beneath that sibling, on the selected
working drive, and restores the caller's environment afterward. Runtime receipts
also survive failures before the comparison directory exists. The launcher owns
a private desktop and cleanup job and retains its completion/failure receipt.
The command preserves working
copies, raw images, source and rebuilt workbook checksums, plan checksum, producer
checksum, native geometry, image identities and exact RGBA inequality observations.
It does not update a corpus verdict or call unequal pixels a product regression.

The observer checks each frozen area against the existing edge and image-memory
budgets. Split oversized reference plans independently before execution. An area
that exceeds the limits fails explicitly. Geometry that changes between preflight
and capture also fails. Zero-area outcomes retain rendering flags and do not
establish pixel equality. A timed-out launcher receipt makes a partial `running`
reference receipt an unsuccessful experiment.

The command verifies exactly its supplied rectangles. It cannot establish that
those rectangles cover every visual effect in a workbook. Independent full-area
sampling needs a declared coverage envelope, including graphical objects and
overflow. Drawings inside a supplied rectangle are rendered with the sheet;
standalone chart sheets need their separate reference method.

## Freeze an outcome-blind sample

`sample.py` freezes direct XLS/XLSX archive occurrences using source ZIP metadata.
Its default chooses three members per size-ranked quartile with the versioned
seed in the evaluation protocol. It retains the complete compressed frame,
source archive SHA-256, source indices, workbook identities and selection ranks.
Identical payloads at distinct source occurrences remain distinct. Native
failures and prior development exposure are assessed after selection and do not
replace selected members.

```powershell
python sample.py --archive 'G:\corpora\source.zip' `
    --corpus-id 'REGISTERED_64_CHARACTER_LOWERCASE_CORPUS_SHA256' `
    --output 'G:\reference-samples\corpus-v1'
```

The output must be new. The sampler reads the ZIP directory and hashes the archive
without extracting workbook payloads. It detects ordinary source changes during
freezing. The registered corpus ID is supplied by the caller; the archive's
separate full checksum binds the actual source used for the sample.

`prepare.py` binds that frozen selection to existing study preparation bundles.
It verifies the complete bundle hash, full workbook identity and both retained
member hashes, then copies only those verified states to fixed working paths.
It uses an independent ElementTree reader to plan A1 through the union of both
stored worksheet dimensions. This includes leading geometry and removed extent.
It never reads candidate decisions or replaces failed sample members.

```powershell
python prepare.py --selection 'G:\reference-samples\corpus-v1\selection.json' `
    --study 'V:\validation-runs\corpus' --output 'G:\reference-inputs\corpus-v1'
```

Stored dimensions define this experiment's declared rectangle. They do not prove
complete coverage of overflow or every rendering effect. Graphical, embedded and
chart-sheet parts require a separate reference envelope and remain explicitly
unsupported by this preparer. Native size limits and timeouts retain their own
outcomes. Public claims must state these limits.

The optional preparer mode `--require-native-objects --tile-rows 256` creates
bounded plans for basic graphical packages with mandatory native qualification.
Its output records `requires_native_objects=true` and the preparation script
hash. Such plans must run with `-VerifyObjects`. It continues to reject legacy
VML, embedded objects, chart sheets and nonempty drawing effects. More than 64
rectangles per workbook exceeds this experiment's declared budget and remains
unsupported. The default dimension-only mode retains its graphical exclusion.
Both modes preserve the frozen selection and record every unsupported outcome.

The frozen Enron sample prepared on 2026-11-08 retained all twelve identities.
Nine produced plans and three require graphical/embedded reference coverage.
The six sampler/preparer tests include grid edges, dimension union, graphical
exclusion, corrupt member hashes and missing preparations. Inputs and receipts
remain privately under `G:\deltasieve-work\reference-samples-20261108`.

The first three native sample observations completed in 38.182, 45.587 and
37.345 seconds, covering nine declared rectangles. Eight matched in geometry
and RGBA pixels. `0966d3e126908018b94a`, `Capital Structure!A1:D15`, had a
264-pixel-high Original and 263-pixel-high Default. All overlapping RGBA bytes
matched exactly; the additional 817 pixels were opaque white. The rebuilt
Original retained an empty trailing row 15, while the rebuilt Default omitted
it. This is a verified exterior-canvas observation under the agreed contract.
The strict reference receipt retains the raw difference. Two of these sample
members had byte-identical prepared inputs, so their observations are
same-input native controls. The remaining selected cases retain pending or
unsupported outcomes, and this initial subset is no coverage-rate estimate.

Each case used at most one reference slot while Enron continued with three
groups, restoring its target to four in a `finally` block. The launcher runtime
change was verified by a successful native case and a rejected empty plan.
Both restored the caller's environment exactly, including absent variables.
The rejected plan started no workbook and retained a failure receipt even
though no comparison directory was created.

## Initial check on 2026-11-08

Two trials used a synthetic worksheet whose reference area was fixed at
`Rows!A1:C128`. The same-input trial completed in 28.799 seconds and retained
identical PNG hashes and RGBA pixels. A known row-height variant completed in
32.612 seconds and retained a geometry and pixel difference. The images measured
317 by 2,593 pixels for the original and 317 by 2,561 for the changed variant.
These observations verify the reference command's basic detection and same-input
path. They do not measure selector recall or corpus-level performance.

Working receipts are under
`G:\deltasieve-work\independent-reference-pilot-20261108`. One Enron admission slot
was reserved temporarily, and its four-group target was restored in a `finally`
block. Existing corpus work continued throughout the experiment.

## Fixed real regression and failure control

The corrected Enron group `005d034f18d6dc340a2c` was independently observed over
`A!A1:T151`, including the previously shortened rows and later visible content.
The test completed in 42.774 seconds. Both images were 1,406 by 5,009 pixels,
with equal native geometry and identical PNG SHA-256
`46590ae30feca8edb24bf5f6d7d43c1a131b37151f69fb73753b3b728bc0a586`.
This verifies the supplied larger area independently of the two production
candidate regions. It remains a development regression case with that stated
reference scope.

An explicit `Rows!A1:XFD1048576` negative control failed at the image-size
preflight. It retained zero reference image pairs and an unsuccessful receipt,
as expected. Both tests positively closed their owned Excel processes. A later
PID check found none of those four processes still present. The four-group Enron
target was restored, and its completed count continued increasing afterward.
