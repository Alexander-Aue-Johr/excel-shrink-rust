# Explicit native reference comparisons

`regression reference-compare` renders an externally specified plan after rebuilding
and saving both working copies in their resident Excel processes. It bypasses the
targeted selector, its dependency exclusions and content-envelope clipping. Both
copies are opened even for byte-identical input, enabling same-input controls.

The plan is a JSON array. Freeze its areas before inspecting candidate coverage.

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

The output directory must be new. The launcher owns a private desktop and cleanup
job and retains its completion/failure receipt. The command preserves working
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
