//! Offline inventory: preserve evidence first, then rank conservative formatting fallbacks.
pub(in crate::regression) mod scan;
#[cfg(test)]
mod tests;

use anyhow::{ensure, Context, Result};
use clap::Parser;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use zip::ZipArchive;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// A corpus ZIP containing XLSX files, one XLSX, or a directory of XLSX files.
    #[arg(long)]
    input: PathBuf,
    /// New inventory directory; existing results are never overwritten.
    #[arg(long)]
    output: PathBuf,
    /// Fixed row grid used only to estimate relative capture cost, not actual pixels.
    #[arg(long, default_value_t=256, value_parser=clap::value_parser!(u32).range(1..))]
    rows_per_tile: u32,
    /// Fixed column grid used only to estimate relative capture cost.
    #[arg(long, default_value_t=32, value_parser=clap::value_parser!(u32).range(1..))]
    columns_per_tile: u32,
    /// Optional file count for focused trials; omit for the full corpus.
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct Sheet {
    workbook: String,
    source_id: usize,
    part: String,
    xml_bytes: u64,
    elapsed_seconds: f64,
    error: Option<String>,
    blocks: Vec<scan::Block>,
}

#[derive(Default, Serialize)]
struct Group {
    blocks: u64,
    tile_proxy_upper_bound: u64,
    unknown_ranges: u64,
}

#[derive(Serialize)]
struct Hotspot {
    workbook: String,
    source_id: usize,
    part: String,
    block_index: usize,
    sqref: String,
    fallback_reason: String,
    tile_proxy_upper_bound: u64,
}

#[derive(Default, Serialize)]
struct Summary {
    status: String,
    total_workbooks: usize,
    workbooks: usize,
    sheets: usize,
    xml_bytes: u64,
    retained_workbooks: usize,
    blocks: usize,
    errors: usize,
    elapsed_seconds: f64,
    by_fallback: BTreeMap<String, Group>,
    by_rule_type: BTreeMap<String, Group>,
    by_rule_classification: BTreeMap<String, Group>,
    largest_blocks: Vec<Hotspot>,
}

struct Inventory<'a> {
    args: &'a Args,
    sheets: BufWriter<File>,
    workbooks: BufWriter<File>,
    summary: Summary,
    started: Instant,
}

/// Scan deterministically without Excel; emit per-file progress and retain every failure explicitly.
pub(crate) fn run(args: Args) -> Result<()> {
    ensure!(
        !args.output.exists(),
        "Use a new inventory output directory"
    );
    fs::create_dir_all(args.output.join("sources"))?;
    fs::create_dir_all(args.output.join("nodes"))?;
    let mut inventory = Inventory {
        sheets: BufWriter::new(File::create(args.output.join("sheets.jsonl"))?),
        workbooks: BufWriter::new(File::create(args.output.join("workbooks.jsonl"))?),
        args: &args,
        summary: Summary::default(),
        started: Instant::now(),
    };
    inventory.summary.status = "running".into();
    fs::write(
        args.output.join("settings.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "input":args.input, "rows_per_tile":args.rows_per_tile, "columns_per_tile":args.columns_per_tile,
            "limit":args.limit, "meaning":"Eligibility is not an invariance proof. Tile costs are fixed-grid proxies, not actual Office.js geometry. Overlapping ranges/blocks can be counted repeatedly.",
            "evidence":"Sources with CF or errors are retained verbatim. Node offsets address uncompressed ZIP-entry bytes. Small raw nodes are inline; medium nodes are XML files; nodes over 1 MiB remain addressable in retained sources."
        }))?,
    )?;
    if args
        .input
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        inventory.summary.total_workbooks = 1;
        inventory.local_file(&args.input)?;
    } else if args.input.is_dir() {
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(&args.input) {
            let entry = entry?;
            if entry.file_type().is_file() && is_xlsx(entry.path().to_string_lossy().as_ref()) {
                files.push(entry.into_path());
            }
        }
        files.sort();
        inventory.summary.total_workbooks = files.len().min(args.limit.unwrap_or(usize::MAX));
        for file in files {
            if inventory.at_limit() {
                break;
            }
            inventory.local_file(&file)?;
        }
    } else {
        let mut archive = ZipArchive::new(File::open(&args.input)?)?;
        let mut entries = Vec::new();
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .context("Read corpus ZIP directory entry")?;
            if is_xlsx(entry.name()) {
                entries.push((entry.name().to_owned(), index));
            }
        }
        entries.sort();
        inventory.summary.total_workbooks = entries.len().min(args.limit.unwrap_or(usize::MAX));
        for (name, index) in entries {
            if inventory.at_limit() {
                break;
            }
            let path = inventory.source_path();
            let extracted = io::copy(&mut archive.by_index(index)?, &mut File::create(&path)?);
            match extracted {
                Ok(_) => inventory.workbook(&path, &name)?,
                Err(error) => inventory.record_extraction_error(&name, &path, error)?,
            }
        }
    }
    inventory.summary.status = if inventory.summary.errors == 0 {
        "completed"
    } else {
        "completed_with_errors"
    }
    .into();
    inventory.checkpoint()?;
    println!(
        "CF inventory complete: {} workbooks, {} sheets, {} blocks, {} errors; {:.2}s",
        inventory.summary.workbooks,
        inventory.summary.sheets,
        inventory.summary.blocks,
        inventory.summary.errors,
        inventory.started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn is_xlsx(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".xlsx")
}

