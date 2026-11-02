//! Recognize only ordinary workbook metadata with no calculation semantics.
use crate::regression::xml_views::{Attributes, Kind, Tag};
use anyhow::Result;

#[derive(Default)]
pub(super) struct WorkbookMetadata {
    compatibility: bool,
    office_2010: bool,
    revision: bool,
    revision6: bool,
    revision10: bool,
    absolute_path_branch: bool,
}

impl WorkbookMetadata {
    /// Office writes path/revision/recovery metadata even for a formula-free
    /// workbook. Recognize the exact static forms; other extension trees remain
    /// blockers, including alternate branches containing formulas or new features.
    pub fn accepts(&mut self, tag: Tag<'_>) -> Result<bool> {
        if tag.depth == 0 {
            self.compatibility = tag.attr(b"xmlns:mc")?
                == Some(b"http://schemas.openxmlformats.org/markup-compatibility/2006");
            self.office_2010 = tag.attr(b"xmlns:x15")?
                == Some(b"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main");
            self.revision = tag.attr(b"xmlns:xr")?
                == Some(b"http://schemas.microsoft.com/office/spreadsheetml/2014/revision");
            self.revision6 = tag.attr(b"xmlns:xr6")?
                == Some(b"http://schemas.microsoft.com/office/spreadsheetml/2016/revision6");
            self.revision10 = tag.attr(b"xmlns:xr10")?
                == Some(b"http://schemas.microsoft.com/office/spreadsheetml/2016/revision10");
            return Ok(true);
        }
        if tag.kind == Kind::End {
            if tag.depth == 1 {
                self.absolute_path_branch = false;
            }
            return Ok(true);
        }
        if tag
            .attr(b"xmlns")?
            .is_some_and(|ns| ns != b"http://schemas.openxmlformats.org/spreadsheetml/2006/main")
        {
            return Ok(false);
        }
        if tag.depth == 1 && tag.name == b"mc:AlternateContent" {
            self.absolute_path_branch = tag.kind == Kind::Start;
            let namespace = tag.attr(b"xmlns:mc")?;
            return Ok(self.compatibility
                && namespace.is_none_or(|ns| {
                    ns == b"http://schemas.openxmlformats.org/markup-compatibility/2006"
                })
                && only_attributes(tag, &[b"xmlns:mc"])?);
        }
        if self.absolute_path_branch {
            return match (tag.depth, tag.name) {
                (2, b"mc:Choice") => Ok(self.office_2010
                    && tag.attr(b"Requires")? == Some(b"x15")
                    && only_attributes(tag, &[b"Requires"])?),
                (3, b"x15ac:absPath") => Ok(tag.kind == Kind::Empty
                    && tag.attr(b"xmlns:x15ac")?
                        == Some(b"http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac")
                    && tag.attr(b"url")?.is_some()
                    && only_attributes(tag, &[b"url", b"xmlns:x15ac"])?),
                _ => Ok(false),
            };
        }
        if tag.depth == 1 && tag.name == b"xr:revisionPtr" {
            return Ok(tag.kind == Kind::Empty
                && self.revision
                && self.revision6
                && self.revision10
                && only_attributes(
                    tag,
                    &[
                        b"revIDLastSave",
                        b"documentId",
                        b"xr6:coauthVersionLast",
                        b"xr6:coauthVersionMax",
                        b"xr10:uidLastSave",
                    ],
                )?);
        }
        if tag.depth == 1 && tag.name == b"fileRecoveryPr" {
            return Ok(tag.kind == Kind::Empty && only_attributes(tag, &[b"autoRecover"])?);
        }
        Ok(match tag.depth {
            1 => matches!(
                tag.name,
                b"fileVersion"
                    | b"workbookPr"
                    | b"bookViews"
                    | b"sheets"
                    | b"calcPr"
                    | b"workbookProtection"
                    | b"definedNames"
            ),
            2 => matches!(tag.name, b"workbookView" | b"sheet"),
            _ => false,
        })
    }
}

fn only_attributes(tag: Tag<'_>, names: &[&[u8]]) -> Result<bool> {
    for attribute in Attributes::new(tag.attrs) {
        if !names.contains(&attribute?.0) {
            return Ok(false);
        }
    }
    Ok(true)
}
