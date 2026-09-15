use derive_builder::Builder;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Default, PartialEq, Eq, Builder)]
#[builder(default)]
pub struct Xf {
    pub font_id: Option<u32>,
    pub fill_id: Option<u32>,
    pub border_id: Option<u32>,
    pub apply_font: bool,
    pub apply_border: bool,
    pub apply_fill: bool,

    pub has_visible_fill: bool,
    pub has_visible_border: bool,

    pub has_fill_or_border: bool,
    pub has_apply_fill_or_border: bool,
    pub has_no_empty_cell_visible_style: bool,
    pub has_apply_protection: bool,
}

impl Xf {
    pub fn builder() -> XfBuilder {
        XfBuilder::default()
    }

    pub fn applies_visible_fill_or_border(&self) -> bool {
        (self.has_visible_fill && self.apply_fill) || (self.has_visible_border && self.apply_border)
    }

    pub fn applies_default_border_override(&self) -> bool {
        self.apply_border && self.border_id.unwrap_or(0) == 0
    }

    pub fn applies_default_fill_override(&self) -> bool {
        self.apply_fill && self.fill_id.unwrap_or(0) == 0
    }

    pub fn applies_default_fill_or_border_override(&self) -> bool {
        self.applies_default_fill_override() || self.applies_default_border_override()
    }
}

