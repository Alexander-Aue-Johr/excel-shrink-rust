#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhereToSlice {
    BeforeTag,
    AfterTag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceAtTagResult {
    pub until_tag: Vec<u8>,
    pub after_tag: Vec<u8>,
    pub is_self_closing_tag: bool,
}

pub fn find_and_slice_at_tag(
    buffer: &[u8],
    tag: &[u8],
    where_to_slice: WhereToSlice,
    search_backwards: bool,
) -> Option<SliceAtTagResult> {
    let index_tag = if search_backwards {
        rfind_bytes(buffer, tag)?
    } else {
        find_bytes(buffer, tag)?
    };

    match where_to_slice {
        WhereToSlice::BeforeTag => Some(SliceAtTagResult {
            until_tag: buffer[..index_tag].to_vec(),
            after_tag: buffer[index_tag..].to_vec(),
            is_self_closing_tag: false,
        }),
        WhereToSlice::AfterTag => {
            let after_tag_name = index_tag + tag.len();
            Some(SliceAtTagResult {
                until_tag: buffer[..after_tag_name].to_vec(),
                after_tag: buffer[after_tag_name..].to_vec(),
                is_self_closing_tag: false,
            })
        }
    }
}

pub fn find_and_slice_before_tag(
    buffer: &[u8],
    tag: &[u8],
    search_backwards: bool,
) -> Option<SliceAtTagResult> {
    find_and_slice_at_tag(buffer, tag, WhereToSlice::BeforeTag, search_backwards)
}

pub fn find_and_slice_after_tag(
    buffer: &[u8],
    tag: &[u8],
    search_backwards: bool,
) -> Option<SliceAtTagResult> {
    find_and_slice_at_tag(buffer, tag, WhereToSlice::AfterTag, search_backwards)
}

pub fn find_and_slice_after_tag_range(
    buffer: &[u8],
    tag_start: &[u8],
    tag_end: &[u8],
    search_backwards: bool,
) -> Option<SliceAtTagResult> {
    let index_tag = if search_backwards {
        rfind_bytes(buffer, tag_start)?
    } else {
        find_bytes(buffer, tag_start)?
    };

    let after_tag_start = index_tag + tag_start.len();
    let relative_end = find_bytes(&buffer[after_tag_start..], tag_end)?;
    let index_start_tag_end = after_tag_start + relative_end;

    let after_tag_end = index_start_tag_end + tag_end.len();
    let is_self_closing_tag = index_start_tag_end > 0 && buffer[index_start_tag_end - 1] == b'/';

    Some(SliceAtTagResult {
        until_tag: buffer[..after_tag_end].to_vec(),
        after_tag: buffer[after_tag_end..].to_vec(),
        is_self_closing_tag,
    })
}

pub fn extract_xml_block(
    xml_bytes: &[u8],
    start_tag: &[u8],
    end_tag: &[u8],
    start_index: usize,
) -> Option<(Vec<u8>, usize, usize, usize)> {
    let start_idx = find_bytes_from(xml_bytes, start_tag, start_index)?;
    let start_tag_end = find_byte_from(xml_bytes, b'>', start_idx)? + 1;
    let end_idx = find_bytes_from(xml_bytes, end_tag, start_tag_end)?;

    let block_start = start_tag_end;
    let block_end = end_idx;
    let new_index = end_idx + end_tag.len();

    Some((
        xml_bytes[block_start..block_end].to_vec(),
        new_index,
        block_start,
        block_end,
    ))
}

pub fn replace_xml_block(
    xml_bytes: &[u8],
    new_block: &[u8],
    block_start: usize,
    block_end: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(xml_bytes.len() - (block_end - block_start) + new_block.len());
    out.extend_from_slice(&xml_bytes[..block_start]);
    out.extend_from_slice(new_block);
    out.extend_from_slice(&xml_bytes[block_end..]);
    out
}

#[inline]
pub fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::find(haystack, needle)
}

#[inline]
pub fn find_bytes_from(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    find_bytes(&haystack[start..], needle).map(|idx| start + idx)
}

#[inline]
pub fn rfind_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::rfind(haystack, needle)
}

#[inline]
pub fn find_byte_from(haystack: &[u8], needle: u8, start: usize) -> Option<usize> {
    haystack[start..]
        .iter()
        .position(|b| *b == needle)
        .map(|idx| start + idx)
}
