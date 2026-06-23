use crate::models::CleanupCause;
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellDecision {
    Removed,
    Kept,
}

#[derive(Debug, Clone, Copy)]
pub struct CellVisitEvent<'a> {
    pub sheet_file: &'a str,
    pub col: u32,
    pub row: &'a [u8],
    pub style_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellVisitTarget {
    pub sheet_file: String,
    pub col: u32,
    pub row: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCellVisit {
    pub sheet_file: String,
    pub col: u32,
    pub row: Vec<u8>,
    pub style_id: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct CellDecisionEvent<'a> {
    pub sheet_file: &'a str,
    pub col: u32,
    pub row: &'a [u8],
    pub style_id: Option<u32>,
    pub decision: CellDecision,
    pub cause: CleanupCause,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCellDecision {
    pub sheet_file: String,
    pub col: u32,
    pub row: Vec<u8>,
    pub style_id: Option<u32>,
    pub decision: CellDecision,
    pub cause: CleanupCause,
}

thread_local! {
    static CELL_VISIT_TARGETS: RefCell<Vec<CellVisitTarget>> = const { RefCell::new(Vec::new()) };
    static CELL_VISITS: RefCell<Vec<RecordedCellVisit>> = const { RefCell::new(Vec::new()) };
    static CELL_DECISIONS: RefCell<Vec<RecordedCellDecision>> = const { RefCell::new(Vec::new()) };
}

pub fn set_cell_visit_targets(targets: Vec<CellVisitTarget>) {
    CELL_VISIT_TARGETS.with_borrow_mut(|stored_targets| {
        *stored_targets = targets;
    });
}

#[inline(always)]
pub fn record_cell_visit_if_target(event: CellVisitEvent<'_>) {
    let matches_target = CELL_VISIT_TARGETS.with_borrow(|targets| {
        targets.iter().any(|target| {
            target.sheet_file == event.sheet_file
                && target.col == event.col
                && target.row == event.row
        })
    });

    if matches_target {
        record_cell_visit(event);
    }
}

#[inline(always)]
pub fn record_cell_visit(event: CellVisitEvent<'_>) {
    CELL_VISITS.with_borrow_mut(|visits| {
        visits.push(RecordedCellVisit {
            sheet_file: event.sheet_file.to_string(),
            col: event.col,
            row: event.row.to_vec(),
            style_id: event.style_id,
        });
    });
}

#[inline(always)]
pub fn record(event: CellDecisionEvent<'_>) {
    CELL_DECISIONS.with_borrow_mut(|decisions| {
        decisions.push(RecordedCellDecision {
            sheet_file: event.sheet_file.to_string(),
            col: event.col,
            row: event.row.to_vec(),
            style_id: event.style_id,
            decision: event.decision,
            cause: event.cause,
        });
    });
}

pub fn clear() {
    CELL_VISIT_TARGETS.with_borrow_mut(Vec::clear);
    CELL_VISITS.with_borrow_mut(Vec::clear);
    CELL_DECISIONS.with_borrow_mut(Vec::clear);
}

pub fn recorded_cell_visits() -> Vec<RecordedCellVisit> {
    CELL_VISITS.with_borrow(Clone::clone)
}

pub fn recorded_cell_decisions() -> Vec<RecordedCellDecision> {
    CELL_DECISIONS.with_borrow(Clone::clone)
}
