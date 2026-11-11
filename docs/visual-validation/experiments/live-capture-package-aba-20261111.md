# Live capture-package A/B/A observation

One ongoing QT/RR Original/Inquire comparison received three sequential,
nominally 180-second observation phases. The frozen runtime identity and run ID
are in the accompanying JSON. The successful-image sequence counts individual
nonempty native image results across both states. It does not count packages.

| Phase | Aggregate budget | Successful images | Observed seconds | Images/s |
| --- | --- | --- | --- | --- |
| Default before | 32 MP | 176 | 178.297 | 0.987 |
| Larger package | 128 MP | 498 | 178.406 | 2.791 |
| Default after | 32 MP | 180 | 178.313 | 1.009 |

The requested range cap remains four throughout. The default aggregate budget
admitted one range per package in the observed workload. The larger budget
admitted four. Boundary observations include packages from the preceding phase.
Elapsed denominators end at the last sampled image sequence, approximately
1.6 seconds before each nominal phase ends. System physical RAM and commit
headroom remained above the declared 8 GiB reserve in all recorded samples.

The scoped control file sits two parents above the active group directory.
The runtime reads it at package boundaries. This probe started with no control
file, atomically applied the larger settings, removed its own settings for the
last phase and confirmed restoration afterward. Existing or externally changed
controls cause an ownership error. The comparison was neither restarted nor
given a different capture plan. Per-image limits remain unchanged.

The roughly 2.8-fold operational throughput observation concerns one live pair.
Adjacent ranges differ between phases, caches remain warm and background work
continues. This is not a randomized corpus-wide speed estimate. Pixel equivalence
is a separate question addressed by the retained Lysimeter package-budget
experiment in `capture-package-pixels/README.md`.

After the pilot, both exact ongoing QT/RR pairs received four-range/128 MP
settings under a separate identity-bound memory watcher. It restores its own
default controls when the pair exits, its run identity changes, it reaches a
terminal state or the memory reserve is crossed. It preserves external changes.
The pilot receipt remains immutable and separate from this subsequent operation.

The public numeric projection omits raw display labels because their original
read introduced a character-decoding artifact. Raw private evidence remains
unchanged. The projection retains the raw-state and probe-source SHA-256 values.
