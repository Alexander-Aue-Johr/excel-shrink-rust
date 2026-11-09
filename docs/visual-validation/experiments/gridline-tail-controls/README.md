# Repeating background control with default gridline visibility

This fixture-specific experiment follows the Mikrozensus investigation in
[the full-grid planning report](../../investigations/destatis-full-grid-plans-20261109.md#visible-gridline-mikrozensus-tail-observations).
It uses a preserved profile original and an explicit reference plan, separately
from the production candidate selector and corpus verdicts.

The supplied `prepare.py` checks the exact original SHA-256 and replaces the sole
indexed-white foreground in `xl/styles.xml` with opaque cyan. Every other ZIP
member retains its uncompressed bytes. Source identity, member uniqueness, CRC
integrity and the exact changed-member set are checked. Existing output files
are rejected.

```powershell
python prepare.py --source 'G:\inputs\mikrozensus.xlsx' --output 'G:\controls\cyan.xlsx'
```

The frozen reference plan contains three areas on `12211-03`.

```json
[
  {"sheet": "12211-03", "range": "M804:Q810"},
  {"sheet": "12211-03", "range": "M10000:Q10006"},
  {"sheet": "12211-03", "range": "M1000000:Q1000006"}
]
```

These areas straddle the column-fill transition at N/O. Two independent native
instances rebuild and save their disposable copies before capture. The original
same-input control produced identical white images in all six exports. Its
elapsed time was 14.2300723 seconds.

The cyan control completed in 15.0589105 seconds. All three pairs reported pixel
differences with unchanged 321-by-120 geometry. Every original image hash was
`d73cdc151672cdbfac323ccc827ec72d8fcfac8487673adf24414d6c5efabbf1`.
Every variant image hash was
`ab809f0f1aa6c6f72a7847d79b25c570d9aa73b4cbc0290c4a922aec206f76ce`.
The variant image was visually inspected and has a cyan filled portion followed
by white. The source ZIP remained unchanged.

The mutation archive SHA-256 is
`7160a273b2c2f2198b80ec7dc532cab802bf4fe176d730f022a689267d6b1328`.
The native receipt SHA-256 is
`ef30e71f50832899c71e5ce9088b9cac34a30f023dc7d0ee4bd60f86b4ddb0db`.
Private evidence remains under
`G:\deltasieve-work\gridline-tail-controls-20261109\native-cyan-control`.

These are bounded rendering and mutation controls. They do not establish
exhaustive whole-grid capture or corpus-wide recall. The production rule must
still preserve every serialized cell, row/column transition and supported object
extent, with unknown styles, inherited borders, patterned fills and unresolved
CF retaining their fallback.

## Planner acceptance

The background-band rule now permits known sheet-wide gridline visibility,
including Excel's default. Gridline visibility adds no new finite transition in
an otherwise supported repeating tail. Missing views, foreign namespaces and
unknown boolean values keep the previous fallback. All existing band-style,
object and CF requirements still apply.

The targeted module test run passed 143 tests with 13 explicit experiments
ignored. The updated view-eligibility test includes default, hidden and mixed
visibility, unknown values, foreign namespaces, missing views and unresolved CF.
The fixture preparer also rejected an existing output without changing its hash
and rejected an unexpected source without creating an output.

The ignored retained-snapshot diagnostic was then run explicitly against
`native-reference/original.xlsx`. `12211-03` has content `A1:E50` and now retains
`A1:XFC804`, with background-tail sampling recorded. Its last finite column
transition and all serialized rows remain included. Eight of the 30 workbook
sheets qualified for sampling. Two still had whole-grid visual bounds. This is
an XML extent result, before native geometry partitioning, and supplies no
end-to-end capture timing for the new planner. Production deployment and a
normal targeted native control remain separate gates.

## Optimized ordinary-workflow controls

The optimized runtime from `3453df30` was packaged and verified as 114 files.
Its DeltaSieve executable SHA-256 is
`56fbbbdf25e120a9dca85fd1dfdd76d95fe6cac7c805836275c5a930b163faf5`.
The existing bounded Destatis background fixture was reused for two normal
`run-targeted-comparison.ps1` attempts. `visible_views.py` enabled gridlines in
both inputs by changing exactly one attribute in each of their two worksheet
parts. All other member bytes and both source files were checked unchanged.

The ordinary unchanged pair completed with four captures per rebuilt state and
zero pixel differences. `Jahr` retained the `A1:V619` background envelope and
three captures. `Titelseite` retained one capture. The report SHA-256 is
`6b3a7b30aff7c1769263ea654357df67f143e4ff6409ab300b1f9bd8a66c44ec`.

A second pair used the existing deliberate cyan-fill mutation with gridlines
enabled on both sides. All three selected `Jahr` regions reported pixel
differences, using three captures per rebuilt state. The variant image was
visually inspected. The report SHA-256 is
`707354856023aead9781948b3f61bf93919b2e1be8d4f9d9b471e9967977ae4d`.
Both attempts completed with the normal explicit coverage limitations, without
Check Performance. Input/output hashes and exact changed worksheet parts are
retained in their respective `preparation.json` files.

These controls exercised the ordinary planner, geometry partitioner, transport
and comparator through the packaged optimized runtime. They did not deploy that
runtime into the active night study or change any corpus verdict.
