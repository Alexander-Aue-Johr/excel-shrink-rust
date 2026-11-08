# SheetShrink and DeltaSieve delivery plan

This is the reviewable work queue for the two tools, their Observatory, and their papers. Execute the numbered tasks in order after approval, keeping each task's evidence and remaining work here. Tasks marked as existing work require verification and integration. Their status does not establish that the planned acceptance checks have passed.

## 00 Wait for approval

- [ ] The user gives an explicit Go for this plan, or manually removes this approval gate after reviewing it.

Begin Tasks 01–19 after that approval. The independently running Enron process may continue while the plan awaits review. The existing monitoring Pursuing Goal is paused at the user's requested approval gate. Resume it after approval and use this queue to preserve the implementation order.

That Go authorizes autonomous execution through Task 19. Resolve routine implementation, sampling, and presentation choices using the agreed requirements and record material assumptions. Perform the reviews and acceptance checks in this plan during execution. The author's later manuscript revision and optional external research can follow the completed working drafts.

## Delivery rules

- Preserve prepared workbooks, source identities, completed evidence, and restart compatibility throughout changes. Bind reusable results to the source, relevant configuration, method, and producing implementation.
- Keep temporary files, builds, probes, and exported working data on V:. Track disk usage and give retained evidence a documented lifetime.
- Use focused tests and reviewable commits. Inspect staged changes, update the relevant behavior specification, and push each completed conceptual change before deploying it into a run.
- Separate mechanical restructuring from behavioral changes. Apply Single Level of Abstraction and useful Rustdoc or Python docstrings in touched code according to the repository agreements.
- Record a task's outcome, verification, commit IDs, and unresolved limitations before checking its box. A failed or unsupported comparison retains its reason and coverage status.
- Keep unknown, unverified, and failed cases distinct from equality. Preserve raw findings alongside reviewed classifications.
- Use ordinary web research and verified primary publications for the literature work. Save optional ChatGPT Deep Research prompts for later use and continue the queue with the research available directly.
- Record unavailable optional archives, comparison tools, credentials, or supplementary research and continue independent tasks. Keep claims within the evidence actually obtained. Escalate a genuine access or correctness blocker with its exact cause while advancing unaffected work.

## Keep Enron advancing throughout implementation

After Go, supervise the existing Enron run throughout Tasks 01–18. Keep it advancing during research, writing, development, and other work whenever native validation capacity is available. Background supervision begins with Task 01 and continues into Task 19's final sustained supervision and reconciliation phase.

Use the current run at `V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\enron-20261107` and its existing results as the mandatory fallback evaluation corpus. Preserve its 20,872-input selection, source manifest, conversion and shrink artifacts, findings, and completed coverage. Advance it toward completion even while preparing a possible Enron XML corpus. Freeze a dated fallback result snapshot for the papers if a replacement corpus is not ready in time.

Prepare fixtures, builds, and expected results before reserving native Excel capacity for a test. Prefer one small fixture and one comparison group initially, using spare capacity or one released group while the remaining Enron groups continue. Give every native experiment a case count, resource allocation, and wall-time budget before starting it. Target five-minute test batches and cap an exclusive Enron pause at ten minutes per batch. Split larger experiments across successive batches or schedule them on spare capacity. Retain timed-out or incomplete test outcomes and complete the required verification before deploying a change.

When an exclusive test window is necessary, checkpoint the run and record why it is paused. Restore the configured Enron admission target after test success, failure, or timeout, clean up owned test processes, and verify that corpus progress resumes. Limit heavy build and benchmark concurrency when it materially reduces Enron throughput. Record interruption durations and resume failures so they can be investigated.

## Priority and dependencies

The approval gate comes first. Establish the baseline and scientific protocols before changing behavior. Deliver unified startup and static result exploration before replay. Prepare paper outlines early and fill them with verified results as they become available. Introduce structural changes in small steps alongside the features they enable.

