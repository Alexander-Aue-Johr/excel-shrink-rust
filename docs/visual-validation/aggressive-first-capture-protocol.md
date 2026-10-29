# Experimental autonomous capture protocol

The `experiment/aggressive-first-hash-sweep` branch enables this transport with
`EXCEL_SHRINK_CAPTURE_SWEEP=1`. Normal live-image and bounded hash-first runs
keep their existing transport.

## Primary sweep

1. Clients negotiate worksheet bounds and common full-width row bands. Worksheet
   geometry is read independently; tile numbers alone never establish identity.
2. The coordinator sends one `capture` command to every initial client before
   waiting for any result. Each client captures every negotiated sheet and tile,
   hashes its finalized PNG, releases the encoded image and continues locally.
   Responses retain geometry and SHA-256 metadata, not image buffers.
3. After all responses arrive, differing PNG hashes or dimensions identify the
   evidence regions. Row-height differences also retain the necessary evidence.
   The sweep bypasses tall-sheet edge sampling and CRC-based deferred omissions.
   Capture and metadata budgets still fail explicitly rather than silently
   certifying unvisited regions.
4. Selected regions are requested through `images`. The recaptured PNG must have
   the original offered hash and dimensions. The host verifies its path, digest
   and decoded dimensions before exact pixel comparison. PNG encoding differences
   can therefore be resolved by decoded pixels rather than reported as changes.

The live snapshot records `sweep_complete` when hash responses are complete and
`replay_verified` only after requested witnesses have been verified. A changed
witness produces `CAPTURE_CHANGED` / `unstable_capture`; it is not evidence of a
shrinker regression. Equal tiles have hash metadata and no fabricated image URL.

`capture_context_tiles` is read once per sweep. It accepts 0..16 and defaults to
zero. `EXCEL_SHRINK_CAPTURE_CONTEXT_TILES` is the fallback without a live value.
Context follows touching row bands on the same sheet, never omitted gaps or a
conceptual two-dimensional dashboard grid.

## Targeted profiles

`ResidentCapture::add_profile` admits a late profile. `compare_selected` plans
only aggressive mismatch regions and captures those images directly, without a
whole-workbook digest sweep. It records exact checked ranges,
`targeted_checks_complete=true` and `checks_complete=false`. Such a result cannot
stand in for the later independent full-profile pass. Parent study scheduling
owns recalculation policy and the distinction between screening and full checks.

Diagnostic replays use new capture plans. Frozen primary evidence is preserved;
intentional in-memory repairs must not be attributed to the original shrink.

## Focused verification

- Rust `sweep::tests`: zero context, adjacency boundaries and exact original
  region/hash selection.
- `resident-host/resident.test.cjs`: autonomous sweeps across sheets, one-image
  encoded-memory peak, no retained PNGs, valid witness replay and drift rejection.
- Desktop Excel end-to-end probes are required in addition to these protocol
  tests before using experimental timing or diagnosis claims in a paper.
