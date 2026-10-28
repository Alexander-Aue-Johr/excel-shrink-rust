//! Application tracing. Per-cell measurements are aggregated within sampled rows;
//! never emit a trace event for every cell in an entire worksheet.
//! Durations describe wall time, including scheduling and nested Rayon work.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub const TARGET: &str = "excel_shrink::profile";
static NEXT_FLOW: AtomicU64 = AtomicU64::new(1);
static NEXT_DROP: AtomicU64 = AtomicU64::new(1);

/// Bracket an explicit drop without entering a span: the value may itself be an
/// entered span, whose destruction changes the current tracing scope.
#[track_caller]
pub fn drop_value<T>(resource: &'static str, value: T) {
    measure_drop(resource, value, std::mem::drop);
}

/// Only Some contains a value to destroy. This preserves row sampling: absent
/// detail spans in the other rows produce neither markers nor clock reads.
#[inline]
#[track_caller]
pub fn drop_optional<T>(resource: &'static str, value: Option<T>) {
    if let Some(value) = value {
        drop_value(resource, value);
    }
}

#[track_caller]
fn measure_drop<T>(resource: &'static str, value: T, destroy: impl FnOnce(T)) {
    if !tracing::enabled!(target: "excel_shrink::profile", tracing::Level::INFO) {
        destroy(value);
        return;
    }
    let drop_id = NEXT_DROP.fetch_add(1, Ordering::Relaxed);
    let caller = std::panic::Location::caller();
    tracing::info!(target: "excel_shrink::profile", event = "before_drop", drop_id, resource,
        resource_type = std::any::type_name::<T>(), source_file = caller.file(), source_line = caller.line());
    // Exclude emitting the endpoint markers from the measured destructor time.
    let started = Instant::now();
    destroy(value);
    let elapsed_ns = started.elapsed().as_nanos() as u64;
    tracing::info!(target: "excel_shrink::profile", event = "after_drop", drop_id, resource,
        source_file = caller.file(), source_line = caller.line(), elapsed_ns);
}

/// Keep the aggregate drop and expose every field in declaration order. The
/// exhaustive pattern makes newly added metrics fields a compile-time reminder.
#[track_caller]
pub fn drop_sheet_metrics(metrics: crate::models::CleanSheetMetrics) {
    if !tracing::enabled!(target: "excel_shrink::profile", tracing::Level::INFO) {
        std::mem::drop(metrics);
        return;
    }
    measure_drop("sheet.metrics", metrics, |metrics| {
        macro_rules! fields {
            ($($field:ident),+ $(,)?) => {
                let crate::models::CleanSheetMetrics { $($field),+ } = metrics;
                $({
                    let _field = phase(concat!("drop.sheet.metrics.", stringify!($field)), "", "");
                    drop_value(concat!("sheet.metrics.", stringify!($field)), $field);
                })+
            };
        }
        fields!(
            column_definitions,
            kept_cell_states,
            deleted_cell_count,
            source_content_cell_count,
            source_content_cell_count_complete,
            tail_content_guard_triggered,
            max_merge_cells_column_and_row,
            merge_cell_ranges,
            whitespace_candidates,
            hidden_candidates,
            candidates,
            cleanup_stats,
            sheet_protection_enabled,
            visible_content_bounds,
            visible_data_range_bounds,
            tail_anchor_rows,
            tail_anchor_max_row,
            has_hints,
        );
    });
}

/// Link an explicit handoff between threads or phases. Zero means tracing is disabled.
#[derive(Debug, Clone, Default)]
pub struct Handoff(u64);

impl Handoff {
    pub fn start(kind: &'static str, xlsx: &str, sheet: &str) -> Self {
        if !tracing::enabled!(target: "excel_shrink::profile", tracing::Level::INFO) {
            return Self::default();
        }
        let id = NEXT_FLOW.fetch_add(1, Ordering::Relaxed);
        tracing::info!(target: "excel_shrink::profile", event = "flow_start", flow_id = id, kind, xlsx, sheet);
        Self(id)
    }

    pub fn finish(self) {
        if self.0 != 0 {
            tracing::info!(target: "excel_shrink::profile", event = "flow_end", flow_id = self.0);
        }
    }
}

pub fn phase(name: &'static str, xlsx: &str, sheet: &str) -> tracing::span::EnteredSpan {
    tracing::info_span!(target: "excel_shrink::profile", "phase", phase = name, xlsx, sheet)
        .entered()
}

/// The false specialization has no tracing calls, including in unoptimized builds.
pub(crate) fn sampled_phase<const SAMPLED: bool>(
    name: &'static str,
) -> Option<tracing::span::EnteredSpan> {
    if SAMPLED {
        Some(phase(name, "", ""))
    } else {
        None
    }
}

static RESOURCES: OnceLock<Mutex<HashMap<&'static str, u64>>> = OnceLock::new();

/// Account for a resource until it leaves a queue or retained-result collection.
/// Constructed only at coarse boundaries; no lock or allocation when disabled.
#[derive(Debug, Default)]
pub struct ResourceGuard(Option<(&'static str, u64)>);

impl ResourceGuard {
    pub fn add(name: &'static str, amount: u64) -> Self {
        if !tracing::enabled!(target: "excel_shrink::profile", tracing::Level::INFO) {
            return Self::default();
        }
        let mut resources = RESOURCES.get_or_init(Mutex::default).lock().unwrap();
        let value = resources.entry(name).or_default();
        *value += amount;
        tracing::info!(target: "excel_shrink::profile", event = "counter", counter_name = name, value = *value);
        Self(Some((name, amount)))
    }
}

impl Drop for ResourceGuard {
    fn drop(&mut self) {
        if let Some((name, amount)) = self.0.take() {
            let mut resources = RESOURCES.get().unwrap().lock().unwrap();
            let value = resources.get_mut(name).unwrap();
            *value -= amount;
            tracing::info!(target: "excel_shrink::profile", event = "counter", counter_name = name, value = *value);
        }
    }
}
