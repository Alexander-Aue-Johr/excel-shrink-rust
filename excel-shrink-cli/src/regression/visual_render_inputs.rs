// Included by excel_automation.rs. Validated package invariants + conservative visual context.
// A render identity describes inputs, never an assumed visually equivalent output.
#[derive(Debug, Clone, Serialize)]
struct VisualPartDigest {
    part: String,
    bytes: u64,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct VisualRelationship {
    kind: String,
    target: String,
    external: bool,
}

/// Read XML attributes without depending on attribute order, quote style or prefix.
/// Relationship files are small. Invalid/unsupported XML fails closed.
fn visual_xml_attributes(tag: &[u8]) -> anyhow::Result<BTreeMap<String, String>> {
    let text = std::str::from_utf8(tag)?;
    let bytes = text.as_bytes();
    let mut p = 0;
    while p < bytes.len() && !bytes[p].is_ascii_whitespace() { p += 1; }
    let mut result = BTreeMap::new();
    while p < bytes.len() {
        while p < bytes.len() && bytes[p].is_ascii_whitespace() { p += 1; }
        if p == bytes.len() || matches!(bytes[p], b'/' | b'>') { break; }
        let start = p;
        while p < bytes.len() && !bytes[p].is_ascii_whitespace() && bytes[p] != b'=' { p += 1; }
        let name = &text[start..p];
        while p < bytes.len() && bytes[p].is_ascii_whitespace() { p += 1; }
        anyhow::ensure!(p < bytes.len() && bytes[p] == b'=', "invalid XML attribute");
        p += 1;
        while p < bytes.len() && bytes[p].is_ascii_whitespace() { p += 1; }
        anyhow::ensure!(p < bytes.len() && matches!(bytes[p], b'\'' | b'"'), "unquoted XML attribute");
        let quote = bytes[p]; p += 1; let start = p;
        while p < bytes.len() && bytes[p] != quote { p += 1; }
        anyhow::ensure!(p < bytes.len(), "unterminated XML attribute");
        let value = visual_xml_unescape(&text[start..p])?; p += 1;
        anyhow::ensure!(result.insert(name.to_owned(), value).is_none(), "duplicate XML attribute");
    }
    Ok(result)
}

fn visual_xml_unescape(text: &str) -> anyhow::Result<String> {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(i) = remaining.find('&') {
        result.push_str(&remaining[..i]);
        remaining = &remaining[i + 1..];
        let end = remaining.find(';').ok_or_else(|| anyhow::anyhow!("unterminated XML entity"))?;
        let entity = &remaining[..end];
        let character = match entity {
            "amp" => '&', "lt" => '<', "gt" => '>', "apos" => '\'', "quot" => '"',
            _ => {
                let number = if let Some(s) = entity.strip_prefix("#x") { u32::from_str_radix(s, 16)? }
                    else if let Some(s) = entity.strip_prefix('#') { s.parse::<u32>()? }
                    else { anyhow::bail!("unsupported XML entity {entity}"); };
                char::from_u32(number).ok_or_else(|| anyhow::anyhow!("invalid XML character"))?
            }
        };
        result.push(character); remaining = &remaining[end + 1..];
    }
    result.push_str(remaining); Ok(result)
}

fn visual_relationships(xml: &[u8]) -> anyhow::Result<Vec<VisualRelationship>> {
    // Strict enough to reject ambiguous relationship graphs without importing a
    // second XML crate. Namespaces do not affect dependency traversal, but every
    // relationship, ID, target and mode must be understood. DTDs are prohibited.
    let xml = xml.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(xml);
    std::str::from_utf8(xml)?;
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    let mut root_seen = false;
    let mut p = 0;
    while p < xml.len() {
        let relative = match memchr::memchr(b'<', &xml[p..]) {
            Some(i) => i,
            None => {
                anyhow::ensure!(xml[p..].iter().all(u8::is_ascii_whitespace), "text after relationship document");
                break;
            }
        };
        anyhow::ensure!(xml[p..p + relative].iter().all(u8::is_ascii_whitespace), "text in relationship document");
        p += relative;
        if xml[p..].starts_with(b"<!--") {
            p += 4 + memchr::memmem::find(&xml[p + 4..], b"-->")
                .ok_or_else(|| anyhow::anyhow!("unterminated XML comment"))? + 3;
            continue;
        }
        if xml[p..].starts_with(b"<?") {
            p += 2 + memchr::memmem::find(&xml[p + 2..], b"?>")
                .ok_or_else(|| anyhow::anyhow!("unterminated XML declaration"))? + 2;
            continue;
        }
        anyhow::ensure!(!xml[p..].starts_with(b"<!"), "DTD/declaration in relationship XML");
        let start = p + 1; let mut end = start; let mut quote = None;
        while end < xml.len() {
            let byte = xml[end];
            if let Some(q) = quote { if byte == q { quote = None; } }
            else if matches!(byte, b'\'' | b'"') { quote = Some(byte); }
            else if byte == b'>' { break; }
            end += 1;
        }
        anyhow::ensure!(end < xml.len(), "unterminated relationship tag");
        let raw = std::str::from_utf8(&xml[start..end])?.trim();
        if let Some(closing) = raw.strip_prefix('/') {
            let expected = stack.pop().ok_or_else(|| anyhow::anyhow!("unexpected closing tag"))?;
            anyhow::ensure!(closing.trim() == expected, "mismatched relationship closing tag");
        } else {
            let name_end = raw.as_bytes().iter().position(|b| b.is_ascii_whitespace() || *b == b'/').unwrap_or(raw.len());
            let name = &raw[..name_end];
            let local = name.rsplit(':').next().unwrap_or(name);
            let attributes = visual_xml_attributes(raw.as_bytes())?;
            if stack.is_empty() {
                anyhow::ensure!(!root_seen && local == "Relationships", "invalid relationship root");
                root_seen = true;
            } else {
                anyhow::ensure!(stack.len() == 1 && local == "Relationship", "unsupported relationship element {name}");
                let field = |name: &str| attributes.get(name).cloned().ok_or_else(|| anyhow::anyhow!("missing relationship {name}"));
                let id = field("Id")?;
                anyhow::ensure!(!id.is_empty() && ids.insert(id), "duplicate/empty relationship ID");
                let mode = attributes.get("TargetMode").map(String::as_str).unwrap_or("Internal");
                anyhow::ensure!(matches!(mode, "Internal" | "External"), "unsupported target mode");
                result.push(VisualRelationship { kind: field("Type")?, target: field("Target")?, external: mode == "External" });
            }
            if !raw.ends_with('/') { stack.push(name.to_owned()); }
        }
        p = end + 1;
    }
    anyhow::ensure!(root_seen && stack.is_empty(), "incomplete relationship document");
    Ok(result)
}

fn visual_resolve_target(source: &str, target: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!target.chars().any(|c| matches!(c, '\\' | '?' | '#' | ':')), "unsupported OPC target {target}");
    let mut decoded = Vec::new(); let bytes = target.as_bytes(); let mut p = 0;
    while p < bytes.len() {
        if bytes[p] == b'%' {
            anyhow::ensure!(p + 2 < bytes.len(), "invalid escaped OPC target");
            let value = u8::from_str_radix(std::str::from_utf8(&bytes[p + 1..p + 3])?, 16)?;
            anyhow::ensure!(!matches!(value, b'/' | b'\\' | 0), "escaped separator in OPC target");
            decoded.push(value); p += 3;
        } else { decoded.push(bytes[p]); p += 1; }
    }
    let decoded = String::from_utf8(decoded)?;
    let mut parts: Vec<&str> = if decoded.starts_with('/') { Vec::new() }
        else { source.rsplit_once('/').map(|(dir, _)| dir.split('/').collect()).unwrap_or_default() };
    for component in decoded.split('/') {
        match component {
            "" | "." => {},
            ".." => { anyhow::ensure!(parts.pop().is_some(), "OPC target escapes package"); },
            other => { anyhow::ensure!(!other.contains('\0'), "NUL in OPC target"); parts.push(other); },
        }
    }
    anyhow::ensure!(!parts.is_empty(), "empty OPC target");
    Ok(parts.join("/"))
}

