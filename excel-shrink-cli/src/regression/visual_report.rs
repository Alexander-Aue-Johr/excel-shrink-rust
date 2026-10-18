use serde_json::Value;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn write_visual_report(output_dir: &Path, report: &Value) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(output_dir)?;
    let root = fs::canonicalize(output_dir)?;
    let sheets = report["sheets"].as_array();
    let sheets = sheets.map(Vec::as_slice).unwrap_or_default();
    let workbook = report["workbook"].as_str().unwrap_or("Workbook");
    let row_changes: usize = sheets
        .iter()
        .flat_map(changed_rows)
        .map(row_span_size)
        .sum();
    let image_changes: usize = sheets
        .iter()
        .map(|sheet| items(sheet, "image_differences").len())
        .sum();

    let mut html = String::new();
    html.push_str(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Excel Shrink | Visual regression</title>
  <style>"##,
    );
    html.push_str(include_str!("visual_report.css"));
    html.push_str(
        r##"</style>
</head>
<body>
<div class="layout">
  <nav class="sidebar" aria-label="Worksheets">
    <div class="brand">EXCEL SHRINK</div>
    <h2>Visual regression</h2>
    <a class="nav-link" href="#overview">Overview</a>
    <input id="sheet-search" class="search" type="search" placeholder="Filter worksheets" aria-label="Filter worksheets">
"##,
    );
    for (ordinal, sheet) in sheets.iter().enumerate() {
        let index = sheet["index"].as_u64().unwrap_or((ordinal + 1) as u64);
        let name = sheet["name"].as_str().unwrap_or("Unnamed worksheet");
        let _ = writeln!(
            html,
            "<a class=\"nav-link\" href=\"#sheet-{}\">{:02} · {}</a>",
            ordinal + 1,
            index,
            escape_html(name)
        );
    }
    html.push_str(
        r##"  </nav>
  <main class="main">
    <header id="overview" class="hero">
      <div class="eyebrow">EXCEL SHRINK</div>
      <h1>Visual regression</h1>
"##,
    );
    let _ = write!(
        html,
        "<p class=\"workbook-name\">{}</p>",
        escape_html(workbook)
    );
    let _ = write!(html, "<div class=\"metrics\"><div class=\"metric\"><strong>{}</strong><span>Worksheets</span></div><div class=\"metric\"><strong>{}</strong><span>Row height changes</span></div><div class=\"metric\"><strong>{}</strong><span>Image differences</span></div></div>", sheets.len(), row_changes, image_changes);
    if let Some(state) = report["image_comparison"].as_str() {
        let _ = write!(
            html,
            "<p class=\"muted\">Image comparison: {}</p>",
            escape_html(state)
        );
    }
    html.push_str("</header>\n");

    for (ordinal, sheet) in sheets.iter().enumerate() {
        let index = sheet["index"].as_u64().unwrap_or((ordinal + 1) as u64);
        let name = sheet["name"].as_str().unwrap_or("Unnamed worksheet");
        let status = sheet["status"].as_str().unwrap_or("unknown");
        let badge_class = match status {
            "same" | "passed" | "unchanged" => "complete",
            "row heights equal" => "neutral",
            "error" | "failed" => "failed",
            _ => "changed",
        };
        let _ = writeln!(html, "<section id=\"sheet-{}\" class=\"worksheet\" data-search=\"{} {}\"><div class=\"sheet-title\"><h2>Sheet {:02} · {}</h2><span class=\"badge {}\">{}</span></div>",
            ordinal + 1, index, escape_html(name), index, escape_html(name), badge_class, escape_html(status));

        for change in changed_rows(sheet) {
            let _ = writeln!(html, "<article class=\"difference\"><h3>{} · height changed</h3><p class=\"geometry\">{} pt → {} pt</p>",
                row_label(change),
                display_number(&change["before"]), display_number(&change["after"]));
            html.push_str("<div class=\"gallery\">");
            image_card(&mut html, &root, "Original", &change["original_image"]);
            image_card(&mut html, &root, "Shrunk", &change["shrunk_image"]);
            html.push_str("</div></article>\n");
        }
        for change in items(sheet, "image_differences") {
            let range = change["range"].as_str().unwrap_or("Range unavailable");
            let _ = writeln!(html, "<article class=\"difference\"><h3>Image difference</h3><p class=\"geometry\">{}</p><div class=\"gallery\">", escape_html(range));
            image_card(&mut html, &root, "Original", &change["original_image"]);
            image_card(&mut html, &root, "Shrunk", &change["shrunk_image"]);
            image_card(&mut html, &root, "Different pixels", &change["diff_image"]);
            html.push_str("</div></article>\n");
        }
        if changed_rows(sheet).next().is_none() && items(sheet, "image_differences").is_empty() {
            html.push_str("<p class=\"muted\">No differences recorded.</p>");
        }
        html.push_str("</section>\n");
    }
    html.push_str(
        r##"  </main>
</div>
<dialog id="image-viewer" class="image-viewer" aria-label="Image viewer">
  <div class="viewer-bar">
    <strong id="viewer-title" class="viewer-title"></strong>
    <a id="viewer-original" class="button" target="_blank" rel="noopener">Open PNG</a>
    <button id="viewer-close" class="button" type="button">Close</button>
  </div>
  <div class="viewer-body"><img id="viewer-image" class="viewer-image" alt=""></div>
</dialog>
<script>
'use strict';
const search = document.querySelector('#sheet-search');
search.addEventListener('input', () => {
  const query = search.value.toLocaleLowerCase();
  for (const section of document.querySelectorAll('.worksheet')) {
    const found = section.dataset.search.toLocaleLowerCase().includes(query);
    section.hidden = !found;
    document.querySelector('a[href="#' + section.id + '"]').hidden = !found;
  }
});
const viewer = document.querySelector('#image-viewer');
document.addEventListener('click', event => {
  const link = event.target.closest('a[data-preview]');
  if (!link || typeof viewer.showModal !== 'function') return;
  event.preventDefault();
  document.querySelector('#viewer-title').textContent = link.dataset.title;
  document.querySelector('#viewer-original').href = link.href;
  const image = document.querySelector('#viewer-image');
  image.src = link.href;
  image.alt = link.dataset.title;
  viewer.showModal();
});
document.querySelector('#viewer-close').addEventListener('click', () => viewer.close());
viewer.addEventListener('close', () => document.querySelector('#viewer-image').removeAttribute('src'));
</script>
</body>
</html>
"##,
    );

    let path = output_dir.join("report.html");
    fs::write(&path, html)?;
    Ok(path)
}

