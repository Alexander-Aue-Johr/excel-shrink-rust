# Controlled memchr dispatch ablation

The frozen source revision is `6568787c76ef686916a0751283ad11c8fe2d8421`.
Both release builds use memchr 2.8.1 from the same isolated dependency copy.
An experimental feature disables its x86_64 architecture dispatch, including
cached substring finders and reverse-search architecture conditions.
Compiler auto-vectorization, compression and checksum implementations remain
enabled. This experiment measures the contribution of this dependency's
architecture dispatch to complete cleanup operations.

## Mechanism and correctness gates

`memchr-portable-override.py` creates the isolated dependency copy and records
the modified source hashes. Its output was checked byte-for-byte against the
dependency used in the experiment. It changes 31 architecture conditions in
five source files and declares the experimental `force-portable` feature.

The native probe logs an AVX2 cached substring searcher. The portable probe
logs scalar two-way substring search and has zero x86 dispatch log entries.
Each mode passes 161 controls covering forward/reverse byte searches and
cached forward/reverse substring searches across boundary-sized inputs.
The existing nine focused XML tests pass in each release configuration.
All 18 cleanup trials have exactly equal uncompressed ZIP member names,
lengths and SHA-256 values within each workbook case. ZIP entry order does
not enter this correctness comparison.

## Execution protocol

The three developer-selected inputs are Migration/Wanderungen and Finance
from Destatis, and QT/RR from Data.gov. Their original hashes are recorded in
`memchr-dispatch-ablation-20261111.json`. Each mode runs three repetitions
with one Rayon thread, one writer and one active-workbook slot. Mode order
alternates by repetition. Timing covers process creation through exit and
excludes subsequent ZIP-member verification.

The machine has an AMD Ryzen 7 7800X3D and sixteen logical processors.
The compiler is rustc 1.96.0 with LLVM 22.1.2, targeting
`x86_64-pc-windows-msvc`. Native visual comparisons and the bounded library
follow-up run concurrently. Filesystem caches are retained. These conditions
limit attribution of small wall-time differences.

Each cleanup process tree is contained in a Windows job with an 8 GiB
aggregate commit ceiling, an 8 GiB system RAM/commit reserve and a 180-second
timeout. Paired builds use two Cargo jobs, a 12 GiB aggregate commit ceiling
and the same system reserve. The experimental workspaces and binaries are
separate from active validation executables.

## Results

| Case | Native median seconds | Portable median seconds | Native range | Portable range |
| --- | ---: | ---: | --- | --- |
| Wanderungen | 3.578 | 3.328 | 3.063–3.672 | 3.265–3.828 |
| Finance | 17.750 | 18.437 | 17.641–17.796 | 18.172–18.531 |
| QT/RR | 22.671 | 24.125 | 22.578–23.031 | 24.094–24.500 |

Portable/native median ratios are 0.930, 1.039 and 1.064 respectively.
Wanderungen's wall-time ranges overlap and its median ordering reverses.
Native median user CPU time is lower in all three cases. The evidence supports
small selected-case effects with explicit concurrent-work context. A universal
speedup, a corpus-level effect and the contribution of all SIMD remain
unmeasured. Logical-core-count cleanup trials remain a separate experiment.

## Reproducing the build switch

Use a fresh checkout of the frozen source and an unmodified memchr 2.8.1 crate
source. Run the override helper with `--memchr-source` and `--output` pointing
to that source and a new experimental directory. Add a Cargo dependency patch
pointing to the helper's output.

```toml
[patch.crates-io]
memchr = { path = "/absolute/path/to/experimental-memchr" }
```

In the experimental checkout only, add this feature to
`excel-shrink-core/Cargo.toml`.

```toml
experiment-portable-search = ["memchr/force-portable"]
```

Build the native executable and preserve a copy before the portable build.
Selecting both packages exposes the core feature to Cargo's feature resolver.

```text
cargo test --release --offline -p excel-shrink-core --lib -- xml::
cargo build --release --offline -p sheetshrink -p excel-shrink-core
cargo test --release --offline -p excel-shrink-core --lib --features excel-shrink-core/experiment-portable-search -- xml::
cargo build --release --offline -p sheetshrink -p excel-shrink-core --features excel-shrink-core/experiment-portable-search
```

For each preserved executable, use the identical source workbook and a new
output directory with these cleanup options.

```text
sheetshrink INPUT.xlsx OUTPUT_DIRECTORY --rayon-threads 1 --writer-workers 1 --max-active-workbooks 1 --console-log-level error --file-log-level error
```

Retain failed outcomes, alternate the mode order, preserve the original input
hashes, and verify every uncompressed ZIP member after each successful trial.
The JSON evidence retains all individual timings, process-tree resource
counters, binary identities and canonical member-manifest digests.
