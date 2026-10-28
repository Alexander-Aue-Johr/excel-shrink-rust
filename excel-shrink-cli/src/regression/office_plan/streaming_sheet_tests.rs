use super::*;
use std::io::{self, Read};

struct Chunks<'a> {
    remaining: &'a [u8],
    size: usize,
}
impl Read for Chunks<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(output.len() <= streaming_xml::READ_BYTES);
        let count = self.remaining.len().min(self.size).min(output.len());
        output[..count].copy_from_slice(&self.remaining[..count]);
        self.remaining = &self.remaining[count..];
        Ok(count)
    }

    fn read_to_end(&mut self, _: &mut Vec<u8>) -> io::Result<usize> {
        panic!("capture planning must not materialize a whole sheet")
    }
}

#[test]
fn content_merges_columns_and_hidden_tails_survive_every_chunk_size() -> Result<()> {
    let xml = br#"<x:worksheet xmlns:x='test'><x:dimension ref='A1:XFD1048576'/>
        <x:sheetFormatPr defaultColWidth='9.5'/><x:cols><x:col min='20' max='16384' hidden='1'/></x:cols>
        <x:sheetData><x:row r='2'><x:c r='C2' t='s'><x:v>0</x:v></x:c><x:c r='ZZ2' s='1'/></x:row>
        <x:row><x:c><x:f/></x:c><x:c t='inlineStr'><x:is><x:r><x:t>text</x:t></x:r></x:is></x:c></x:row>
        <x:row r='106' hidden='1'/><x:row r='108' hidden='1'/></x:sheetData>
        <x:mergeCells><x:mergeCell ref='A10:K12'/></x:mergeCells>
        <x:extLst><x:row r='999'><x:c r='XFD999'><x:v>1</x:v></x:c></x:row></x:extLst></x:worksheet>"#;
    for size in 1..=xml.len() {
        let summary = scan(Chunks { remaining: xml, size })?;
        assert_eq!((summary.max_row, summary.max_column), (12, 11));
        assert!(summary.has_text);
        assert_eq!(summary.trailing_hidden_row_start, Some(106));
        assert_eq!(summary.columns.default_width, 9.5);
        assert_eq!(summary.columns.trailing_hidden_start()?, Some(19));
    }
    Ok(())
}

#[test]
fn formula_only_cells_count_but_empty_styled_cells_and_whitespace_do_not() -> Result<()> {
    let xml = b"<worksheet><sheetData><row r='4'><c r='D4'><f/></c><c r='Z4' s='2'/></row><row r='8'><c r='X8' t='inlineStr'><is><t> \r\n\t </t></is></c></row></sheetData></worksheet>";
    let summary = scan(Chunks { remaining: xml, size: 1 })?;
    assert_eq!((summary.max_row, summary.max_column), (4, 4));
    assert!(!summary.has_text);
    Ok(())
}

#[test]
fn large_inline_cell_content_is_not_buffered() -> Result<()> {
    let prefix = b"<worksheet><sheetData><row r='9'><c r='D9' t='inlineStr'><is><t>";
    let suffix = b"x</t></is></c></row></sheetData></worksheet>";
    // The non-whitespace byte only arrives after 16 MiB of streamed whitespace.
    let input = prefix.as_slice()
        .chain(io::repeat(b' ').take(16 * 1024 * 1024))
        .chain(suffix.as_slice());
    let summary = scan(input)?;
    assert_eq!((summary.max_row, summary.max_column), (9, 4));
    assert!(summary.has_text);
    Ok(())
}

#[test]
fn incorrect_coordinates_or_hierarchy_are_not_accepted() {
    for xml in [
        b"<worksheet><sheetData><row r='0'/></sheetData></worksheet>".as_slice(),
        b"<worksheet><sheetData><row r='1'><c r='A2'/></row></sheetData></worksheet>",
        b"<worksheet><sheetData><row r='1'><c r='XFD1'/><c/></row></sheetData></worksheet>",
        b"<worksheet><sheetData><row><c></row></c></sheetData></worksheet>",
        b"<workbook/>",
    ] {
        assert!(scan(Chunks { remaining: xml, size: 1 }).is_err(), "{xml:?}");
    }
}

#[test]
fn large_comments_do_not_hide_later_cell_contents() -> Result<()> {
    let input = b"<worksheet><sheetData><!--".as_slice()
        .chain(io::repeat(b'-').take(4 * 1024 * 1024))
        .chain(b"--><row r='20'><c r='F20'><v>42</v></c></row></sheetData></worksheet>".as_slice());
    let summary = scan(input)?;
    assert_eq!((summary.max_row, summary.max_column), (20, 6));
    Ok(())
}

#[test]
fn read_errors_reach_the_capture_planner() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::InvalidData, "broken compressed data"))
        }
    }
    assert!(format!("{:#}", scan(Broken).unwrap_err()).contains("broken compressed data"));
}
