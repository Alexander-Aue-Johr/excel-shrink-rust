use crate::xml::{find_byte_from, find_bytes};

pub fn find_whitespace_sst_indices(shared_strings_xml: &[u8]) -> Vec<usize> {
    // A prefixed <t> or <si> would otherwise look empty or shift SST indices.
    if crate::xml::namespaces::requires_namespace_passthrough(shared_strings_xml).unwrap_or(true) {
        return Vec::new();
    }
    let mut whitespace_indices = Vec::new();
    let mut si_index = 0;
    let mut pos = 0;

    while let Some(si_start_rel) = find_bytes(&shared_strings_xml[pos..], b"<si") {
        let si_start = pos + si_start_rel;

        if !is_tag_start(shared_strings_xml, si_start, b"<si") {
            pos = si_start + b"<si".len();
            continue;
        }

        let Some(si_tag_end) = find_byte_from(shared_strings_xml, b'>', si_start) else {
            break;
        };

        let si_end = if si_tag_end > si_start && shared_strings_xml[si_tag_end - 1] == b'/' {
            si_tag_end + 1
        } else {
            let content_start = si_tag_end + 1;
            let Some(close_rel) = find_bytes(&shared_strings_xml[content_start..], b"</si>") else {
                break;
            };
            content_start + close_rel + b"</si>".len()
        };

        if si_is_empty_or_whitespace(&shared_strings_xml[si_start..si_end]) {
            whitespace_indices.push(si_index);
        }

        si_index += 1;
        pos = si_end;
    }

    whitespace_indices
}

fn si_is_empty_or_whitespace(si_block: &[u8]) -> bool {
    let mut pos = 0;

    while let Some(t_start_rel) = find_bytes(&si_block[pos..], b"<t") {
        let t_start = pos + t_start_rel;

        if !is_tag_start(si_block, t_start, b"<t") {
            pos = t_start + b"<t".len();
            continue;
        }

        let Some(t_tag_end) = find_byte_from(si_block, b'>', t_start) else {
            break;
        };

        if t_tag_end > t_start && si_block[t_tag_end - 1] == b'/' {
            pos = t_tag_end + 1;
            continue;
        }

        let text_start = t_tag_end + 1;
        let Some(t_close_rel) = find_bytes(&si_block[text_start..], b"</t>") else {
            break;
        };

        let text_end = text_start + t_close_rel;
        if !si_block[text_start..text_end]
            .iter()
            .all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            return false;
        }

        pos = text_end + b"</t>".len();
    }

    true
}

fn is_tag_start(data: &[u8], tag_start: usize, tag_name: &[u8]) -> bool {
    matches!(
        data.get(tag_start + tag_name.len()),
        Some(byte) if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
    )
}

#[cfg(test)]
#[path = "shared_strings_tests.rs"]
mod tests;
