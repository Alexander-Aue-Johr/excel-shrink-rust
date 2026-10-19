#[cfg(feature = "cell-tracing")]
use crate::cleaning_trace::{CellDecision, CellDecisionEvent};
use crate::models::{
    CellAttributes, CleanSheetMetrics, CleanSheetParameters, CleanupCandidate, CleanupCause,
    ColumnDefinition, InactiveProtectionCellPolicy, Xf,
};
use crate::sheet::row_view::RowView;
use crate::utils::{digits_byte_string_to_integer, letters_byte_string_to_integer};
use regex::bytes::Captures;
use std::borrow::Cow;

#[cfg(feature = "cell-tracing")]
macro_rules! trace_cell_decision {
    ($event:expr) => {
        crate::cleaning_trace::record($event);
    };
}

#[cfg(not(feature = "cell-tracing"))]
macro_rules! trace_cell_decision {
    ($event:expr) => {};
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParsedCell<'a, 'r> {
    pub(crate) row_view: Option<&'r RowView<'r, 'r>>,
    pub(crate) column: Option<Option<&'r ColumnDefinition>>,
    pub(crate) raw: &'a [u8],
    pub(crate) attr: &'a [u8],
    pub(crate) reference_attr: &'a [u8],
    pub(crate) row: &'a [u8],
    pub(crate) col_num: u32,
    pub(crate) style_id: Option<u32>,
}

impl<'a, 'r> ParsedCell<'a, 'r> {
    fn from_captures(caps: &Captures<'a>) -> Option<Self> {
        let raw = caps.get(0)?.as_bytes();
        let attr = caps.get(1)?.as_bytes();
        let reference_attr = extract_attr_assignment(attr, b"r")?;
        let column = caps.get(2)?.as_bytes();
        let row = caps.get(3)?.as_bytes();

        let col_num = letters_byte_string_to_integer(column);
        let style_id = caps
            .get(4)
            .map(|m| digits_byte_string_to_integer(m.as_bytes()));

        Some(Self {
            column: None,
            row_view: None,
            raw,
            attr,
            reference_attr,
            row,
            col_num,
            style_id,
        })
    }

    #[inline]
    fn row_view(&self) -> Option<&RowView<'_, '_>> {
        self.row_view.filter(|row| row.reference == self.row)
    }

    #[inline]
    fn column<'s>(&'s self, metrics: &'s CleanSheetMetrics) -> Option<&'s ColumnDefinition> {
        self.column
            .unwrap_or_else(|| metrics.column_definitions.get(self.col_num))
    }

    #[inline]
    fn style<'p>(&self, params: &'p CleanSheetParameters) -> Option<&'p Xf> {
        self.style_id
            .and_then(|style_id| params.patterns.xfs_by_index.get(&style_id))
    }

    fn to_cell_attributes_with_protection_only(
        &self,
        kept_only_for_protection: bool,
    ) -> CellAttributes {
        CellAttributes {
            col: self.col_num,
            row: digits_byte_string_to_integer(self.row),
            kept_only_for_protection,
            is_tail_anchor: false,
        }
    }

    fn to_cleanup_candidate(&self, cause: CleanupCause) -> CleanupCandidate {
        CleanupCandidate {
            col: self.col_num,
            row: digits_byte_string_to_integer(self.row),
            cause,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct EmptyCellStylePolicy {
    keep_no_visible_style: bool,
    keep_other: bool,
    keep_default: bool,
    keep_column_font_override: bool,
}

impl EmptyCellStylePolicy {
    #[inline]
    fn keeps_reference_sensitive_candidate(self) -> bool {
        self.keep_no_visible_style
            || self.keep_other
            || self.keep_default
            || self.keep_column_font_override
    }
}

#[derive(Debug, Clone, Copy)]
struct ReferenceSensitiveCleanup {
    whitespace: bool,
    hidden_content: bool,
}

impl ReferenceSensitiveCleanup {
    #[inline]
    fn from_params(params: &CleanSheetParameters) -> Self {
        Self {
            whitespace: unreferenced_whitespace_cleanup_enabled(params),
            hidden_content: unreferenced_hidden_content_cleanup_enabled(params),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct InheritedCellStyles<'a> {
    row: Option<&'a Xf>,
    column: Option<&'a Xf>,
}

pub fn replace_cell(
    caps: &Captures,
    row_view: Option<&RowView<'_, '_>>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Vec<u8> {
    let Some(mut cell) = ParsedCell::from_captures(caps) else {
        return caps.get(0).unwrap().as_bytes().to_vec();
    };

    cell.row_view = row_view;
    let column = metrics.column_definitions.get(cell.col_num).cloned();
    cell.column = Some(column.as_ref());
    replace_parsed_cell(&cell, params, metrics).into_owned()
}

pub(crate) fn replace_parsed_cell<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    replace_parsed_cell_with_empty_style_policy(
        cell,
        params,
        metrics,
        parsed_cell_has_content(cell),
        false,
        false,
        false,
    )
}

pub(crate) fn replace_parsed_cell_with_empty_style_policy<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    has_cell_content: bool,
    keep_no_empty_cell_visible_style_cell: bool,
    keep_other_empty_cell: bool,
    keep_default_empty_cell: bool,
) -> Cow<'a, [u8]> {
    let style = cell.style(params);
    let is_hidden = cell_is_hidden(cell, metrics);
    record_visible_cell_bounds(cell, style, metrics, has_cell_content);
    let style_policy = EmptyCellStylePolicy {
        keep_no_visible_style: keep_no_empty_cell_visible_style_cell,
        keep_other: keep_other_empty_cell,
        keep_default: keep_default_empty_cell,
        keep_column_font_override: parsed_empty_cell_has_column_font_style_override(
            cell,
            params,
            metrics,
            has_cell_content,
        ),
    };

    if let Some(candidate) = keep_reference_sensitive_candidate(
        cell,
        params,
        metrics,
        style,
        style_policy,
        is_hidden,
        has_cell_content,
    ) {
        return candidate;
    }

    if is_hidden {
        return clean_hidden_cell(cell, params, metrics, style, style_policy, has_cell_content);
    }

    clean_visible_cell(cell, params, metrics, style, style_policy, has_cell_content)
}

fn keep_reference_sensitive_candidate<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style: Option<&Xf>,
    style_policy: EmptyCellStylePolicy,
    is_hidden: bool,
    has_cell_content: bool,
) -> Option<Cow<'a, [u8]>> {
    let cause = reference_sensitive_candidate_cause(
        cell,
        params,
        metrics,
        ReferenceSensitiveCleanup::from_params(params),
        style_policy,
        is_hidden,
        has_cell_content,
    )?;
    let is_excel_protected = cell_has_relevant_protection_style(cell, style, params, metrics);

    if !is_excel_protected {
        metrics.candidates.push(cell.to_cleanup_candidate(cause));
    }
    Some(keep_cell_unchanged(
        cell,
        params,
        metrics,
        style,
        has_cell_content,
        is_excel_protected,
    ))
}