Give correctness, a reproducible evaluation, and a shareable results explorer priority over replay and larger structural work. Preserve a dated evaluation snapshot for submission even if corpus supervision continues afterward. Express its coverage precisely.

Scheduling experiments and new corpus acquisition have bounded initial acceptance criteria. Their depth depends on measured benefit and the paper schedule. Background Enron supervision spans the entire queue. Task 19 remains the final open task until its completion condition is met.

## 01 Establish the implementation and run baseline

- [ ] Inspect the active checkout, relevant remote branches, current run configuration, saved evidence, and outstanding investigations.

Compare the current implementation with `origin/codex/sheetscope-validation` and the corpus preparation branch in the paper repository. Identify completed features, reusable changes, incompatible assumptions, and remaining tests before integrating anything.

Preserve the active run's launch receipt, producer identity, selected corpus, partition configuration, preparation results, visual coverage, failures, and raw differences. Capture the restart command and ordinary launcher behavior.

Known starting points for verification are the `xlExclusive` conversion change in `135f7d1`, process costs and per-sheet timings in `2e100c2`, manual inspection in `ed8754d`, inspection cleanup in `61defff`, and inherited-border preservation in `abc9e79`.

Acceptance requires a concise implementation inventory, a reproducible baseline, and a recovery procedure that can resume compatible results. Existing open boundary and chart findings retain their evidence and unresolved status.

## 02 Freeze the behavioral contracts and evaluation protocols

- [ ] Reconcile the SheetShrink and targeted-validation specifications with the agreed requirements and define the evaluation before selecting its samples.

SheetShrink Default is the production subject. Maximum and Too Aggressive remain historical test artifacts. Preserve values, formulas, and required visual behavior while removing deterministically irrelevant structures with a small preservation footprint. Cover row-font sentinels, overflow blockers, inherited formats, and protection within the actual visible content area. That area includes visible fills, borders, and objects. Layout changes beyond its boundary have a different contract.

Record the distinction between stored original, Excel-saved original, rebuilt original, shrunk output, and optional Check Performance output. The fast primary visual comparison uses rebuilt Original against rebuilt Default. Saved and pre-rebuild controls explain attribution where needed and affordable. Opening, saving, conversion, recalculation, volatile formulas, and rendering variability need separate accounting.

Specify evidence needed to distinguish a real regression, a capture-extent difference, environment-dependent values, and small rendering variation. Preserve the underlying pixel results. A discovered volatile dependency alone cannot explain every changed pixel in its capture.

Acceptance requires versioned contracts, predefined sampling and timeout policies, and explicit limits for the paper claims.

## 03 Research the paper design and create fresh LaTeX projects

- [ ] Research suitable paper designs and create two fresh, independently compilable LaTeX projects in the existing paper repository.

Create `papers/sheetshrink/` and `papers/deltasieve/` with a main LaTeX source, section sources where useful, bibliography, figure sources, and build instructions. Use the installed TeX Live on D: and place temporary compilation data on V:. Shared bibliography or figure tooling is appropriate where it removes duplicated work.

Treat historical manuscripts and reviews as sources of observations, experiments, requirements, and development history. Develop the new papers' argument, organization, wording, and figure choices independently from those earlier manuscripts.

Before selecting the structures, conduct a documented full-text study of accepted ICDE papers and relevant papers from comparable venues such as SIGMOD and VLDB. Verify publication and track from primary sources. Select examples for methodological relevance, including systems, optimization, data preparation, validation, and experimental evaluation. Record the selection rationale and evidence supporting each recommendation. Distinguish observed practices from official requirements and from hypotheses about what contributed to acceptance.

Complete this study autonomously using available research and publication sources. Optional ChatGPT Deep Research reports may supplement it later. Use known author metadata and explicit placeholders for missing details while preparing the working projects.