fn visual_external_relationship_requires_runtime(kind: &str) -> bool {
    let local = kind.rsplit('/').next().unwrap_or(kind);
    matches!(local, "image" | "oleObject" | "control")
}

fn visual_hash_dependencies(parts: &[VisualPartDigest]) -> String {
    super::artifact_cache::sha256(&serde_json::to_vec(parts).expect("serializable render inputs"))
}

fn workbook_render_identities(workbook_path: &Path) -> anyhow::Result<BTreeMap<usize, WorkbookScreenshotSheetIdentity>> {
    use super::artifact_cache as cache;
    let _phase = visual_diagnostics::phase("screenshot.render_inputs", workbook_path.display().to_string());
    let package = cache::manifest(workbook_path)?;
    let order = workbook_visual_sheet_order(workbook_path)?;
    let visual_sheets: BTreeSet<_> = order.iter().cloned().collect();
    let mut reason = None;
    let mut runtime = package.parts.values().any(|p| p.visual_runtime);
    if runtime {
        reason = Some("volatile or refresh-on-open rendering input".into());
    }
    let mut archive = ZipArchive::new(File::open(workbook_path)?)?;
    // Relationship files are small. Validate all of them once per invocation;
    // no worksheet inflation and no dependency graph traversal is needed here.
    for name in package.parts.keys().filter(|n| n.ends_with(".rels")) {
        let source = if name == "_rels/.rels" { String::new() }
            else if let Some((directory, tail)) = name.rsplit_once("/_rels/") {
                format!("{directory}/{}", tail.strip_suffix(".rels").unwrap_or(tail))
            } else { runtime = true; reason = Some(format!("unsupported relationship part {name}")); continue; };
        let mut data = Vec::new(); archive.by_name(name)?.read_to_end(&mut data)?;
        match visual_relationships(&data) {
            Ok(relationships) => for rel in relationships {
                if rel.external {
                    if visual_external_relationship_requires_runtime(&rel.kind) {
                        runtime = true;
                        reason = Some(format!(
                            "external visual relationship {}",
                            rel.kind
                        ));
                    }
                    continue;
                }
                match visual_resolve_target(&source, &rel.target) {
                    Ok(target) if package.parts.contains_key(&target) => {},
                    Ok(target) => { runtime = true; reason = Some(format!("missing relationship target {target}")); },
                    Err(error) => { runtime = true; reason = Some(format!("unresolved relationship target: {error}")); },
                }
            },
            Err(error) => { runtime = true; reason = Some(format!("invalid relationship XML {name}: {error}")); },
        }
    }
    let global: Vec<_> = package.parts.iter()
        .filter(|(name, _)| !cache::is_sheet(name) && !visual_sheets.contains(*name) && name.as_str() != "xl/calcChain.xml")
        .map(|(name, p)| VisualPartDigest { part: name.clone(), bytes: p.bytes, content_hash: p.sha256.clone() }).collect();
    let all: Vec<_> = package.parts.iter().filter(|(name, _)| name.as_str() != "xl/calcChain.xml")
        .map(|(name,p)| VisualPartDigest { part: name.clone(), bytes: p.bytes, content_hash: p.sha256.clone() }).collect();
    let mut result = BTreeMap::new();
    for (index, sheet_file) in order.into_iter().enumerate() {
        let own = package.parts.get(&sheet_file).ok_or_else(|| anyhow::anyhow!("missing worksheet part {sheet_file}"))?;
        // Cached formula text on an ordinary sheet is not evaluated in manual
        // screenshot mode. CF/charts/linked shapes can still depend on other
        // sheets, so they receive the whole package context (except calcChain).
        let contextual = own.visual_context || sheet_file.starts_with("xl/chartsheets/");
        let mut dependencies = if contextual { all.clone() } else {
            let mut dependencies = global.clone();
            dependencies.push(VisualPartDigest { part: sheet_file.clone(), bytes: own.bytes, content_hash: own.sha256.clone() });
            dependencies.sort_by(|a,b| a.part.cmp(&b.part)); dependencies
        };
        dependencies.dedup_by(|a,b| a.part == b.part);
        let cacheable = !runtime;
        let fallback = reason.clone().or_else(|| {
            contextual.then(|| "cross-sheet visual context: full package inputs".into())
        });
        result.insert(index + 1, WorkbookScreenshotSheetIdentity {
            sheet_file, sheet_xml_hash: own.sha256.clone(), sheet_render_hash: visual_hash_dependencies(&dependencies),
            render_dependencies: dependencies, render_fallback_reason: fallback, render_cacheable: cacheable,
        });
    }
    anyhow::ensure!(cache::hash_file(workbook_path)? == package.archive_sha256, "workbook changed while planning screenshots");
    Ok(result)
}

