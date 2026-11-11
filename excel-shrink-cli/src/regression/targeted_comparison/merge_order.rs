//! Canonicalize only bounded, valid, disjoint worksheet merge definitions.
use super::content_bounds::Rectangle;
use crate::regression::xml_views::{Attributes, Kind, Tag};
use anyhow::Result;
use sha2::{Digest, Sha256};

const MAX_NORMALIZED_MERGES: usize = 256;

pub(super) struct MergeOrder {
    eligible: bool,
    containers: usize,
    active_depth: Option<usize>,
    expected_count: Option<usize>,
    count: usize,
    ranges: Vec<Rectangle>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::streaming_xml::{self, Event};

    fn proof(xml: &str, standard_namespace: bool) -> Result<Option<[u8; 32]>> {
        let mut merges = MergeOrder::default();
        streaming_xml::scan_with_offsets(xml.as_bytes(), |event| {
            match event {
                Event::Tag(tag) => {
                    merges.observe(tag, standard_namespace)?;
                }
                Event::Text(bytes) => merges.observe_text(bytes, false),
                Event::Cdata(bytes) => merges.observe_text(bytes, true),
            }
            Ok(())
        })?;
        Ok(merges.finish())
    }

    fn worksheet(body: &str) -> String {
        format!("<worksheet>{body}</worksheet>")
    }

    #[test]
    fn reordered_disjoint_ranges_have_identical_identity() -> Result<()> {
        let left = worksheet(
            r#"<mergeCells count="2"><mergeCell ref="A1:B1"/><mergeCell ref="C2:D2"/></mergeCells>"#,
        );
        let right = worksheet(
            r#"<mergeCells count="2"><mergeCell ref="C2:D2"/><mergeCell ref="A1:B1"/></mergeCells>"#,
        );
        let changed = right.replace("C2:D2", "C3:D3");
        assert!(proof(&left, true)?.is_some());
        assert_eq!(proof(&left, true)?, proof(&right, true)?);
        assert_ne!(proof(&left, true)?, proof(&changed, true)?);
        assert!(proof(&left, false)?.is_none());
        Ok(())
    }

    #[test]
    fn ambiguous_merges_keep_raw_layout_fallback() -> Result<()> {
        for body in [
            r#"<mergeCells count="2"><mergeCell ref="A1:C2"/><mergeCell ref="B2:D3"/></mergeCells>"#,
            r#"<mergeCells count="2"><mergeCell ref="A1:B1"/><mergeCell ref="A1:B1"/></mergeCells>"#,
            r#"<mergeCells count="2"><mergeCell ref="A1:B1"/></mergeCells>"#,
            r#"<mergeCells count="1"><mergeCell ref="A1:B1" custom="1"/></mergeCells>"#,
            r#"<mergeCells count="1"><mergeCell ref="B1:A1"/></mergeCells>"#,
            r#"<mergeCells count="1"><mergeCell ref="A0:B1"/></mergeCells>"#,
            r#"<mergeCells count="1"><mergeCell ref="A1:XFE1"/></mergeCells>"#,
            r#"<mergeCells count="1"><mergeCell ref="A1:B1"></mergeCell></mergeCells>"#,
            r#"<mergeCells count="1">unexpected<mergeCell ref="A1:B1"/></mergeCells>"#,
            r#"<mergeCells count="1"><![CDATA[ ]]><mergeCell ref="A1:B1"/></mergeCells>"#,
            r#"<mergeCells count="0"/><mergeCells count="0"/>"#,
            r#"<mergeCells count="0"></mergeCells>"#,
            r#"<unknown><mergeCells count="0"/></unknown>"#,
        ] {
            assert!(proof(&worksheet(body), true)?.is_none(), "{body}");
        }
        Ok(())
    }

    #[test]
    fn normalization_work_is_bounded() -> Result<()> {
        let mut body = format!("<mergeCells count=\"{}\">", MAX_NORMALIZED_MERGES + 1);
        for row in 1..=MAX_NORMALIZED_MERGES + 1 {
            body.push_str(&format!("<mergeCell ref=\"A{row}:B{row}\"/>"));
        }
        body.push_str("</mergeCells>");
        assert!(proof(&worksheet(&body), true)?.is_none());
        Ok(())
    }
}