fn reference_sensitive_candidate_cause(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    cleanup: ReferenceSensitiveCleanup,
    style_policy: EmptyCellStylePolicy,
    is_hidden: bool,
    has_cell_content: bool,
) -> Option<CleanupCause> {
    if style_policy.keeps_reference_sensitive_candidate() {
        return None;
    }

    // Hidden columns still participate in Excel's automatic row sizing. Keep
    // their content (including whitespace) unless the row is itself hidden or
    // has an explicit fixed height. Cached ht alone does not fix the height.
    // Use the already parsed row/column metadata; no text/style decoding needed.
    if is_hidden
        && has_cell_content
        && cell.row_view().is_none_or(|row| {
            !row.hidden && !(row.has_height && row.has_custom_height)
        })
    {
        return None;
    }

    if cleanup.whitespace && is_whitespace_cell(cell.raw, params) {
        metrics.whitespace_candidates += 1;
        return Some(CleanupCause::UnreferencedWhitespaceCells);
    }

    if cleanup.hidden_content && is_hidden && has_cell_content && !has_formula(cell.raw) {
        metrics.hidden_candidates += 1;
        return Some(CleanupCause::UnreferencedHiddenContentCells);
    }

    None
}

fn clean_hidden_cell<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style: Option<&Xf>,
    style_policy: EmptyCellStylePolicy,
    has_cell_content: bool,
) -> Cow<'a, [u8]> {
    if has_cell_content {
        return keep_cell_unchanged(cell, params, metrics, style, true, false);
    }

    if empty_hidden_cell_is_style_representative(cell, style, params, metrics, style_policy) {
        let kept_only_for_protection =
            hidden_style_representative_is_protection_only(cell, style, params, metrics);
        return keep_empty_cell_as_self_closing(
            cell,
            params,
            metrics,
            style,
            kept_only_for_protection,
        );
    }

    if params.cleanup.empty_hidden_cells {
        return remove_hidden_empty_cell(cell, metrics);
    }

    keep_cell_unchanged(cell, params, metrics, style, false, false)
}

