//! Locate publication-sensitive metadata without rewriting scientific evidence.

use super::{evidence, read_json_file};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

const MAX_RETAINED_FINDINGS: usize = 1_000;

#[derive(Default)]
struct Findings {
    total: usize,
    retained: Vec<Value>,
}

impl Findings {
    fn record(&mut self, file: &str, pointer: &str, category: &str) {
        self.total += 1;
        if self.retained.len() < MAX_RETAINED_FINDINGS {
            self.retained
                .push(json!({"file": file, "pointer": pointer, "category": category}));
        }
    }
}

/// Audit the exact JSON files copied into a static export. Findings contain only
/// JSON pointers and categories, so the audit does not duplicate sensitive values.
/// Workbook names, formulas, and image contents still require human review.
pub(super) fn audit(root: &Path, files: &[String]) -> Result<Value> {
    let mut findings = Findings::default();
    for relative in files {
        let path = evidence::confined_file(root, relative)?;
        let value = if relative.starts_with("replay/") && relative.ends_with(".json.gz") {
            super::replay::read_chunk(&path)?
        } else {
            read_json_file(&path)?
        };
        inspect(&value, "", relative, &mut findings);
    }
    Ok(json!({
        "version": 1,
        "status": "publication_review_required",
        "scanned_json_files": files.len(),
        "metadata_finding_count": findings.total,
        "metadata_findings_truncated": findings.total > findings.retained.len(),
        "metadata_findings": findings.retained,
        "limitations": [
            "This inventory does not certify anonymity or publication rights.",
            "Review workbook names, cell values, formulas, screenshots, and source-corpus distribution terms.",
            "Scientific reports and integrity manifests are preserved without automatic redaction."
        ]
    }))
}

fn inspect(value: &Value, pointer: &str, file: &str, findings: &mut Findings) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let path = format!("{pointer}/{escaped}");
                if !child.is_null() && sensitive_field(key) {
                    findings.record(file, &path, "sensitive_metadata_field");
                }
                inspect(child, &path, file, findings);
            }
        }
        Value::Array(children) => {
            for (index, child) in children.iter().enumerate() {
                inspect(child, &format!("{pointer}/{index}"), file, findings);
            }
        }
        Value::String(text) => {
            if contains_local_path(text) {
                findings.record(file, pointer, "local_path");
            }
            if contains_url_credentials(text) {
                findings.record(file, pointer, "url_credentials");
            }
        }
        _ => {}
    }
}

fn sensitive_field(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "password"
            | "secret"
            | "token"
            | "access_token"
            | "refresh_token"
            | "api_key"
            | "authorization"
            | "username"
            | "user_name"
            | "hostname"
            | "host_name"
            | "computername"
            | "machine_name"
            | "environment_key"
            | "command"
            | "arguments"
    )
}

/// Match Windows paths inside diagnostic prose as well as common Unix homes.
/// Ambiguous drive-like text is intentionally reported for review.
fn contains_local_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(3).enumerate().any(|(index, window)| {
        (index == 0 || !bytes[index - 1].is_ascii_alphanumeric())
            && window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && matches!(window[2], b'\\' | b'/')
    }) || text.contains("\\\\")
        || ["/home/", "/Users/", "/mnt/", "file://"]
            .iter()
            .any(|prefix| text.contains(prefix))
}

fn contains_url_credentials(text: &str) -> bool {
    text.split_whitespace().any(|word| {
        let Some((_, rest)) = word.split_once("://") else {
            return false;
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        authority.contains('@')
            || rest.split(['?', '&']).skip(1).any(|parameter| {
                parameter
                    .split_once('=')
                    .is_some_and(|(key, _)| sensitive_field(key))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_credentials_are_found_without_copying_their_values() {
        let input = json!({
            "error": "Failed to open C:\\Users\\Private\\book.xlsx",
            "path": "/home/private/book.xlsx",
            "credentials": "https://user:password@example.org/report",
            "url": "https://example.org/data?token=private",
            "nested/key": [{"machine_name": "private-machine"}],
            "image": "evidence/012345.png", "formula": "SUM(A1:B2)"
        });
        let mut findings = Findings::default();
        inspect(&input, "", "run.json", &mut findings);
        assert_eq!(findings.total, 5);
        assert!(findings
            .retained
            .iter()
            .any(|finding| finding["pointer"] == "/nested~1key/0/machine_name"));
        assert!(!serde_json::to_string(&findings.retained)
            .unwrap()
            .contains("Private"));
        assert!(!findings
            .retained
            .iter()
            .any(|finding| finding["pointer"] == "/image" || finding["pointer"] == "/formula"));
    }

    #[test]
    fn inventory_is_bounded_but_keeps_the_complete_concern_count() {
        let input = json!(vec!["C:/private/book.xlsx"; MAX_RETAINED_FINDINGS + 5]);
        let mut findings = Findings::default();
        inspect(&input, "", "run.json", &mut findings);
        assert_eq!(findings.total, MAX_RETAINED_FINDINGS + 5);
        assert_eq!(findings.retained.len(), MAX_RETAINED_FINDINGS);
    }

    #[test]
    fn ordinary_urls_and_relative_evidence_paths_are_not_local_paths() {
        for value in [
            "https://example.org/data",
            "http://example.org",
            "evidence/a.png",
        ] {
            assert!(!contains_local_path(value), "{value}");
        }
        for value in [
            "C:/book.xlsx",
            "Open 'V:\\private\\book.xlsx'",
            "\\\\server\\share",
        ] {
            assert!(contains_local_path(value), "{value}");
        }
    }
}
