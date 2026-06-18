use crate::regression;

#[test]
#[cfg_attr(
    not(feature = "corpus-regression-tests"),
    ignore = "enable with --features corpus-regression-tests"
)]
fn corpus_regression_default_smoke_suite() {
    for case_id in regression::smoke_cases_for_corpus_regression_tests(false) {
        regression::run_smoke_case_for_corpus_regression_test(&case_id);
    }
}

#[test]
#[cfg_attr(
    not(feature = "corpus-regression-tests"),
    ignore = "enable with --features corpus-regression-tests"
)]
fn corpus_regression_full_smoke_suite() {
    for case_id in regression::smoke_cases_for_corpus_regression_tests(true) {
        regression::run_smoke_case_for_corpus_regression_test(&case_id);
    }
}

#[test]
#[cfg_attr(
    not(feature = "corpus-regression-tests"),
    ignore = "enable with --features corpus-regression-tests"
)]
fn corpus_regression_selected_smoke_case() {
    let case_id = match std::env::var("EXCEL_SHRINK_CORPUS_REGRESSION_CASE_ID") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => {
            let available =
                regression::available_smoke_cases_for_corpus_regression_tests().join(", ");
            eprintln!(
                "Skipping selected corpus regression smoke case because EXCEL_SHRINK_CORPUS_REGRESSION_CASE_ID is not set. Available case ids: {available}"
            );
            return;
        }
    };

    regression::run_smoke_case_for_corpus_regression_test(&case_id);
}
