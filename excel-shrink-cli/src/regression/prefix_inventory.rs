//! Find actual prefixed worksheet data elements without decoding worksheet text or building a DOM.
use super::{
    streaming_xml::{self, Event, Scanner},
    xml_views::{Attributes, Kind, Tag},
};
use anyhow::{ensure, Context, Result};
use clap::Parser;
use rayon::prelude::*;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use zip::ZipArchive;

const MAIN: &[u8] = b"http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub(super) mod normalize;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    #[arg(long)]
    input: PathBuf,
    /// New evidence directory; matching workbooks and errors are retained verbatim.
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t=4, value_parser=clap::value_parser!(u32).range(1..))]
    workers: u32,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Default)]
struct Detector {
    bindings: BTreeMap<Vec<u8>, Vec<u8>>,
    scopes: Vec<(usize, Vec<u8>, Option<Vec<u8>>)>,
    worksheet: bool,
    data: Option<usize>,
    row: Option<usize>,
}

impl Detector {
    /// Track only namespace declarations and worksheet/sheetData/row ancestry.
    fn tag(&mut self, tag: Tag<'_>) -> Result<Option<String>> {
        let opening = matches!(tag.kind, Kind::Start | Kind::Empty);
        if opening && memchr::memmem::find(tag.attrs, b"xmlns").is_some() {
            for attr in Attributes::new(tag.attrs) {
                let (name, value) = attr?;
                let prefix = if name == b"xmlns" {
                    Some(b"".as_slice())
                } else {
                    name.strip_prefix(b"xmlns:")
                };
                if let Some(prefix) = prefix {
                    let old = self.bindings.insert(prefix.to_vec(), value.to_vec());
                    self.scopes.push((tag.depth, prefix.to_vec(), old));
                }
            }
        }
        let prefix = tag
            .name
            .iter()
            .position(|b| *b == b':')
            .map(|index| &tag.name[..index]);
        let main = self
            .bindings
            .get(prefix.unwrap_or(b""))
            .is_some_and(|value| value == MAIN);
        let local = tag.local();
        if tag.depth == 0 && opening {
            self.worksheet = local == b"worksheet" && main;
        }
        let data = self.worksheet && tag.depth == 1 && local == b"sheetData" && main;
        let row = self.data.is_some_and(|depth| tag.depth == depth + 1) && local == b"row" && main;
        let cell = self.row.is_some_and(|depth| tag.depth == depth + 1) && local == b"c" && main;
        let found = (opening && prefix.is_some() && (data || row || cell))
            .then(|| String::from_utf8_lossy(tag.name).into_owned());
        if tag.kind == Kind::Start {
            if data {
                self.data = Some(tag.depth);
            }
            if row {
                self.row = Some(tag.depth);
            }
        } else {
            if self.row == Some(tag.depth) {
                self.row = None;
            }
            if self.data == Some(tag.depth) {
                self.data = None;
            }
        }
        if matches!(tag.kind, Kind::End | Kind::Empty) {
            while self
                .scopes
                .last()
                .is_some_and(|(depth, _, _)| *depth == tag.depth)
            {
                let (_, prefix, old) = self.scopes.pop().unwrap();
                if let Some(value) = old {
                    self.bindings.insert(prefix, value);
                } else {
                    self.bindings.remove(&prefix);
                }
            }
        }
        Ok(found)
    }
}

#[derive(Serialize)]
struct Hit {
    part: String,
    element: String,
    xml_bytes: u64,
    inspected_bytes: u64,
}

/// Stop after the first proven data-element prefix; errors before a match remain explicit.
fn inspect(mut input: impl Read) -> Result<(Option<String>, u64)> {
    let mut parser = Scanner::new();
    let mut detector = Detector::default();
    let mut buffer = [0; streaming_xml::READ_BYTES];
    let mut inspected = 0;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            parser.finish()?;
            return Ok((None, inspected));
        }
        inspected += count as u64;
        let mut found = None;
        parser.feed(&buffer[..count], |event| {
            if found.is_none() {
                if let Event::Tag(tag) = event {
                    found = detector.tag(tag)?;
                }
            }
            Ok(())
        })?;
        if found.is_some() {
            return Ok((found, inspected));
        }
    }
}

#[derive(Serialize)]
struct Record {
    source_id: usize,
    workbook: String,
    input_bytes: u64,
    sheets: usize,
    inspected_bytes: u64,
    hits: Vec<Hit>,
    error: Option<String>,
    seconds: f64,
}

fn workbook(path: &Path, name: &str, id: usize) -> Record {
    let start = Instant::now();
    let mut result = Record {
        source_id: id,
        workbook: name.into(),
        input_bytes: 0,
        sheets: 0,
        inspected_bytes: 0,
        hits: Vec::new(),
        error: None,
        seconds: 0.,
    };
    let scan = (|| -> Result<()> {
        result.input_bytes = fs::metadata(path)?.len();
        let mut zip = ZipArchive::new(File::open(path)?)?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            if !entry.name().starts_with("xl/worksheets/") || !entry.name().ends_with(".xml") {
                continue;
            }
            result.sheets += 1;
            let (found, inspected) =
                inspect(&mut entry).with_context(|| format!("{}", entry.name()))?;
            result.inspected_bytes += inspected;
            if let Some(element) = found {
                result.hits.push(Hit {
                    part: entry.name().into(),
                    element,
                    xml_bytes: entry.size(),
                    inspected_bytes: inspected,
                });
            }
        }
        Ok(())
    })();
    result.error = scan.err().map(|e| format!("{e:#}"));
    result.seconds = start.elapsed().as_secs_f64();
    result
}

