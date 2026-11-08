//! Probe-only OOXML add-in references. Worksheet payloads are copied compressed.
use super::{
    streaming_xml::{self, Event},
    xml_views::Kind,
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zip::{write::FileOptions, ZipArchive, ZipWriter};

const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const WEB: &str = "http://schemas.microsoft.com/office/webextensions/webextension/2010/11";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    input: PathBuf,
    output: PathBuf,
    id: String,
    version: String,
    store: String,
    store_type: String,
    #[serde(default)]
    workbook_relationship: bool,
}

pub(crate) fn run(path: &Path) -> Result<()> {
    let settings: Settings = serde_json::from_slice(&fs::read(path)?)?;
    uuid::Uuid::parse_str(&settings.id).context("Add-in ID must be a GUID")?;
    ensure!(
        matches!(settings.store_type.as_str(), "Registry" | "FileSystem"),
        "Unsupported probe catalog"
    );
    let mut source = ZipArchive::new(File::open(&settings.input)?)?;
    let mut names = BTreeSet::new();
    for index in 0..source.len() {
        let entry = source.by_index(index)?;
        ensure!(
            names.insert(entry.name().to_owned()),
            "Duplicate ZIP member"
        );
        ensure!(
            !entry.name().starts_with("xl/webextensions/"),
            "Input already contains web extensions"
        );
    }
    ensure!(
        names.contains("[Content_Types].xml")
            && names.contains(if settings.workbook_relationship {
                "xl/_rels/workbook.xml.rels"
            } else {
                "_rels/.rels"
            }),
        "Missing package metadata"
    );
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&settings.output)?;
    let result = write_package(&mut source, file, &settings);
    if result.is_err() {
        fs::remove_file(&settings.output).context("Remove incomplete probe output")?;
    }
    result
}

