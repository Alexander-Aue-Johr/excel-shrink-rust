# Recommended invocation

Build the runner once:

```bash
cargo build --release --manifest-path tools/corpus-compare/Cargo.toml
```

Start a new comparison with the resulting binary:

```bash
tools/corpus-compare/target/release/corpus-compare \
  --corpus-root ../original_destatis_xlsx_files \
  --run-root .tmp/corpus-comparison/full
```

Resume without compiling either the runner or the two cached shrink binaries:

```bash
tools/corpus-compare/target/release/corpus-compare \
  --corpus-root ../original_destatis_xlsx_files \
  --run-root .tmp/corpus-comparison/full \
  --resume
```

If HEAD or any staged, unstaged, or untracked working-tree file has changed, the fingerprint no
longer matches and the cache is not used. Select a new run directory for a different comparison,
or pass `--rebuild` when intentionally rebuilding the cache in the same directory.
