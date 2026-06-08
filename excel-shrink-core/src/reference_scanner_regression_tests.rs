use super::*;
use crate::models::ProtectedCellRefs;

#[test]
fn ignores_dimension_ref_when_scanning_references() {
    let xml = br#"
        <worksheet>
            <dimension ref="A1:XFD99"/>
        </worksheet>
    "#;

    let mut protected = ProtectedCellRefs::default();

    let result = scan_for_references(&[xml.as_slice()], &mut protected);

    assert!(matches!(result, ScanResult::Safe));
    assert_eq!(protected.cell_ref_count(), 0);
    assert_eq!(protected.range_ref_count(), 0);
}

#[test]
fn still_extracts_real_ref_attributes_except_dimension() {
    let xml = br#"
        <worksheet>
            <dimension ref="A1:XFD99"/>
            <autoFilter ref="A1:B10"/>
        </worksheet>
    "#;

    let mut protected = ProtectedCellRefs::default();

    let result = scan_for_references(&[xml.as_slice()], &mut protected);

    assert!(matches!(result, ScanResult::Safe));
    assert_eq!(protected.cell_ref_count(), 0);
    assert_eq!(protected.range_ref_count(), 1);
    assert!(protected.contains(1, 1));
    assert!(protected.contains(2, 10));
    assert!(!protected.contains(3, 10));
}