fn hidden_style_representative_is_protection_only(
    cell: &ParsedCell<'_, '_>,
    style: Option<&Xf>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> bool {
    style.is_some_and(|style| {
        cell_has_relevant_protection_style(cell, Some(style), params, metrics)
            && !cell_has_effective_visual_style(style, cell, metrics)
            && !empty_cell_has_column_font_style_override(cell, style, params, metrics, false)
    })
}

fn remove_hidden_empty_cell<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    metrics: &mut CleanSheetMetrics,
) -> Cow<'a, [u8]> {
    metrics
        .cleanup_stats
        .record_removed(CleanupCause::EmptyHiddenCells, cell.raw.len());
    metrics.deleted_cell_count += 1;
    Cow::Owned(Vec::new())
}

fn clean_visible_cell<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style: Option<&Xf>,
    style_policy: EmptyCellStylePolicy,
    has_cell_content: bool,
) -> Cow<'a, [u8]> {
    match style {
        None => clean_visible_unstyled_cell(cell, params, metrics, style_policy, has_cell_content),
        Some(style) if has_cell_content => {
            keep_cell_unchanged(cell, params, metrics, Some(style), true, false)
        }
        Some(style) => clean_visible_styled_empty_cell(cell, params, metrics, style, style_policy),
    }
}

fn clean_visible_unstyled_cell<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style_policy: EmptyCellStylePolicy,
    has_cell_content: bool,
) -> Cow<'a, [u8]> {
    if has_cell_content {
        return keep_cell_unchanged(cell, params, metrics, None, true, false);
    }

    if style_policy.keep_default || style_policy.keep_other {
        return keep_empty_cell_as_self_closing(cell, params, metrics, None, false);
    }

    remove_or_keep_empty_cell(
        cell,
        params,
        metrics,
        CleanupCause::EmptyCellsWithoutContent,
        None,
    )
}

fn clean_visible_styled_empty_cell<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style: &Xf,
    style_policy: EmptyCellStylePolicy,
) -> Cow<'a, [u8]> {
    let has_effective_fill_or_border = cell_has_effective_visual_style(style, cell, metrics);

    if style.has_no_empty_cell_visible_style {
        return clean_empty_cell_with_nonvisible_style(
            cell,
            params,
            metrics,
            style,
            style_policy,
            has_effective_fill_or_border,
        );
    }

    if style_policy.keep_other || style_policy.keep_column_font_override {
        return keep_empty_cell_as_self_closing(cell, params, metrics, Some(style), false);
    }

    let has_relevant_protection =
        cell_has_relevant_protection_style(cell, Some(style), params, metrics);
    if has_relevant_protection || has_effective_fill_or_border {
        return keep_empty_cell_as_self_closing(
            cell,
            params,
            metrics,
            Some(style),
            has_relevant_protection && !has_effective_fill_or_border,
        );
    }

    remove_or_keep_empty_cell(
        cell,
        params,
        metrics,
        CleanupCause::EmptyCellsWithoutContent,
        Some(style),
    )
}

fn clean_empty_cell_with_nonvisible_style<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style: &Xf,
    style_policy: EmptyCellStylePolicy,
    has_effective_fill_or_border: bool,
) -> Cow<'a, [u8]> {
    if style_policy.keep_no_visible_style
        || style_policy.keep_other
        || style_policy.keep_column_font_override
        || has_effective_fill_or_border
    {
        return keep_empty_cell_as_self_closing(cell, params, metrics, Some(style), false);
    }

    remove_or_keep_empty_cell(
        cell,
        params,
        metrics,
        CleanupCause::EmptyCellsWithoutContent,
        Some(style),
    )
}

