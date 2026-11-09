//! Multi-archive acquisition metadata, separate from an extracted-workbook manifest.

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceProfile {
    version: u32,
    id: String,
    name: String,
    source: SourceReference,
    archives: Vec<SourceArchive>,
    extraction: Extraction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceReference {
    identifier: String,
    url: String,
    metadata_url: String,
    title: String,
    creator: String,
    retrieved_at: String,
    metadata_document_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceArchive {
    name: String,
    url: String,
    bytes: u64,
    sha1: String,
    md5: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Extraction {
    kind: String,
    status: String,
}

pub(super) fn enron_catalog_entry() -> Result<Value> {
    let profile: SourceProfile =
        serde_json::from_str(include_str!("../../../profiles/corpora/enron-edrm-v2.json"))?;
    ensure!(
        profile.version == 2 && profile.id == "enron-edrm-v2",
        "Invalid Enron source profile"
    );
    ensure!(
        profile.source.identifier == "edrm.enron.email.data.set.v2.xml"
            && profile.source.url
                == format!("https://archive.org/details/{}", profile.source.identifier)
            && profile.source.metadata_url
                == format!("https://archive.org/metadata/{}", profile.source.identifier)
            && valid_sha256(&profile.source.metadata_document_sha256),
        "Invalid Enron source reference"
    );
    ensure!(
        !profile.source.title.is_empty()
            && !profile.source.creator.is_empty()
            && chrono::DateTime::parse_from_rfc3339(&profile.source.retrieved_at).is_ok(),
        "Incomplete Enron source provenance"
    );
    ensure!(
        profile.extraction.kind == "recursive-edrm-attachments"
            && profile.extraction.status == "pending-recipe",
        "Unsupported Enron extraction recipe state"
    );
    let mut names = BTreeSet::new();
    let mut total_bytes = 0u64;
    for archive in &profile.archives {
        let name = crate::validation_sources::relative_name(&archive.name)?;
        ensure!(
            !name.contains('/')
                && name.ends_with(".zip")
                && names.insert(name.to_ascii_lowercase()),
            "Invalid or duplicate Enron ZIP name"
        );
        ensure!(
            archive.url
                == format!(
                    "https://archive.org/download/{}/{}",
                    profile.source.identifier, name
                )
                && archive.bytes > 0
                && valid_hex(&archive.sha1, 40)
                && valid_hex(&archive.md5, 32),
            "Invalid Enron ZIP identity"
        );
        total_bytes = total_bytes
            .checked_add(archive.bytes)
            .context("Enron source size overflow")?;
    }
    ensure!(
        !profile.archives.is_empty(),
        "Empty Enron source collection"
    );
    Ok(json!({
        "id":profile.id,"name":profile.name,"version":profile.version,
        "source_url":profile.source.url,"archive_count":profile.archives.len(),
        "archive_bytes":total_bytes,"workbook_count":null,"workbook_bytes":null,
        "acquisition_ready":false,"acquisition_status":"awaiting-extraction-recipe",
    }))
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enron_sources_are_listed_without_claiming_extracted_workbook_coverage() -> Result<()> {
        let entry = enron_catalog_entry()?;
        assert_eq!(entry["archive_count"], 159);
        assert_eq!(entry["archive_bytes"], 79_087_225_630u64);
        assert_eq!(entry["acquisition_ready"], false);
        assert!(entry["workbook_count"].is_null());
        assert!(entry.get("archives").is_none());
        Ok(())
    }
}
