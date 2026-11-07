//! Inventory chart XML without opening Excel or decompressing worksheet parts.
use anyhow::{ensure, Result};
use clap::Parser;
use serde_json::json;
use sha2::{Digest, Sha256};
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
    /// A corpus ZIP containing XLSX members, or one XLSX file.
    #[arg(long)]
    input: PathBuf,
    /// New evidence directory; raw chart XML is retained by content hash.
    #[arg(long)]
    output: PathBuf,
    /// Restrict focused trials; omit for the entire archive.
    #[arg(long)]
    limit: Option<usize>,
}

struct Inventory {
    output: PathBuf,
    rows: BufWriter<File>,
    workbooks: usize,
    chart_parts: usize,
    errors: usize,
    reasons: BTreeMap<String, usize>,
    formulas: usize,
    simple_formulas: usize,
    started: Instant,
}

impl Inventory {
    /// Write each retained part once; duplicate XML still has a separate provenance row.
    fn workbook(&mut self, path: &Path, name: &str) -> Result<()> {
        let result = self.scan(path, name);
        self.workbooks += 1;
        if let Err(error) = result {
            self.errors += 1;
            serde_json::to_writer(
                &mut self.rows,
                &json!({"workbook":name,"error":format!("{error:#}")}),
            )?;
            writeln!(self.rows)?;
        }
        if self.workbooks % 100 == 0 {
            self.checkpoint("running")?;
        }
        Ok(())
    }

    fn scan(&mut self, path: &Path, name: &str) -> Result<()> {
        let mut zip = ZipArchive::new(File::open(path)?)?;
        let mut parts: Vec<_> = zip
            .file_names()
            .filter(|n| n.starts_with("xl/charts/") && n.ends_with(".xml"))
            .map(str::to_owned)
            .collect();
        parts.sort();
        for part in parts {
            let mut bytes = Vec::new();
            zip.by_name(&part)?.read_to_end(&mut bytes)?;
            let hash = format!("{:x}", Sha256::digest(&bytes));
            let relative = format!("xml/{hash}.xml");
            let target = self.output.join(&relative);
            if !target.exists() {
                fs::write(target, &bytes)?;
            }
            let syntax = match super::targeted_comparison::inspect_chart_xml(&bytes) {
                Ok(syntax) => syntax,
                Err(error) => {
                    self.errors += 1;
                    json!({"error":format!("{error:#}")})
                }
            };
            if syntax["root"] == "chartSpace" {
                self.chart_parts += 1;
                self.formulas += syntax["formulas"].as_array().map_or(0, Vec::len);
                self.simple_formulas += syntax["simple_a1_formulas"].as_u64().unwrap_or(0) as usize;
                let reason = syntax["fallback_reason"]
                    .as_str()
                    .unwrap_or("supported dependency syntax");
                *self.reasons.entry(reason.into()).or_default() += 1;
            }
            serde_json::to_writer(
                &mut self.rows,
                &json!({"workbook":name,"part":part,
                "xml_bytes":bytes.len(),"sha256":hash,"retained_xml":relative,"syntax":syntax}),
            )?;
            writeln!(self.rows)?;
        }
        Ok(())
    }

    fn checkpoint(&mut self, status: &str) -> Result<()> {
        self.rows.flush()?;
        super::artifact_cache::atomic_write(
            &self.output.join("summary.json"),
            &serde_json::to_vec_pretty(&json!({"status":status,"workbooks":self.workbooks,
                "chart_parts":self.chart_parts,"errors":self.errors,"formulas":self.formulas,
                "simple_a1_formulas":self.simple_formulas,"by_syntax":self.reasons,
                "elapsed_seconds":self.started.elapsed().as_secs_f64()}))?,
        )?;
        Ok(())
    }
}

/// Stream one outer member to a single owned scratch file; never materialize a corpus tree.
pub(crate) fn run(args: Args) -> Result<()> {
    ensure!(
        !args.output.exists(),
        "Use a new chart inventory output directory"
    );
    fs::create_dir_all(args.output.join("xml"))?;
    fs::write(
        args.output.join("settings.json"),
        serde_json::to_vec_pretty(&json!({
        "input":args.input,"limit":args.limit,"cache_policy":"Cache identity alone is insufficient.",
        "evidence":"All xl/charts XML parts are retained verbatim by SHA-256; JSONL rows identify source members. Worksheet XML is not decompressed."}))?,
    )?;
    let mut inventory = Inventory {
        rows: BufWriter::new(File::create(args.output.join("parts.jsonl"))?),
        output: args.output.clone(),
        workbooks: 0,
        chart_parts: 0,
        errors: 0,
        reasons: BTreeMap::new(),
        formulas: 0,
        simple_formulas: 0,
        started: Instant::now(),
    };
    if args
        .input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
    {
        inventory.workbook(&args.input, &args.input.to_string_lossy())?;
    } else {
        let mut archive = ZipArchive::new(File::open(&args.input)?)?;
        let mut entries = Vec::new();
        for index in 0..archive.len() {
            let part = archive.by_index(index)?;
            if part.name().to_ascii_lowercase().ends_with(".xlsx") {
                entries.push((part.name().to_owned(), index));
            }
        }
        entries.sort();
        let scratch = args.output.join("current.xlsx");
        for (name, index) in entries.into_iter().take(args.limit.unwrap_or(usize::MAX)) {
            // A failed extraction must not be inspected as the previous workbook.
            let extracted = io::copy(&mut archive.by_index(index)?, &mut File::create(&scratch)?);
            if let Err(error) = extracted {
                inventory.workbooks += 1;
                inventory.errors += 1;
                serde_json::to_writer(
                    &mut inventory.rows,
                    &json!({"workbook":name,"error":error.to_string()}),
                )?;
                writeln!(inventory.rows)?;
                continue;
            }
            inventory.workbook(&scratch, &name)?;
        }
        if scratch.exists() {
            fs::remove_file(scratch)?;
        }
    }
    let status = if inventory.errors == 0 {
        "completed"
    } else {
        "completed_with_errors"
    };
    inventory.checkpoint(status)?;
    println!(
        "Chart inventory: {} workbooks, {} charts, {} errors",
        inventory.workbooks, inventory.chart_parts, inventory.errors
    );
    Ok(())
}