fn keep_cell_unchanged<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style: Option<&Xf>,
    has_cell_content: bool,
    kept_only_for_protection: bool,
) -> Cow<'a, [u8]> {
    record_kept_cell(
        params,
        metrics,
        cell,
        style,
        has_cell_content,
        kept_only_for_protection,
    );
    Cow::Borrowed(cell.raw)
}

fn keep_empty_cell_as_self_closing<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    style: Option<&Xf>,
    kept_only_for_protection: bool,
) -> Cow<'a, [u8]> {
    let output = if cell.raw.ends_with(b"/>") {
        Cow::Borrowed(cell.raw)
    } else {
        Cow::Owned(generate_self_closing_cell_xml(cell.attr))
    };
    record_kept_cell(
        params,
        metrics,
        cell,
        style,
        false,
        kept_only_for_protection,
    );
    output
}

pub(crate) fn parsed_empty_cell_has_effective_fill_or_border(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    if has_cell_content {
        return false;
    }

    cell.style(params)
        .is_some_and(|style| cell_has_effective_visual_style(style, cell, metrics))
}

fn record_kept_cell(
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    cell: &ParsedCell<'_, '_>,
    cell_style: Option<&Xf>,
    has_cell_content: bool,
    kept_only_for_protection: bool,
) {
    let row = digits_byte_string_to_integer(cell.row);
    let is_tail_anchor = kept_cell_is_tail_anchor(
        params,
        metrics,
        cell,
        cell_style,
        has_cell_content,
        kept_only_for_protection,
    );

    let mut attributes = cell.to_cell_attributes_with_protection_only(kept_only_for_protection);
    attributes.is_tail_anchor = is_tail_anchor;
    metrics.kept_cell_states.push(attributes);

    if is_tail_anchor {
        metrics.record_tail_anchor_row(row);
    }
}

fn kept_cell_is_tail_anchor(
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    cell: &ParsedCell<'_, '_>,
    cell_style: Option<&Xf>,
    has_cell_content: bool,
    kept_only_for_protection: bool,
) -> bool {
    if has_cell_content {
        return true;
    }

    if cell_style.is_none()
        && cell.style_id.is_none()
        && params.cleanup.keep_textless_default_font_representatives
    {
        return true;
    }

    if cell_style.is_none() && cell.style_id.is_some() {
        return true;
    }

    let column_style = cell
        .column(metrics)
        .and_then(|column| column.cell_xf.as_ref());
    if column_style.is_some_and(|style| style.has_fill_or_border) {
        return true;
    }

    let has_visible_style_anchor =
        cell_style.is_some_and(|style| cell_has_effective_visual_style(style, cell, metrics));
    if has_visible_style_anchor {
        return true;
    }

    if kept_only_for_protection {
        return !metrics.sheet_protection_enabled
            && params.cleanup.inactive_protection_cell_policy
                == InactiveProtectionCellPolicy::KeepAll;
    }

    if parsed_empty_cell_has_column_font_style_override(cell, params, metrics, false) {
        return false;
    }

    false
}

#[inline]
pub(crate) fn parsed_cell_has_content(cell: &ParsedCell<'_, '_>) -> bool {
    has_content(cell.raw)
}

fn remove_or_keep_empty_cell<'a, 'r>(
    cell: &ParsedCell<'a, 'r>,
    params: &CleanSheetParameters,
    metrics: &mut CleanSheetMetrics,
    cause: CleanupCause,
    cell_style: Option<&Xf>,
) -> Cow<'a, [u8]> {
    if params.cleanup.empty_cells_without_content {
        metrics.cleanup_stats.record_removed(cause, cell.raw.len());
        metrics.deleted_cell_count += 1;
        trace_cell_decision!(CellDecisionEvent {
            sheet_file: &params.zip_info_filename,
            col: cell.col_num,
            row: cell.row,
            style_id: cell.style_id,
            decision: CellDecision::Removed,
            cause,
        });
        Cow::Owned(Vec::new())
    } else {
        record_kept_cell(params, metrics, cell, cell_style, false, false);
        trace_cell_decision!(CellDecisionEvent {
            sheet_file: &params.zip_info_filename,
            col: cell.col_num,
            row: cell.row,
            style_id: cell.style_id,
            decision: CellDecision::Kept,
            cause,
        });
        Cow::Borrowed(cell.raw)
    }
}