Analyze problem and contribution framing, narrative and section order, space allocation, scientific English, terminology, claim strength, evaluation design, limitations, and reproducibility. Examine how Related Work is grouped, how finely methods are compared, where the discussion appears, and how citations support the introduction, method, and evaluation as well as a dedicated section. Derive a justified design for each paper from that analysis and the actual evidence available.

Study figures as part of the argument, including motivating examples, method diagrams, performance plots, and interface screenshots. Evaluate a compact first-page Observatory figure for its ability to explain DeltaSieve's contribution, its readability at publication size, and its use of the page budget. Compare a screenshot, an annotated detail, and a conceptual overview where relevant. Record the inclusion and placement decision under the current venue template. Select images for their explanatory contribution to each paper.

SheetShrink must cover spreadsheet bloat, its preservation contract, compact retention mechanisms, byte-oriented processing, parallel execution, and measured end-to-end benefit. DeltaSieve must cover candidate selection, dependency reasoning, native Excel verification, coverage, conservative fallback, and measured detection effectiveness and cost. Let the research and evidence determine how these topics form each paper's argument.

Link every planned claim, table, and figure to an experiment, retained result, or verified publication. Begin the new manuscripts while evaluation continues and update the outlines when the evidence warrants it. Task 14 delivers complete first drafts for the author's thorough review and rewriting.

Acceptance requires both projects to compile, an annotated paper-design study with verified sources, a justified outline and figure plan for each manuscript, a claim-to-evidence map, and clear separation of the papers' contributions.

## 04 Integrate the web configuration entry point

- [ ] Provide the ordinary startup path through a web configuration page and the Observatory.

Reuse the standalone branch's working components where their behavior matches the contracts. Share configuration validation and run coordination across CLI and web entry points. Opening the application without a complete run configuration should expose configuration. Starting a configured run should lead to its Observatory.

Replace the WPF-dependent ordinary launch path, including the VS Code task entry point. Preserve direct CLI operation and saved configurations. Make new-run, resume, and open-existing-results choices explicit.

Acceptance requires equivalent resolved configuration through the ordinary launcher and CLI, a successful resume of a controlled existing run, and documented start commands.

## 05 Add stable local service names

- [ ] Expose understandable local addresses using the DeltaSieve name.

Prefer `deltasieve.localhost` and clear service names such as `configure.deltasieve.localhost` and `addin.deltasieve.localhost` where useful. Establish a stable port and discovery mechanism. Evaluate a local reverse proxy for standard ports only if the integration remains practical. A hostname itself does not select a service port.

Preserve loopback binding, Host and Origin checks, session protection, and required Office add-in HTTPS behavior. Handle occupied ports with an explicit working address and identify stale servers by run and build identity.

Acceptance requires working links from startup through the Observatory and add-in, including a port-conflict test and an understandable fallback address.

## 06 Unify profiles, resume, and partitions

- [ ] Replace special-purpose launch configuration with one versioned configuration model.

Provide corpus profiles, immutable resolved run settings, configurable partition counts, and grouped partition selection. Assign files deterministically to partitions and report selected and total coverage. Changing the partition layout must preserve reusable per-file evidence when its compatibility requirements still hold.

Separate conversion concurrency, shrink processes and threads, visual groups, geometry batch size, capture batch size, and image retention. Preserve independent API adjustment where already supported. Keep memory admission and ownership checks explicit.

Acceptance requires tests for restart, interrupted attempts, duplicate requests, changed methods, changed inputs, changed partitions, and missing evidence. The ordinary Enron configuration must reproduce the intended current settings without a hidden launcher.

## 07 Verify conversion and environment-dependent formulas

- [ ] Verify completed conversion fixes and finish the handling of `CELL("filename")` and related evidence.

Retain the `xlExclusive` behavior that avoids carrying legacy shared-workbook history into converted outputs. Check it through the normal conversion entry point with read-only input handling and recorded conversion effects.