impl Default for MergeOrder {
    fn default() -> Self {
        Self {
            eligible: true,
            containers: 0,
            active_depth: None,
            expected_count: None,
            count: 0,
            ranges: Vec::new(),
        }
    }
}

impl MergeOrder {
    /// Return whether the tag belongs to a merge subtree omitted from the canonical hash.
    /// The caller always retains the original hash for unsupported input.
    pub fn observe(&mut self, tag: Tag<'_>, standard_namespace: bool) -> Result<bool> {
        if let Some(depth) = self.active_depth {
            if tag.depth == depth && tag.kind == Kind::End {
                self.active_depth = None;
                self.eligible &= self.expected_count.is_none_or(|count| count == self.count);
            } else if tag.depth == depth + 1
                && tag.name == b"mergeCell"
                && standard_namespace
                && tag.kind == Kind::Empty
            {
                self.observe_range(tag)?;
            } else {
                self.eligible = false;
            }
            return Ok(true);
        }
        if tag.local() == b"mergeCell" {
            self.eligible = false;
            return Ok(true);
        }
        if tag.local() != b"mergeCells" {
            return Ok(false);
        }
        self.containers += 1;
        self.eligible &= self.containers == 1
            && tag.name == b"mergeCells"
            && standard_namespace
            && tag.depth == 1
            && tag.kind == Kind::Start;
        for attribute in Attributes::new(tag.attrs) {
            let (key, value) = attribute?;
            if key != b"count" || self.expected_count.is_some() {
                self.eligible = false;
                continue;
            }
            self.expected_count = std::str::from_utf8(value).ok().and_then(|v| v.parse().ok());
            self.eligible &= self
                .expected_count
                .is_some_and(|count| count <= MAX_NORMALIZED_MERGES);
        }
        if tag.kind == Kind::Start {
            self.active_depth = Some(tag.depth);
        } else {
            self.eligible &= self.expected_count.is_none_or(|count| count == 0);
        }
        Ok(true)
    }

    pub fn observe_text(&mut self, bytes: &[u8], cdata: bool) {
        if self.active_depth.is_some() && (cdata || !bytes.iter().all(u8::is_ascii_whitespace)) {
            self.eligible = false;
        }
    }

    fn observe_range(&mut self, tag: Tag<'_>) -> Result<()> {
        self.count += 1;
        if self.count > MAX_NORMALIZED_MERGES {
            self.eligible = false;
            return Ok(());
        }
        let mut reference = None;
        for attribute in Attributes::new(tag.attrs) {
            let (key, value) = attribute?;
            if key != b"ref" || reference.is_some() {
                self.eligible = false;
            } else {
                reference = std::str::from_utf8(value).ok();
            }
        }
        let Some((reference, area)) =
            reference.and_then(|value| Rectangle::parse(value).ok().map(|area| (value, area)))
        else {
            self.eligible = false;
            return Ok(());
        };
        self.eligible &= area.top >= 1
            && area.bottom <= 1_048_576
            && area.left >= 1
            && area.right <= 16_384
            && area.address() == reference;
        self.eligible &= !self.ranges.iter().any(|other| {
            area.top <= other.bottom
                && other.top <= area.bottom
                && area.left <= other.right
                && other.left <= area.right
        });
        self.ranges.push(area);
        Ok(())
    }

    pub fn finish(mut self) -> Option<[u8; 32]> {
        if !self.eligible || self.active_depth.is_some() || (self.containers > 0 && self.count == 0)
        {
            return None;
        }
        self.ranges
            .sort_by_key(|area| (area.top, area.left, area.bottom, area.right));
        let mut hash = Sha256::new();
        hash.update(b"disjoint-merge-ranges-v1");
        for area in self.ranges {
            for value in [area.top, area.left, area.bottom, area.right] {
                hash.update(value.to_le_bytes());
            }
        }
        Some(hash.finalize().into())
    }
}
