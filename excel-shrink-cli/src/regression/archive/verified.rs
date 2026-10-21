//! A portable, one-row-per-workbook list of completed visual checks.
use super::*;

const HEADER: &[&str] = &[
    "corpus",
    "file",
    "verified_profile",
    "saved_percent",
    "shrink_seconds",
    "original_bytes",
    "shrunk_bytes",
    "identity",
];

pub(super) fn write(output: &Path, records: &[Value]) -> Result<()> {
    let mut text = HEADER.join(",");
    text.push('\n');
    for record in records
        .iter()
        .filter(|r| r["complete"] == true && r["status"] == "unchanged")
    {
        let Some(profile) = record["results"].as_array().and_then(|profiles| {
            profiles
                .iter()
                .find(|p| p["checks_complete"] == true && p["status"] == "visually unchanged")
        }) else {
            continue;
        };
        let number = |key: &str| {
            profile[key]
                .as_f64()
                .map(|v| format!("{v:.3}"))
                .unwrap_or_default()
        };
        let row = [
            record["corpus_name"]
                .as_str()
                .or_else(|| {
                    record["name"]
                        .as_str()?
                        .split_once(" / ")
                        .map(|(corpus, _)| corpus)
                })
                .unwrap_or("")
                .to_owned(),
            record["member_name"]
                .as_str()
                .or(record["name"].as_str())
                .unwrap_or("")
                .to_owned(),
            profile["profile"].as_str().unwrap_or("").to_owned(),
            profile["reduction_ratio"]
                .as_f64()
                .map(|n| format!("{:.3}", n * 100.0))
                .unwrap_or_default(),
            number("shrink_seconds"),
            profile["original_bytes"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            profile["shrunk_bytes"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            record["identity"].as_str().unwrap_or("").to_owned(),
        ];
        text.push_str(&row.iter().map(|s| quote(s)).collect::<Vec<_>>().join(","));
        text.push('\n');
    }
    artifact_cache::atomic_write(&output.join("verified-unchanged.csv"), text.as_bytes())
}

fn quote(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

pub(super) fn read(path: &Path) -> Result<Vec<Value>> {
    let rows = parse(&fs::read_to_string(path)?)?;
    ensure!(
        rows.first()
            .is_some_and(|row| row.iter().map(String::as_str).eq(HEADER.iter().copied())),
        "unexpected verified-workbook CSV columns"
    );
    rows.into_iter()
        .skip(1)
        .map(|row| {
            ensure!(
                row.len() == HEADER.len(),
                "invalid verified-workbook CSV row"
            );
            let num = |i: usize| row[i].parse::<f64>().ok();
            let bytes = |i: usize| row[i].parse::<u64>().ok();
            Ok(
                json!({"corpus_name":row[0], "member_name":row[1], "name":row[1],
            "identity":row[7], "complete":true,"status":"unchanged", "previously_verified":true,
            "results":[{"profile":row[2],"status":"visually unchanged","checks_complete":true,
                "reduction_ratio":num(3).map(|v|v/100.0),"shrink_seconds":num(4),
                "original_bytes":bytes(5),"shrunk_bytes":bytes(6),"sheets":[],"failures":[]}]}),
            )
        })
        .collect()
}

fn parse(text: &str) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted => {}
            _ => field.push(c),
        }
    }
    ensure!(!quoted, "unterminated quoted CSV field");
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_round_trip_including_quotes_commas_and_newlines() -> Result<()> {
        let names = [
            "ordinary.xlsx",
            "a, b.xlsx",
            "quoted\"file.xlsx",
            "two\nlines.xlsx",
        ];
        let text = names.iter().map(|s| quote(s)).collect::<Vec<_>>().join(",");
        assert_eq!(parse(&text)?, vec![names.map(str::to_owned).to_vec()]);
        assert!(parse("\"incomplete").is_err());
        Ok(())
    }
}