Trace straightforward environment-dependent and volatile formula paths where supported. Show source coordinates, values, formulas, and the causal path in the viewer. Distinguish an explained cell-value change from unrelated formatting changes in the same image. Unsupported references remain explicit.

Acceptance requires targeted conversion and formula fixtures, retained raw differences, and honest classifications in both the list and Atelier.

## 08 Verify process metrics and improve inspection access

- [ ] Exercise the existing timing, RAM comparison, and manual inspection features through the final startup path.

Show launch-to-readiness time and memory measurements with their exact meaning. Use paired bars that emphasize the card's own process and keep the counterpart visible. Expand cumulative phase timings into per-sheet detail where that evidence exists. Distinguish overlapping work from additive durations and retained historical measurements from current measurements.

Inspection should reuse owned Excel instances, open temporary copies of the correct retained states, select the requested region, and place Original and comparison side by side. Handle a user closing one workbook or application and switching to another comparison. Preserve the user's choice of Atelier fullscreen mode and stable navigation-button positions.

Provide compact, filtered monitoring queries for run summaries, active attempts, phase durations, new failures, differences, and resource trends. Support bounded responses, event cursors, and detailed evidence on demand. Use the existing API and SQLite evidence where suitable. Add a thin MCP adapter if it materially simplifies model access. Keep control operations explicit, attributable, and scoped to the intended run. Bound diagnostic caches and retained telemetry so polling and inspection remain affordable.

Acceptance requires normal UI checks, closing and reopening inspection cases, consistent unavailable-data presentation for older results, and representative monitoring queries with measured response size and cost.

## 09 Export a portable results explorer

- [ ] Deliver a static report that reuses the Observatory's results exploration and Atelier presentation.

Support workbook lists, difference summaries, paired images, pixel measurements, masks, enlarged regions, and available formula evidence. Include corpus identity, configuration, hardware, Excel version, source and producer versions, checksums, coverage, exclusions, and methodological limits.

Use relative assets and a data source suitable for static hosting. Present the capabilities available in an exported report accurately. Review exports for local paths, usernames, machine names, secrets, and unnecessary raw data.

Organize exports by corpus, date, implementation, and an anonymous hardware description. Keep large corpus archives and bulk image artifacts in suitable artifact storage. Prepare a GitHub Pages deployment procedure and a stable per-run link format.

Acceptance requires a served static-export smoke test with the local runner unavailable, checksum verification, and a privacy review performed during execution. Select a dated, frozen result set for the export and record its identity. Prepare the deployment procedure and release files so later public publication can use the verified package when the destination and access are available.

## 10 Measure the performance report's proposals

- [ ] Test the four proposals from `R:\excel-shrink-performance-review-a7445eb-20261107.zip` individually before adopting them.

The proposals concern an additional attribute pass, first represented-style bookkeeping, small style-frequency tables, and early rejection of an impossible font target. Treat the supplied review as hypotheses. Measure the current implementation and each relevant proposal with identical inputs and output checks.

Use the large Destatis cases identified by the report and representative dense, sparse, hidden-row, and column-format cases. Record wall time, CPU time, peak memory, execution environment, and run variation. Separate Windows-local and WSL or network-path effects. Check decompressed workbook-member equivalence where expected and investigate legitimate semantic differences.

Acceptance requires a reproducible experiment and a separate decision for each proposal. Integrate measured improvements with focused correctness checks and separate commits.

## 11 Benchmark workbook and sheet scheduling

- [ ] Measure whether current scheduling leaves capacity unused for small workbooks and mixed corpora.

Create bounded synthetic corpora covering many small one-sheet workbooks, many sheets, a few massive sheets, and mixed sizes. Inspect current large-sheet chunking, task priority, work stealing, and memory admission using timings and Perfetto where useful.

Apply the shared native-test budgets when an experiment needs Excel or validation-engine resources. Start with a small pilot and expand in bounded batches according to its measured cost and benefit.

