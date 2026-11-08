# Optional literature cross-checks

These prompts are for later use by the author. Their results supplement the
ongoing primary-source study. Code, evaluation, drafts and Enron supervision
continue while those results are unavailable. Verify every returned source and
claim before incorporating it.

## SheetShrink

Review the novelty, evaluation requirements and Related Work of a proposed
data-engineering paper about SheetShrink. Use primary publications and official
tool documentation. Search for evidence that challenges the proposed contribution.

The system transforms XLSX packages using byte-oriented worksheet processing. It
removes eligible empty or redundant cell structures while preserving required
data, dependencies and visible presentation under an explicit contract. Empty
font representatives may preserve automatic row height. Empty formatting can
cancel an inherited fill or border. Whitespace can block text overflow. Default
retains such effects, using compact witnesses where safe. The tool reuses style
identities and leaves general shared-string/style-table compaction to Excel.
It has separate guarded hidden-constant-tail handling. It does not execute Excel
formulas itself. Legacy XLS conversion is a separately measured preprocessing
stage. Historical Python DOM and regex implementations have different output
behavior and safety limits.

Find closely related spreadsheet cleanup, format redundancy, workbook repair,
specialized spreadsheet parsing, data preparation and document-preserving
transformation research. Include SheetReader, spreadsheet testing and commercial
cleanup capabilities, then identify missing predecessors. Explain which methods
preserve values, formulas, dependencies, layout or only selected observations.
Distinguish task differences before comparing performance.

Evaluate the proposed end-to-end experiment measuring cleanup plus opening the
cleaned workbook against opening the original. Examine single-thread and parallel
conditions, low-bloat overhead, file diversity, peak memory, ZIP/decompression
costs and the fairness of historical baselines with unequal outputs. Specify
independent functional and visual checks needed for bounded claims. Microsoft
Check Performance is an optional cleanup baseline with an ordinary-save control.

Return verified bibliographic metadata, direct primary-source links, relevant
method sections, comparison dimensions, counterexamples and an evidence-gap
table. Separate demonstrated facts from inferred design advice. Examine accepted
ICDE/VLDB/SIGMOD paper structures for comparable systems contributions, including
motivating figures and Related Work integration. Do not invent performance
results, novelty guarantees or proof of visual preservation.

## DeltaSieve

Review the novelty, evaluation requirements and Related Work of DeltaSieve, a
targeted native-rendering validator for spreadsheet transformations. Prioritize
primary research and official comparator documentation. Actively look for
existing systems that already provide equivalent change selection or evidence.

DeltaSieve compares cell values, effective styles across ID renumbering, and
layout. It follows supported conditional-formatting and chart dependencies.
Unchanged supported rules and inputs can exclude rendering. Changed dependencies
make dependent areas candidates without evaluating whether a rule's displayed
outcome would happen to remain equal. Unsupported dependencies retain explicit
fallback or incomplete coverage. Captures are limited to justified relevant
content, including visible fills, borders, overflow and objects. Native geometry
controls image subdivision and batching. The fast comparison uses separately
rebuilt Original and transformed states. Saving, recalculation, volatile formulas,
environment-dependent values and rendering variability complicate attribution.

The Observatory and static reports expose retained differences, exact pixel
measurements, available dependency paths, timings, coverage and failures. A future
recorded-event replay demonstrates execution. Presentation and replay do not
establish selector recall or authenticity by themselves.

Find spreadsheet comparison/testing, dependency slicing, selective regression
testing, change-impact analysis, visual regression and rendering-based document
validation work. Include spreadsheet slicing, Ekstazi and ReDeCheck, then identify
closer predecessors. Check Microsoft Spreadsheet Compare/Inquire, Synkronizer,
DiffEngineX and other documented comparators. Separate structural workbook reports
from native rendered-pixel observation and separately evaluate selection cost.

Propose independent mutation classes and stratified full-reference samples for
estimating missed visible changes and avoided rendering. Include same-input
controls, identity-preserving serialization changes, indirect references, objects,
layout, timeouts and failed workbooks. Explain detection denominators, selection
bias, uncertainty and limits of full-corpus extrapolation from visible geometry.
Identify claims that finite experiments can and cannot support.

Return primary-source bibliographic records and links, a technique-by-technique
comparison, counterexamples to novelty and soundness, an evaluation critique,
and recommendations for the paper's argument and scientific figures. Keep
inferences distinct from published findings. Do not equate a detailed replay or
equal selected images with a complete preservation guarantee.
