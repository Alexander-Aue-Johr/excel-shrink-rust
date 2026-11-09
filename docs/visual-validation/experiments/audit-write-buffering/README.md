# Buffering JSON audit writes before compression

## Question and boundary

Does a buffer before the gzip encoder reduce the cost of many small JSON writes
without changing the decoded audit? This microbenchmark measures serialization
and compression into memory. It excludes XML parsing, change detection, source
attribution, disk writes and the rest of the corpus workflow.

`main.rs` writes the same 100,000 rows in both modes. Each row includes a cell
address, two deterministic SHA-256 strings and attribution fields. Digest
generation and subsequent decoding occur outside the timed section. Four trials
alternate the order of buffered and direct writes. Every trial compares the
complete decoded byte arrays. The production 100,000-change roundtrip test
additionally checks file publication and row contents.

## Measurement on 2026-11-09

The optimized standalone program reused the release build's `flate2`,
`serde_json` and `sha2` libraries. The normal four-group corpus run and separate
Check Performance preparation remained active, so these are local observations
under contention rather than isolated hardware benchmarks.

| Trial | First mode | Direct seconds | Buffered seconds |
| --- | --- | ---: | ---: |
| 0 | Direct | 0.8590132 | 0.0842765 |
| 1 | Buffered | 0.8256214 | 0.0945307 |
| 2 | Direct | 0.8435994 | 0.1174731 |
| 3 | Buffered | 0.8451204 | 0.0852975 |

The buffered section was about 7.2–10.2 times faster in these four trials.
All runs produced 4,447,721 compressed bytes and 16,689,048 decoded bytes. Every
decoded payload had SHA-256
`2f5f3938c5cd2e64afd166452242ab60b61f22a9c89be48e8880e3d7ec080e3b`.
This does not establish a corresponding whole-workbook or corpus speedup.

The implementation in `change_evidence::write` now buffers before gzip as well
as before the file. It flushes the outer buffer before finishing compression,
flushes and synchronizes the file, and only then publishes the final path.
The audit schema and retained rows are unchanged.

## Reproduction

Build `excel-shrink-cli` in release mode. Compile `main.rs` with Rust 2021 and
optimization enabled, passing the matching release dependency directory with
`-L dependency=<directory>` and its `libflate2-*.rlib`,
`libserde_json-*.rlib` and `libsha2-*.rlib` using `--extern`. Run the resulting
program with `100000` as its argument. Preserve the printed JSON lines.

The original local result file is
`G:\deltasieve-work\audit-write-buffering-20261109\results.jsonl`.
