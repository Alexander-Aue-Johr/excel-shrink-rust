# Blank-margin report validation, 2026-11-04

The paired native EPA ScienceHub run produced 262 region comparisons without
content differences. Of these, 259 carried `blank_margin_ignored=true`: their
measured content cores matched, their top-left-aligned nonblank pixels matched,
and only a trailing context edge had a different size. The report consumer still
required raw geometry equality and rejected the completed report.

The consumer now shares the producer's content-core geometry guard. An ignored
margin requires both images, unchanged valid nonempty core geometry contained
inside both images, unchanged capture origin and a false pixel-difference verdict.
Missing core measurements, moved/clipped content, changed core dimensions or a
pixel difference still reject that exclusion. Raw dimensions remain in evidence.
This fixes publication of the existing comparison policy without broadening it.