fn extract_attr_assignment<'a>(data: &'a [u8], attr_name: &[u8]) -> Option<&'a [u8]> {
    let mut search_start = 0;

    while search_start < data.len() {
        let relative = memchr::memmem::find(&data[search_start..], attr_name)?;
        let attr_start = search_start + relative;
        let value_marker = attr_start + attr_name.len();

        if attr_start > 0
            && data[attr_start - 1].is_ascii_whitespace()
            && data.get(value_marker..value_marker + 2) == Some(b"=\"")
        {
            let value_start = value_marker + 2;
            let value_end = data[value_start..].iter().position(|byte| *byte == b'"')?;
            return Some(&data[attr_start..value_start + value_end + 1]);
        }

        search_start = attr_start + attr_name.len();
    }

    None
}

fn empty_hidden_cell_is_style_representative(
    cell: &ParsedCell<'_, '_>,
    cell_style: Option<&Xf>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    style_policy: EmptyCellStylePolicy,
) -> bool {
    if style_policy.keep_default && cell.style_id.is_none() {
        return true;
    }

    let Some(style) = cell_style else {
        return false;
    };

    if style_policy.keep_other {
        return true;
    }

    if style_policy.keep_no_visible_style {
        return true;
    }

    cell_has_relevant_protection_style(cell, Some(style), params, metrics)
        || cell_has_effective_visual_style(style, cell, metrics)
}

#[inline]
fn unreferenced_whitespace_cleanup_enabled(params: &CleanSheetParameters) -> bool {
    params.cleanup.unreferenced_whitespace_cells
        || params
            .advanced_cleanup
            .as_ref()
            .is_some_and(|advanced| advanced.remove_unreferenced_whitespace)
}

#[inline]
fn unreferenced_hidden_content_cleanup_enabled(params: &CleanSheetParameters) -> bool {
    params.cleanup.unreferenced_hidden_content_cells
        || params
            .advanced_cleanup
            .as_ref()
            .is_some_and(|advanced| advanced.remove_unreferenced_hidden_content)
}

#[inline]
fn cell_is_hidden(cell: &ParsedCell<'_, '_>, metrics: &CleanSheetMetrics) -> bool {
    if cell.row_view().is_some_and(|row| row.hidden) {
        return true;
    }

    cell.column(metrics).is_some_and(|column| column.hidden)
}

fn cell_has_effective_visual_style(
    cell_style: &Xf,
    cell: &ParsedCell<'_, '_>,
    metrics: &CleanSheetMetrics,
) -> bool {
    if cell
        .row_view()
        .is_some_and(|r| r.promoted_style_id.is_some() && r.promoted_style_id == cell.style_id)
    {
        // Exact full XF moved from a dense row's cells to its row header.
        return false;
    }
    let inherited = inherited_cell_styles(cell, metrics);

    cell_has_effective_fill(cell_style, inherited.row, inherited.column)
        || cell_has_effective_border(cell_style, inherited.row, inherited.column)
        || cell_applies_effective_default_fill_override(cell_style, inherited)
        || cell_applies_effective_default_border_override(cell_style, inherited)
}

fn cell_has_effective_fill_or_border(
    cell_style: &Xf,
    cell: &ParsedCell<'_, '_>,
    metrics: &CleanSheetMetrics,
) -> bool {
    let inherited = inherited_cell_styles(cell, metrics);
    cell_has_effective_fill(cell_style, inherited.row, inherited.column)
        || cell_has_effective_border(cell_style, inherited.row, inherited.column)
}

fn inherited_cell_styles<'a>(
    cell: &'a ParsedCell<'_, '_>,
    metrics: &'a CleanSheetMetrics,
) -> InheritedCellStyles<'a> {
    let column = cell
        .column(metrics)
        .and_then(|column| column.cell_xf.as_ref());
    let row = cell.row_view().and_then(|row| row.style);

    InheritedCellStyles { row, column }
}

