//! Persistent, run-owned Excel sessions for legacy XLS conversion.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod client;
pub(super) mod performance;
use crate::process_job as job;
#[cfg(test)]
mod tests;
mod worker;
pub(super) use client::convert;

#[derive(Serialize, Deserialize)]
struct Member {
    directory: PathBuf,
    ready: bool,
    retiring: bool,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    workers: Vec<Member>,
}

#[derive(Serialize, Deserialize)]
struct Request {
    id: String,
    input: PathBuf,
    output: PathBuf,
    directory: PathBuf,
    #[serde(default)]
    performance: bool,
}

pub(super) struct Pool {
    root: PathBuf,
    workers: Vec<worker::Worker>,
    owner_started_ms: u64,
    startup_failures: usize,
    last_publish: Instant,
    job: job::Job,
}

impl Pool {
    /// Keep UI-automation helpers on each optimization worker's private desktop.
    pub fn new_performance(root: PathBuf) -> Result<Self> {
        let pool = Self::new(root)?;
        fs::write(pool.root.join("performance-pool"), b"check-performance")?;
        Ok(pool)
    }

    /// Publish one immutable helper program and seed workbook for this run.
    pub fn new(root: PathBuf) -> Result<Self> {
        let owner_started_ms = crate::native_process::current_started_unix_ms()
            .context("conversion pool owner identity unavailable")?
            as u64;
        fs::create_dir_all(&root)?;
        // Windows PowerShell 5 needs a BOM for non-ASCII failure classifiers.
        fs::write(
            root.join("worker.ps1"),
            format!("\u{feff}{}", include_str!("worker.ps1")),
        )?;
        fs::write(
            root.join("CaptureProcesses.cs"),
            include_str!("../../../office-capture/CaptureProcesses.cs"),
        )?;
        fs::write(
            root.join("CaptureDesktop.cs"),
            include_str!("../../../office-capture/CaptureDesktop.cs"),
        )?;
        fs::write(
            root.join("CaptureSetup.psm1"),
            include_str!("../../../office-capture/CaptureSetup.psm1"),
        )?;
        fs::write(
            root.join("ExcelConversionErrors.psm1"),
            format!("\u{feff}{}", include_str!("../ExcelConversionErrors.psm1")),
        )?;
        for name in [
            "CheckPerformance.psm1",
            "NativeWindow.cs",
            "ExcelProcessId.cs",
        ] {
            fs::copy(
                crate::validation::capture::capture_directory()?.join(name),
                root.join(name),
            )?;
        }
        fs::write(
            root.join("WorkerPerformance.psm1"),
            format!("\u{feff}{}", include_str!("performance.psm1")),
        )?;
        write_seed(&root.join("seed.xlsx"))?;
        fs::write(root.join("heartbeat"), b"running")?;
        write_json(
            &root.join("manifest.json"),
            &Manifest {
                workers: Vec::new(),
            },
        )?;
        Ok(Self {
            root,
            workers: Vec::new(),
            owner_started_ms,
            startup_failures: 0,
            last_publish: Instant::now(),
            job: job::Job::new()?,
        })
    }

    /// Refill individual vacancies; reductions retire sessions at workbook boundaries.
    pub fn reconcile(&mut self, target: usize, reserve_mib: u64) -> Result<()> {
        self.reconcile_inner(target, reserve_mib.max(512))
    }

    /// Stop owned processes before telling clients to release request directories.
    pub fn abort(&mut self, error: &anyhow::Error) -> Result<()> {
        if self.root.join("error.json").exists() {
            return Ok(());
        }
        let mut cleanup_error = None;
        for worker in &mut self.workers {
            if let Err(error) = worker.retire() {
                cleanup_error = Some(error);
            }
        }
        for worker in &mut self.workers {
            if let Err(error) = worker.stop() {
                cleanup_error = Some(error);
            }
        }
        write_json(
            &self.root.join("error.json"),
            &json!({
                "error":format!("{error:#}"),"cleanup_error":cleanup_error.as_ref().map(|error| format!("{error:#}")),
            }),
        )?;
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        Ok(())
    }

