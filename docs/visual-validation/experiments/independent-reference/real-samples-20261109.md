# Real-input reference sample reconciliation

## Historical Enron development sample

The existing seeded twelve-occurrence Enron sample was reconciled on 2026-11-09
against its frozen selection, prepared member identities, reference images and
native launcher receipts. The selection SHA-256 is
`921c9fb98808c7c6583fec284ee0d01eaef6861a35ad6309973c0d001839b309`.
No source occurrence was replaced after its outcome became known.

| Outcome | Selected workbooks | Native rectangles retained |
| --- | --- | --- |
| Completed native reference | 7 | 33 |
| Rejected at image-size preflight | 2 | 0 |
| Unsupported graphical or embedded envelope | 3 | 0 |
| Total selected | 12 | 33 |

All retained source, candidate and image hashes were checked against the receipts.
Launcher success was required alongside the native reference status. The 33
completed rectangles contain zero differing overlapping RGBA pixels. Two
rectangles have unequal image extents.

- `0966d3e126908018b94a`, `Capital Structure!A1:D15`, has images 817 by 264
  and 817 by 263. The additional 817 pixels are opaque white.
- `62927fc6b2b57ebb6a87`, `Sheet1!A1:C268`, has images 765 by 1093 and
  765 by 837. The additional 195,840 pixels are opaque white.

The original raw dimension/pixel inequality observations remain retained. The
later interpretation records exterior canvas under the agreed overlap contract.
Neither image was resized or resampled to obtain equality. This is a historical
development sample with declared stored-dimension rectangles. It is not held out,
does not certify ink outside those rectangles and does not estimate selector
recall. The two failed and three unsupported inputs remain in the denominator.

The working evidence is under `G:\deltasieve-work\reference-samples-20261108`.
`reconciliation-20261109.json` retains all twelve identities, receipt hashes,
native failures, reference counts and the exact overlapping/exterior pixel totals.
This reconciliation reread earlier observations. It did not restart the old
Enron corpus or execute new native trials.

## Destatis selection frozen on 2026-11-09

The same versioned outcome-independent sampling tool selected three occurrences
per size quartile from the full 1,182-member Destatis archive. Its archive SHA-256
is `f0b39b1a30778a80227b01ae64109db964c6f71f83e6ccb3097e2f9e26f4ac45`.
All twelve selected preparation bundles and member identities were verified.

All twelve were classified unsupported by the current stored-dimension envelope
preparer because their packages contain graphical or embedded parts. No native
reference claim is made for this sample. The selected identities remain fixed
under `G:\deltasieve-work\reference-samples-20261109\destatis`, with their twelve
unsupported outcomes in the sibling `destatis-inputs/preparation.json`.

Inspection of three selected packages found normal worksheet pictures, including
two-cell anchors, and one package also contains an embedded Word document and
alternate drawing markup. A blanket dimension-only reference is inadequate for
that population. The next envelope implementation must account for actual object
extent or use an independently supplied bounded area with explicit coverage
limits. Choosing replacement inputs without graphics would change the experiment
and is not a remedy for these unsupported outcomes.

## Native object qualification extension

The optional native-object preparer retains the same twelve Destatis identities.
It permits basic graphical packages only with the native `--verify-objects` check.
Legacy VML, embedded objects, chart sheets, nonempty drawing effects and more than
64 reference rectangles remain unsupported. Eleven selected inputs still fail
that declared envelope. One selected input produces two fixed reference areas.

`a69fa2ddf6a9a3204409`, `31121992_Jahr.xlsx`, completed in 12.659 seconds.
Its `Deckblatt ` sheet has the native image `Grafik 2`, with rectangle left
236.25, top 32.25, width 240.75 and height 30.75 points in both states. That image
fits inside `A1:O38`. The data sheet `31.12.1992` has no native image or chart and
uses `A1:T40`. Both reference areas contain zero changed overlapping RGBA pixels.
The data image heights differ by 20 pixels, with an additional 25,820 opaque-white
pixels at the original's bottom edge. The raw extent difference remains retained.
This is one completed input out of twelve selected, with eleven unsupported
outcomes, under a reported-object-rectangle model. Decorative ink beyond native
rectangles remains outside this qualification.

The separate synthetic object-guard control retained a chart within `Data!A1:N30`
and completed in 10.933 seconds. An intentionally insufficient `A1:D4` plan failed
with zero reference images and retained its observed chart rectangle. Excel also
exposes this chart as an `Unsupported` shape. The guard accepts that alias only
when its name and complete native geometry equal the separately enumerated chart.
The first attempt rejected that alias and remains separate failed evidence.

