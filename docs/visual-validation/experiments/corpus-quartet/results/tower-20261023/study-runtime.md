# Complete study runtime on GOTHAM, 23 October 2026

The candidate completed two selected Data.gov workbooks in about **20% less wall
time**, with identical comparison coverage, classifications and decoded image
evidence. This is a development sample, not an estimate for the complete corpus.

| Order | Baseline, seconds | Candidate, seconds | Baseline / candidate |
| --- | ---: | ---: | ---: |
| AB | 64.142 | 51.848 | 1.237 |
| BA | 63.547 | 50.799 | 1.251 |
| Median | 63.845 | 51.324 | 1.244 |

The clock includes shrinking, three fresh Excel opening measurements, all
required profile/value checks, visual captures, report writing and cleanup.
Each invocation used a fresh study and application cache. Windows and Office
caches were retained. The Rust worker limit was 8 GiB; Excel's separate default
limit was 4 GiB. Both AB/BA pairs were technically complete and matched the same
semantic signature, including decoded PNG pixels.

The sample contains `Data submission for A-4tmz_a8d396e49ecb.xlsx` and
`2020-03 Taxonomy final_0f1beb8dad80.xlsx`. Each candidate invocation reused three
visual profile comparisons whose original/output SHA-256 pairs were identical
and whose runtime inputs and sheet coverage passed the conservative guards.
Profile-specific value checks and all opening measurements remained fresh.
The candidate also includes the consistent capture-copy selection preparation;
this comparison does not isolate the contribution of each individual change.

The baseline uses main commit `af2f8d4`. The candidate's runtime checkout was
`ca143bf`; the final reviewed main commit `2fccb62` has the identical source tree.
Actual executable SHA-256 values, source workbook hashes, paired observations
and reuse references are retained in [study-runtime.json](study-runtime.json).

The portable CLI exported the first candidate study and imported it into a fresh
study twice. Both durable records and all referenced images remained byte-identical;
the package contained 155 entries, including machine/run manifests. The second
import was idempotent. Private requests and prepared XLSX bundles were excluded.

A separate three-file sample of historical taskpane-restart failures remained
technically skipped. Both sides started and answered their height check, then
the shrunk taskpane requested configuration again and received HTTP 409. CAPs'
Maximum comparison was reused successfully, but its fresh Default comparison
still restarted. The selection fix does not establish a general cure for these
failures. Their historical results were not reset or promoted to successful checks.

Reproduce the timing with `compare_study_runtime.py --baseline-cli ...
--candidate-cli ... --corpus Sample ... --output ... --repeats 2`, using separate
clean runtime checkouts and the original two files. The harness rejects technical
failures, missing coverage and unequal image evidence before reporting speedups.
The raw local runs remain in `.tmp/distributed-setup/study-ab-20261023/`.
