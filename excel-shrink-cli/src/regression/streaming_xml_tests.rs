use super::*;
use crate::regression::xml_views::Reader;

type TagRecord = (Kind, Vec<u8>, Vec<u8>, usize);

fn streamed(bytes: &[u8], chunk_size: usize) -> Result<(Vec<TagRecord>, Vec<u8>)> {
    let mut scanner = Scanner::new();
    let mut tags = Vec::new();
    let mut text = Vec::new();
    for chunk in bytes.chunks(chunk_size) {
        scanner.feed(chunk, |event| {
            match event {
                Event::Tag(tag) => tags.push((tag.kind, tag.name.to_vec(), tag.attrs.to_vec(), tag.depth)),
                Event::Text(bytes) | Event::Cdata(bytes) => text.extend_from_slice(bytes),
            }
            Ok(())
        })?;
    }
    scanner.finish()?;
    Ok((tags, text))
}

#[test]
fn byte_views_and_streaming_tags_agree_at_every_chunk_size() -> Result<()> {
    let xml = br#"<?xml version='1.0'?><x:worksheet xmlns:x='urn:test'>
        <!-- a > b, -- and ----> <x:sheetData><x:row r='1'>
        <x:c r="A1" custom='a>b'><x:f>SUM(A1:A9)</x:f><x:v>3</x:v></x:c>
        <x:c r='B1'/></x:row></x:sheetData><?tail ok?></x:worksheet>"#;
    let mut reader = Reader::<128>::new(xml)?;
    let mut expected = Vec::new();
    while let Some(tag) = reader.next()? {
        expected.push((tag.kind, tag.name.to_vec(), tag.attrs.to_vec(), tag.depth));
    }
    for size in 1..=xml.len() {
        let (tags, text) = streamed(xml, size)?;
        assert_eq!(tags, expected, "chunk size {size}");
        assert!(std::str::from_utf8(&text)?.contains("SUM(A1:A9)3"));
    }
    Ok(())
}

#[test]
fn comment_and_processing_instruction_delimiters_survive_all_splits() -> Result<()> {
    for xml in [
        b"<root><!-- ----->A<?test????>B</root>".as_slice(),
        b"<!--a--><root>A<!--b-->B</root><!--c-->",
    ] {
        for size in 1..=xml.len() {
            assert_eq!(streamed(xml, size)?.1, b"AB", "chunk size {size}");
        }
    }
    Ok(())
}

#[test]
fn cdata_survives_all_chunk_boundaries_and_preserves_cell_text() -> Result<()> {
    let xml = b"<worksheet><headerFooter><oddFooter><![CDATA[&C Page < 1 ]]]]></oddFooter></headerFooter><sheetData><row><c><v>7</v></c></row></sheetData></worksheet>";
    for size in 1..=xml.len() {
        let (tags, text) = streamed(xml, size)?;
        assert!(tags.iter().any(|(_, name, _, _)| name == b"oddFooter"));
        assert_eq!(text, b"&C Page < 1 ]]7", "chunk size {size}");
    }
    assert!(streamed(b"<worksheet><oddFooter><![CDATA[unterminated</oddFooter></worksheet>", 3).is_err());
    for size in 1..=b"<root><v><![CDATA[a<&b]]></v></root>".len() {
        assert_eq!(streamed(b"<root><v><![CDATA[a<&b]]></v></root>", size)?.1, b"a<&b");
    }
    Ok(())
}

#[test]
fn malformed_or_unsupported_xml_never_finishes_successfully() {
    for xml in [
        b"<root><child></root>".as_slice(), b"<root>", b"<root attr='open>",
        b"<root attr=value/>", b"<root attr='a<b'/>", b"<root/><!--open",
        b"<root/><?open", b"<!DOCTYPE root><root/>", b"<root><![CDATA[x</root>",
        b"<root/><second/>", b"</root>", b"<root></root bad='1'>", b"<>",
        b"<\0r\0o\0o\0t\0/\0>\0", b"plain text",
    ] {
        for size in 1..=xml.len() {
            assert!(streamed(xml, size).is_err(), "accepted {xml:?}, chunk size {size}");
        }
    }
}

#[test]
fn long_text_and_comments_do_not_grow_the_token_buffer() -> Result<()> {
    let mut scanner = Scanner::new();
    let mut text_bytes = 0usize;
    let mut count = |event: Event<'_>| -> Result<()> {
        if let Event::Text(bytes) = event {
            text_bytes += bytes.len();
        }
        Ok(())
    };
    let block = [b'x'; READ_BYTES];
    scanner.feed(b"<root><text>", &mut count)?;
    for _ in 0..64 {
        scanner.feed(&block, &mut count)?;
    }
    scanner.feed(b"</text><!--", &mut count)?;
    for _ in 0..64 {
        scanner.feed(&block, &mut count)?;
    }
    scanner.feed(b"--", &mut count)?;
    scanner.feed(b"></root>", &mut count)?;
    scanner.finish()?;
    assert_eq!(text_bytes, READ_BYTES * 64);
    assert!(scanner.token.capacity() < 1024);
    assert!(scanner.names.iter().map(Vec::capacity).sum::<usize>() < 1024);
    Ok(())
}

#[test]
fn tag_and_hierarchy_limits_fail_explicitly_instead_of_allocating_without_bound() -> Result<()> {
    let mut scanner = Scanner::new();
    scanner.feed(b"<root attr='", |_| Ok(()))?;
    let error = scanner.feed(&vec![b'x'; MAX_TAG_BYTES], |_| Ok(())).unwrap_err();
    assert!(error.to_string().contains("XML tag exceeds streaming limit"));
    assert!(scanner.token.len() <= MAX_TAG_BYTES);

    let mut scanner = Scanner::new();
    for _ in 0..MAX_DEPTH {
        scanner.feed(b"<x>", |_| Ok(()))?;
    }
    assert!(scanner.feed(b"<x>", |_| Ok(())).is_err());
    assert_eq!(scanner.names.len(), MAX_DEPTH);
    Ok(())
}

#[test]
fn sibling_names_reuse_stack_storage() -> Result<()> {
    let mut scanner = Scanner::new();
    scanner.feed(b"<worksheet><sheetData>", |_| Ok(()))?;
    for _ in 0..10_000 {
        scanner.feed(b"<row><c><v>1</v></c></row>", |_| Ok(()))?;
    }
    scanner.feed(b"</sheetData></worksheet>", |_| Ok(()))?;
    scanner.finish()?;
    assert_eq!(scanner.names.len(), 5);
    assert!(scanner.names.iter().map(Vec::capacity).sum::<usize>() < 1024);
    Ok(())
}