fn record_visible_cell_bounds(
    cell: &ParsedCell<'_, '_>,
    cell_style: Option<&Xf>,
    metrics: &mut CleanSheetMetrics,
    has_cell_content: bool,
) {
    let Some(row_num) = crate::utils::parse_unsigned_bytes::<u32>(cell.row) else {
        return;
    };

    let column_style = cell
        .column(metrics)
        .and_then(|column| column.cell_xf.as_ref());

    let is_visible = has_cell_content
        || cell_style.is_some_and(|style| cell_has_effective_fill_or_border(style, cell, metrics))
        || column_style.is_some_and(|style| style.has_fill_or_border);

    if is_visible {
        metrics
            .visible_content_bounds
            .include_cell(cell.col_num, row_num);
    }
}

fn cell_has_effective_fill(
    cell_style: &Xf,
    row_style: Option<&Xf>,
    column_style: Option<&Xf>,
) -> bool {
    let cell_fill_id = cell_style.fill_id.unwrap_or(0);

    match (
        row_style.filter(|style| style.apply_fill || style.has_visible_fill),
        column_style.filter(|style| style.apply_fill),
    ) {
        (Some(row), Some(column)) => {
            cell_fill_id != row.fill_id.unwrap_or(0) || cell_fill_id != column.fill_id.unwrap_or(0)
        }
        (Some(row), None) => cell_fill_id != row.fill_id.unwrap_or(0),
        (None, Some(column)) => cell_fill_id != column.fill_id.unwrap_or(0),
        (None, None) => cell_style.has_visible_fill,
    }
}

fn cell_has_effective_border(
    cell_style: &Xf,
    row_style: Option<&Xf>,
    column_style: Option<&Xf>,
) -> bool {
    let cell_border_id = cell_style.border_id.unwrap_or(0);

    match (
        row_style.filter(|style| style.apply_border),
        column_style.filter(|style| style.apply_border),
    ) {
        (Some(row), Some(column)) => {
            cell_border_id != row.border_id.unwrap_or(0)
                || cell_border_id != column.border_id.unwrap_or(0)
        }
        (Some(row), None) => cell_border_id != row.border_id.unwrap_or(0),
        (None, Some(column)) => cell_border_id != column.border_id.unwrap_or(0),
        (None, None) => cell_style.has_visible_border,
    }
}

fn empty_cell_has_column_font_style_override(
    cell: &ParsedCell<'_, '_>,
    cell_style: &Xf,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    if has_cell_content || !params.cleanup.keep_textless_column_style_representatives {
        return false;
    }

    let row_attributes = cell.row_view();
    // Fixed height does not freeze the text baseline. A promoted row font,
    // however, supplies the same override without one empty cell per column.
    if let Some(row) = row_attributes.filter(|r| r.promoted_style_id.is_some()) {
        return cell_style.font_identity() != row.style.map(Xf::font_identity).unwrap_or(0);
    }
    if empty_cell_overrides_row_font(cell, params, has_cell_content) {
        return true;
    }
    let cell_font_id = cell_style.font_id.unwrap_or(0);
    let row_uses_auto_height =
        row_attributes.is_some_and(|row| row.has_height && !row.has_custom_height);

    if cell_font_id == 0 && !cell_style.apply_font && !row_uses_auto_height {
        return false;
    }

    let Some(column_style) = cell
        .column(metrics)
        .and_then(|column| column.cell_xf.as_ref())
    else {
        return false;
    };

    let column_font = column_style.font_identity();

    if row_uses_auto_height
        && row_attributes
            .and_then(|row| row.style)
            .is_some_and(|row_style| {
                cell_style
                    .font_id
                    .map_or(column_font, |_| cell_style.font_identity())
                    != row_style.font_identity()
            })
    {
        return true;
    }

    cell_style.font_identity() != column_font
}

pub(super) fn empty_cell_overrides_row_font(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    has_cell_content: bool,
) -> bool {
    !has_cell_content
        && params.cleanup.keep_textless_column_style_representatives
        && cell.row_view().and_then(|row| row.style)
            .filter(|row_style| row_style.apply_font)
            .zip(cell.style(params))
            .is_some_and(|(row_style, cell_style)| {
                row_style.font_identity() != cell_style.font_identity()
            })
}

fn parsed_empty_cell_has_column_font_style_override(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
    has_cell_content: bool,
) -> bool {
    cell.style(params).is_some_and(|style| {
        empty_cell_has_column_font_style_override(cell, style, params, metrics, has_cell_content)
    })
}

