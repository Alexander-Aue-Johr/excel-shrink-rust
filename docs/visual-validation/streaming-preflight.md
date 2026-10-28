# Streaming validation preflight

The package-manifest pass and worksheet capture-bound pass now consume ZIP
members in 128 KiB blocks. Previously the manifest's `read_to_end` retained the
largest decompressed member, and the capture planner materialized each worksheet
again. This could exhaust a worker's allocation budget before cleaning started.
Neither path used a DOM; the problem was whole-member ownership before parsing.

## Entry points and invariants

- `regression/artifact_cache/scan.rs` computes each member's exact SHA-256 and
  length while deriving visual/runtime/formula metadata from a byte-token stream.
  ZIP I/O, CRC and declared-length failures remain errors. Uncertifiable XML
  disables cache reuse but does not stop hashing the remaining member bytes.
- `regression/streaming_xml.rs` uses `memchr`/`memmem`, borrowed text chunks and a
  reusable open-name stack. It retains neither cell bodies nor comments. Tags
  crossing read boundaries are buffered up to 64 KiB; names are limited to 256
  bytes and nesting to 128 levels. DTDs, CDATA and UTF-16 remain unsupported.
- `regression/office_plan/streaming_sheet.rs` retains the current row/cell state
  and column definitions. It preserves content/merge bounds, formula-only cells,
  inline text, default widths and the explicit-hidden-tail heuristic. Inflated
  worksheet dimensions and style-only empty cells do not extend capture bounds.
  More than 16,384 column definitions fails explicitly rather than growing an
  unbounded result vector.

Formula certification retains at most 64 KiB of a formula. Longer or unsupported
representations disable reuse conservatively; they do not allocate a larger
formula buffer. Package, baseline-repair and recalculation-reference keys include
all sources that implement the new certificate, invalidating old cached metadata
without deleting user data.

This is not a claim that the entire validation pipeline has constant memory.
Workbook/relationship/drawing metadata, value comparison, cleaning and native
Excel capture have their own storage requirements and are otherwise unchanged.

## Verification

Run these focused tests from the workspace root:

```sh
cargo test -p excel-shrink-cli regression::streaming_xml
cargo test -p excel-shrink-cli regression::artifact_cache
cargo test -p excel-shrink-cli regression::office_plan
cargo test -p excel-shrink-cli --no-run
```

The added tests exercise every input chunk size for representative XML, quoted
`>` characters, namespace prefixes, split comments/PIs/formulas, large generated
text streams, hierarchy/token bounds, short/interrupted reads, invalid lengths,
and corrupt ZIP CRCs. The existing capture-plan and certificate tests still run
against the streaming paths.

All four Rust commands above passed on the branch rebased onto `origin/main`
(`e34d358`). The Windows/Excel end-to-end workflow has not been run for this
change. The workbook shown in the error screenshot was not available in the
workspace, so that specific file could not be replayed. Build/redeploy the CLI
using the existing launcher workflow before retrying the workbook; an
already-running worker keeps its old executable.