fn items<'a>(parent: &'a Value, key: &str) -> &'a [Value] {
    parent[key]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn changed_rows(sheet: &Value) -> impl Iterator<Item = &Value> {
    items(sheet, "row_heights")
        .iter()
        .filter(|row| row["before"].as_f64() != row["after"].as_f64())
}

fn row_span_size(change: &Value) -> usize {
    let start = change["row"].as_u64().unwrap_or(0);
    let end = change["end_row"].as_u64().unwrap_or(start).max(start);
    usize::try_from(end.saturating_sub(start).saturating_add(1)).unwrap_or(usize::MAX)
}

fn row_label(change: &Value) -> String {
    let start = change["row"].as_u64().unwrap_or(0);
    match change["end_row"].as_u64().filter(|end| *end > start) {
        Some(end) => format!("Row {start}–{end}"),
        None => format!("Row {start}"),
    }
}

fn display_number(value: &Value) -> String {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .map(|number| number.to_string())
        .unwrap_or_else(|| "—".to_owned())
}

fn image_card(html: &mut String, root: &Path, title: &str, value: &Value) {
    let Some(path) = value.as_str().and_then(|path| image_href(root, path)) else {
        return;
    };
    let href = escape_html(&path);
    let title = escape_html(title);
    let _ = write!(html, "<figure class=\"capture\"><figcaption class=\"capture-head\">{title}</figcaption><a class=\"image-link\" href=\"{href}\" data-preview data-title=\"{title}\"><img class=\"capture-image\" src=\"{href}\" alt=\"{title}\" loading=\"lazy\" decoding=\"async\"></a></figure>");
}

fn image_href(root: &Path, path: &str) -> Option<String> {
    let source = Path::new(path);
    let full = if source.is_absolute() {
        source.to_path_buf()
    } else {
        root.join(source)
    };
    let full = fs::canonicalize(full).ok()?;
    let relative = full.strip_prefix(root).ok()?;
    if !full.is_file() {
        return None;
    }
    let parts: Vec<_> = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => name.to_str().map(encode_url_segment),
            _ => None,
        })
        .collect::<Option<_>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn encode_url_segment(segment: &str) -> String {
    let mut out = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_escapes_names_and_rejects_images_outside_its_directory() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "visual-report-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        fs::write(root.join("safe image.png"), b"png")?;
        let outside = root.with_extension("png");
        fs::write(&outside, b"png")?;
        let outside_name = format!("../{}", outside.file_name().unwrap().to_string_lossy());
        let report = json!({"workbook":"<Book & Co>","sheets":[{"index":1,"name":"<script>alert(1)</script>","status":"changed","row_heights":[{"row":1,"before":15,"after":15},{"row":2,"end_row":4,"before":15,"after":16}],"image_differences":[{"range":"A1:B2","original_image":"safe image.png","shrunk_image":outside_name}]}]});
        let path = write_visual_report(&root, &report)?;
        let html = fs::read_to_string(path)?;
        assert!(html.contains("&lt;Book &amp; Co&gt;"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("safe%20image.png"));
        assert!(!html.contains(outside.file_name().unwrap().to_string_lossy().as_ref()));
        assert!(html.contains("Row 2–4 · height changed"));
        assert!(!html.contains("Row 1 · height changed"));
        fs::remove_dir_all(root)?;
        fs::remove_file(outside)?;
        Ok(())
    }
}