fn same_stored_render_inputs(a: &WorkbookScreenshotSheetIdentity, b: &WorkbookScreenshotSheetIdentity) -> bool {
    a.render_cacheable && b.render_cacheable && a.sheet_file == b.sheet_file
        && a.sheet_xml_hash == b.sheet_xml_hash && a.sheet_render_hash == b.sheet_render_hash
}

fn write_render_input_report(path: &Path, original: &WorkbookScreenshotCache, shrunk: &WorkbookScreenshotCache) -> anyhow::Result<()> {
    let side = |cache: &WorkbookScreenshotCache| -> Vec<serde_json::Value> {
        cache.sheet_identities.iter().map(|(index, identity)| serde_json::json!({
            "sheet_index": index, "sheet_file": identity.sheet_file, "sheet_xml_hash": identity.sheet_xml_hash,
            "sheet_render_hash": identity.sheet_render_hash, "cacheable": identity.render_cacheable,
            "fallback_reason": identity.render_fallback_reason, "dependencies": identity.render_dependencies,
            "capture_range_hash": cache.sheet_capture_range_hash(*index),
            "manifest": shared_sheet_screenshot_manifest_path(cache, *index, identity),
        })).collect()
    };
    atomic_cache_write(path, &serde_json::to_vec_pretty(&serde_json::json!({
        "version": SCREENSHOT_CACHE_VERSION, "cache_root": original.cache_root,
        "original_package_hash": original.workbook_hash, "shrunk_package_hash": shrunk.workbook_hash,
        "original": side(original), "shrunk": side(shrunk),
    }))?)
}