fn write_package(source: &mut ZipArchive<File>, file: File, settings: &Settings) -> Result<()> {
    let mut writer = ZipWriter::new(file);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let relationship_part = if settings.workbook_relationship {
        "xl/_rels/workbook.xml.rels"
    } else {
        "_rels/.rels"
    };
    let target = if settings.workbook_relationship {
        "webextensions/taskpanes.xml"
    } else {
        "/xl/webextensions/taskpanes.xml"
    };
    for index in 0..source.len() {
        let mut entry = source.by_index(index)?;
        let name = entry.name().to_owned();
        let fragment = match name.as_str() {
            "[Content_Types].xml" => format!(
                r#"
<Override xmlns="{TYPES}" PartName="/xl/webextensions/taskpanes.xml"
  ContentType="application/vnd.ms-office.webextensiontaskpanes+xml"/>
<Override xmlns="{TYPES}" PartName="/xl/webextensions/webextension1.xml"
  ContentType="application/vnd.ms-office.webextension+xml"/>
"#
            ),
            name if name == relationship_part => format!(
                r#"
<Relationship xmlns="{RELS}" Id="probe-{}"
  Type="http://schemas.microsoft.com/office/2011/relationships/webextensiontaskpanes"
  Target="{target}"/>
"#,
                settings.id
            ),
            _ => {
                writer.raw_copy_file(entry)?;
                continue;
            }
        };
        ensure!(
            entry.size() <= 1024 * 1024,
            "Probe package metadata exceeds 1 MiB"
        );
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        let expected = if name == "[Content_Types].xml" {
            b"Types".as_slice()
        } else {
            b"Relationships".as_slice()
        };
        let updated = append_root_child(&bytes, expected, fragment.as_bytes())?;
        writer.start_file(name, options)?;
        writer.write_all(&updated)?;
    }
    let reference = format!(
        r#"<we:webextension xmlns:we="{WEB}" id="{{{}}}">
  <we:reference id="{}" version="{}" store="{}" storeType="{}"/>
  <we:alternateReferences/>
  <we:properties>
    <we:property name="Office.AutoShowTaskpaneWithDocument" value="true"/>
  </we:properties>
  <we:bindings/>
  <we:snapshot/>
</we:webextension>"#,
        settings.id,
        settings.id,
        escape(&settings.version),
        escape(&settings.store),
        settings.store_type
    );
    let parts = [
        ("xl/webextensions/webextension1.xml", reference),
        ("xl/webextensions/taskpanes.xml", r#"
<wetp:taskpanes xmlns:wetp="http://schemas.microsoft.com/office/webextensions/taskpanes/2010/11">
  <wetp:taskpane dockstate="right" visibility="1" width="350" row="4">
    <wetp:webextensionref xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/>
  </wetp:taskpane>
</wetp:taskpanes>"#.into()),
        ("xl/webextensions/_rels/taskpanes.xml.rels", format!(r#"
<Relationships xmlns="{RELS}">
  <Relationship Id="rId1"
    Type="http://schemas.microsoft.com/office/2011/relationships/webextension"
    Target="webextension1.xml"/>
</Relationships>"#)),
    ];
    for (name, body) in parts {
        writer.start_file(name, options)?;
        writer.write_all(body.as_bytes())?;
    }
    writer.finish()?.sync_all()?;
    Ok(())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn append_root_child(xml: &[u8], expected: &[u8], child: &[u8]) -> Result<Vec<u8>> {
    let mut root_name = Vec::new();
    let mut closing = None;
    streaming_xml::scan_with_offsets(xml, |event| {
        if let Event::Tag(tag) = event {
            if tag.depth == 0 {
                ensure!(tag.local() == expected, "Unexpected metadata root");
                match tag.kind {
                    Kind::Start => root_name = tag.name.to_vec(),
                    Kind::Empty => {
                        root_name = tag.name.to_vec();
                        closing = Some((tag.end - 2, tag.end));
                    }
                    Kind::End => closing = Some((tag.start, tag.start)),
                }
            }
        }
        Ok(())
    })?;
    let (start, end) = closing.context("Missing metadata root close")?;
    let mut result = xml[..start].to_vec();
    if start != end {
        result.push(b'>');
    }
    result.extend_from_slice(child);
    if start != end {
        result.extend_from_slice(b"</");
        result.extend_from_slice(&root_name);
        result.push(b'>');
    }
    result.extend_from_slice(&xml[end..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_insertion_preserves_prefixes_and_rejects_malformed_xml() -> Result<()> {
        assert_eq!(
            append_root_child(b"<p:Types xmlns:p='n'/>", b"Types", b"<x/>")?,
            b"<p:Types xmlns:p='n'><x/></p:Types>"
        );
        assert_eq!(
            append_root_child(b"<Types><!-- </Types> --></Types>", b"Types", b"<x/>")?,
            b"<Types><!-- </Types> --><x/></Types>"
        );
        assert!(append_root_child(b"<Types>", b"Types", b"<x/>").is_err());
        Ok(())
    }

    #[test]
    fn copies_payloads_and_refuses_to_overwrite_existing_output() -> Result<()> {
        let root = std::env::temp_dir().join(format!("addin-probe-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let input = root.join("source.xlsx");
        let output = root.join("copy.xlsx");
        let mut writer = ZipWriter::new(File::create(&input)?);
        for (name, bytes) in [
            ("[Content_Types].xml", b"<Types/>".as_slice()),
            ("_rels/.rels", b"<Relationships/>".as_slice()),
            ("xl/_rels/workbook.xml.rels", b"<Relationships/>".as_slice()),
            (
                "xl/worksheets/sheet1.xml",
                b"<worksheet><sheetData/></worksheet>".as_slice(),
            ),
            ("xl/styles.xml", b"styles stay byte-exact".as_slice()),
        ] {
            writer.start_file(name, FileOptions::default())?;
            writer.write_all(bytes)?;
        }
        writer.finish()?;
        let original = fs::read(&input)?;
        let settings = root.join("settings.json");
        fs::write(
            &settings,
            serde_json::to_vec(&serde_json::json!({
                "input":input,"output":output,"id":uuid::Uuid::new_v4().to_string(),
                "version":"1.0.0.0","store":"local & test","store_type":"FileSystem"
            }))?,
        )?;
        run(&settings)?;
        assert_eq!(fs::read(&input)?, original);
        let retained = fs::read(&output)?;
        assert!(run(&settings).is_err());
        assert_eq!(fs::read(&output)?, retained);
        let mut source = ZipArchive::new(File::open(input)?)?;
        let mut copy = ZipArchive::new(File::open(output)?)?;
        for name in ["xl/worksheets/sheet1.xml", "xl/styles.xml"] {
            let mut before = Vec::new();
            let mut after = Vec::new();
            source.by_name(name)?.read_to_end(&mut before)?;
            copy.by_name(name)?.read_to_end(&mut after)?;
            assert_eq!(before, after);
        }
        drop(copy);
        drop(source);
        let workbook_output = root.join("workbook-copy.xlsx");
        let mut alternate: serde_json::Value = serde_json::from_slice(&fs::read(&settings)?)?;
        alternate["output"] = serde_json::json!(workbook_output);
        alternate["workbook_relationship"] = serde_json::json!(true);
        fs::write(&settings, serde_json::to_vec(&alternate)?)?;
        run(&settings)?;
        let mut copy = ZipArchive::new(File::open(&workbook_output)?)?;
        let mut relationships = String::new();
        copy.by_name("xl/_rels/workbook.xml.rels")?
            .read_to_string(&mut relationships)?;
        assert!(relationships.contains("Target=\"webextensions/taskpanes.xml\""));
        let mut root_relationships = String::new();
        copy.by_name("_rels/.rels")?
            .read_to_string(&mut root_relationships)?;
        assert_eq!(root_relationships, "<Relationships/>");
        drop(copy);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
