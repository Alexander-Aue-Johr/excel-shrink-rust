# Paper design research

This working bibliography records inspected primary publications and design hypotheses for the two new manuscripts. Extend the study as the arguments and evaluation mature.

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

## Comparison-tool evidence checked on 2026-11-08

Microsoft Spreadsheet Compare provides a workbook difference report, including
formula and cell-format categories and hidden worksheets. The paper should
therefore evaluate its documented scope and available installation before making
claims about existing comparators. Its documented interface alone does not
establish renderer-level pixel equivalence or candidate-planning cost.
[Microsoft documentation](https://support.microsoft.com/en-gb/excel/basic-tasks-in-spreadsheet-compare)

ExcelCompare's vendor describes formula, value and text comparison with a
detailed difference report and PDF output. This is a candidate for a separately
scoped functional comparison. These are documented capabilities, with no local
runtime or detection evaluation yet performed.
[Vendor documentation](https://spreadsheetsoftware.com/excelcompare/)

## Specialized semi-structured parsing

Li and colleagues' *Mison* appeared in PVLDB 10(10), 2017. The inspected
introduction opens with a measured parsing-cost figure, then explains structural
indexing and speculative field access. Its Related Work groups application
systems and prior parsing mechanisms while positioning the contribution within
each group. A useful design inference for SheetShrink is to connect its byte
processing to a precise transformation obligation and measure complete user
cost. [Author-hosted publisher paper](https://www.microsoft.com/en-us/research/wp-content/uploads/2017/05/mison-vldb17.pdf)

Langdale and Lemire's *Parsing Gigabytes of JSON per Second*, VLDB Journal 28(6),
2019, separates structural processing, validation and conversion costs. Its
evaluation specifies compiler, hardware, single-threading and in-memory inputs.
This motivates explicitly separating parsing throughput from ZIP I/O and native
Excel opening in our measurements. JSON parsing supplies methodological context
for specialized execution, with a different format and output contract.
[Author manuscript](https://arxiv.org/pdf/1902.08318)

## Selective testing and visual layout models

Gligoric, Eloussi and Marinov's *Practical Regression Test Selection with Dynamic
File Dependencies*, ISSTA 2015, introduces dependency granularity through an
example and separates analysis, execution and dependency collection. Its
end-to-end evaluation accounts for selection overhead. For DeltaSieve, use an
explicit dependency model and assess selection cost alongside avoided rendering.
The related four-page *Ekstazi* demonstration is an ICSE 2015 publication, a
separate item from this research paper. [Author manuscript](https://users.ece.utexas.edu/~gligoric/papers/GligoricETAL15Ekstazi.pdf)

Walsh, Kapfhammer and McMinn's *ReDeCheck*, ISSTA 2017, compares responsive web
layout models and reports layout failures with diagnostic screenshots. Its
model extraction samples viewport widths and refines detected transitions.
It distinguishes observable failures from structural issues. This is relevant
to separating DeltaSieve's candidate model, native observations and human
classification, while the underlying document and rendering models differ.
[Author manuscript](https://philmcminn.com/publications/walsh2017a.pdf)

