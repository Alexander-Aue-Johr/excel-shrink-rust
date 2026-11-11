# Ryzen-host streamed cell comparison follow-up

The same 21-case protocol completed 63 fresh-process release measurements on
an AMD Ryzen 7 7800X3D, with sixteen logical processors and 102,066,212,864
OS-reported physical bytes. The executable uses rustc 1.96.0 and LLVM 22.1.2.
All expected addition, removal and change counts passed. The release executable
also passed seven cell-stream tests, eight scanner tests and twelve style tests.
These include the independent 6,561-pair small-grid oracle.

| One-million-cell dense mutation | Immediate consumption | Retained changes |
| --- | ---: | ---: |
| Median combined stream construction (ms) | 671.849 | 755.226 |
| Median merge (ms) | 66.536 | 94.204 |
| Maximum process working set (MiB) | 5.703 | 94.973 |
| Scratch bytes for both streams | 80,000,000 | 80,000,000 |

Immediate consumption reduces the measured working-set maximum by 94.0% in
this matched selected case. The experiment isolates retention of emitted
`CellChange` objects. Full-pipeline evidence storage is outside this comparison.
Working set and private commit are separate counters in every raw receipt.

The three real Enron worksheet inputs have 3,060,845 to 6,486,777 cells.
Each fixed single-value mutation emitted exactly one change in all repetitions.
The largest pair's median construction time is 4,886.604 ms, with a 393.002 ms
median merge and 518,942,160 scratch bytes. Its maximum working set is 5.934 MiB.
Across all real-input trials, one repetition of the smaller `70a7994fc2a6` case
has a 42.121 MiB working-set peak. Its other repetitions peak at 6.105 and
6.078 MiB. The larger observation is retained without attributing a cause.

Preparation, ZIP decompression and independent ElementTree checks precede the
timed test process. Caches remain enabled and native DeltaSieve comparisons
continue concurrently. A 4 GiB owned Windows-job commit limit covers the
controller and descendants, with an 8 GiB system RAM/commit reserve. Minimum
recorded headroom remained above that reserve. The job ended with no active
descendants. These host/compiler measurements remain separate from the earlier
Intel i7-8700/rustc 1.99.1 lane and do not isolate a hardware effect.

The adjacent JSON retains all 63 samples, inputs and source hashes, per-case
medians/ranges, memory guards and executable identity. The companion paper
artifact freezes exact measured source bytes under `measured-code`, with Git
text conversion disabled. Input paths are projected to basenames for publication.
This component experiment does not establish population-wide validation recall.
