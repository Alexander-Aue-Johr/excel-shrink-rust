use anyhow::{Context, Result, bail};
use excel_com::{ComApartment, OwnedApplication, SaveChanges, WorkbookCloseOptions};
use std::{
    collections::HashSet,
    env,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct SheetResult {
    name: String,
    right: f64,
    bottom: f64,
    duration_ms: f64,
}

struct FileResult {
    name: String,
    sheets: Vec<SheetResult>,
}

fn main() -> Result<()> {
    let mut args = env::args_os();
    let program = args.next().unwrap_or_default();
    let file_list = args.next().map(PathBuf::from);
    let output = args.next().map(PathBuf::from);

    if file_list.is_none() || output.is_none() || args.next().is_some() {
        bail!(
            "usage: {} <files.txt> <output.json>",
            PathBuf::from(program).display()
        );
    }

    let file_list = file_list.unwrap();
    let output = output.unwrap();

    let apartment = ComApartment::sta().context("initialize COM STA")?;
    let excel = OwnedApplication::new(&apartment).context("start Excel")?;

    let experiment_result = run_experiment(&excel, &file_list, &output);
    let quit_result = excel.quit_and_wait(Duration::from_secs(30));

    experiment_result?;
    quit_result.context("quit Excel")?;
    Ok(())
}

fn run_experiment(
    excel: &OwnedApplication<'_>,
    file_list: &Path,
    output: &Path,
) -> Result<()> {
    excel.set_visible(false)?;
    excel.set_display_alerts(false)?;
    excel.set_enable_events(false)?;
    eprintln!("Excel version: {}", excel.version()?);

    let file_list_text = fs::read_to_string(file_list)
        .with_context(|| format!("read {}", file_list.display()))?;

    let paths: Vec<PathBuf> = file_list_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect();

    if paths.is_empty() {
        bail!("file list is empty: {}", file_list.display());
    }

    let workbooks = excel.workbooks()?;
    let mut results = Vec::with_capacity(paths.len());
    let mut seen_names = HashSet::new();

    for path in paths {
        let workbook = workbooks
            .open_without_link_updates(&path, true)
            .with_context(|| format!("open workbook {}", path.display()))?;

        let file_result = measure_workbook(&workbook)
            .with_context(|| format!("measure workbook {}", path.display()));

        let close_result = workbook.close(WorkbookCloseOptions {
            save_changes: SaveChanges::Discard,
            ..WorkbookCloseOptions::new()
        });

        let file_result = file_result?;
        close_result.with_context(|| format!("close workbook {}", path.display()))?;

        if !seen_names.insert(file_result.name.clone()) {
            bail!(
                "duplicate workbook filename in corpus: {}",
                file_result.name
            );
        }

        results.push(file_result);
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }

    let json = render_json(&results)?;
    fs::write(output, json)
        .with_context(|| format!("write {}", output.display()))?;

    Ok(())
}

fn measure_workbook(workbook: &excel_com::Workbook) -> Result<FileResult> {
    let name = file_name_from_excel_full_name(&workbook.full_name()?);
    let worksheets = workbook.worksheets()?;
    let count = usize::try_from(worksheets.count()?)
        .context("worksheet count was negative")?;

    let mut sheets = Vec::with_capacity(count);

    // Explicit 1-based indexing makes the workbook tab order unmistakable.
    for index in 1..=count {
        let sheet = worksheets.item_by_index(index)?;
        let sheet_name = sheet.name()?;

        let started = Instant::now();
        let (right, bottom) = visible_drawing_bounds(&sheet)?;
        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;

        sheets.push(SheetResult {
            name: sheet_name,
            right,
            bottom,
            duration_ms,
        });
    }

    Ok(FileResult { name, sheets })
}

fn visible_drawing_bounds(sheet: &excel_com::Worksheet) -> Result<(f64, f64)> {
    let shapes = sheet.shapes()?;
    let count = shapes.count()?;

    let mut max_right = 0.0_f64;
    let mut max_bottom = 0.0_f64;

    for index in 1..=count {
        let shape = shapes.item_by_index(index)?;

        // Known dependency caveat: Excel returns Shape.Visible as Office
        // MsoTriState / VT_I4, while the current excel-com API exposes it as bool.
        // Keep this filter disabled until the dependency accepts that representation.
        // The experiment runner records Rust geometry matches as corpus-specific
        // while this caveat remains.
        // if !shape.visible()? {
        //     continue;
        // }

        let left = shape.left()?;
        let top = shape.top()?;
        let width = shape.width()?;
        let height = shape.height()?;
        let angle = shape.rotation()?.to_radians();

        let rotated_width =
            (width * angle.cos()).abs() + (height * angle.sin()).abs();
        let rotated_height =
            (width * angle.sin()).abs() + (height * angle.cos()).abs();

        let right = left + width / 2.0 + rotated_width / 2.0;
        let bottom = top + height / 2.0 + rotated_height / 2.0;

        max_right = max_right.max(right);
        max_bottom = max_bottom.max(bottom);
    }

    Ok((max_right, max_bottom))
}

fn file_name_from_excel_full_name(full_name: &str) -> String {
    full_name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(full_name)
        .to_owned()
}

fn render_json(files: &[FileResult]) -> Result<String> {
    // This is intentionally a text writer, not an ordered-map abstraction:
    // the serialized insertion order is part of the human-facing experiment artifact.
    let mut out = String::from("{\n");

    for (file_index, file) in files.iter().enumerate() {
        if file_index != 0 {
            out.push_str(",\n");
        }

        out.push_str("  ");
        out.push_str(&serde_json::to_string(&file.name)?);
        out.push_str(": {\n");

        for (sheet_index, sheet) in file.sheets.iter().enumerate() {
            if sheet_index != 0 {
                out.push_str(",\n");
            }

            out.push_str("    ");
            out.push_str(&serde_json::to_string(&sheet.name)?);
            out.push_str(": {\n");
            out.push_str(&format!("      \"right\": {:.6},\n", sheet.right));
            out.push_str(&format!("      \"bottom\": {:.6},\n", sheet.bottom));
            out.push_str(&format!(
                "      \"duration_ms\": {:.6}\n",
                sheet.duration_ms
            ));
            out.push_str("    }");
        }

        out.push_str("\n  }");
    }

    out.push_str("\n}\n");
    Ok(out)
}
