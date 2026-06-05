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
            let visible =
                cd.hidden ||
                cd.cell_xf.as_ref().is_some_and(|xf| {
                    xf.has_fill_or_border || xf.has_apply_protection
                });

            if visible && last_visible.map_or(true, |last| cd.max > last.max) {
                last_visible = Some(cd);
            }
        }

        last_visible
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
}

#[derive(Debug, Clone)]
pub struct WorkbookSpecificPatterns {
    pub xfs_by_index: HashMap<u32, Xf>,

    // Wird später für manuelle/dynamische Pattern-Logik gebraucht.
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

#[derive(Debug, Clone)]
pub struct CleanSheetParameters {
    pub xlsx_file_path: String,
    pub zip_info_filename: String,
    pub compress_method: zip::CompressionMethod,
    pub patterns: WorkbookSpecificPatterns,
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