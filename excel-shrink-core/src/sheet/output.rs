use crate::error::Result;
use crate::models::CleanSheetParameters;
use crate::sheet::cols::merge_extraneous_columns;
use crate::sheet::ranges::{get_max_column, retrieve_sheet_data_range_containing_cells};
use crate::sheet::sheet_data::{
    clean_empty_rows_from_after_cell_range, clean_sheet_rows_and_cells_in_cell_range,
};
use crate::sheet::streaming::PreprocessedSheetXml;

pub fn build_clean_sheet_xml(
    preprocessed: PreprocessedSheetXml,
    params: &CleanSheetParameters,
    disable_sheetdata_splitting: bool,
) -> Result<Vec<u8>> {
    let mut metrics = preprocessed.clean_sheet_metrics;

    let final_sheet_data = if disable_sheetdata_splitting {
        clean_sheet_rows_and_cells_in_cell_range(
            &preprocessed.sheet_data,
            params,
            &mut metrics,
        )?
    } else {
        let ranges = retrieve_sheet_data_range_containing_cells(&preprocessed.sheet_data, params);

        let cleaned_cells_range =
            clean_sheet_rows_and_cells_in_cell_range(&ranges.cells_range, params, &mut metrics)?;

        let cleaned_visible_rows_range =
            clean_empty_rows_from_after_cell_range(&ranges.rows_range, params, &mut metrics)?;

        [cleaned_cells_range, cleaned_visible_rows_range].concat()
    };

    let max_column = get_max_column(&metrics);
    let in_cols_node_output_chunks = match max_column {
        Some(max_column) => merge_extraneous_columns(
            &preprocessed.in_cols_node_output_chunks,
            max_column,
        ),
        None => preprocessed.in_cols_node_output_chunks,
    };

    let mut out = Vec::new();
    out.extend_from_slice(&preprocessed.until_worksheet_start_output_chunks);
    out.extend_from_slice(&preprocessed.before_cols_node_output_chunks);
    out.extend_from_slice(&in_cols_node_output_chunks);
    out.extend_from_slice(&preprocessed.before_sheet_data_node_output_chunks);
    out.extend_from_slice(&final_sheet_data);
    out.extend_from_slice(&preprocessed.before_merge_cells_node_output_chunks);
    out.extend_from_slice(&preprocessed.in_merge_cells_node_output_chunks);
    out.extend_from_slice(&preprocessed.until_worksheet_end_output_chunks);

    Ok(out)
}