# Five-pair explicit reference pilot

The five controlled pairs from the external-comparator pilot were independently
rendered on 2026-11-09. The reference plan bypassed candidate selection, dependency
exclusion and content clipping. Original and candidate were both rebuilt and
saved before rendering. Source SHA-256 values in all five reference receipts match
the ten previously frozen pilot inputs.

## Frozen areas and resource limits

The identical, direct-presentation and dependent-presentation pairs use
`Data!A1:N30`. The embedded pie chart fits inside the dependent reference image,
which was inspected visually. The offscreen/hidden pair uses `Data!A1:N3072` and
the sparse-formatting pair uses `Data!A1:P2304`. Each tall envelope was split into
continuous 256-row rectangles before running the reference comparison.

The tests used one extra Excel pair at a time, a 60-second command deadline and
a 90-second per-case launcher deadline. The first three cases completed within
a 35.89-second launch batch. The two tall cases were a separate bounded batch.
The four-group Destatis comparison continued independently. Its preparation
continued during a brief admission gate while the reference startup path was
checked, and its four comparison groups were restored afterward.

## Observations

| Pair | Reference rectangles | Rectangles with differences | Overlapping RGBA pixels changed | Native reference seconds |
| --- | --- | --- | --- | --- |
| Identical | 1 | None | 0 | 10.918 |
| Direct presentation | 1 | A1:N30 | 7,114 | 11.190 |
| Dependent presentation | 1 | A1:N30 | 40,211 | 12.158 |
| Offscreen and hidden | 12 | A1793:N2048 | 47 | 17.886 |
| Sparse formatting | 9 | A2049:P2304 | 1,281 | 16.372 |

The direct pair also changed image dimensions and native geometry. Its reference
images were 917 by 601 and 1001 by 621 pixels. The other four pairs retained
matching geometry within each supplied rectangle. Only the tile containing B2000
changed in the offscreen pair. The tile containing hidden B3000 was pixel-equal.
Only the tile containing J2099 changed in the sparse-formatting pair.

These observations agree with the earlier candidate-selected run's pair-level
outcomes. They do not establish pixel-union coverage or corpus recall. Pixel
totals from differently bounded captures are not directly interchangeable. The
selected direct capture has a smaller overlapping extent than the explicit
reference. A standalone chart image also has different rasterization boundaries
from a chart rendered inside a worksheet rectangle. No matched cost or speedup
claim follows from these single runs under concurrent background load.

## Reproduction and retained evidence

Use `tools/run-reference-comparison.ps1` with each frozen source pair, an explicit
JSON plan and the deadlines above. The short plan contains one entry with sheet
`Data` and range `A1:N30`. To reproduce a tall plan, enumerate starts 1, 257, 513,
and so on through the declared bottom row, with each end equal to start plus 255.
Use right column N for 3072 rows and P for 2304 rows. Keep each pair's output new.

The local evidence root is
`G:\deltasieve-work\comparator-reference-20261109`. It contains the plans,
protocols, raw source and rebuilt copies, images, process telemetry, launcher
receipts and `verified-summary.json`. The first three successful directories end
in `-retry`. The initial identical attempt failed before Excel startup because
the default host-script path retained a Windows verbatim prefix. That failed
receipt remains separate. Commit `4d191cc0` normalizes that PowerShell boundary
and its real PowerShell regression test passed.

The successful optimized reference producer SHA-256 is
`7fdcbfbbbd625069e01f2a236d467d9598041688afa6cf10c90edfa14deb9473`.
Its separate runtime contains 113 verified files. The production night queue
retains its earlier frozen runtime. No corpus verdict was replaced by these tests.