/// Extract bounded batches, scan independently, and retain only matches or analysis errors.
pub(crate) fn run(args: Args) -> Result<()> {
    ensure!(!args.output.exists(), "Use a new inventory directory");
    fs::create_dir_all(args.output.join("sources"))?;
    let started = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(args.workers as usize)
        .build()?;
    let mut archive = ZipArchive::new(File::open(&args.input)?)?;
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let e = archive.by_index(i)?;
        if e.name().to_ascii_lowercase().ends_with(".xlsx") {
            entries.push((e.name().to_owned(), i));
        }
    }
    entries.sort();
    entries.truncate(args.limit.unwrap_or(usize::MAX));
    ensure!(!entries.is_empty(), "No XLSX workbooks selected");
    fs::write(
        args.output.join("settings.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"input":args.input,"workers":args.workers,"total":entries.len(),"method":"byte-FSM with namespace scopes; early exit after first matching element per worksheet; no Excel"}),
        )?,
    )?;
    let mut records = BufWriter::new(File::create(args.output.join("workbooks.jsonl"))?);
    let (mut completed, mut matched, mut errors, mut sheets, mut bytes) = (0, 0, 0, 0, 0u64);
    for batch in entries.chunks(args.workers as usize) {
        let mut jobs = Vec::new();
        for (offset, (name, index)) in batch.iter().enumerate() {
            let id = completed + offset;
            let path = args.output.join("sources").join(format!("{id:05}.xlsx"));
            io::copy(&mut archive.by_index(*index)?, &mut File::create(&path)?)?;
            jobs.push((id, name, path));
        }
        let results: Vec<_> = pool.install(|| {
            jobs.par_iter()
                .map(|(id, name, path)| workbook(path, name, *id))
                .collect()
        });
        for (record, (_, _, path)) in results.into_iter().zip(jobs) {
            completed += 1;
            sheets += record.sheets;
            bytes += record.inspected_bytes;
            if !record.hits.is_empty() {
                matched += 1;
            }
            if record.error.is_some() {
                errors += 1;
            }
            if record.hits.is_empty() && record.error.is_none() {
                fs::remove_file(path)?;
            }
            serde_json::to_writer(&mut records, &record)?;
            writeln!(records)?;
        }
        records.flush()?;
        let status = match (completed == entries.len(), errors) {
            (false, _) => "running",
            (true, 0) => "completed",
            (true, _) => "completed_with_errors",
        };
        let summary = serde_json::json!({"status":status,"total":entries.len(),"completed":completed,"matched_workbooks":matched,"errors":errors,"sheets":sheets,"inspected_bytes":bytes,"seconds":started.elapsed().as_secs_f64()});
        fs::write(
            args.output.join("summary.json"),
            serde_json::to_vec_pretty(&summary)?,
        )?;
        if completed % 100 == 0 || completed == entries.len() {
            println!("{summary}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaces_are_scoped_and_tokens_can_cross_read_boundaries() {
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                let count = self.0.len().min(output.len()).min(1);
                output[..count].copy_from_slice(&self.0[..count]);
                self.0 = &self.0[count..];
                Ok(count)
            }
        }
        let main = String::from_utf8_lossy(MAIN);
        let xml = format!("<worksheet xmlns='{main}' xmlns:p='{main}'><extLst xmlns:p='foreign'/><sheetData><row><p:c r='A1'/></row></sheetData></worksheet>");
        assert_eq!(
            inspect(OneByte(xml.as_bytes())).unwrap().0.as_deref(),
            Some("p:c")
        );
        let foreign = format!("<worksheet xmlns='{main}' xmlns:p='{main}'><sheetData xmlns:p='foreign'><p:row/></sheetData></worksheet>");
        assert!(inspect(OneByte(foreign.as_bytes())).unwrap().0.is_none());
    }
    #[test]
    fn only_actual_prefixed_data_elements_with_the_correct_ancestry_match() {
        for body in [
            "<p:sheetData/>",
            "<sheetData><p:row r='1'/></sheetData>",
            "<sheetData><row r='1'><p:c r='A1'/></row></sheetData>",
        ] {
            let xml = format!(
                "<worksheet xmlns='{}' xmlns:p='{}'>{body}</worksheet>",
                String::from_utf8_lossy(MAIN),
                String::from_utf8_lossy(MAIN)
            );
            assert!(inspect(xml.as_bytes()).unwrap().0.is_some());
        }
        let xml=format!("<worksheet xmlns='{}' xmlns:p='foreign' xmlns:x14='extension'><sheetData><row><c/></row></sheetData><extLst><p:row/><x14:c/></extLst><!-- <p:c/> --></worksheet>",String::from_utf8_lossy(MAIN));
        assert!(inspect(xml.as_bytes()).unwrap().0.is_none());
    }
}
