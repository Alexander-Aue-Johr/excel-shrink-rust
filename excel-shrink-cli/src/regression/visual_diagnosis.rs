use super::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const CONSERVATIVE: &[&str] = &[
    "keep-unreferenced-hidden-content-cells",
    "keep-inactive-protection-cells-in-data-range",
    "keep-all-inactive-protection-cells",
];
const AGGRESSIVE: &[&str] = &[
    "remove-basic-textless-representatives",
    "remove-all-textless-representatives",
    "remove-unreferenced-whitespace-cells",
];

// These are selections for diagnosis, not claims that one flag caused a change.
fn follow_up_sheets(
    maximum: &BTreeSet<usize>,
    default: &Value,
) -> (BTreeSet<usize>, BTreeSet<usize>) {
    let mut conservative = BTreeSet::new();
    let mut aggressive = BTreeSet::new();
    for sheet in default["sheets"].as_array().into_iter().flatten() {
        let Some(index) = sheet["index"].as_u64().map(|n| n as usize) else {
            continue;
        };
        if !maximum.contains(&index) {
            continue;
        }
        match sheet["status"].as_str() {
            Some("changed") => {
                conservative.insert(index);
            }
            Some("same") => {
                aggressive.insert(index);
            }
            _ => {} // Incomplete checks must not select the unchanged branch.
        }
    }
    (conservative, aggressive)
}

