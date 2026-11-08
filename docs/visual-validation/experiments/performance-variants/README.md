# Isolated row-processing performance proposals

The input proposals and regression fixtures came from the static review archive
`excel-shrink-performance-review-a7445eb-20261107.zip`. The archive did not
contain executed Rust tests or runtime measurements. Its original patch base was
`a7445eb`. Measurements here use explicitly identified executable hashes.

The common fixtures were first run against the current algorithm with proposal D
already present. They passed before applying A. With A, the Core library suite
passed 292 tests with one ignored benchmark. These fixtures cover metadata and
malformed-cell fallbacks, dominance and tie rules, and preservation of visible
formatting. Production adoption remains separate from fixture integration.

Seven of the eight local review inputs matched the supplied SHA-256 identities.
`wanderungen-2010120217005.xlsx` differed and was excluded from the matched-input
experiment. Its expected hash was
`774914b4925b642e1a083da9eb342c047a4e2a681eef937d04783ef0efe8a237`, while the
local hash was
`bedf8677328fb15ffd967d03ebdf860ae4b1d0942d38091b4b4bef90e574f712`.

`compare_variants.py` accepts baseline/candidate executables, the review's input
manifest, its local input root, a new output directory, a repetition count and
an explicit concurrent-workload note. It verifies inputs before execution,
alternates execution order, and compares every decompressed ZIP member after
each pair. Duplicate members, missing outputs and changed output content fail
the experiment. Inputs and outputs remain available for inspection.

Use the script's `--help` for arguments. Set `TEMP` and `TMP` to the working
drive before execution. The caller must supply immutable binary copies and
preserve their producing revision. Timing observations made alongside Enron
are exploratory. Controlled speedup claims require separate repeated trials
with a recorded, stable machine workload.

The initial working receipts live under
`validation-temp/performance-variants-20261108` on V:. The pinned Enron runner
continues independently throughout these tests.
