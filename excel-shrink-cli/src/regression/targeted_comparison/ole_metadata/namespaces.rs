//! Track lexical namespace bindings for the streaming OLE observation.
use crate::regression::xml_views::{Attributes, Kind, Tag};
use anyhow::{ensure, Result};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Namespaces {
    bindings: BTreeMap<String, String>,
    scopes: Vec<(usize, Vec<(String, Option<String>)>)>,
}

impl Namespaces {
    /// Restore completed scopes before applying this element's declarations.
    pub fn observe(&mut self, tag: Tag<'_>) -> Result<()> {
        while self
            .scopes
            .last()
            .is_some_and(|(depth, _)| *depth >= tag.depth)
        {
            let (_, changes) = self.scopes.pop().unwrap();
            for (prefix, previous) in changes.into_iter().rev() {
                match previous {
                    Some(uri) => {
                        self.bindings.insert(prefix, uri);
                    }
                    None => {
                        self.bindings.remove(&prefix);
                    }
                }
            }
        }
        if tag.kind == Kind::End || memchr::memmem::find(tag.attrs, b"xmlns").is_none() {
            return Ok(());
        }
        let mut changes = Vec::new();
        let mut declared = BTreeSet::new();
        for attribute in Attributes::new(tag.attrs) {
            let (key, value) = attribute?;
            let prefix = if key == b"xmlns" {
                ""
            } else if let Some(prefix) = key.strip_prefix(b"xmlns:") {
                std::str::from_utf8(prefix)?
            } else {
                continue;
            };
            ensure!(
                declared.insert(prefix.to_owned()),
                "Duplicate namespace declaration"
            );
            let previous = self
                .bindings
                .insert(prefix.into(), super::decode_attr(value)?);
            changes.push((prefix.into(), previous));
        }
        if !changes.is_empty() {
            self.scopes.push((tag.depth, changes));
        }
        Ok(())
    }

    /// An undeclared prefix is unknown. An absent default namespace is empty.
    pub fn element(&self, name: &[u8]) -> Result<Option<String>> {
        let name = std::str::from_utf8(name)?;
        Ok(match name.split_once(':') {
            Some((prefix, _)) => self.bindings.get(prefix).cloned(),
            None => Some(self.bindings.get("").cloned().unwrap_or_default()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::xml_views::Reader;

    #[test]
    fn bindings_follow_nested_and_empty_element_scopes() -> Result<()> {
        let xml = br#"<root xmlns="main" xmlns:r="relationships"><wrapper xmlns="foreign"><oleObject r:id="x"/></wrapper><oleObject xmlns:r="foreign"/><oleObject/><x:oleObject/></root>"#;
        let mut reader = Reader::<32>::new(xml.as_slice())?;
        let mut namespaces = Namespaces::default();
        let mut observed = Vec::new();
        while let Some(tag) = reader.next()? {
            namespaces.observe(tag)?;
            if tag.local() == b"oleObject" && tag.kind != Kind::End {
                observed.push((namespaces.element(tag.name)?, namespaces.element(b"r:id")?));
            }
        }
        assert_eq!(
            observed,
            vec![
                (Some("foreign".into()), Some("relationships".into())),
                (Some("main".into()), Some("foreign".into())),
                (Some("main".into()), Some("relationships".into())),
                (None, Some("relationships".into())),
            ]
        );
        Ok(())
    }

    #[test]
    fn repeated_declarations_on_one_element_are_rejected() -> Result<()> {
        let mut reader = Reader::<32>::new(br#"<root xmlns:r="one" xmlns:r="two"/>"#)?;
        let mut namespaces = Namespaces::default();
        assert!(namespaces.observe(reader.next()?.unwrap()).is_err());
        Ok(())
    }
}
