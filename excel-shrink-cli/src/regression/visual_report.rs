use anyhow::{ensure, Context};
use serde_json::Value;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn open_visual_report(path: &Path) -> anyhow::Result<()> {
    let launcher = super::repo_root().join("excel-shrink-cli/src/regression/open_report.ps1");
    let result = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(super::excel_automation::power_shell_path(&launcher)?)
        .arg("-ReportPath")
        .arg(super::excel_automation::power_shell_path(path)?)
        .output()
        .context("start the visual report browser")?;
    ensure!(
        result.status.success(),
        "report browser failed ({}): {}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

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
    let changed_sheets: Vec<_> = sheets
        .iter()
        .enumerate()
        .filter(|(_, sheet)| has_differences(sheet))
        .collect();
    let unchanged = sheets
        .iter()
        .filter(|sheet| !has_differences(sheet) && sheet["status"].as_str() == Some("same"))
        .count();
    let unchecked = sheets.len() - unchanged - changed_sheets.len();

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
"##,
    );
    if !changed_sheets.is_empty() {
        html.push_str("    <input id=\"sheet-search\" class=\"search\" type=\"search\" placeholder=\"Filter differences\" aria-label=\"Filter differences\">\n");
    }
    for &(ordinal, sheet) in &changed_sheets {
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
    if let Some(profile) = report["profile"].as_str() {
        let _ = write!(
            html,
            "<p class=\"muted\">Profile: {}</p>",
            escape_html(profile)
        );
    }
    let _ = write!(html, "<p class=\"muted\">{} worksheets</p><div class=\"metrics\"><div class=\"metric\"><strong>{}</strong><span>Unchanged</span></div><div class=\"metric\"><strong>{}</strong><span>With differences</span></div><div class=\"metric\"><strong>{}</strong><span>Not fully checked</span></div></div>", sheets.len(), unchanged, changed_sheets.len(), unchecked);
    if row_changes > 0 || image_changes > 0 {
        let _ = write!(html, "<p class=\"muted\">{row_changes} row height changes · {image_changes} image differences</p>");
    } else if unchanged > 0 && unchanged == sheets.len() {
        html.push_str("<p>No visual differences found.</p>");
    }
    if let Some(state) = report["image_comparison"].as_str() {
        let _ = write!(
            html,
            "<p class=\"muted\">Image comparison: {}</p>",
            escape_html(state)
        );
    }
    html.push_str("</header>\n");

    for &(ordinal, sheet) in &changed_sheets {
        let index = sheet["index"].as_u64().unwrap_or((ordinal + 1) as u64);
        let name = sheet["name"].as_str().unwrap_or("Unnamed worksheet");
        let status = sheet["status"].as_str().unwrap_or("unknown");
        let _ = writeln!(html, "<section id=\"sheet-{}\" class=\"worksheet\" data-search=\"{} {}\"><div class=\"sheet-title\"><h2>Sheet {:02} · {}</h2><span class=\"badge changed\">{}</span></div>",
            ordinal + 1, index, escape_html(name), index, escape_html(name), escape_html(status));

        render_differences(
            &mut html,
            &root,
            sheet,
            &format!("{}:{}:{}", workbook, report["profile"], name),
        );
        html.push_str("</section>\n");
    }
    html.push_str(REPORT_END);

    let path = output_dir.join("report.html");
    super::artifact_cache::atomic_write(&path, html.as_bytes())?;
    Ok(path)
}

pub(super) const REPORT_END: &str = r##"  </main>
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
search?.addEventListener('input', () => {
  const query = search.value.toLocaleLowerCase();
  for (const section of document.querySelectorAll('.worksheet')) {
    const found = section.dataset.search.toLocaleLowerCase().includes(query);
    section.hidden = !found;
    const link = document.querySelector('a[href="#' + section.id + '"]');
    if (link) link.hidden = !found;
  }
});
function revealTarget() {
  const target = document.getElementById(decodeURIComponent(location.hash.slice(1)));
  if (!target) return;
  if (target.tagName === 'DETAILS') target.open = true;
  const section = target.closest('.worksheet');
  if (section) section.hidden = false;
  for (let node = target.parentElement; node; node = node.parentElement) {
    if (node.tagName === 'DETAILS') node.open = true;
  }
  target.scrollIntoView();
}
window.addEventListener('hashchange', revealTarget);
const categoryControls = [...document.querySelectorAll('[data-collapse-category]')];
function applyCategories() {
  for (const control of categoryControls) {
    for (const section of document.querySelectorAll('details[data-change-category="' + control.dataset.collapseCategory + '"]')) {
      section.open = !control.checked;
    }
    try { localStorage.setItem('excel-shrink-collapse-' + control.dataset.collapseCategory, String(control.checked)); } catch (_) {}
  }
}
for (const control of categoryControls) {
  try {
    const saved = localStorage.getItem('excel-shrink-collapse-' + control.dataset.collapseCategory);
    if (saved !== null) control.checked = saved === 'true';
  } catch (_) {}
  control.addEventListener('change', applyCategories);
}
document.querySelector('[data-expand-categories]')?.addEventListener('click', () => {
  for (const control of categoryControls) control.checked = false;
  applyCategories();
});
applyCategories();
revealTarget();
document.addEventListener('click', async event => {
  const link = event.target.closest('[data-permalink]');
  if (!link) return;
  try { await navigator.clipboard.writeText(link.href); } catch (_) { /* The anchor still navigates without clipboard permission. */ }
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
document.addEventListener('error', event => {
  const image = event.target;
  if (!image.matches?.('.capture-image')) return;
  const message = document.createElement('p');
  message.className = 'image-unavailable';
  message.textContent = 'Image could not be loaded. Open PNG to check the file.';
  image.closest('figure').append(message);
}, true);
</script>
</body>
</html>
"##;

pub(super) fn anchor(kind: &str, identity: &str) -> String {
    format!(
        "{kind}-{}",
        &super::artifact_cache::sha256(identity.as_bytes())[..24]
    )
}

pub(super) fn permalink(id: &str) -> String {
    format!("<a class=\"permalink\" href=\"#{id}\" data-permalink>Link</a>")
}

pub(super) fn render_differences(html: &mut String, root: &Path, sheet: &Value, identity: &str) {
    for change in changed_rows(sheet) {
        let id = anchor(
            "difference",
            &format!("{identity}:row:{}:{}", change["row"], change["end_row"]),
        );
        let _ = write!(html, "<div id=\"{id}\">{}", permalink(&id));
        let _ = writeln!(html, "<article class=\"difference\"><h3>{} · height changed</h3><p class=\"geometry\">{} pt → {} pt</p>",
                row_label(change),
                display_number(&change["before"]), display_number(&change["after"]));
        html.push_str("<div class=\"gallery\">");
        image_card(html, root, "Original", &change["original_image"]);
        image_card(html, root, "Shrunk", &change["shrunk_image"]);
        html.push_str("</div></article></div>\n");
    }
    for change in items(sheet, "image_differences") {
        let id = anchor(
            "difference",
            &format!("{identity}:image:{}:{}", change["range"], change["region"]),
        );
        let _ = write!(html, "<div id=\"{id}\">{}", permalink(&id));
        let range = change["range"].as_str().unwrap_or("Range unavailable");
        let _ = writeln!(html, "<article class=\"difference\"><h3>Image difference</h3><p class=\"geometry\">{}</p><div class=\"gallery\">", escape_html(range));
        image_card(html, root, "Original", &change["original_image"]);
        image_card(html, root, "Shrunk", &change["shrunk_image"]);
        image_card(html, root, "Different pixels", &change["diff_image"]);
        html.push_str("</div></article></div>\n");
    }
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

fn has_differences(sheet: &Value) -> bool {
    changed_rows(sheet).next().is_some() || !items(sheet, "image_differences").is_empty()
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

pub(super) fn escape_html(value: &str) -> String {
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
    fn lists_only_differences_and_keeps_incomplete_checks_out_of_the_unchanged_count(
    ) -> anyhow::Result<()> {
        let root =
            std::env::temp_dir().join(format!("visual-report-summary-{}", std::process::id()));
        let report = json!({"workbook":"Book", "sheets":[
            {"index":1,"name":"Equal sheet","status":"same"},
            {"index":2,"name":"Unchecked sheet","status":"row heights equal"},
            {"index":3,"name":"Changed sheet","status":"changed",
             "row_heights":[{"row":4,"before":12,"after":16}]}
        ]});
        let html = fs::read_to_string(write_visual_report(&root, &report)?)?;
        assert!(!html.contains("Equal sheet"));
        assert!(!html.contains("Unchecked sheet"));
        assert!(html.contains("Changed sheet"));
        assert_eq!(html.matches("class=\"worksheet\"").count(), 1);
        for label in ["Unchanged", "With differences", "Not fully checked"] {
            assert!(html.contains(&format!("<strong>1</strong><span>{label}</span>")));
        }
        assert!(!html.contains("No visual differences found."));

        let report = json!({"workbook":"Book", "sheets":[
            {"index":1,"name":"Equal sheet","status":"same"}
        ]});
        let html = fs::read_to_string(write_visual_report(&root, &report)?)?;
        assert!(html.contains("No visual differences found."));
        assert!(!html.contains("class=\"worksheet\""));
        fs::remove_dir_all(root)?;
        Ok(())
    }

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