    fn reconcile_inner(&mut self, target: usize, reserve_mib: u64) -> Result<()> {
        let mut changed = false;
        for index in (0..self.workers.len()).rev() {
            if self.workers[index].should_stop()? {
                let mut worker = self.workers.remove(index);
                if !worker.ready() && !worker.is_retiring() {
                    self.startup_failures += 1;
                }
                worker.stop()?;
                changed = true;
            }
        }
        if self.workers.iter_mut().any(worker::Worker::became_ready) {
            self.startup_failures = 0;
        }
        anyhow::ensure!(
            self.startup_failures < 3,
            "persistent Excel worker startup failed three times; inspect pool session logs"
        );
        let active = self
            .workers
            .iter()
            .filter(|worker| !worker.is_retiring())
            .count();
        for worker in self
            .workers
            .iter_mut()
            .rev()
            .filter(|worker| !worker.is_retiring())
            .take(active.saturating_sub(target))
        {
            worker.retire()?;
            changed = true;
        }
        let mut available_bytes = available_memory()?;
        let ram_blocked = !has_startup_reserve(available_bytes, reserve_mib);
        if !ram_blocked {
            // Count retiring processes until they exit, avoiding transient oversubscription.
            for _ in self.workers.len()..target {
                available_bytes = available_memory()?;
                if !has_startup_reserve(available_bytes, reserve_mib) {
                    break;
                }
                self.workers.push(worker::Worker::start(
                    &self.root,
                    &self.root.join("worker.ps1"),
                    self.owner_started_ms,
                    &self.job,
                )?);
                fs::write(self.root.join("heartbeat"), b"running")?;
                changed = true;
            }
        }
        if changed || self.last_publish.elapsed() >= Duration::from_secs(1) {
            fs::write(self.root.join("heartbeat"), b"running")?;
            let members: Vec<_> = self
                .workers
                .iter()
                .map(|worker| Member {
                    directory: worker.directory.clone(),
                    ready: worker.ready(),
                    retiring: worker.is_retiring(),
                })
                .collect();
            write_json(
                &self.root.join("manifest.json"),
                &Manifest { workers: members },
            )?;
            let ready = self.workers.iter().filter(|worker| worker.ready()).count();
            let busy = self
                .workers
                .iter()
                .filter(|worker| worker.directory.join("active.json").exists())
                .count();
            write_json(
                &self.root.join("status.json"),
                &json!({
                    "target":target,"processes":self.workers.len(),"ready":ready,
                    "starting":self.workers.len()-ready,"busy":busy,"ram_blocked":ram_blocked,
                    "retiring":self.workers.iter().filter(|worker| worker.is_retiring()).count(),
                    "reserve_mib":reserve_mib,"available_physical_bytes":available_bytes,
                }),
            )?;
            self.last_publish = Instant::now();
        }
        Ok(())
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        for worker in &mut self.workers {
            if let Err(error) = worker.retire() {
                eprintln!("Cannot retire conversion worker: {error:#}");
            }
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline
            && self
                .workers
                .iter()
                .any(|worker| !worker.directory.join("stopped").exists())
        {
            for worker in &mut self.workers {
                if worker.should_stop().unwrap_or(true) {
                    if let Err(error) = worker.stop() {
                        eprintln!("Cannot reap conversion worker: {error:#}");
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        for worker in &mut self.workers {
            if let Err(error) = worker.stop() {
                eprintln!("Conversion cleanup unconfirmed: {error:#}");
            }
        }
        let remaining = self
            .workers
            .iter()
            .filter(|worker| !worker.directory.join("stopped").exists())
            .count();
        if let Err(error) = write_json(
            &self.root.join("status.json"),
            &json!({
                "processes":remaining,"ready":0,"busy":0,"starting":0,"stopped":remaining==0,
            }),
        ) {
            eprintln!("Cannot publish converter shutdown: {error:#}");
        }
    }
}

fn open_lease(directory: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("lease.lock"))?)
}

fn read_json(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    super::artifact_cache::atomic_write(path, &serde_json::to_vec(value)?)
}

fn write_seed(path: &Path) -> Result<()> {
    let mut archive = zip::ZipWriter::new(File::create(path)?);
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Conversion" sheetId="1" r:id="rId1"/></sheets><calcPr calcMode="manual"/></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
        ),
    ];
    for (name, xml) in parts {
        archive.start_file(name, zip::write::FileOptions::default())?;
        archive.write_all(xml.as_bytes())?;
    }
    archive.finish()?;
    Ok(())
}

fn available_memory() -> Result<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: The structure has the required length and is writable.
    anyhow::ensure!(
        unsafe { GlobalMemoryStatusEx(&mut status) } != 0,
        "available Windows RAM could not be read"
    );
    Ok(status.ullAvailPhys.min(status.ullAvailPageFile))
}

fn has_startup_reserve(available_bytes: u64, reserve_mib: u64) -> bool {
    available_bytes >= reserve_mib.max(512).saturating_mul(1_048_576)
}
