# Streamed cell comparison verification

The production cell writer and merge now expose a test-only verification module
under `regression::targeted_comparison::cell_stream::verification`.
The harness bytes were imported from the frozen preliminary paper artifact
`papers/deltasieve/evidence/cell-stream/measured-code/verification.rs`.
The preliminary artifact's canonical LF snapshot has SHA-256
`ba184e552d7960dcabce0e0dbee05913cc403f1b0cd5d67dc3a9e3936b6d9a88`.
The production writer, fingerprint and merge behavior is unchanged.

The independent coordinate/value oracle enumerates 6,561 pairs of small grids,
including the Z/AA boundary and the last legal Excel cell. Other controls cover
byte-fragmented XML, styles, formulas, typed blanks, invalid order and XML,
scratch cleanup, truncated disk records and consumer errors. The focused suite
passes seven tests and retains one explicitly ignored measurement entry point.

The opt-in release measurement uses `CELL_STREAM_BEFORE`, `CELL_STREAM_AFTER`,
`CELL_STREAM_SCRATCH`, optional `CELL_STREAM_STYLES` and `CELL_STREAM_COLLECT`.
Run the exact ignored test in a fresh process for every sample. The retained
paper runner generates inputs before timing and checks emitted difference counts
against its independent expected mutations. Preserve source, compiler, binary,
host, input and repetition identities for each additional measurement lane.
The measurement compares immediate consumption with retaining emitted
`CellChange` records. It does not represent all full-pipeline evidence storage.