impl Inventory<'_> {
    fn at_limit(&self) -> bool {
        self.args
            .limit
            .is_some_and(|limit| self.summary.workbooks >= limit)
    }
    fn source_path(&self) -> PathBuf {
        self.args
            .output
            .join("sources")
            .join(format!("{:05}.xlsx", self.summary.workbooks))
    }

    fn local_file(&mut self, path: &Path) -> Result<()> {
        let source = self.source_path();
        fs::copy(path, &source)?;
        self.workbook(&source, &path.to_string_lossy())
    }

    fn record_extraction_error(&mut self, name: &str, path: &Path, error: io::Error) -> Result<()> {
        self.summary.errors += 1;
        self.summary.retained_workbooks += 1;
        serde_json::to_writer(
            &mut self.workbooks,
            &serde_json::json!({"source_id":self.summary.workbooks,
            "workbook":name,"source":path,"error":error.to_string(),"partial_source":true}),
        )?;
        writeln!(self.workbooks)?;
        self.summary.workbooks += 1;
        self.checkpoint()
    }

    fn workbook(&mut self, path: &Path, name: &str) -> Result<()> {
        let started = Instant::now();
        let id = self.summary.workbooks;
        let before_errors = self.summary.errors;
        let before_blocks = self.summary.blocks;
        let result = self.worksheet_entries(path, name, id);
        if result.is_err() {
            self.summary.errors += 1;
        }
        let retained = self.summary.blocks > before_blocks || self.summary.errors > before_errors;
        if retained {
            self.summary.retained_workbooks += 1;
        }
        serde_json::to_writer(
            &mut self.workbooks,
            &serde_json::json!({
                "source_id":id,"workbook":name,"source":if retained {Some(path)} else {None},
                "blocks":self.summary.blocks-before_blocks,"errors":self.summary.errors-before_errors,
                "elapsed_seconds":started.elapsed().as_secs_f64(),"error":result.err().map(|error|format!("{error:#}"))
            }),
        )?;
        writeln!(self.workbooks)?;
        if !retained {
            fs::remove_file(path)?;
        }
        self.summary.workbooks += 1;
        if self.summary.workbooks % 25 == 0 {
            self.checkpoint()?;
        }
        if self.summary.workbooks % 100 == 0 {
            println!(
                "{} workbooks; {} CF blocks; {} errors; {:.1}s",
                self.summary.workbooks,
                self.summary.blocks,
                self.summary.errors,
                self.started.elapsed().as_secs_f64()
            );
        }
        Ok(())
    }

    fn worksheet_entries(&mut self, path: &Path, workbook: &str, id: usize) -> Result<()> {
        let mut archive = ZipArchive::new(File::open(path)?)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if !entry.name().starts_with("xl/worksheets/") || !entry.name().ends_with(".xml") {
                continue;
            }
            let started = Instant::now();
            let mut sheet = Sheet {
                workbook: workbook.into(),
                source_id: id,
                part: entry.name().into(),
                xml_bytes: entry.size(),
                elapsed_seconds: 0.,
                error: None,
                blocks: Vec::new(),
            };
            let result = scan::scan(
                &mut entry,
                &mut sheet.blocks,
                self.args.rows_per_tile,
                self.args.columns_per_tile,
            );
            sheet.error = result.err().map(|error| format!("{error:#}"));
            // Successful scans read through EOF, including ZIP CRC validation.
            drop(entry);
            if sheet.error.is_none() {
                if let Err(error) = self.retain_nodes(&mut archive, index, id, &mut sheet.blocks) {
                    sheet.error = Some(format!("Retain XML evidence: {error:#}"));
                }
            }
            sheet.elapsed_seconds = started.elapsed().as_secs_f64();
            self.include_sheet(&sheet)?;
        }
        Ok(())
    }

    /// Reopen only matching worksheet entries and copy selected raw byte spans, including comments.
    fn retain_nodes(
        &self,
        archive: &mut ZipArchive<File>,
        index: usize,
        id: usize,
        blocks: &mut [scan::Block],
    ) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let mut input = archive.by_index(index)?;
        let mut position = 0;
        for (number, block) in blocks.iter_mut().enumerate() {
            ensure!(
                block.end >= block.start && block.start >= position,
                "Invalid CF source span"
            );
            let skipped = io::copy(
                &mut (&mut input).take((block.start - position) as u64),
                &mut io::sink(),
            )?;
            ensure!(
                skipped == (block.start - position) as u64,
                "Truncated CF prefix"
            );
            let length = block.end - block.start;
            if length <= 16 * 1024 {
                let mut bytes = Vec::with_capacity(length);
                (&mut input).take(length as u64).read_to_end(&mut bytes)?;
                ensure!(bytes.len() == length, "Truncated CF node");
                block.raw_xml = Some(String::from_utf8(bytes)?);
            } else if length <= 1024 * 1024 {
                let relative = format!("nodes/{id:05}-{index}-{number}.xml");
                let copied = io::copy(
                    &mut (&mut input).take(length as u64),
                    &mut File::create(self.args.output.join(&relative))?,
                )?;
                ensure!(copied == length as u64, "Truncated CF node");
                block.raw_file = Some(relative);
            } else {
                let copied = io::copy(&mut (&mut input).take(length as u64), &mut io::sink())?;
                ensure!(copied == length as u64, "Truncated CF node");
            }
            position = block.end;
        }
        Ok(())
    }

    fn include_sheet(&mut self, sheet: &Sheet) -> Result<()> {
        self.summary.sheets += 1;
        self.summary.xml_bytes += sheet.xml_bytes;
        self.summary.blocks += sheet.blocks.len();
        self.summary.errors += usize::from(sheet.error.is_some());
        for (index, block) in sheet.blocks.iter().enumerate() {
            let reason = if sheet.error.is_some() {
                "worksheet-analysis-error"
            } else {
                &block.fallback_reason
            };
            let group = self.summary.by_fallback.entry(reason.into()).or_default();
            count(group, block.tile_proxy_upper_bound);
            let types: std::collections::BTreeSet<_> = block
                .rules
                .iter()
                .map(|rule| {
                    rule.attributes
                        .get("type")
                        .map(String::as_str)
                        .unwrap_or("missing")
                })
                .collect();
            for kind in types {
                count(
                    self.summary.by_rule_type.entry(kind.into()).or_default(),
                    block.tile_proxy_upper_bound,
                );
            }
            let classifications: std::collections::BTreeSet<_> = block
                .rules
                .iter()
                .map(|rule| rule.classification.as_str())
                .collect();
            for classification in classifications {
                count(
                    self.summary
                        .by_rule_classification
                        .entry(classification.into())
                        .or_default(),
                    block.tile_proxy_upper_bound,
                );
            }
            self.summary.largest_blocks.push(Hotspot {
                workbook: sheet.workbook.clone(),
                source_id: sheet.source_id,
                part: sheet.part.clone(),
                block_index: index,
                sqref: block.sqref.clone(),
                fallback_reason: reason.into(),
                tile_proxy_upper_bound: block.tile_proxy_upper_bound.unwrap_or(0),
            });
            self.summary
                .largest_blocks
                .sort_by(|a, b| b.tile_proxy_upper_bound.cmp(&a.tile_proxy_upper_bound));
            self.summary.largest_blocks.truncate(100);
        }
        serde_json::to_writer(&mut self.sheets, sheet)?;
        writeln!(self.sheets)?;
        self.sheets.flush()?;
        Ok(())
    }

    fn checkpoint(&mut self) -> Result<()> {
        self.sheets.flush()?;
        self.workbooks.flush()?;
        self.summary.elapsed_seconds = self.started.elapsed().as_secs_f64();
        let path = self.args.output.join("summary.json");
        let pending = self.args.output.join("summary.pending.json");
        fs::write(&pending, serde_json::to_vec_pretty(&self.summary)?)?;
        fs::rename(pending, path).context("Publish inventory summary")
    }
}

fn count(group: &mut Group, tiles: Option<u64>) {
    group.blocks += 1;
    group.tile_proxy_upper_bound += tiles.unwrap_or(0);
    group.unknown_ranges += u64::from(tiles.is_none());
}
