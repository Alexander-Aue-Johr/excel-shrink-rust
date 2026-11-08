# Paper design research

This working bibliography records inspected primary publications and design hypotheses for the two new manuscripts. Extend the study with accepted ICDE papers before selecting the final structures.

## SheetReader

Gavriilidis, Henze, Zacharatou, and Markl, *SheetReader: Efficient Specialized Spreadsheet Parsing*, Information Systems, 2023, DOI `10.1016/j.is.2023.102183`.

The introduction motivates parsing through a concrete data-science pipeline and quantifies runtime and memory before stating contributions. It distinguishes two execution strategies by their resource trade-offs. For SheetShrink, investigate an introductory end-to-end opening-cost figure and an explicit preservation contract alongside the algorithmic contribution. SheetReader is a relevant parsing baseline whose task differs from cleanup. [Author manuscript](https://heltzi.github.io/publications/gavriilidis_et_al.pdf)

## SQuaLity

Zhong and Rigger, *Understanding and Reusing Test Suites Across Database Systems*, Proceedings of the ACM on Management of Data, 2(6), Article 253, 2024, DOI `10.1145/3698829`.

The introduction derives research questions from concrete portability barriers. Its discussion includes configuration dependencies and inconsistent results between environments. For DeltaSieve, investigate explicit research questions for selection coverage, rendering reproducibility, and cost. Any analogy to database testing requires explaining the native spreadsheet rendering oracle and its limitations. [Author manuscript](https://suyang.zone/assets/pdf/sigmod25.pdf)

## Spreadsheet slicing and testing

Reichwein, Rothermel, and Burnett, *Slicing Spreadsheets: An Integrated
Methodology for Spreadsheet Testing and Debugging*, DSL 1999, pages 25–38.
The inspected introduction and methodology connect spreadsheet dependencies,
incremental testing, and fault localization in Forms/3. Their visual feedback
communicates testedness and suspected faults. DeltaSieve's image comparison and
OOXML change selection require a separate characterization. Investigate this
work as a dependency-analysis predecessor and distinguish the oracle and
coverage criteria in Related Work. [Publisher full text](https://www.usenix.org/legacy/publications/library/proceedings/dsl99/full_papers/reichwein/reichwein_html/index.html)

The discovered author-hosted ICDE 2023 `subtab-full.pdf` URL returned HTTP 404
when opened on 2026-11-08. Its search snippet is insufficient evidence for a
paper-design recommendation. Locate another primary full-text copy before use.

## ICDE example with verified research-track placement

Gavriilidis, Beedkar, Quiané-Ruiz, and Markl, *In-Situ Cross-Database Query
Processing*, ICDE 2023, DOI `10.1109/ICDE55515.2023.00214`. The conference program
lists this paper in Research Session 10. Its author-hosted preprint was inspected
for the introduction, evaluation setup and additional Related Work. The first
page motivates the bottleneck with a measured breakdown figure. Evaluation states
hardware, software versions, workload selection and measurement boundaries.
Related Work is organized by technical approaches, supplementing earlier baseline
discussion. For our drafts, infer a compact bottleneck figure and explicit
measurement boundaries. These observations do not establish why the paper was
accepted. [Author preprint](https://web.iitd.ac.in/~kbeedkar/publication/xdb-icde-23/GavriilidisBQM23_preprint.pdf), [official program](https://icde2023.ics.uci.edu/detailed-program/)

## Further design evidence

Inspect complete evaluation and Related Work sections, verify additional ICDE examples, and examine representative first-page layouts at publication size. Record the difference between an observed writing practice and a recommendation inferred for these manuscripts. The first-page Observatory figure remains an open design decision.
