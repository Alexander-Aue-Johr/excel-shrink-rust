use super::*;
use std::io::{self, Read};

struct Fragmented<'a> {
    remaining: &'a [u8],
    chunk_size: usize,
    interrupt: bool,
}

impl Read for Fragmented<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(output.len() <= streaming_xml::READ_BYTES);
        if self.interrupt {
            self.interrupt = false;
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = self.remaining.len().min(self.chunk_size).min(output.len());
        output[..count].copy_from_slice(&self.remaining[..count]);
        self.remaining = &self.remaining[count..];
        Ok(count)
    }

    fn read_to_end(&mut self, _: &mut Vec<u8>) -> io::Result<usize> {
        panic!("manifest must not materialize a complete ZIP member")
    }
}

fn part(name: &str, xml: &[u8], chunk_size: usize) -> Result<PartDigest> {
    digest_part(name, &mut Fragmented { remaining: xml, chunk_size, interrupt: true }, xml.len() as u64)
}

#[test]
fn digest_is_exact_and_short_reads_are_not_eof() -> Result<()> {
    let value = part("xl/media/test.bin", b"abc", 1)?;
    assert_eq!(value.bytes, 3);
    assert_eq!(value.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert!(value.recalc_cacheable);
    assert!(!value.has_formula);
    Ok(())
}

#[test]
fn formula_and_visual_flags_survive_every_input_boundary() -> Result<()> {
    let xml = br#"<x:worksheet xmlns:x='test'><x:sheetData><x:row r='1'><x:c r='A1'>
        <x:f>SUM(A2:A5)+_xlfn.RANDARRAY(2,2)+NO<!--gap-->W()</x:f>
        </x:c></x:row></x:sheetData><x:conditionalFormatting/></x:worksheet>"#;
    for size in 1..=xml.len() {
        let value = part("xl/worksheets/sheet1.xml", xml, size)?;
        assert_eq!(value.bytes, xml.len() as u64);
        assert_eq!(value.sha256, super::super::sha256(xml));
        assert!(value.visual_context && value.visual_runtime && value.has_formula);
        assert!(!value.recalc_cacheable);
    }
    Ok(())
}

#[test]
fn a_pure_formula_remains_reusable_across_chunks() -> Result<()> {
    let xml = b"<worksheet><sheetData><row><c><f>IF(A1,ROUND(B2,2),SUM(C1:C8))</f><v>1</v></c></row></sheetData></worksheet>";
    for size in 1..=xml.len() {
        let value = part("xl/worksheets/sheet1.xml", xml, size)?;
        assert!(value.recalc_cacheable && value.has_formula);
        assert!(!value.visual_runtime && !value.visual_context);
    }
    Ok(())
}

#[test]
fn external_relationships_and_refresh_flags_are_not_reusable() -> Result<()> {
    for xml in [
        b"<Relationships><Relationship TargetMode='External' Type='x/externalLink'/></Relationships>".as_slice(),
        b"<Relationships><Relationship TargetMode='Externa&#108;' Type='x/externalLink'/></Relationships>",
    ] {
        for size in 1..=xml.len() {
            assert!(!part("xl/_rels/workbook.xml.rels", xml, size)?.recalc_cacheable);
        }
    }
    let hyperlink = b"<Relationships><Relationship TargetMode='External' Type='x/hyperlink'/></Relationships>";
    assert!(part("xl/_rels/workbook.xml.rels", hyperlink, 1)?.recalc_cacheable);
    for flag in ["1", "true", "&#49;"] {
        let xml = format!("<worksheet><queryTable refreshOnLoad='{flag}'/></worksheet>");
        let value = part("xl/worksheets/sheet1.xml", xml.as_bytes(), 1)?;
        assert!(value.visual_runtime);
        assert!(!value.recalc_cacheable);
    }
    assert!(!part("xl/workbook.xml", b"<workbook><calcPr iterate='1'/></workbook>", 1)?.recalc_cacheable);
    Ok(())
}

#[test]
fn unsupported_xml_disables_reuse_without_truncating_the_hash() -> Result<()> {
    for xml in [
        b"<worksheet><![CDATA[NOW()]]></worksheet>trailing bytes".as_slice(),
        b"<!DOCTYPE worksheet><worksheet/>",
        b"<worksheet><f>SUM(A1)",
        b"<worksheet><f>RAND&#40;)</f></worksheet>",
        b"<worksheet><f><nested/></f></worksheet>",
        b"<worksheet><c></worksheet>",
    ] {
        let value = part("xl/worksheets/sheet1.xml", xml, 1)?;
        assert!(!value.recalc_cacheable);
        assert!(value.has_formula && value.visual_runtime);
        assert_eq!(value.bytes, xml.len() as u64);
        assert_eq!(value.sha256, super::super::sha256(xml));
    }
    Ok(())
}

#[test]
fn oversized_formula_is_an_uncertain_certificate_not_a_growing_buffer() -> Result<()> {
    let mut xml = b"<worksheet><f>".to_vec();
    xml.extend(std::iter::repeat(b'1').take(MAX_FORMULA_BYTES + 1));
    xml.extend_from_slice(b"</f><drawing/></worksheet>");
    let value = part("xl/worksheets/sheet1.xml", &xml, 997)?;
    assert!(!value.recalc_cacheable);
    assert!(value.has_formula && value.visual_runtime && value.visual_context);
    assert_eq!(value.sha256, super::super::sha256(&xml));
    Ok(())
}

#[test]
fn large_inline_text_is_streamed_without_formula_or_text_storage() -> Result<()> {
    let prefix = b"<worksheet><sheetData><row><c t='inlineStr'><is><t>";
    let suffix = b"</t></is></c></row></sheetData></worksheet>";
    let size = 16 * 1024 * 1024;
    let mut input = prefix.as_slice().chain(io::repeat(b'x').take(size)).chain(suffix.as_slice());
    let value = digest_part("xl/worksheets/sheet1.xml", &mut input, prefix.len() as u64 + size + suffix.len() as u64)?;
    assert_eq!(value.bytes, prefix.len() as u64 + size + suffix.len() as u64);
    assert!(value.recalc_cacheable);
    assert!(!value.has_formula && !value.visual_runtime);
    Ok(())
}

#[test]
fn declared_length_mismatches_are_errors() {
    assert!(digest_part("part.bin", &mut b"abc".as_slice(), 2).is_err());
    assert!(digest_part("part.bin", &mut b"abc".as_slice(), 4).is_err());
}

#[test]
fn input_errors_still_propagate_after_xml_certification_fails() {
    struct Broken { first: bool }
    impl Read for Broken {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.first {
                self.first = false;
                let bytes = b"<!DOCTYPE worksheet>";
                output[..bytes.len()].copy_from_slice(bytes);
                Ok(bytes.len())
            } else {
                Err(io::Error::new(io::ErrorKind::InvalidData, "CRC mismatch"))
            }
        }
    }
    let error = digest_part("xl/worksheets/sheet1.xml", &mut Broken { first: true }, 100).unwrap_err();
    assert!(format!("{error:#}").contains("CRC mismatch"));
}