fn cell_applies_effective_default_fill_override(
    cell_style: &Xf,
    inherited: InheritedCellStyles<'_>,
) -> bool {
    if !cell_style.applies_default_fill_override() {
        return false;
    }

    match (inherited.row, inherited.column) {
        (None, None) => true,
        (row, column) => {
            row.is_some_and(|style| style.apply_fill && style.has_visible_fill)
                || column.is_some_and(|style| style.apply_fill && style.has_visible_fill)
        }
    }
}

fn cell_applies_effective_default_border_override(
    cell_style: &Xf,
    inherited: InheritedCellStyles<'_>,
) -> bool {
    if !cell_style.applies_default_border_override() {
        return false;
    }

    match (inherited.row, inherited.column) {
        (None, None) => true,
        (row, column) => {
            row.is_some_and(|style| style.apply_border && style.has_visible_border)
                || column.is_some_and(|style| style.apply_border && style.has_visible_border)
        }
    }
}

fn generate_self_closing_cell_xml(cell_attr: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cell_attr.len() + 4);
    out.extend_from_slice(b"<c");
    out.extend_from_slice(cell_attr);
    out.extend_from_slice(b"/>");
    out
}

pub(crate) fn has_content(cell: &[u8]) -> bool {
    crate::xml::find_bytes(cell, b"<v>").is_some()
        || has_formula(cell)
        || crate::xml::find_bytes(cell, b"<is>").is_some()
}

fn has_formula(cell: &[u8]) -> bool {
    has_child_tag(cell, b"<f")
}

fn has_child_tag(cell: &[u8], needle: &[u8]) -> bool {
    let mut pos = 0;
    while let Some(rel) = crate::xml::find_bytes(&cell[pos..], needle) {
        let start = pos + rel;
        let next = cell.get(start + needle.len()).copied();
        if matches!(next, Some(b' ' | b'/' | b'>')) {
            return true;
        }
        pos = start + needle.len();
    }

    false
}

fn is_whitespace_cell(cell: &[u8], params: &CleanSheetParameters) -> bool {
    let Some(indices) = &params.patterns.whitespace_indices else {
        return false;
    };

    shared_string_index(cell).is_some_and(|index| indices.contains(&index))
}

fn shared_string_index(cell: &[u8]) -> Option<usize> {
    if !cell
        .windows(b"t=\"s\"".len())
        .any(|window| window == b"t=\"s\"")
    {
        return None;
    }

    let value_tag = crate::xml::find_bytes(cell, b"<v>")?;
    let value_start = value_tag + b"<v>".len();
    let value_length = crate::xml::find_bytes(&cell[value_start..], b"</v>")?;
    crate::utils::parse_unsigned_bytes(&cell[value_start..value_start + value_length])
}

fn cell_has_protection_style(
    cell: &ParsedCell<'_, '_>,
    cell_style: Option<&Xf>,
    metrics: &CleanSheetMetrics,
) -> bool {
    if cell_style.is_some_and(|style| style.has_apply_protection) {
        return true;
    }

    if cell
        .row_view()
        .and_then(|row| row.style)
        .is_some_and(|style| style.has_apply_protection)
    {
        return true;
    }

    cell.column(metrics)
        .and_then(|column| column.cell_xf.as_ref())
        .is_some_and(|style| style.has_apply_protection)
}

fn cell_has_relevant_protection_style(
    cell: &ParsedCell<'_, '_>,
    cell_style: Option<&Xf>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> bool {
    if !metrics.sheet_protection_enabled
        && !inactive_protection_policy_keeps_cell(cell, params, metrics)
    {
        return false;
    }

    cell_has_protection_style(cell, cell_style, metrics)
}

fn inactive_protection_policy_keeps_cell(
    cell: &ParsedCell<'_, '_>,
    params: &CleanSheetParameters,
    metrics: &CleanSheetMetrics,
) -> bool {
    match params.cleanup.inactive_protection_cell_policy {
        InactiveProtectionCellPolicy::Remove => false,
        InactiveProtectionCellPolicy::KeepInDataRange => {
            let row = digits_byte_string_to_integer(cell.row);
            !metrics
                .visible_data_range_bounds
                .cell_is_outside(cell.col_num, row)
        }
        InactiveProtectionCellPolicy::KeepAll => true,
    }
}

#[cfg(test)]
#[path = "cells_regression_tests.rs"]
mod regression_tests;