Change scheduling only where measurements identify a bottleneck. Preserve bounded memory and consistent outputs. Record useful throughput across process and thread configurations, including oversubscription where appropriate for conversion.

Acceptance requires a reproducible workload matrix, throughput and memory results, and evidence for any adopted scheduling change.

## 12 Evaluate SheetShrink across the selected corpora

- [ ] Produce a versioned Default evaluation snapshot for Data.gov, Destatis, and Enron with explicit coverage accounting. Task 19 carries supervision through the remaining corpus work.

Report file-size reduction, relevant XML reduction, shrink time, CPU time, peak RAM, and end-to-end shrink-plus-open versus direct-open time. Include low-bloat overhead and corpus distributions. Measure serial and parallel conditions fairly and record Excel's observed behavior.

Separate input validity, conversion effects, open failures, repair requirements, preparation exclusions, completed comparisons, and unresolved cases. Fix confirmed Default regressions with compact preservation mechanisms and revalidate affected outputs.

Treat Python DOM and regex implementations as historical systems with different capabilities and safety properties. If benchmarked, report their output differences and validation limits alongside speed. Archive meaningful versions and development milestones with traceable provenance.

Use a predefined, feasible Check Performance sample with saved-original controls if the schedule permits. Its results should distinguish optimization effects from Excel's ordinary saving and rebuild behavior.

Acceptance requires reproducible tables, declared denominators, evidence for claimed invariants, and a reviewed regression ledger.

## 13 Evaluate DeltaSieve detection and cost

- [ ] Validate candidate selection against an independent full-capture reference and controlled mutations.

Choose a predefined stratified real-world subset with a seed, time budget, and inclusion criteria. Record timeouts and failures. Include held-out cases. Generate reference captures independently of the candidate selector being evaluated.

Run native validation and full-reference experiments in bounded batches under the shared interruption policy. Preserve partial experiment progress between batches so required tests can finish while Enron continues to advance.

Cover values, formulas, relative and absolute references, ranges, cross-sheet references, supported INDIRECT patterns, names, unknown and external dependencies, mixed conditional-formatting rules, data bars, duplicate rules, color scales, charts, drawings, merges, heights, widths, hidden dimensions, wrapping, overflow, inherited styles, protection, and reference errors. Mutations need a verified expected visual effect or an explicit invariance expectation.

Measure detection of changed visible regions, candidate coverage, unnecessary captures, analysis cost, native capture cost, and fallback behavior. Include same-input rendering controls and representation-only changes such as reordered equivalent style and shared-string identities.

Estimate full-capture cost from actual visible geometry, object extent, image limits, and calibrated capture throughput. Account for hidden dimensions and opening cost. Report estimation uncertainty separately from measured runtime.

Acceptance requires reproducible mutation fixtures, independent reference comparisons, ablations of major selection mechanisms, and a precise statement of the supported model and remaining limits.

## 14 Write and review complete first manuscript drafts

- [ ] Produce a complete first draft of each new paper using Task 03's research-derived design and the verified evaluation evidence.

Write substantive scientific English for every planned section, including the abstract, introduction, problem and scope, contributions, method, evaluation, Related Work, limitations, and conclusion as appropriate to each design. Include the planned figures, captions, tables, bibliography, and artifact descriptions. Adjust the structures when deeper research or results justify a change. Mark missing measurements and unresolved claims explicitly in the working drafts and their evidence checklist.

These drafts serve as editable starting points for the author's comprehensive review, rewriting, restructuring, and replacement of text or figures. Preserve a clear draft status and record substantive author decisions. Prepare the complete drafts and accompanying review notes for that human revision.

Deliver the working drafts and proceed through the remaining tasks while the author's revision is pending. Record open author questions in the handoff notes and use provisional wording or metadata where necessary.

