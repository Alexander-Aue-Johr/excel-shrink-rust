use crate::regression;

#[test]
fn corpus_regression_smoke_suite() {
    regression::run_smoke_suite_for_corpus_regression_test();
}

#[test]
#[ignore = "set EXCEL_SHRINK_CORPUS_REGRESSION_CASE_ID and run explicitly"]
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

#[cfg(feature = "cell-tracing")]
#[test]
#[ignore = "real-file trace regression; run explicitly"]
fn corpus_regression_trace_seeschifffahrt_sheet11_cells_removed() {
    regression::run_seeschifffahrt_sheet11_trace_removed_cells_for_corpus_regression_test(None);
}

#[cfg(feature = "cell-tracing")]
#[test]
#[ignore = "real-file trace regression; run explicitly"]
fn corpus_regression_trace_kernhaushalt_sheet84_d1048576_removed() {
    regression::run_kernhaushalt_sheet84_trace_removed_cell_for_corpus_regression_test();
}

#[cfg(feature = "cell-tracing")]
#[test]
#[ignore = "real-file trace regression; run explicitly"]
fn corpus_regression_trace_seeschifffahrt_sheet1_h15_visited() {
    regression::run_seeschifffahrt_sheet1_h15_trace_visited_cell_for_corpus_regression_test();
}

#[cfg(feature = "cell-tracing")]
#[test]
#[ignore = "real-file trace regression; run explicitly"]
fn corpus_regression_trace_foerderprogramme_sheet2_a5_visited() {
    regression::run_foerderprogramme_sheet2_a5_trace_visited_cell_for_corpus_regression_test();
}

#[cfg(feature = "cell-tracing")]
#[test]
#[ignore = "set EXCEL_SHRINK_CELL_TRACE_TARGET_CELL and run explicitly"]
fn corpus_regression_trace_selected_seeschifffahrt_sheet11_cell_removed() {
    let selected_cell = std::env::var("EXCEL_SHRINK_CELL_TRACE_TARGET_CELL")
        .ok()
        .filter(|value| !value.trim().is_empty());

    regression::run_seeschifffahrt_sheet11_trace_removed_cells_for_corpus_regression_test(
        selected_cell.as_deref(),
    );
}
