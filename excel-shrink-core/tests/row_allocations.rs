//! Measure the allocation contract independently of timings and optimizer output.
use excel_shrink_core::models::{
    CleanSheetMetrics, CleanSheetParameters, WorkbookSpecificPatterns,
};
use excel_shrink_core::sheet::row_view::RowView;
use excel_shrink_core::sheet::rows::{replace_empty_row_after_cell_range_bytes, replace_row_bytes};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

struct CountingAllocator;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
fn record_allocation() {
    if COUNTING.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}
// Safety: all allocation/deallocation operations are forwarded unchanged to
// System. The thread-local counters use no heap storage themselves.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count_allocations(work: impl FnOnce()) -> usize {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COUNTING.with(|enabled| enabled.set(false));
        }
    }
    ALLOCATIONS.with(|count| count.set(0));
    COUNTING.with(|enabled| enabled.set(true));
    let _reset = Reset;
    work();
    ALLOCATIONS.with(Cell::get)
}

fn params() -> CleanSheetParameters {
    CleanSheetParameters {
        xlsx_file_path: String::new(),
        zip_info_filename: String::new(),
        compress_method: zip::CompressionMethod::Deflated,
        patterns: WorkbookSpecificPatterns {
            xfs_by_index: Default::default(),
            no_empty_cell_visible_style_ids: Vec::new(),
            fill_or_border_or_protection_ids: Vec::new(),
            whitespace_indices: None,
        },
        advanced_cleanup: None,
        preprocess: Default::default(),
        cleanup: Default::default(),
        collect_cleanup_stats: false,
    }
}

#[test]
fn repeated_header_inspection_allocates_nothing() {
    let mut params = params();
    params.patterns.xfs_by_index.insert(2, Default::default());
    let attributes = br#" r="1048576" hidden="1" s="2" ht="12.5" customHeight="1" spans="1:42""#;
    let allocations = count_allocations(|| {
        for _ in 0..10_000 {
            black_box(RowView::parse(black_box(attributes), black_box(&params)).unwrap());
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn unchanged_empty_rows_allocate_nothing_in_both_cleanup_paths() {
    let params = params();
    let mut metrics = CleanSheetMetrics::default();
    let raw = br#"<row r="1048576" hidden="1" ht="12.5"/>"#;
    let attr = br#" r="1048576" hidden="1" ht="12.5""#;
    // Initialize tracing's callsite cache before measuring row work.
    black_box(replace_row_bytes(raw, attr, None, &params, &mut metrics));
    let allocations = count_allocations(|| {
        for _ in 0..10_000 {
            black_box(replace_row_bytes(
                black_box(raw),
                attr,
                None,
                &params,
                &mut metrics,
            ));
            black_box(replace_empty_row_after_cell_range_bytes(
                black_box(raw),
                attr,
                &params,
                &mut metrics,
            ));
        }
    });
    assert_eq!(allocations, 0);
}
