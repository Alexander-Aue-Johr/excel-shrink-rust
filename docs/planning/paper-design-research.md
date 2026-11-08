# Paper design research

This working bibliography records inspected primary publications and design hypotheses for the two new manuscripts. Extend the study with accepted ICDE papers before selecting the final structures.

## SheetReader

Gavriilidis, Henze, Zacharatou, and Markl, *SheetReader: Efficient Specialized Spreadsheet Parsing*, Information Systems, 2023, DOI `10.1016/j.is.2023.102183`.

The introduction motivates parsing through a concrete data-science pipeline and quantifies runtime and memory before stating contributions. It distinguishes two execution strategies by their resource trade-offs. For SheetShrink, investigate an introductory end-to-end opening-cost figure and an explicit preservation contract alongside the algorithmic contribution. SheetReader is a relevant parsing baseline whose task differs from cleanup. [Author manuscript](https://heltzi.github.io/publications/gavriilidis_et_al.pdf)

## SQuaLity

Zhong and Rigger, *Understanding and Reusing Test Suites Across Database Systems*, Proceedings of the ACM on Management of Data, 2(6), Article 253, 2024, DOI `10.1145/3698829`.

The introduction derives research questions from concrete portability barriers. Its discussion includes configuration dependencies and inconsistent results between environments. For DeltaSieve, investigate explicit research questions for selection coverage, rendering reproducibility, and cost. Any analogy to database testing requires explaining the native spreadsheet rendering oracle and its limitations. [Author manuscript](https://suyang.zone/assets/pdf/sigmod25.pdf)

## Outstanding design evidence

Inspect complete evaluation and Related Work sections, verify additional ICDE examples, and examine representative first-page layouts at publication size. Record the difference between an observed writing practice and a recommendation inferred for these manuscripts. The first-page Observatory figure remains an open design decision.
