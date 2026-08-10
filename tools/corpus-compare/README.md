# Cached corpus comparison runner

Run this tool from WSL at the repository root:

```bash
cargo run --release --manifest-path tools/corpus-compare/Cargo.toml -- \
  --corpus-root ../original_destatis_xlsx_files \
  --run-root .tmp/corpus-comparison/full
```

It inventories the complete corpus before processing. For every input it runs the cached `HEAD`
binary first, then the cached working-tree binary, compares the generated XLSX and immediately
appends a `file_comparison` object to `report.jsonl`.

Resume an interrupted run without rebuilding:

```bash
cargo run --release --manifest-path tools/corpus-compare/Cargo.toml -- \
  --corpus-root ../original_destatis_xlsx_files \
  --run-root .tmp/corpus-comparison/full \
  --resume
```

The cache is reused only when the HEAD commit and the complete working-tree fingerprint match.
The fingerprint includes staged, unstaged, and untracked files. Use `--rebuild` to force both
binaries to be rebuilt. Extra shrink options can be supplied repeatedly as
`--cli-arg=--option`.

Comparison statuses in the JSONL report are `identical`, `zip_metadata_only`,
`content_changed`, `head_missing`, `working_tree_missing`, `both_missing`, and
`invalid_output`. Changed textual ZIP entries have linked unified diffs below `diffs/`.