Examine Microsoft Spreadsheet Compare and Inquire, Check Performance, Synkronizer, DiffEngineX, efficient spreadsheet readers, and relevant dependency-based and visual regression-testing research. Verify whole-workbook reports, property coverage, chart handling, automation, scalability, and licensing constraints where they affect experiments.

Build Related Work around the closest methods and the specific dimensions on which they differ. Explain their assumptions, guarantees, scope, dependencies, cost, and evaluation where supported by the sources. Integrate this positioning into the argument wherever it motivates a design or comparison. Verify citations against the full publications and use precise, evidence-qualified language.

Save one optional Deep Research prompt per paper for the user's later use. Request primary sources, competing methods, and evidence that challenges novelty. Complete the drafts using the autonomous primary-source research and continue through Task 19 while these optional reports are pending. When the user later supplies them, verify the sources and integrate relevant additions or corrections into the literature review, claims, figures, and experiments. Record which conclusions or results require further work.

Select the preservation and selection mechanisms that carry the papers' central arguments. Keep comprehensive rule inventories in versioned documentation. Report corpus and method versions, sampling, hardware, Excel build, limitations, artifact availability, and required AI-use disclosure under the applicable venue rules.

Apply the current IEEE template and ICDE submission requirements, including figure placement and the page budget. Record substantive AI assistance in drafting, figures, code, data processing, and experiments so the final disclosure can reflect the actual use and subsequent human revisions. ICDE 2027 requires disclosure of AI-generated content and relevant experimental assistance, with an exception generally intended for editing and grammar enhancement. [ICDE 2027 submission guidelines](https://icde2027.github.io/submission-guidelines.html)

Preserve useful historical unpublished manuscripts as clearly dated artifacts. Summarize prior feedback as requirements and research motivation. Check applicable confidentiality conditions before distributing full review text.

The historical Excel Shrink papers and reviews are available in the local archive `V:\excel_shrink_papers_and_reviews.7z`. The user reports favorable reviews and attributes the unsuccessful submission outcome in substantial part to an overlooked instruction concerning a review letter. If this history becomes relevant, inspect the archived documents to establish the exact sequence, requirements, and decision. Include any required review or response letter in the submission checklist for the new papers.

Acceptance requires two complete, compilable first-draft PDFs and their fresh LaTeX projects, verified citations, claim-to-evidence checks, reproducible figures and tables, the first-page figure decision, a list of unresolved evidence gaps, and an author-review checklist. Recheck current venue requirements and disclosure when preparing a later submission version.

## 15 Add bounded event history and replay

- [ ] Add replay using compact events, periodic checkpoints, and a virtual clock.

Provide play, pause, seek, time jumps, acceleration, and slowed playback. Reuse the live components and recorded state transitions. Preserve useful opening, phase, sheet, capture-progress, process, and heartbeat history according to the recording policy.

Store compressed, independently loadable chunks and a manifest. Use relative same-origin requests on static hosting, bounded decoding and caches, and cancellation when seeking. Retain difference images according to the evidence policy. Identify unavailable historical telemetry explicitly.

Acceptance requires deterministic reconstruction at selected timestamps, correct seeking, bounded memory on a representative long run, and static-host playback. Explain which events were recorded, sampled, or unavailable. Replay demonstrates recorded execution and supports exploration alongside provenance and checksums.

## 16 Extract coherent software components

- [ ] Establish the shared toolset structure incrementally as features create concrete boundaries.

Use one software repository with coherent crates for shrinking and validation, applications for SheetShrink, DeltaSieve, and the Observatory, and separate behavior and experiment documentation. Keep reusable validation available independently and integrate SheetShrink efficiently through shared Rust APIs where appropriate.

Move code in reviewable steps with clear ownership of domain decisions, orchestration, persistence, transport, and presentation. Preserve ordinary entry points and saved-result compatibility. Select final package and folder names after inspecting current dependencies.

Acceptance requires a standalone validation entry point, a working integrated shrink-and-validate path, and focused compatibility tests after each extraction.

## 17 Version the Enron acquisition and extraction profile

- [ ] Reuse the corpus-preparation branch to define a reproducible Enron source profile and extraction workflow.

Inspect the Archive.org EDRM sources, source adapter, recursive container handlers, and existing checkpoint formats. Preserve source URLs, hashes, archive and attachment lineage, duplicate-content identities, and occurrence counts. Give unsupported, corrupt, encrypted, and resource-limited cases explicit outcomes.

Separate reusable extraction mechanisms from Enron-specific source descriptions. Expose preparation preview, download, extraction, and resume through the configuration workflow. Bound recursive expansion and record its limits.

The current 20,872-file Enron selection remains an identifiable corpus version. A newly extracted selection receives its own manifest and run identity.

Protect the current run's progress and retained artifacts while preparing the XML-derived corpus. Keep evaluation, drafting, and release preparation moving with the current corpus as the fallback if the new corpus misses the evaluation freeze. Identify each result's corpus version and coverage explicitly.

Initial acceptance requires a reviewed profile, representative nested-container tests, restart verification, and a frozen corpus manifest. Schedule a full new acquisition according to its cost and the evaluation plan.

## 18 Prepare releases and assess the shrinker replay extension

- [ ] Freeze the paper artifacts, validate the ordinary start and resume instructions, and evaluate the value of a SheetShrink scheduling replay.

Create versioned manifests and checksums for code, profiles, evaluation scripts, results, and static reports. Verify existing publication links and artifact storage boundaries, and record deployment status for prepared exports. Retain useful Perfetto traces and figures that explain scheduling.

For a possible shrinker Observatory, determine whether existing traces can support a small useful demonstration of workbook, sheet, and chunk scheduling with slowed playback. Record the decision and remaining work. Give paper evidence and the delivered results explorer priority when scheduling that extension.

Acceptance requires reproducible release instructions, an artifact inventory, verified start and resume examples, a publication-ready export package, and a documented decision on the extension. Final author metadata, author approval, paper submission, and external publication can follow later. Record any required credentials or destination choices in the handoff.

## 19 Continue sustained pipeline supervision through completion

- [ ] Continue the supervision maintained during Tasks 01–18 until every selected input has a final, evidenced outcome or the user explicitly stops the work.

Continue the current Enron selection with compatible existing results. Follow up on remaining Data.gov and Destatis coverage required by Task 12, identifying already completed work from its manifests. Monitor conversion, shrinking, opening, rebuilding, geometry, captures, comparisons, failures, resource usage, and remaining coverage.

Investigate unusual stalls using retained diagnostics and owned Excel windows where authorized. Recognize security and rights-management dialogs, read-only prompts, repair requirements, external-link problems, printer prompts, and save failures promptly. Use controlled inspection copies for interventions and record their effects. A repaired workbook that loses source data requires an explicit outcome and reason.

Keep C: and V: capacity under observation. Maintain safe, owned graphics-cache and temporary-data cleanup and the configured evidence-retention policy. Preserve the original corpus and needed results during cleanup and restart.

Examine meaningful visual findings. Distinguish confirmed regressions, rendering variability, volatile or environment-dependent values, and capture-boundary artifacts using suitable controls. Fix real SheetShrink regressions without unnecessarily retaining large irrelevant regions. Retest and reprocess the affected files when their produced outputs or validation method change.

Keep monitoring summaries compact and inspect detailed evidence on meaningful changes. Commit, test, push, and deploy fixes in recoverable steps. Record every input's final status and unresolved limitation.

Acceptance requires reconciled corpus totals, completed or explicitly justified skipped outcomes, a reviewed difference ledger, preserved evidence, and a final report of coverage and remaining limitations. Starting a run or observing temporary progress does not complete this task.

## Progress record

Approval is pending. The next action is the user's review of Task 00 and the planned scope. Record subsequent decisions and completed-task evidence below as work proceeds.
