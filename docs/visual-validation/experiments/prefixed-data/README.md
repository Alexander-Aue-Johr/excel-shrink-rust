# Prefixed worksheet data inventory

This experiment tests actual prefixed SpreadsheetML `sheetData`, `row` and `c`
elements. An unused namespace declaration, `x14` extension, comment, or foreign
same-name element is not a match. Namespace scopes and ancestry are tracked by
the existing streaming byte scanner. Worksheet text is not decoded or loaded
into a DOM. A matching sheet stops at its first proven data-element prefix.

## Reproduce

Use new output directories. The inventory preserves matching sources verbatim
and retains failures for inspection. IDs follow sorted ZIP member names.

```powershell
cargo build --release -p excel-shrink-cli
$runner = 'target/release/excel-shrink-cli.exe'
& $runner regression prefix-inventory --input CORPUS.zip --output INVENTORY --workers 4
python docs/visual-validation/experiments/prefixed-data/run_trials.py --inventory INVENTORY --output TRIALS --binary $runner --rust-normalize
```

`run_trials.py --help` documents resumable trials. Each trial first runs the
untouched source through the actual shrinker. Its existing namespace guard
skips it. That skip alone says nothing about possible savings. Therefore the
experiment makes a separate namespace-normalized copy and runs default shrink
on that copy. Savings compare normalized input against shrunk normalized input;
normalization and ZIP recompression are not counted as shrink savings.

The Rust normalizer streams expanded element and attribute names plus raw
payload into fingerprints and verifies the written copy by rereading it. It
rejects unsupported namespace constructs rather than silently approximating.
These fingerprints are not XML canonicalization: lexical differences may
produce unequal fingerprints. Equality is evidence that the recorded expanded
names and payload were preserved. Comments and processing instructions are
outside this fingerprint. The independent Expat implementation in `normalize.py`
checks decoded names and payload and preserves comments in its output. It was
also used to independently verify a real Rust-normalized case (00752).

Normalization is experimental, never an automatic production conversion.
Original source SHA-256 hashes are checked before and after every trial.
Per-case reports, normalization evidence, original skip reports and shrink
reports remain in the chosen output directory.

## Data.gov measurement, 2026-11-02

- 9,117 workbooks; 47,032 sheets; zero inventory errors.
- 493 matching workbooks. Their first detected data tag was `x:sheetData`.
- 147,208,175,886 decompressed XML bytes inspected in 651.263 seconds, four workers.
- All 493 trials completed without errors: 8 independent Python normalizations,
  then 485 Rust normalizations.
- 310 outputs byte-identical to their normalized baseline; 183 changed XML.
  Expanded-name evidence changed in 151 cases; remaining differences were lexical.
- Only **two** produced smaller compressed XLSX files. Other cases can still
  remove blank cells without making the compressed output smaller.
- Across matching sources the maximum row coordinate was 694,981. Some contain
  millions of cells; none of these 493 exceeds Excel's row limit.

| Source ID | Workbook (abbreviated) | Baseline bytes | Shrunk bytes | Savings |
| --- | --- | ---: | ---: | ---: |
| 05916 | blm-or-water-quality-and-quantity-stream-grab-sample… | 4,049,434 | 3,789,647 | 259,787 (6.415%) |
| 05803 | blm-nm-region-cadnsdi-survey-points-for-ks… | 20,793,070 | 20,783,201 | 9,869 (0.047%) |

The complete filenames are recorded in the inventory and each trial JSON.

## Positive-case validation

`audit_cells.py BEFORE AFTER OUTPUT.json` independently streams both sheets
with Expat. It compares expanded cell payloads and attributes, treating omitted
`t="n"` as the numeric default. It classifies a deleted cell as empty and
unstyled only when it has no child payload or remaining attributes.

| Source ID | Unchanged remaining cells | Removed empty, unstyled cells | Other cell changes |
| --- | ---: | ---: | ---: |
| 05916 | 583,853 | 268,016 | 0 |
| 05803 | 4,580,967 | 1,219,443 | 0 |

The existing Rust raw-cell fingerprints additionally report lexical changes;
these are not automatically interpreted as changed values. Row metadata,
column definitions, sheet defaults and value context matched in both cases.

For selected-region native Excel evidence:

```powershell
# Local measurement paths are intentionally not corpus configuration.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File docs/visual-validation/experiments/prefixed-data/run-inspection.ps1 -Case 05916
```

The wrapper opens only copies on a private desktop. `regression prefix-inspect`
compares prefixed source, normalized baseline and default-shrunk baseline.
Sampling chooses the first deleted cell in each of eight vertical strata and
16-column bands, plus top and bottom regions. Each region is at most 20 columns
by eight rows. PNGs and `visual-samples.json` retain exact ranges and outcomes.
All 25 sampled regions in 05916 and all 17 in 05803 were pixel-identical across all three states (126 PNGs). This is **sampled inspection**, not a whole-sheet visual certificate. No rebuild,
save or Check Performance optimization is applied to the states being compared.

## Production decision

The hypothesis that prefix files never offer savings is disproven by these two
cases. Per the user's decision, do not introduce a blanket visual-validation
exclusion. Keep the saving cases and experimental copies available for further
validation. Existing production namespace handling remains unchanged: root-header
checks and normal parsing already skip unsupported prefixed inputs, and corpus
preparation consumes the explicit shrink skip reason. There is no new full-XML
prepass or production normalization. The production corpus was not restarted.