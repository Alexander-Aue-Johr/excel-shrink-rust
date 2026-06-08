use regex::bytes::Regex;
use std::sync::OnceLock;

static T_NODE_PATTERN: OnceLock<Regex> = OnceLock::new();
static SI_NODE_PATTERN: OnceLock<Regex> = OnceLock::new();

pub fn find_whitespace_sst_indices(shared_strings_xml: &[u8]) -> Vec<usize> {
    let si_pattern = SI_NODE_PATTERN.get_or_init(|| Regex::new(r"(?s)<si>.*?</si>").unwrap());
    let t_pattern = T_NODE_PATTERN.get_or_init(|| Regex::new(r"(?s)<t[^>]*>(.*?)</t>").unwrap());

    let mut whitespace_indices = Vec::new();

    for (idx, mat) in si_pattern.find_iter(shared_strings_xml).enumerate() {
        let si_block = mat.as_bytes();
        let mut is_empty_or_whitespace = true;

        for t_mat in t_pattern.captures_iter(si_block) {
            if let Some(text_match) = t_mat.get(1) {
                let text = text_match.as_bytes();
                if !text
                    .iter()
                    .all(|&b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r')
                {
                    is_empty_or_whitespace = false;
                    break;
                }
            }
        }

        if is_empty_or_whitespace {
            whitespace_indices.push(idx);
        }
    }

    whitespace_indices
}

#[cfg(test)]
#[path = "shared_strings_tests.rs"]
mod tests;
