use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Xf {
    pub fill_id: Option<u32>,
    pub border_id: Option<u32>,
    pub apply_border: bool,
    pub apply_fill: bool,
    pub has_fill_or_border: bool,
    pub has_apply_fill_or_border: bool,
    pub has_only_font_style: bool,
    pub has_apply_protection: bool,
}

impl Xf {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fill_id: Option<u32>,
        border_id: Option<u32>,
        apply_border: bool,
        apply_fill: bool,
        has_fill_or_border: bool,
        has_apply_fill_or_border: bool,
        has_only_font_style: bool,
        has_apply_protection: bool,
    ) -> Self {
        Self {
            fill_id,
            border_id,
            apply_border,
            apply_fill,
            has_fill_or_border,
            has_apply_fill_or_border,
            has_only_font_style,
            has_apply_protection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellCoordinate {
    pub column: Vec<u8>,
    pub row: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupCause {
    PreprocessEmptySelfClosingCells,
    PreprocessConsecutiveHiddenEmptyRows,
    EmptyCellsWithoutContent,
    EmptyHiddenCells,
    UnreferencedWhitespaceCells,
    UnreferencedHiddenContentCells,
    EmptyRowsWithoutVisibleStyleOrHeight,
    ConsecutiveHiddenEmptyRows,
    TrailingLayoutOnlyRows,
    EmptyRowAttributeCleanup,
    SheetDataRangeTailPruning,
    ExtraneousColumnRangeMerge,
    WorkbookDefinedNameCleanup,
}

impl CleanupCause {
    pub const ALL: [CleanupCause; 13] = [
        CleanupCause::PreprocessEmptySelfClosingCells,
        CleanupCause::PreprocessConsecutiveHiddenEmptyRows,
        CleanupCause::EmptyCellsWithoutContent,
        CleanupCause::EmptyHiddenCells,
        CleanupCause::UnreferencedWhitespaceCells,
        CleanupCause::UnreferencedHiddenContentCells,
        CleanupCause::EmptyRowsWithoutVisibleStyleOrHeight,
        CleanupCause::ConsecutiveHiddenEmptyRows,
        CleanupCause::TrailingLayoutOnlyRows,
        CleanupCause::EmptyRowAttributeCleanup,
        CleanupCause::SheetDataRangeTailPruning,
        CleanupCause::ExtraneousColumnRangeMerge,
        CleanupCause::WorkbookDefinedNameCleanup,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CleanupCause::PreprocessEmptySelfClosingCells => "PreprocessEmptySelfClosingCells",
            CleanupCause::PreprocessConsecutiveHiddenEmptyRows => {
                "PreprocessConsecutiveHiddenEmptyRows"
            }
            CleanupCause::EmptyCellsWithoutContent => "EmptyCellsWithoutContent",
            CleanupCause::EmptyHiddenCells => "EmptyHiddenCells",
            CleanupCause::UnreferencedWhitespaceCells => "UnreferencedWhitespaceCells",
            CleanupCause::UnreferencedHiddenContentCells => "UnreferencedHiddenContentCells",
            CleanupCause::EmptyRowsWithoutVisibleStyleOrHeight => {
                "EmptyRowsWithoutVisibleStyleOrHeight"
            }
            CleanupCause::ConsecutiveHiddenEmptyRows => "ConsecutiveHiddenEmptyRows",
            CleanupCause::TrailingLayoutOnlyRows => "TrailingLayoutOnlyRows",
            CleanupCause::EmptyRowAttributeCleanup => "EmptyRowAttributeCleanup",
            CleanupCause::SheetDataRangeTailPruning => "SheetDataRangeTailPruning",
            CleanupCause::ExtraneousColumnRangeMerge => "ExtraneousColumnRangeMerge",
            CleanupCause::WorkbookDefinedNameCleanup => "WorkbookDefinedNameCleanup",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CleanupSwitches {
    pub preprocess_empty_self_closing_cells: bool,
    pub preprocess_consecutive_hidden_empty_rows: bool,

    pub empty_cells_without_content: bool,
    pub empty_hidden_cells: bool,
    pub unreferenced_whitespace_cells: bool,
    pub unreferenced_hidden_content_cells: bool,

    pub empty_rows_without_visible_style_or_height: bool,
    pub consecutive_hidden_empty_rows: bool,
    pub trailing_layout_only_rows: bool,

    pub empty_row_attribute_cleanup: bool,
    pub sheetdata_range_tail_pruning: bool,
    pub extraneous_column_range_merge: bool,
}

impl Default for CleanupSwitches {
    fn default() -> Self {
        Self {
            preprocess_empty_self_closing_cells: false,
            preprocess_consecutive_hidden_empty_rows: false,

            empty_cells_without_content: true,
            empty_hidden_cells: true,
            unreferenced_whitespace_cells: false,
            unreferenced_hidden_content_cells: false,

            empty_rows_without_visible_style_or_height: true,
            consecutive_hidden_empty_rows: true,
            trailing_layout_only_rows: true,

            empty_row_attribute_cleanup: true,
            sheetdata_range_tail_pruning: true,
            extraneous_column_range_merge: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CleanupStats {
    pub enabled: bool,
    pub removed_bytes_by_cause: HashMap<CleanupCause, usize>,
    pub added_bytes_by_cause: HashMap<CleanupCause, usize>,
    pub removed_items_by_cause: HashMap<CleanupCause, usize>,
}

impl CleanupStats {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    #[inline(always)]
    pub fn record_removed(&mut self, cause: CleanupCause, bytes: usize) {
        if !self.enabled || bytes == 0 {
            return;
        }

        *self.removed_bytes_by_cause.entry(cause).or_default() += bytes;
        *self.removed_items_by_cause.entry(cause).or_default() += 1;
    }

    #[inline(always)]
    pub fn record_delta(&mut self, cause: CleanupCause, before: usize, after: usize) {
        if !self.enabled {
            return;
        }

        if before > after {
            self.record_removed(cause, before - after);
        } else if after > before {
            *self.added_bytes_by_cause.entry(cause).or_default() += after - before;
        }
    }

    pub fn merge(&mut self, other: CleanupStats) {
        self.enabled |= other.enabled;

        for (cause, bytes) in other.removed_bytes_by_cause {
            *self.removed_bytes_by_cause.entry(cause).or_default() += bytes;
        }
        for (cause, bytes) in other.added_bytes_by_cause {
            *self.added_bytes_by_cause.entry(cause).or_default() += bytes;
        }
        for (cause, items) in other.removed_items_by_cause {
            *self.removed_items_by_cause.entry(cause).or_default() += items;
        }
    }
}

#[derive(Debug, Clone)]
pub struct CleanupCandidate {
    pub coordinate: CellCoordinate,
    pub cause: CleanupCause,
}

#[derive(Debug, Clone)]
pub struct ColumnDefinition {
    pub min: u32,
    pub max: u32,
    pub cell_xf: Option<Xf>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ColumnRangeMap {
    exact: HashMap<u32, ColumnDefinition>,
    intervals: Vec<ColumnDefinition>,
    interval_starts: Vec<u32>,
}

impl ColumnRangeMap {
    pub fn add(&mut self, col_def: ColumnDefinition) {
        if col_def.min == col_def.max {
            self.exact.insert(col_def.min, col_def);
        } else {
            self.intervals.push(col_def);
        }
    }

    pub fn finalize(&mut self) {
        self.intervals.sort_by_key(|cd| cd.min);
        self.interval_starts = self.intervals.iter().map(|cd| cd.min).collect();
    }

    pub fn get(&self, col: u32) -> Option<&ColumnDefinition> {
        if let Some(cd) = self.exact.get(&col) {
            return Some(cd);
        }

        let idx = self.interval_starts.partition_point(|start| *start <= col);
        if idx == 0 {
            return None;
        }

        let cd = &self.intervals[idx - 1];
        if cd.min <= col && col <= cd.max {
            Some(cd)
        } else {
            None
        }
    }

    pub fn get_last_visible_range(&self) -> Option<&ColumnDefinition> {
        let mut last_visible: Option<&ColumnDefinition> = None;

        for cd in self.exact.values().chain(self.intervals.iter()) {
            let visible = cd.hidden
                || cd
                    .cell_xf
                    .as_ref()
                    .is_some_and(|xf| xf.has_fill_or_border || xf.has_apply_protection);

            if visible && last_visible.map_or(true, |last| cd.max > last.max) {
                last_visible = Some(cd);
            }
        }

        last_visible
    }

    pub fn has_any_non_default_fill_or_border_style(&self) -> bool {
        self.exact.values().chain(self.intervals.iter()).any(|cd| {
            cd.cell_xf
                .as_ref()
                .is_some_and(|xf| xf.fill_id.unwrap_or(0) != 0 || xf.border_id.unwrap_or(0) != 0)
        })
    }
}

#[derive(Debug, Clone)]
pub struct RowAttributes {
    pub row_attr: Vec<u8>,
    pub ref_: Vec<u8>,
    pub hidden: bool,
    pub style: Option<Xf>,
    pub has_height: bool,
    pub is_empty: bool,
}

#[derive(Debug, Clone)]
pub struct CellAttributes {
    pub cell_attribute_string: Vec<u8>,
    pub cell_coordinate: CellCoordinate,
    pub style: Option<Xf>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanSheetMetrics {
    pub column_definitions: ColumnRangeMap,
    pub kept_row_states: Vec<RowAttributes>,
    pub deleted_row_states: Vec<RowAttributes>,
    pub kept_cell_states: Vec<CellAttributes>,
    pub deleted_cell_states: Vec<CellAttributes>,
    pub row_attributes: HashMap<Vec<u8>, RowAttributes>,
    pub max_merge_cells_column_and_row: Option<CellCoordinate>,

    pub whitespace_candidates: usize,
    pub hidden_candidates: usize,

    pub candidates: Vec<CleanupCandidate>,
    pub cleanup_stats: CleanupStats,
    pub has_hints: bool,
}

#[derive(Debug, Clone)]
pub struct CleanedSheetData {
    pub xml: Vec<u8>,
    pub metrics: CleanSheetMetrics,
}

#[derive(Debug, Clone)]
pub struct WorkbookSpecificPatterns {
    pub xfs_by_index: HashMap<u32, Xf>,

    pub only_font_style_ids: Vec<u32>,
    pub fill_or_border_or_protection_ids: Vec<u32>,
    pub whitespace_indices: Option<Vec<usize>>,
}

impl WorkbookSpecificPatterns {
    pub fn new(xfs: Vec<Xf>, whitespace_indices: Option<Vec<usize>>) -> Self {
        let mut xfs_by_index = std::collections::HashMap::new();
        let mut only_font_style_ids = Vec::new();
        let mut fill_or_border_or_protection_ids = Vec::new();

        for (i, xf) in xfs.into_iter().enumerate() {
            let id = i as u32;
            if xf.has_only_font_style {
                only_font_style_ids.push(id);
            }
            if xf.has_fill_or_border || xf.has_apply_protection {
                fill_or_border_or_protection_ids.push(id);
            }
            xfs_by_index.insert(id, xf);
        }

        Self {
            xfs_by_index,
            only_font_style_ids,
            fill_or_border_or_protection_ids,
            whitespace_indices,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    pub min_col: u32,
    pub max_col: u32,
    pub min_row: u32,
    pub max_row: u32,
}

impl CellRect {
    pub fn contains(&self, col: u32, row: u32) -> bool {
        self.min_col <= col && col <= self.max_col && self.min_row <= row && row <= self.max_row
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProtectedCellRefs {
    pub cells: std::collections::HashSet<u64>,
    pub ranges: Vec<CellRect>,
}

#[derive(Debug, Clone, Default)]
pub struct ProtectedWorkbookRefs {
    pub by_sheet: HashMap<String, ProtectedCellRefs>,
    pub global_or_unknown: ProtectedCellRefs,
}

impl ProtectedWorkbookRefs {
    pub fn contains(&self, sheet: &str, col: u32, row: u32) -> bool {
        self.by_sheet
            .get(sheet)
            .is_some_and(|refs| refs.contains(col, row))
            || self.global_or_unknown.contains(col, row)
    }

    pub fn refs_for_sheet_mut(&mut self, sheet: &str) -> &mut ProtectedCellRefs {
        self.by_sheet.entry(sheet.to_string()).or_default()
    }
}

pub struct SheetScanBuffer<'a> {
    pub sheet_file: &'a str,
    pub sheet_name: &'a str,
    pub xml: &'a [u8],
}

impl ProtectedCellRefs {
    pub fn cell_ref_count(&self) -> usize {
        self.cells.len()
    }

    pub fn range_ref_count(&self) -> usize {
        self.ranges.len()
    }

    pub fn insert_cell(&mut self, col: u32, row: u32) {
        self.cells.insert(encode_coord(col, row));
    }

    pub fn insert_range(&mut self, col1: u32, row1: u32, col2: u32, row2: u32) {
        self.ranges.push(CellRect {
            min_col: col1.min(col2),
            max_col: col1.max(col2),
            min_row: row1.min(row2),
            max_row: row1.max(row2),
        });
    }

    pub fn contains(&self, col: u32, row: u32) -> bool {
        self.cells.contains(&encode_coord(col, row))
            || self.ranges.iter().any(|range| range.contains(col, row))
    }
}

pub fn encode_coord(col: u32, row: u32) -> u64 {
    ((col as u64) << 32) | (row as u64)
}

#[derive(Debug, Clone)]
pub struct AdvancedCleanupConfig {
    pub remove_unreferenced_whitespace: bool,
    pub remove_unreferenced_hidden_content: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SheetPreprocessConfig {
    pub remove_hidden_rows: bool,
    pub remove_empty_cells: bool,
}

#[derive(Debug, Clone)]
pub struct CleanSheetParameters {
    pub xlsx_file_path: String,
    pub zip_info_filename: String,
    pub compress_method: zip::CompressionMethod,
    pub patterns: WorkbookSpecificPatterns,
    pub advanced_cleanup: Option<std::sync::Arc<AdvancedCleanupConfig>>,
    pub preprocess: SheetPreprocessConfig,
    pub cleanup: CleanupSwitches,
    pub collect_cleanup_stats: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetStreamingState {
    SeekWorksheetNode,
    InWorksheetNode,
    InColsNode,
    InSheetDataNode,
    InMergeCellsNode,
    FoundWorksheetEnd,
}