fn changed_sheets(report: &Value) -> BTreeSet<usize> {
    report["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|sheet| sheet["status"] == "changed")
        .filter_map(|sheet| sheet["index"].as_u64().map(|n| n as usize))
        .collect()
}

pub(super) fn run(
    mode: &str,
    baseline_relative: &str,
    args: RegressionRunArgs,
    log_level: LogLevel,
) -> anyhow::Result<()> {
    ensure_no_visual_profile_cli_args(&args.cli_args)?;
    let root = repo_root();
    let baseline = resolve_path(
        &root,
        args.baseline_path
            .clone()
            .unwrap_or_else(|| baseline_relative.into()),
    );
    let suite = load_suite(&baseline);
    let corpus = args
        .corpus_root
        .clone()
        .or_else(|| std::env::var_os("EXCEL_SHRINK_CORPUS_ROOT").map(PathBuf::from))
        .map(|path| resolve_path(&root, path));
    let mut cases = suite.cases;
    let aggregate_expectation = suite.aggregate;
    if cases.is_empty() && mode == "corpus" {
        let discovery = corpus
            .clone()
            .or_else(|| existing_default_corpus_root(&root))
            .ok_or_else(|| anyhow::anyhow!("pass --corpus-root to discover workbooks"))?;
        cases = discover_corpus_cases(&discovery)?;
    }
    if let Some(id) = &args.case_id {
        cases.retain(|case| case.id == *id);
        anyhow::ensure!(!cases.is_empty(), "case id '{id}' not found");
    }
    anyhow::ensure!(
        args.allow_empty || !cases.is_empty(),
        "baseline contains no cases"
    );
    let output = resolve_path(
        &root,
        args.output_root.clone().unwrap_or_else(|| {
            format!(".tmp/regression/{mode}-visual-diagnosis-{}", timestamp()).into()
        }),
    );
    fs::create_dir_all(&output)?;
    let report_path = args
        .report_path
        .as_ref()
        .map(|p| resolve_path(&root, p.clone()))
        .unwrap_or_else(|| output.join("diagnosis.json"));
    let mut workbooks = Vec::<Value>::new();
    let mut any_failure = false;
    let mut opened = false;
    let mut maximum_reports = Vec::new();

    // One capture per source file, even when several sentinels use that file.
    let mut groups: Vec<(PathBuf, Vec<RegressionCase>)> = Vec::new();
    let mut group_indices = HashMap::<PathBuf, usize>::new();
    for case in cases {
        let path = resolve_case_input(&root, corpus.as_deref(), &case, log_level)?;
        if let Some(&index) = group_indices.get(&path) {
            groups[index].1.push(case);
        } else {
            group_indices.insert(path.clone(), groups.len());
            groups.push((path, vec![case]));
        }
    }
    let total_workbooks = groups.len();
    publish(
        &output,
        &report_path,
        &workbooks,
        total_workbooks,
        false,
        None,
    )?;
    for (source, group) in groups {
        let mut case = group[0].clone();
        case.min_reduction_ratio = group
            .iter()
            .filter_map(|c| c.min_reduction_ratio)
            .reduce(f64::max);
        case.max_output_bytes = group.iter().filter_map(|c| c.max_output_bytes).min();
        let checks = group
            .iter()
            .flat_map(|c| c.analysis_checks.iter().flatten().cloned())
            .collect::<Vec<_>>();
        if !checks.is_empty() {
            case.analysis_checks = Some(checks);
        }
        if group.iter().any(|c| c.analysis_mode == Some(true)) {
            case.analysis_mode = Some(true);
        }
        anyhow::ensure!(
            group.iter().all(|c| c.cli_args == group[0].cli_args),
            "sentinels sharing an input must use the same non-profile CLI arguments"
        );
        // Profile configurations are exact; baseline cleanup flags are not added.
        for item in &group {
            ensure_no_visual_profile_cli_args(item.cli_args.as_deref().unwrap_or_default())?;
        }
        let identity = case
            .relative_input
            .clone()
            .or(case.input_path.clone())
            .unwrap_or_else(|| source.to_string_lossy().into_owned())
            .replace('\\', "/");
        let key = artifact_cache::sha256(identity.as_bytes());
        let file_root = output.join(&key[..20]);
        case.id = key[..20].to_owned();
        workbooks.push(json!({"identity":identity, "name":source.file_name().unwrap_or_default().to_string_lossy(),
            "cases":group.iter().map(|c| &c.id).collect::<Vec<_>>(), "results":[], "complete":false}));
        let current = workbooks.len() - 1;
        let mut execute = |name: &str, sheets: Option<BTreeSet<usize>>| -> anyhow::Result<Value> {
            let predefined = predefined_visual_profiles()
                .iter()
                .find(|p| p.name == name)
                .unwrap();
            let profile = ResolvedVisualProfile::from_config(predefined.config);
            let mut cli_args = args.cli_args.clone();
            cli_args.extend(predefined.config.cli_args());
            eprintln!(
                "[{mode}] {} · {name} · {}",
                source.display(),
                sheets
                    .as_ref()
                    .map(|s| format!("{} selected sheets", s.len()))
                    .unwrap_or_else(|| "all visible sheets".into())
            );
            let options = visual_validation::VisualCheckOptions {
                profile: name.into(),
                sheets,
                keep_screenshots: args.keep_screenshots,
            };
            let result = run_case(
                mode,
                &root,
                &file_root.join(name),
                corpus.as_deref(),
                None,
                &cli_args,
                true,
                true,
                &options,
                log_level,
                &case,
            );
            let mut visual = json!({"profile":name, "status":"not checked", "sheets":[]});
            match result {
                Ok(report) => {
                    visual["status"] = json!(if report
                        .value_comparison_details
                        .as_ref()
                        .is_some_and(|v| !v.differences.is_empty())
                    {
                        "cell value differences"
                    } else {
                        "technical error"
                    });
                    if let Some(details) = &report.excel_screenshot_details {
                        visual = serde_json::from_slice(&fs::read(
                            details.output_dir.join("visual-result.json"),
                        )?)?;
                        visual["status"] = json!(if details.differences_found {
                            "visual differences"
                        } else {
                            "visually unchanged"
                        });
                    }
                    visual["profile"] = json!(profile.profile_name());
                    visual["failures"] = json!(report.failures);
                    visual["reduction_ratio"] = json!(report.reduction_ratio);
                    any_failure |= !report.failures.is_empty();
                    artifact_cache::atomic_write(
                        &file_root.join(name).join("case-result.json"),
                        &serde_json::to_vec_pretty(&report)?,
                    )?;
                    if name == "maximum-cleaning" {
                        maximum_reports.push(report);
                    }
                }
                Err(error) => {
                    any_failure = true;
                    visual["status"] = json!("technical error");
                    visual["failures"] = json!([format!("{error:#}")]);
                }
            }
            // Keep the shared report compact: unchanged sheets outside a focused
            // follow-up need a count, not hundreds of empty worksheet records.
            if let Some(sheets) = visual["sheets"].as_array_mut() {
                let checked = sheets.len();
                let unchanged = sheets.iter().filter(|s| s["status"] == "same").count();
                sheets.retain(|s| {
                    s["status"] != "same"
                        || options.sheets.as_ref().is_some_and(|selected| {
                            s["index"]
                                .as_u64()
                                .is_some_and(|i| selected.contains(&(i as usize)))
                        })
                });
                visual["checked_sheets"] = json!(checked);
                visual["unchanged_sheets"] = json!(unchanged);
            }
            visual["requested_sheets"] = json!(options.sheets);
            workbooks[current]["results"]
                .as_array_mut()
                .unwrap()
                .push(visual.clone());
            publish(
                &output,
                &report_path,
                &workbooks,
                total_workbooks,
                false,
                None,
            )?;
            if !opened {
                if let Err(error) = visual_report::open_visual_report(&output.join("report.html")) {
                    eprintln!("Could not open diagnosis report: {error:#}");
                }
                opened = true;
            }
            Ok(visual)
        };
        let maximum = execute("maximum-cleaning", None)?;
        let changed = changed_sheets(&maximum);
        if !changed.is_empty() {
            let default = execute("default", Some(changed.clone()))?;
            let (conservative, aggressive) = follow_up_sheets(&changed, &default);
            for (names, sheets) in [(CONSERVATIVE, conservative), (AGGRESSIVE, aggressive)] {
                if !sheets.is_empty() {
                    for name in names {
                        execute(name, Some(sheets.clone()))?;
                    }
                }
            }
        }
        workbooks[current]["complete"] = json!(true);
        publish(
            &output,
            &report_path,
            &workbooks,
            total_workbooks,
            false,
            None,
        )?;
    }
    let aggregate = aggregate_expectation
        .as_ref()
        .map(|expectation| aggregate_report(&maximum_reports, expectation));
    any_failure |= aggregate.as_ref().is_some_and(|a| !a.failures.is_empty());
    publish(
        &output,
        &report_path,
        &workbooks,
        total_workbooks,
        true,
        aggregate.as_ref(),
    )?;
    if !opened {
        if let Err(error) = visual_report::open_visual_report(&output.join("report.html")) {
            eprintln!("Could not open diagnosis report: {error:#}");
        }
    }
    eprintln!(
        "[{mode}] Visual diagnosis: {}",
        output.join("report.html").display()
    );
    anyhow::ensure!(
        !any_failure,
        "One or more regression checks failed; report: {}",
        output.join("report.html").display()
    );
    Ok(())
}

fn publish(
    output: &Path,
    report_path: &Path,
    workbooks: &[Value],
    total: usize,
    complete: bool,
    aggregate: Option<&AggregateReport>,
) -> anyhow::Result<()> {
    let report = json!({"workbooks":workbooks,"total_workbooks":total,"complete":complete,"aggregate":aggregate});
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    artifact_cache::atomic_write(report_path, &serde_json::to_vec_pretty(&report)?)?;
    diagnosis_report::write(output, &report)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_the_existing_visual_check_command_as_an_alias() {
        for flag in ["--diagnose-visual-changes", "--all-visual-profile-checks"] {
            let command = RegressionCli::try_parse_from(["regression", "smoke", flag]).unwrap();
            let RegressionCommand::Smoke(args) = command.command else {
                panic!("expected smoke command")
            };
            assert!(args.all_visual_profile_checks);
            assert!(!args.keep_screenshots);
        }
    }

    #[test]
    fn routes_each_sheet_without_treating_incomplete_checks_as_equal() {
        let maximum = BTreeSet::from([1, 2, 3, 4]);
        let default = json!({"sheets":[{"index":1,"status":"changed"},
            {"index":2,"status":"same"},{"index":3,"status":"row heights equal"},
            {"index":9,"status":"changed"}]});
        assert_eq!(
            follow_up_sheets(&maximum, &default),
            (BTreeSet::from([1]), BTreeSet::from([2]))
        );
        assert!(changed_sheets(&json!({"status":"technical error"})).is_empty());
    }
}