The cases used one additional Excel pair at a time, a 60-second command timeout
and a 90-second total launcher timeout. The four-group production comparison
continued. Evidence is under `reference-object-guard-20261109` and the fixed
Destatis sample's `destatis-native-a69fa2ddf6a9a3204409` directory. Fifteen JavaScript
tests, seven sampler/preparer tests and the focused Rust qualifier test passed.
These tests establish the extension's bounded native path and rejection behavior.
They do not establish a full-corpus detection rate.

## Prepared Enron XML-derived sample on 2026-11-11

A separate four-input sample uses the prepared Enron XLSX population of 30,301
inputs. The source frame is `inputs-manifest.json` from
`G:/deltasieve-work/enron-sheetshrink-20261110`. Each frame entry contributes its
list index, input basename, byte size, source SHA-256 and XLSX SHA-256. The
existing `sample.select` function uses seed
`deltasieve-prepared-enron-reference-20261111-v1` and one input per size quartile.
The manifest SHA-256 supplies its corpus identity. Selection precedes inspection
of the selected outputs and is retained without replacement.

The [frozen selection and observations](enron-prepared-sample-20261111.json)
record the manifest identity, quartile populations, all four inputs, native
producer, plans and per-image hashes. Candidate files are the matching
`optimized-release` outputs indexed by `ranking-absolute.csv`. Their selection
does not use reduction rank. Prior development exposure remains unclassified.

`prepare.make_plan(original, candidate, True, 256)` supplies continuous
stored-dimension envelopes with a 64-region limit. All four inputs qualified.
Each reference used `run-reference-comparison.ps1 -VerifyObjects` with a
60-second operation limit and 90-second total limit, one extra pair at a time.
Native reference durations were 14.273, 14.915, 21.579 and 33.589 seconds.
The 41 retained image pairs have matching dimensions and zero changed RGBA
pixels. Independent decoding verified all 82 PNG hashes and pixel comparisons.
The two production QT/RR comparisons continued during these tests.

The local raw evidence is retained in
`G:/deltasieve-work/enron-prepared-reference-20261111`, including the frozen
frame, selection, preparation, source copies, native receipts and images.
These four observations concern the supplied envelopes of prepared workbooks.
They supply no whole-workbook or population recall estimate.

The ordinary v15 selector subsequently completed all four of the same pairs.
Its per-quartile region counts were 1, 0, 4 and 2. Quartile 2 has byte-identical
input packages and supplies no new targeted image observation. The other three
pairs rendered fourteen images totaling 77,982,732 pixels. The independent
reference rendered 82 images totaling 398,046,292 pixels across all four inputs.
All seven targeted image pairs have equal pixels and geometry. These counts
measure actual rendering work under the two recorded plans. Fixed 256-row
reference tiles and adaptive targeted partitions differ, and the reference
also qualifies native objects. These runs supply no matched timing estimate.
# Data.gov inventory sample on 2026-11-11

`profile_sample.py` freezes one occurrence per size quartile from the versioned
9,117-member `profiles/corpora/datagov.json` inventory. It uses seed
`deltasieve-datagov-reference-20261111-v1`, the inventory's archive identity,
member index/name and stored bytes. Selection was written before joining the
existing SheetShrink preparations. A second execution reproduced the selection
exactly. Prior development exposure remains unclassified.

```powershell
python docs/visual-validation/experiments/independent-reference/profile_sample.py `
  --profile profiles/corpora/datagov.json `
  --seed deltasieve-datagov-reference-20261111-v1 `
  --output 'G:\deltasieve-work\datagov-reference-reproduction'
```

Original bytes were checked against the selected inventory SHA-256. Candidate
paths came from the existing preparation and recovered-output manifests. Plans
used `prepare.make_plan(original_bytes, candidate_bytes, True, 256)`, retaining
its 64-area limit. Each native reference additionally requested object
qualification and had a 90-second total deadline on an owned private desktop.
The frozen selection, plans, native receipts and raw PNGs remain under
`G:\deltasieve-work\datagov-reference-20261111`.

Two references completed over eleven areas, with equal image dimensions and
zero changed RGBA pixels. All 22 PNG hashes and decoded pixels were checked.
Quartile one was byte-identical and selected no targeted captures. Quartile two
selected eight equal image pairs. The other two frozen references failed native
image-size preflight before capture and remain failed selected observations.
They were neither replaced nor counted as equal. Their 256-row plans contain
two and 46 regions respectively. These results do not estimate corpus recall.
`datagov-sample-20261111.json` retains all four selected identities, outcomes,
native receipt identities, rendered image hashes and targeted observations.