impl XfBuilder {
    pub fn build_xf(&self) -> Xf {
        self.build()
            .expect("XfBuilder has defaults for every field")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellCoordinate {
    pub column: Vec<u8>,
    pub row: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRange {
    pub start_col: u32,
    pub start_row: u32,
    pub end_col: u32,
    pub end_row: u32,
}

impl CellRange {
    #[inline]
    pub fn contains(self, col: u32, row: u32) -> bool {
        self.start_col <= col && col <= self.end_col && self.start_row <= row && row <= self.end_row
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupCause {
    PreprocessEmptySelfClosingCells,
    EmptyCellsWithoutContent,
    EmptyHiddenCells,
    UnreferencedWhitespaceCells,
    UnreferencedHiddenContentCells,
    TrailingLayoutOnlyRows,
    EmptyRowAttributeCleanup,
    SheetDataRangeTailPruning,
    ExtraneousColumnRangeMerge,
    WorkbookDefinedNameCleanup,
}

impl CleanupCause {
    pub const ALL: [CleanupCause; 10] = [
        CleanupCause::PreprocessEmptySelfClosingCells,
        CleanupCause::EmptyCellsWithoutContent,
        CleanupCause::EmptyHiddenCells,
        CleanupCause::UnreferencedWhitespaceCells,
        CleanupCause::UnreferencedHiddenContentCells,
        CleanupCause::TrailingLayoutOnlyRows,
        CleanupCause::EmptyRowAttributeCleanup,
        CleanupCause::SheetDataRangeTailPruning,
        CleanupCause::ExtraneousColumnRangeMerge,
        CleanupCause::WorkbookDefinedNameCleanup,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CleanupCause::PreprocessEmptySelfClosingCells => "PreprocessEmptySelfClosingCells",
            CleanupCause::EmptyCellsWithoutContent => "EmptyCellsWithoutContent",
            CleanupCause::EmptyHiddenCells => "EmptyHiddenCells",
            CleanupCause::UnreferencedWhitespaceCells => "UnreferencedWhitespaceCells",
            CleanupCause::UnreferencedHiddenContentCells => "UnreferencedHiddenContentCells",
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

    pub empty_cells_without_content: bool,
    pub empty_hidden_cells: bool,
    pub unreferenced_whitespace_cells: bool,
    pub unreferenced_hidden_content_cells: bool,

    pub trailing_layout_only_rows: bool,
    pub keep_textless_default_font_representatives: bool,
    pub keep_textless_style_representatives: bool,
    pub keep_textless_column_style_representatives: bool,
    pub inactive_protection_cell_policy: InactiveProtectionCellPolicy,

    pub empty_row_attribute_cleanup: bool,
    pub sheetdata_range_tail_pruning: bool,
    pub extraneous_column_range_merge: bool,
    pub column_tail_fill_merge: bool,
}

impl Default for CleanupSwitches {
    fn default() -> Self {
        Self {
            preprocess_empty_self_closing_cells: false,

            empty_cells_without_content: true,
            empty_hidden_cells: true,
            unreferenced_whitespace_cells: false,
            unreferenced_hidden_content_cells: false,

            trailing_layout_only_rows: true,
            keep_textless_default_font_representatives: true,
            keep_textless_style_representatives: true,
            keep_textless_column_style_representatives: true,
            inactive_protection_cell_policy: InactiveProtectionCellPolicy::Remove,

            empty_row_attribute_cleanup: true,
            sheetdata_range_tail_pruning: true,
            extraneous_column_range_merge: true,
            column_tail_fill_merge: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InactiveProtectionCellPolicy {
    Remove,
    KeepInDataRange,
    KeepAll,
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
    pub col: u32,
    pub row: u32,
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

    #[inline]
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

    pub fn get_last_visible_range(&self, include_protection: bool) -> Option<&ColumnDefinition> {
        let mut last_visible: Option<&ColumnDefinition> = None;

        for cd in self.exact.values().chain(self.intervals.iter()) {
            let visible = cd.hidden
                || cd.cell_xf.as_ref().is_some_and(|xf| {
                    xf.has_fill_or_border || (include_protection && xf.has_apply_protection)
                });

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
                .is_some_and(Xf::applies_visible_fill_or_border)
        })
    }
}

#[derive(Debug, Clone)]
pub struct CellAttributes {
    pub col: u32,
    pub row: u32,
    pub kept_only_for_protection: bool,
    pub is_tail_anchor: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VisibleContentBounds {
    pub max_col: Option<u32>,
    pub max_row: Option<u32>,
}

impl VisibleContentBounds {
    #[inline]
    pub fn include_cell(&mut self, col: u32, row: u32) {
        self.max_col = Some(self.max_col.map_or(col, |old| old.max(col)));
        self.max_row = Some(self.max_row.map_or(row, |old| old.max(row)));
    }

    #[inline]
    pub fn include_col(&mut self, col: u32) {
        self.max_col = Some(self.max_col.map_or(col, |old| old.max(col)));
    }

    #[inline]
    pub fn include_row(&mut self, row: u32) {
        self.max_row = Some(self.max_row.map_or(row, |old| old.max(row)));
    }

    #[inline]
    pub fn cell_is_outside(&self, col: u32, row: u32) -> bool {
        self.max_col.map_or(true, |max| col > max) || self.max_row.map_or(true, |max| row > max)
    }

    #[inline]
    pub fn row_is_below(&self, row: u32) -> bool {
        self.max_row.map_or(true, |max| row > max)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CleanSheetMetrics {
    pub column_definitions: ColumnRangeMap,
    pub kept_cell_states: Vec<CellAttributes>,
    pub deleted_cell_states: Vec<CellAttributes>,
    pub max_merge_cells_column_and_row: Option<CellCoordinate>,
    pub merge_cell_ranges: Vec<CellRange>,

    pub whitespace_candidates: usize,
    pub hidden_candidates: usize,

    pub candidates: Vec<CleanupCandidate>,
    pub cleanup_stats: CleanupStats,
    pub sheet_protection_enabled: bool,
    pub visible_content_bounds: VisibleContentBounds,
    pub visible_data_range_bounds: VisibleContentBounds,
    pub tail_anchor_rows: BTreeMap<u32, usize>,
    pub tail_anchor_max_row: Option<u32>,
    pub has_hints: bool,
}

impl CleanSheetMetrics {
    pub fn record_tail_anchor_row(&mut self, row: u32) {
        *self.tail_anchor_rows.entry(row).or_default() += 1;
        self.tail_anchor_max_row = Some(self.tail_anchor_max_row.map_or(row, |old| old.max(row)));
    }

    pub fn remove_tail_anchor_row(&mut self, row: u32) {
        let mut remove_entry = false;
        if let Some(count) = self.tail_anchor_rows.get_mut(&row) {
            if *count > 1 {
                *count -= 1;
            } else {
                remove_entry = true;
            }
        }

        if remove_entry {
            self.tail_anchor_rows.remove(&row);
        }

        if self.tail_anchor_max_row == Some(row) && !self.tail_anchor_rows.contains_key(&row) {
            self.refresh_tail_anchor_max_row();
        }
    }

    pub fn remove_all_tail_anchors_for_row(&mut self, row: u32) {
        let removed = self.tail_anchor_rows.remove(&row).is_some();
        if removed || self.tail_anchor_max_row == Some(row) {
            self.refresh_tail_anchor_max_row();
        }
    }

    pub fn clear_tail_anchor_rows(&mut self) {
        self.tail_anchor_rows.clear();
        self.tail_anchor_max_row = None;
    }

    pub fn merge_tail_anchor_rows(&mut self, rows: BTreeMap<u32, usize>) {
        for (row, count) in rows {
            if count == 0 {
                continue;
            }
            *self.tail_anchor_rows.entry(row).or_default() += count;
            self.tail_anchor_max_row =
                Some(self.tail_anchor_max_row.map_or(row, |old| old.max(row)));
        }
    }

    fn refresh_tail_anchor_max_row(&mut self) {
        self.tail_anchor_max_row = self.tail_anchor_rows.keys().next_back().copied();
    }
}

#[derive(Debug, Clone)]
pub struct CleanedSheetData {
    pub xml: Vec<u8>,
    pub sheet_data_range: std::ops::Range<usize>,
    pub metrics: CleanSheetMetrics,
}

#[derive(Debug, Clone)]
pub struct WorkbookSpecificPatterns {
    pub xfs_by_index: HashMap<u32, Xf>,

    pub no_empty_cell_visible_style_ids: Vec<u32>,
    pub fill_or_border_or_protection_ids: Vec<u32>,
    pub whitespace_indices: Option<Vec<usize>>,
}

impl WorkbookSpecificPatterns {
    pub fn new(xfs: Vec<Xf>, whitespace_indices: Option<Vec<usize>>) -> Self {
        let mut xfs_by_index = std::collections::HashMap::new();
        let mut no_empty_cell_visible_style_ids = Vec::new();
        let mut fill_or_border_or_protection_ids = Vec::new();

        for (i, xf) in xfs.into_iter().enumerate() {
            let id = i as u32;
            if xf.has_no_empty_cell_visible_style {
                no_empty_cell_visible_style_ids.push(id);
            }
            if xf.has_fill_or_border
                || xf.applies_default_fill_or_border_override()
                || xf.has_apply_protection
            {
                fill_or_border_or_protection_ids.push(id);
            }
            xfs_by_index.insert(id, xf);
        }

        Self {
            xfs_by_index,
            no_empty_cell_visible_style_ids,
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
    #[inline]
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
    #[inline]
    pub fn contains(&self, sheet: &str, col: u32, row: u32) -> bool {
        self.by_sheet
            .get(sheet)
            .is_some_and(|refs| refs.contains(col, row))
            || self.global_or_unknown.contains(col, row)
    }

    #[inline]
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

    #[inline]
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

    #[inline]
    pub fn contains(&self, col: u32, row: u32) -> bool {
        self.cells.contains(&encode_coord(col, row))
            || self.ranges.iter().any(|range| range.contains(col, row))
    }
}

#[inline]
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
