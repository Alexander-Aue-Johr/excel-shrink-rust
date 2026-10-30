//! Keep the original and profile workbooks open throughout their comparisons.
//! Live images and hash-first captures share exact pixel comparison and evidence.
use super::*;
use crate::regression::archive::{batch_cleanup, opening_pause::OpeningPause};
use crate::regression::{artifact_cache, visual_compare, visual_validation};
use serde::Serialize;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

#[path = "resident_events.rs"]
mod events;
#[path = "resident_capture_live.rs"]
mod live;
#[path = "tall_sheet_analysis.rs"]
mod tall_sheet_analysis;
#[path = "tall_sheet_selection.rs"]
mod tall_sheet_selection;

#[path = "resident_capture_diagnosis.rs"]
mod diagnosis;

#[derive(Clone, Deserialize, Serialize)]
struct Tile {
    sheet_index: usize,
    sheet_name: String,
    tile_index: usize,
    range: String,
    core_rows: [u32; 2],
    capture_right_column: u32,
    width: u32,
    height: u32,
    #[serde(default)]
    x: u32,
    #[serde(default)]
    y: u32,
    #[serde(default)]
    capture_pixels: u64,
    #[serde(default)]
    pixel_budget: u64,
    #[serde(default)]
    capture_budget_fallback: Option<String>,
    sha256: String,
}

#[derive(Clone, Deserialize)]
struct Snapshot {
    heights: Vec<SheetHeights>,
    tiles: Vec<Tile>,
    #[serde(skip)]
    command: String,
}

pub(in crate::regression) struct ResidentCapture {
    original: PathBuf,
    directory: PathBuf,
    child: Child,
    events: events::ResidentEvents,
    timings: live::PipelineTimings,
    plans: BTreeMap<String, PairedPlan>,
    configs: BTreeMap<String, Vec<Value>>,
    aliases: BTreeMap<String, String>,
    reusable_clients: BTreeMap<String, String>,
    closed: BTreeSet<String>,
    physical_closed: BTreeSet<String>,
    initial_aligned: bool,
    sweep_mode: bool,
    screening_mode: bool,
    screening_pass: usize,
    comparisons: BTreeMap<(String, String, String), Value>,
    snapshots: BTreeMap<String, Snapshot>,
    compared_originals: BTreeMap<String, Snapshot>,
    pending: BTreeMap<String, String>,
    images: BTreeMap<(String, String, usize, usize), PathBuf>,
    sequence: usize,
    timeout: Duration,
    finished: bool,
    cleanup_barrier: Option<batch_cleanup::Participant>,
    live: Option<live::LiveCapture>,
    variants: BTreeMap<String, PathBuf>,
    capture_coverage: BTreeMap<usize, Value>,
    drawing_bounds: BTreeMap<String, Value>,
}

fn publish(path: &Path, value: &Value) -> Result<()> {
    artifact_cache::atomic_write(path, &serde_json::to_vec(value)?)
}

// Byte equality alone cannot certify volatile calculations or refresh-on-open.
// Keep this certificate identical to prepared::reusable_visual_manifest.
fn reusable_capture_manifest(manifest: &artifact_cache::PackageManifest) -> bool {
    !manifest.parts.is_empty()
        && manifest
            .parts
            .values()
            .all(|part| !part.visual_runtime && part.recalc_cacheable)
}

struct CaptureClients {
    aliases: BTreeMap<String, String>,
    reusable: BTreeMap<String, String>,
}

fn capture_clients(inputs: &[(String, PathBuf)]) -> Result<CaptureClients> {
    let hashed: Vec<_> = inputs
        .iter()
        .map(|(name, path)| Ok((name.clone(), path, artifact_cache::hash_file(path)?)))
        .collect::<Result<_>>()?;
    let mut certified = BTreeMap::new();
    for (_, path, hash) in &hashed {
        if !certified.contains_key(hash) {
            let safe = artifact_cache::manifest(path).is_ok_and(|manifest| {
                manifest.archive_sha256 == *hash && reusable_capture_manifest(&manifest)
            });
            certified.insert(hash.clone(), safe);
        }
    }
    let aliases = aliases_for_certificates(
        hashed
            .iter()
            .map(|(name, _, hash)| (name.as_str(), hash.as_str(), certified[hash])),
    );
    // A later whitespace control can match a client whose bytes were unique at
    // startup. Preserve its certificate too, rather than reopening the same file.
    let reusable = hashed
        .iter()
        .filter(|(_, _, hash)| certified[hash])
        .map(|(name, _, hash)| (hash.clone(), aliases[name].clone()))
        .collect();
    Ok(CaptureClients { aliases, reusable })
}

fn aliases_for_certificates<'a>(
    inputs: impl IntoIterator<Item = (&'a str, &'a str, bool)>,
) -> BTreeMap<String, String> {
    let mut shared = BTreeMap::<String, String>::new();
    let mut aliases = BTreeMap::new();
    for (name, hash, safe) in inputs {
        let client = if safe {
            shared
                .entry(hash.into())
                .or_insert_with(|| name.into())
                .clone()
        } else {
            name.into()
        };
        aliases.insert(name.into(), client);
    }
    aliases
}

fn union_tile_rows<'a>(
    snapshots: impl IntoIterator<Item = &'a Snapshot>,
) -> BTreeMap<usize, Vec<[u32; 2]>> {
    let mut cuts = BTreeMap::<usize, BTreeSet<u32>>::new();
    for snapshot in snapshots {
        for tile in &snapshot.tiles {
            cuts.entry(tile.sheet_index)
                .or_default()
                .extend(tile.core_rows);
        }
    }
    cuts.into_iter()
        .map(|(sheet, cuts)| {
            let cuts: Vec<_> = cuts.into_iter().collect();
            (
                sheet,
                cuts.windows(2).map(|span| [span[0], span[1]]).collect(),
            )
        })
        .collect()
}

fn has_tile_rows(snapshot: &Snapshot, rows: &BTreeMap<usize, Vec<[u32; 2]>>) -> bool {
    rows.iter().all(|(sheet, expected)| {
        snapshot
            .tiles
            .iter()
            .filter(|tile| tile.sheet_index == *sheet)
            .map(|tile| tile.core_rows)
            .eq(expected.iter().copied())
    })
}

fn select_mismatch_snapshot(original: &Snapshot, primary: &Value) -> Result<Snapshot> {
    let ranges: BTreeSet<(usize, String)> = primary["sheets"]
        .as_array()
        .context("missing primary sheets")?
        .iter()
        .filter(|sheet| {
            sheet["row_heights"]
                .as_array()
                .is_none_or(|rows| rows.is_empty())
        })
        .flat_map(|sheet| {
            sheet["image_differences"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |difference| {
                    Some((
                        sheet["index"].as_u64()? as usize,
                        difference["range"].as_str()?.to_owned(),
                    ))
                })
        })
        .collect();
    ensure!(
        !ranges.is_empty(),
        "no mismatch tiles selected for whitespace control"
    );
    let tiles: Vec<_> = original
        .tiles
        .iter()
        .filter(|tile| ranges.contains(&(tile.sheet_index, tile.range.clone())))
        .cloned()
        .collect();
    ensure!(
        tiles.len() == ranges.len(),
        "primary mismatch ranges no longer match their original capture"
    );
    let sheets: BTreeSet<_> = tiles.iter().map(|tile| tile.sheet_index).collect();
    Ok(Snapshot {
        command: original.command.clone(),
        tiles,
        heights: original
            .heights
            .iter()
            .filter(|sheet| sheets.contains(&sheet.sheet_index))
            .cloned()
            .collect(),
    })
}

fn select_existing_snapshot(captured: &Snapshot, requested: &Snapshot) -> Result<Snapshot> {
    let mut tiles = Vec::with_capacity(requested.tiles.len());
    for target in &requested.tiles {
        let tile = captured
            .tiles
            .iter()
            .find(|tile| {
                tile.sheet_index == target.sheet_index
                    && tile.sheet_name == target.sheet_name
                    && tile.range == target.range
                    && tile.core_rows == target.core_rows
                    && tile.capture_right_column == target.capture_right_column
            })
            .context("resident control does not cover the original mismatch range")?;
        tiles.push(tile.clone());
    }
    let sheets: BTreeSet<_> = tiles.iter().map(|tile| tile.sheet_index).collect();
    Ok(Snapshot {
        command: captured.command.clone(),
        tiles,
        heights: captured
            .heights
            .iter()
            .filter(|sheet| sheets.contains(&sheet.sheet_index))
            .cloned()
            .collect(),
    })
}

fn fingerprint_control_snapshot(snapshot: &Snapshot, bounds: &[Value]) -> Result<String> {
    let tiles: Vec<_> = snapshot
        .tiles
        .iter()
        .map(|tile| {
            json!({
                "sheet_index":tile.sheet_index,"sheet_name":tile.sheet_name,"range":tile.range,
                "core_rows":tile.core_rows,"capture_right_column":tile.capture_right_column,
                "width":tile.width,"height":tile.height,"sha256":tile.sha256,
            })
        })
        .collect();
    Ok(artifact_cache::sha256(&serde_json::to_vec(&json!({
        "version":1,"method":"original mismatch snapshots","tiles":tiles,
        "row_heights":snapshot.heights,"bounds":bounds,
    }))?))
}

fn screen_sheet_hashes(original: &Snapshot, variant: &Snapshot) -> Result<Vec<Value>> {
    ensure!(
        same_ranges(original, variant),
        "screening tile ranges differ"
    );
    let indexes: BTreeSet<_> = original.tiles.iter().map(|tile| tile.sheet_index).collect();
    let mut sheets = Vec::new();
    for index in indexes {
        let left: Vec<_> = original
            .tiles
            .iter()
            .filter(|tile| tile.sheet_index == index)
            .collect();
        let right: Vec<_> = variant
            .tiles
            .iter()
            .filter(|tile| tile.sheet_index == index)
            .collect();
        let left_heights = original
            .heights
            .iter()
            .find(|sheet| sheet.sheet_index == index);
        let right_heights = variant
            .heights
            .iter()
            .find(|sheet| sheet.sheet_index == index);
        ensure!(
            left_heights.is_some() && right_heights.is_some(),
            "screening sheet has no row heights"
        );
        let original_sha256 = artifact_cache::sha256(&serde_json::to_vec(&json!({
            "tiles": left, "row_heights": left_heights,
        }))?);
        let variant_sha256 = artifact_cache::sha256(&serde_json::to_vec(&json!({
            "tiles": right, "row_heights": right_heights,
        }))?);
        sheets.push(json!({
            "sheet_index": index,
            "sheet_name": left[0].sheet_name,
            "tile_count": left.len(),
            "tiles": left.iter().zip(&right).map(|(original, variant)| json!({
                "tile_index": original.tile_index,
                "range": original.range,
                "core_rows": original.core_rows,
                "width": original.width,
                "height": original.height,
                "original_sha256": original.sha256,
                "variant_sha256": variant.sha256,
            })).collect::<Vec<_>>(),
            "original_sha256": original_sha256,
            "variant_sha256": variant_sha256,
        }));
    }
    Ok(sheets)
}

impl ResidentCapture {
    /// Capture selected negotiated tiles twice while both Excel workbooks remain open.
    /// The returned hashes are screening evidence, not a pixel comparison result.
    /// Captures both open workbooks twice, with a Full recalculation between sweeps.
    ///
    /// Returns stable tile and sheet hashes for the selected profile. An unstable
    /// sweep cannot be treated as equal during preparation screening.
    pub(in crate::regression) fn screen_twice(&mut self, profile: &str) -> Result<Value> {
        self.align_initial_clients()?;
        let first_original = self.snapshot("original")?;
        let first_variant = self.snapshot(profile)?;
        let clients: BTreeSet<_> = self.aliases.values().cloned().collect();
        // A second screenshot without calculation would miss formulas whose
        // display changes only when Excel recalculates the already open file.
        let mut calculations = Vec::new();
        for client in &clients {
            let command = self.issue(
                client,
                json!({
                    "kind": "diagnosis", "operation": "recalculate",
                }),
            )?;
            calculations.push((client.clone(), command));
        }
        for (client, command) in calculations {
            self.wait(&client, &command)?;
        }
        self.screening_pass = 2;
        self.capture_selected_sweep(&clients)?;
        let second_original = self.snapshot("original")?;
        let second_variant = self.snapshot(profile)?;
        let mut first = screen_sheet_hashes(&first_original, &first_variant)?;
        let mut second = screen_sheet_hashes(&second_original, &second_variant)?;
        for sheet in first.iter_mut().chain(second.iter_mut()) {
            let index = sheet["sheet_index"].as_u64().context("missing screening sheet index")?;
            if let Some(coverage) = self.capture_coverage.get(&(index as usize)) {
                sheet["capture_coverage"] = coverage.clone();
            }
        }
        let stable = first == second;
        let changed_sheets: Vec<_> = second
            .iter()
            .filter(|sheet| sheet["original_sha256"] != sheet["variant_sha256"])
            .map(|sheet| sheet["sheet_index"].clone())
            .collect();
        Ok(json!({
            "version": 1,
            "capture_policy": "tall-sheet-v1",
            "profile": profile,
            "hash_algorithm": "SHA-256",
            "hash_domain": "ordered tile metadata, finalized PNG hashes and row heights",
            "calculation_between_sweeps": "Excel Full",
            "first_sheets": first,
            "sheets": second,
            "stable": stable,
            "changed_sheets": changed_sheets,
            "global_sha256": artifact_cache::sha256(&serde_json::to_vec(&json!({"first":first,"second":second}))?),
        }))
    }

    pub(in crate::regression) fn start(
        original: &Path,
        variants: &[(&str, PathBuf)],
        directory: &Path,
    ) -> Result<Self> {
        Self::start_inner(original, variants, directory, false, None, 0)
    }

    /// Starts a capture that may open targeted follow-up profiles after the first comparison.
    ///
    /// `additional_profiles` reserves their Excel slots with the initial clients so
    /// concurrent groups cannot each hold a partial allocation while waiting.
    pub(in crate::regression) fn start_prioritized(
        original: &Path,
        variants: &[(&str, PathBuf)],
        directory: &Path,
        sheets: Option<&BTreeSet<usize>>,
        additional_profiles: usize,
    ) -> Result<Self> {
        Self::start_inner(
            original,
            variants,
            directory,
            false,
            sheets,
            additional_profiles,
        )
    }

    /// Opens the original and one prepared profile for two capture sweeps in one session.
    pub(in crate::regression) fn start_screening(
        original: &Path,
        profile: &str,
        variant: &Path,
        directory: &Path,
    ) -> Result<Self> {
        Self::start_inner(
            original,
            &[(profile, variant.to_owned())],
            directory,
            true,
            None,
            0,
        )
    }

    fn start_inner(
        original: &Path,
        variants: &[(&str, PathBuf)],
        directory: &Path,
        screening: bool,
        selected_sheets: Option<&BTreeSet<usize>>,
        additional_profiles: usize,
    ) -> Result<Self> {
        ensure!(!variants.is_empty(), "resident comparison has no profiles");
        let cleanup_barrier = batch_cleanup::Participant::from_environment()?;
        ensure!(
            !cleanup_barrier
                .as_ref()
                .is_some_and(|barrier| barrier.restart_requested()),
            "resident comparison restart requested before startup"
        );
        fs::create_dir_all(directory)?;
        let directory = fs::canonicalize(directory)?;
        ensure!(
            !directory.join("clients").exists(),
            "resident session already exists"
        );
        let mut plans = BTreeMap::new();
        for (name, path) in variants {
            let mut plan = plan_pair(original, path)?;
            if let Some(selected) = selected_sheets {
                plan.sheets
                    .retain(|sheet| selected.contains(&sheet.sheet_index));
                ensure!(
                    !plan.sheets.is_empty(),
                    "screening selected no visible sheets"
                );
            }
            plans.insert((*name).to_owned(), plan);
        }
        // Every client uses the union of the required bounds. Its actual row
        // geometry is measured independently; differing tile cuts are reconciled.
        let mut union = plans.values().next().unwrap().clone();
        for plan in plans.values() {
            for (target, sheet) in union.sheets.iter_mut().zip(&plan.sheets) {
                ensure!(
                    target.sheet_index == sheet.sheet_index,
                    "profile sheet order changed"
                );
                target.bottom_row = target.bottom_row.max(sheet.bottom_row);
                target.row_height_bottom_row = target
                    .row_height_bottom_row
                    .max(sheet.row_height_bottom_row);
                target.capture_right_column =
                    target.capture_right_column.max(sheet.capture_right_column);
            }
        }
        for plan in plans.values_mut() {
            for (sheet, bounds) in plan.sheets.iter_mut().zip(&union.sheets) {
                sheet.bottom_row = bounds.bottom_row;
                sheet.row_height_bottom_row = bounds.row_height_bottom_row;
                sheet.capture_right_column = bounds.capture_right_column;
                for config in [&mut sheet.original_config, &mut sheet.shrunk_config] {
                    config["bottom_row"] = json!(bounds.bottom_row);
                    config["capture_right_column"] = json!(bounds.capture_right_column);
                }
            }
        }
        let first = plans.values().next().unwrap();
        let mut configs = BTreeMap::from([(
            "original".into(),
            first
                .sheets
                .iter()
                .map(|s| s.original_config.clone())
                .collect::<Vec<_>>(),
        )]);
        for (name, plan) in &plans {
            configs.insert(
                name.clone(),
                plan.sheets
                    .iter()
                    .map(|s| s.shrunk_config.clone())
                    .collect(),
            );
        }
        let inputs: Vec<_> = std::iter::once(("original".to_owned(), original.to_owned()))
            .chain(
                variants
                    .iter()
                    .map(|(name, path)| ((*name).to_owned(), path.clone())),
            )
            .collect();
        let CaptureClients {
            aliases,
            reusable: reusable_clients,
        } = capture_clients(&inputs)?;
        let reserved_clients =
            aliases.values().collect::<BTreeSet<_>>().len() + additional_profiles;
        ensure!(
            reserved_clients <= 16,
            "resident session exceeded 16 clients"
        );
        publish(
            &directory.join("capacity.json"),
            &json!({"reserved_clients": reserved_clients}),
        )?;
        let live = live::LiveCapture::from_environment(&directory, original, &aliases)?;
        tall_sheet_selection::prepare_recalc_inputs(
            original,
            variants,
            &directory,
            &union,
            live.as_ref(),
        )?;
        let mut pending = BTreeMap::new();
        for (name, path) in &inputs {
            if aliases[name] != *name {
                continue;
            }
            publish(
                &directory.join("clients").join(format!("{name}.json")),
                &json!({
                    "client_id":name,"source_path":windows_path(&path)?,"workbook":format!("{name}.xlsx"),"sheets":configs[name]
                }),
            )?;
            let command = "000001";
            publish(
                &directory
                    .join("commands")
                    .join(name)
                    .join(format!("{command}.json")),
                &json!({
                    "id":command,"kind":"plan","sheets":configs[name],
                    "queued_unix_ms":chrono::Utc::now().timestamp_millis(),
                    "tile_megapixels":live.as_ref().map_or(32, |live| live.tile_megapixels())
                }),
            )?;
            pending.insert(name.to_owned(), command.to_owned());
        }
        let timeout = std::env::var("EXCEL_SHRINK_CAPTURE_TIMEOUT_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1800);
        let script =
            crate::regression::repo_root().join("excel-shrink-cli/office-capture/resident.ps1");
        let mut command = Command::new("powershell.exe");
        let startup =
            std::env::var("EXCEL_SHRINK_CAPTURE_STARTUP_SECONDS").unwrap_or_else(|_| "300".into());
        command
            .args([
                "-STA",
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(windows_path(&script)?)
            .arg("-HostExecutable")
            .arg(windows_path(&crate::capture_host::executable()?)?)
            .arg("-SessionPath")
            .arg(windows_path(&directory)?)
            .arg("-StartupTimeoutSeconds")
            .arg(startup)
            .arg("-CaptureTimeoutSeconds")
            .arg(timeout.to_string())
            .stdout(Stdio::from(fs::File::create(
                directory.join("launcher.stdout.log"),
            )?))
            .stderr(Stdio::from(fs::File::create(
                directory.join("launcher.stderr.log"),
            )?));
        if std::env::var("EXCEL_SHRINK_HIDE_EXCEL").as_deref() == Ok("1") {
            command.arg("-HideExcel");
        } else if std::env::var("EXCEL_SHRINK_RESIDENT_VISIBLE").as_deref() == Ok("1") {
            command.arg("-Visible");
        }
        if let Some(barrier) = &cleanup_barrier {
            ensure!(
                !barrier.restart_requested(),
                "resident comparison restart requested before Excel launch"
            );
            publish(
                &directory.join("batch-lifetime.json"),
                &serde_json::to_value(barrier)?,
            )?;
        }
        let event_token = uuid::Uuid::new_v4().simple().to_string();
        publish(
            &directory.join("coordinator-token.json"),
            &json!({"token":event_token}),
        )?;
        let events = events::ResidentEvents::start(&directory, event_token)?;
        let child = command.spawn().context("start resident Excel capture")?;
        Ok(Self {
            original: original.to_owned(),
            directory,
            child,
            events,
            timings: live::PipelineTimings::default(),
            plans,
            configs,
            aliases,
            reusable_clients,
            closed: BTreeSet::new(),
            physical_closed: BTreeSet::new(),
            initial_aligned: false,
            sweep_mode: screening
                || std::env::var("EXCEL_SHRINK_CAPTURE_SWEEP").as_deref() == Ok("1"),
            screening_mode: screening,
            screening_pass: 1,
            comparisons: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            compared_originals: BTreeMap::new(),
            pending,
            images: BTreeMap::new(),
            sequence: 1,
            timeout: Duration::from_secs(timeout + 1200),
            finished: false,
            cleanup_barrier,
            live,
            variants: variants
                .iter()
                .map(|(name, path)| ((*name).to_owned(), path.clone()))
                .collect(),
            capture_coverage: BTreeMap::new(),
            drawing_bounds: BTreeMap::new(),
        })
    }

    fn client(&self, logical: &str) -> Result<&str> {
        self.aliases
            .get(logical)
            .map(String::as_str)
            .context("unknown resident client")
    }

    /// A launcher snapshot is available before cleanup, including on a failed
    /// comparison. It records only measurements actually completed by clients.
    pub(in crate::regression) fn startup_metrics(&self) -> Result<Option<Value>> {
        let path = self.directory.join("metrics.json");
        if !path.is_file() {
            return Ok(None);
        }
        let mut metrics: Value = serde_json::from_slice(&fs::read(path)?)?;
        metrics["capture_client_aliases"] = json!(self.aliases);
        metrics["event_transport"] = self.events.snapshot();
        metrics["pipeline_timings"] = serde_json::to_value(&self.timings)?;
        if let Some(live) = &self.live {
            metrics["capture_pipeline"] = live.capture_pipeline_metadata();
        }
        Ok(Some(metrics))
    }

    fn issue(&mut self, client: &str, mut body: Value) -> Result<String> {
        self.check_restart()?;
        let client = self.client(client)?.to_owned();
        ensure!(
            !self.physical_closed.contains(&client),
            "resident client already closed"
        );
        self.sequence += 1;
        let id = format!("{:06}", self.sequence);
        if self.screening_mode && body["kind"] == "capture" {
            body["progress_pass"] = json!(self.screening_pass);
            body["progress_passes"] = json!(2);
        }
        body["id"] = json!(id);
        body["queued_unix_ms"] = json!(chrono::Utc::now().timestamp_millis());
        crate::regression::archive::work_state::event(
            "waiting_client",
            "command_queued",
            "Capture command queued for Excel client",
            false,
            Some("Office.js command result"),
            json!({"client":client,"command":id,"kind":body["kind"]}),
        );
        publish(
            &self
                .directory
                .join("commands")
                .join(client)
                .join(format!("{id}.json")),
            &body,
        )?;
        self.events.command_published();
        Ok(id)
    }

    fn check_restart(&self) -> Result<()> {
        ensure!(
            !self
                .cleanup_barrier
                .as_ref()
                .is_some_and(|barrier| barrier.restart_requested()),
            "resident comparison restart requested"
        );
        Ok(())
    }

    fn wait(&mut self, client: &str, command: &str) -> Result<Value> {
        let client = self.client(client)?.to_owned();
        let response = self
            .directory
            .join("responses")
            .join(&client)
            .join(format!("{command}.json"));
        let mut clock = OpeningPause::new(
            self.cleanup_barrier
                .as_ref()
                .and_then(|barrier| barrier.pause_path.as_deref()),
        );
        let mut sampled = Instant::now();
        let mut user_pause = Duration::ZERO;
        let dispatched = self
            .directory
            .join("dispatched")
            .join(&client)
            .join(format!("{command}.json"));
        let mut command_started = self.live.is_none();
        loop {
            self.check_restart()?;
            // The launcher owns opening deadlines. Queued jobs do not consume
            // their response budget while Excel is still opening another peer.
            if !command_started
                && (dispatched.is_file() || live::client_ready_for_jobs(&self.directory, &client))
            {
                clock = OpeningPause::new(
                    self.cleanup_barrier
                        .as_ref()
                        .and_then(|barrier| barrier.pause_path.as_deref()),
                );
                sampled = Instant::now();
                user_pause = Duration::ZERO;
                command_started = true;
            }
            let now = Instant::now();
            if self.live.as_ref().is_some_and(|live| live.paused()) {
                user_pause += now.duration_since(sampled);
            }
            sampled = now;
            let failure = self.directory.join("launcher-error.json");
            if failure.is_file() {
                bail!(
                    "resident Excel launcher failed: {}",
                    fs::read_to_string(failure)?
                );
            }
            if response.exists() {
                let result: Value = serde_json::from_slice(&fs::read(&response)?)?;
                self.timings.commands_completed += 1;
                ensure!(
                    result["ok"] == true,
                    "resident {client}/{command}: {}",
                    result["error"]
                );
                crate::regression::archive::work_state::event(
                    "processing_result",
                    "command_complete",
                    "Excel client command completed",
                    true,
                    None,
                    json!({"client":client,"command":command}),
                );
                return Ok(result);
            }
            if let Some(status) = self.child.try_wait()? {
                crate::regression::launcher_journal::mark_exited(&mut self.child)?;
                let detail = fs::read_to_string(self.directory.join("launcher-result.json"))
                    .unwrap_or_default();
                bail!("resident Excel launcher exited ({status}): {detail}");
            }
            let limit = self
                .live
                .as_ref()
                .map_or(self.timeout, |live| live.job_timeout(self.timeout));
            ensure!(
                !command_started || clock.active_elapsed().saturating_sub(user_pause) < limit,
                "resident command deadline exceeded: {client}/{command}; {:.1}s wall time, {:.1}s opening pause",
                clock.wall_elapsed().as_secs_f64(),
                clock.credited().as_secs_f64()
            );
            let waiting = Instant::now();
            self.events.wait()?;
            self.timings.wait_ms += waiting.elapsed().as_secs_f64() * 1000.0;
        }
    }

    fn snapshot(&mut self, logical: &str) -> Result<Snapshot> {
        let client = self.client(logical)?.to_owned();
        if let Some(command) = self.pending.remove(&client) {
            let result = self.wait(&client, &command)?;
            let drawing_bounds = json!(result["sheets"].as_array().into_iter().flatten()
                .map(|sheet| json!({"sheet_index":sheet["sheet_index"],
                    "drawing_bounds":sheet["drawing_bounds"]})).collect::<Vec<_>>());
            if self.capture_coverage.values().any(|coverage|
                coverage["omitted_tiles"].as_u64().unwrap_or(0) > 0)
            {
                if let Some(previous) = self.drawing_bounds.get(&client) {
                    ensure!(previous == &drawing_bounds,
                        "Drawing bounds changed after capture selection; omitted ranges require a full-capture retry");
                }
            }
            self.drawing_bounds.insert(client.clone(), drawing_bounds);
            let mut snapshot: Snapshot = serde_json::from_value(result)?;
            snapshot.command = command;
            validate_snapshot(&snapshot, &self.configs[&client])?;
            self.snapshots.insert(client.clone(), snapshot);
        }
        self.snapshots
            .get(&client)
            .cloned()
            .context("resident client has no capture")
    }

    fn recapture(&mut self, logical: &str, configs: Vec<Value>) -> Result<()> {
        let client = self.client(logical)?.to_owned();
        let id = self.issue(
            &client,
            json!({"kind":"plan","sheets":configs,
                "tile_megapixels":self.live.as_ref().map_or(32, |live| live.tile_megapixels())}),
        )?;
        self.configs.insert(client.clone(), configs);
        self.pending.insert(client, id);
        Ok(())
    }

    fn align_initial_clients(&mut self) -> Result<()> {
        if self.initial_aligned {
            return Ok(());
        }
        if let Some(live) = &mut self.live {
            live.planning()?;
        }
        let clients: BTreeSet<_> = self.aliases.values().cloned().collect();
        // Initial captures still run as soon as each client is online. Once all
        // offers arrive, one common refinement serves every profile comparison.
        let mut offered = BTreeMap::new();
        for client in &clients {
            offered.insert(client.clone(), self.snapshot(client)?);
        }
        let mut ends = BTreeMap::<usize, u32>::new();
        for snapshot in offered.values() {
            for tile in &snapshot.tiles {
                ends.entry(tile.sheet_index)
                    .and_modify(|end| *end = (*end).max(tile.capture_right_column))
                    .or_insert(tile.capture_right_column);
            }
        }
        let mut widened = Vec::new();
        for (client, snapshot) in &offered {
            if snapshot
                .tiles
                .iter()
                .all(|tile| ends[&tile.sheet_index] == tile.capture_right_column)
            {
                continue;
            }
            let mut configs = self.configs[client].clone();
            for config in &mut configs {
                let index = config["index"]
                    .as_u64()
                    .context("missing worksheet index")? as usize;
                config["capture_end_column"] = json!(ends[&index]);
            }
            // Wider captures may need smaller tiles. Obtain their natural cuts
            // before negotiating the one common row partition for all profiles.
            self.recapture(client, configs)?;
            widened.push(client.clone());
        }
        for client in widened {
            offered.insert(client.clone(), self.snapshot(&client)?);
        }
        let rows = union_tile_rows(offered.values());
        for (client, snapshot) in &offered {
            if has_tile_rows(snapshot, &rows) {
                continue;
            }
            let mut configs = self.configs[client].clone();
            for config in &mut configs {
                let index = config["index"]
                    .as_u64()
                    .context("missing worksheet index")? as usize;
                config["capture_end_column"] = json!(ends[&index]);
                config["tile_rows"] =
                    json!(rows.get(&index).context("missing common sheet partition")?);
            }
            self.recapture(client, configs)?;
        }
        let original = self.snapshot("original")?;
        for client in &clients {
            ensure!(
                same_ranges(&original, &self.snapshot(client)?),
                "resident tile partitions still differ"
            );
        }
        self.select_tall_sheet_captures(&clients)?;
        if self.sweep_mode {
            self.capture_selected_sweep(&clients)?;
            self.initial_aligned = true;
            return Ok(());
        }
        if self.live.is_some() {
            self.capture_synchronized(&clients)?;
        } else {
            // The headless digest transport also plans before taking any images.
            // Ask for only the selected row bands, preserving the same policy.
            for client in &clients {
                let selected = self.snapshots[client].clone();
                let mut configs = self.configs[client].clone();
                for config in &mut configs {
                    let index = config["index"].as_u64().unwrap() as usize;
                    let tiles: Vec<_> = selected
                        .tiles
                        .iter()
                        .filter(|t| t.sheet_index == index)
                        .collect();
                    config["tile_rows"] =
                        json!(tiles.iter().map(|t| t.core_rows).collect::<Vec<_>>());
                    config["capture_end_column"] = json!(tiles[0].capture_right_column);
                }
                let id = self.issue(
                    client,
                    json!({"kind":"capture","sheets":configs,"tile_megapixels":32}),
                )?;
                self.configs.insert(client.clone(), configs);
                self.pending.insert(client.clone(), id);
            }
            for client in &clients {
                self.snapshot(client)?;
            }
        }
        self.initial_aligned = true;
        Ok(())
    }

    fn aligned(&mut self, profile: &str) -> Result<(Snapshot, Snapshot)> {
        self.align_initial_clients()?;
        Ok((self.snapshot("original")?, self.snapshot(profile)?))
    }

    fn request_images(
        &mut self,
        logical: &str,
        snapshot: &Snapshot,
        tiles: &[Tile],
    ) -> Result<Option<String>> {
        let client = self.client(logical)?.to_owned();
        let missing: Vec<_> = tiles
            .iter()
            .filter(|tile| {
                !self.images.contains_key(&(
                    client.clone(),
                    snapshot.command.clone(),
                    tile.sheet_index,
                    tile.tile_index,
                ))
            })
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(None);
        }
        ensure!(
            self.live.is_none() || self.sweep_mode,
            "image-first comparison lost a retained evidence tile"
        );
        self.issue(
            &client,
            json!({"kind":"images","capture_id":snapshot.command,"tiles":missing}),
        )
        .map(Some)
    }

    fn receive_images(
        &mut self,
        logical: &str,
        snapshot: &Snapshot,
        command: Option<String>,
        requested: &[Tile],
    ) -> Result<()> {
        let Some(command) = command else {
            return Ok(());
        };
        let client = self.client(logical)?.to_owned();
        let result = self.wait(&client, &command)?;
        self.receive_image_result(logical, snapshot, requested, &result)
    }

    fn receive_image_result(
        &mut self,
        logical: &str,
        snapshot: &Snapshot,
        requested: &[Tile],
        result: &Value,
    ) -> Result<()> {
        let client = self.client(logical)?.to_owned();
        for item in result["images"]
            .as_array()
            .context("missing resident images")?
        {
            let tile: Tile = serde_json::from_value(item.clone())?;
            let expected = requested
                .iter()
                .find(|t| t.sheet_index == tile.sheet_index && t.tile_index == tile.tile_index)
                .context("unrequested resident image")?;
            ensure!(
                tile.range == expected.range
                    && tile.sha256 == expected.sha256
                    && tile.width == expected.width
                    && tile.height == expected.height,
                "image receipt differs from digest offer"
            );
            let path =
                fs::canonicalize(item["image_path"].as_str().context("missing image path")?)?;
            ensure!(
                path.starts_with(self.directory.join("images")),
                "resident image outside session"
            );
            ensure!(
                fs::metadata(&path)?.len() <= 512 * 1024 * 1024,
                "resident PNG too large"
            );
            ensure!(
                artifact_cache::hash_file(&path)? == tile.sha256,
                "resident PNG digest mismatch"
            );
            let dimensions = image::ImageReader::open(&path)?
                .with_guessed_format()?
                .into_dimensions()?;
            ensure!(
                dimensions == (tile.width, tile.height),
                "resident PNG dimensions mismatch"
            );
            self.timings.images_received += 1;
            for (name, total) in [
                ("upload_receive_ms", &mut self.timings.upload_receive_ms),
                (
                    "upload_validate_publish_ms",
                    &mut self.timings.upload_validate_publish_ms,
                ),
            ] {
                if let Some(value) = item["timings"][name]
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                {
                    *total += value;
                }
            }
            self.images.insert(
                (
                    client.clone(),
                    snapshot.command.clone(),
                    tile.sheet_index,
                    tile.tile_index,
                ),
                path,
            );
        }
        for tile in requested {
            ensure!(
                self.images.contains_key(&(
                    client.clone(),
                    snapshot.command.clone(),
                    tile.sheet_index,
                    tile.tile_index
                )),
                "missing requested PNG"
            );
        }
        Ok(())
    }

    fn compare_snapshots(
        &mut self,
        client: &str,
        plan: &PairedPlan,
        left: &Snapshot,
        right: &Snapshot,
        output: &Path,
    ) -> Result<PairedVisualResult> {
        ensure!(
            same_ranges(left, right),
            "cannot compare different resident tile ranges"
        );
        let physical = self.client(client)?.to_owned();
        let extents: Vec<_> = plan
            .sheets
            .iter()
            .map(|s| SheetExtent {
                index: s.sheet_index,
                name: s.sheet_name.clone(),
                last_row: s.bottom_row + 1,
                last_checked_row: s.row_height_bottom_row + 1,
            })
            .collect();
        let heights = row_heights::compare_row_heights(&left.heights, &right.heights, &extents)?;
        let height_tile = |a: &Tile| {
            heights.iter().any(|h| {
                h.sheet_index == a.sheet_index
                    && h.row <= a.core_rows[1]
                    && h.end_row > a.core_rows[0]
            })
        };
        let need = |a: &Tile, b: &Tile| {
            height_tile(a) || a.sha256 != b.sha256 || a.width != b.width || a.height != b.height
        };
        let a: Vec<_> = left
            .tiles
            .iter()
            .zip(&right.tiles)
            .filter(|(a, b)| need(a, b))
            .map(|(a, _)| a.clone())
            .collect();
        let b: Vec<_> = left
            .tiles
            .iter()
            .zip(&right.tiles)
            .filter(|(a, b)| need(a, b))
            .map(|(_, b)| b.clone())
            .collect();
        let first = self.request_images("original", left, &a)?;
        let second = self.request_images(client, right, &b)?;
        self.receive_images("original", left, first, &a)?;
        self.receive_images(client, right, second, &b)?;
        let mut tiles = Vec::new();
        let mut differences = Vec::new();
        for (a, b) in left.tiles.iter().zip(&right.tiles) {
            let mut tile = PairedImageTile {
                sheet_index: a.sheet_index,
                sheet_name: a.sheet_name.clone(),
                range: a.range.clone(),
                tile_index: a.tile_index,
                original_image: PathBuf::new(),
                shrunk_image: PathBuf::new(),
            };
            if need(a, b) {
                let ap = &self.images[&(
                    "original".into(),
                    left.command.clone(),
                    a.sheet_index,
                    a.tile_index,
                )];
                let bp = &self.images[&(
                    physical.clone(),
                    right.command.clone(),
                    b.sheet_index,
                    b.tile_index,
                )];
                if height_tile(a) {
                    tile.original_image = ap.clone();
                    tile.shrunk_image = bp.clone();
                }
                if !height_tile(a) || self.sweep_mode {
                    let ai = image::open(ap)?.to_rgba8();
                    let bi = image::open(bp)?.to_rgba8();
                    differences.extend(visual_compare::compare_tile_images(
                        &tile,
                        &ai,
                        &bi,
                        &output.join("differences"),
                    )?);
                }
            }
            tiles.push(tile);
        }
        fs::create_dir_all(output)?;
        let transport = self.live.as_ref().map(|live| live.capture_transport());
        // On-demand controls already target known mismatches and always send
        // their images. Only primary profile captures can skip equal uploads.
        let image_first = self.live.is_some()
            && !self.sweep_mode
            && (transport == Some("live-images") || !self.plans.contains_key(client));
        let method = if self.sweep_mode {
            "autonomous PNG SHA-256 sweep; exact verified mismatch replay"
        } else {
            match (transport, image_first) {
                (Some("hash-first"), false) => {
                    "retained PNG SHA-256; host exact pixels for mismatches; adjacent context"
                }
                (Some(_), _) => "PNG uploads; host exact pixels",
                (None, _) => "resident PNG SHA-256 then exact pixels",
            }
        };
        fs::write(
            output.join("transport.json"),
            serde_json::to_vec_pretty(&json!({
                "method":method, "tiles":left.tiles.len(),
                "client_checksum_tiles":if image_first {0} else {left.tiles.len()+right.tiles.len()},
                "image_first":image_first,
                "primary_capture_transport":transport,
                "requested_image_pairs":a.len(),"hash_equal_tiles":left.tiles.iter().zip(&right.tiles)
                    .filter(|(a,b)|a.sha256 == b.sha256 && a.width == b.width && a.height == b.height).count()
            }))?,
        )?;
        Ok(PairedVisualResult {
            tiles,
            differences,
            heights,
        })
    }

    pub(in crate::regression) fn compare(&mut self, profile: &str, output: &Path) -> Result<Value> {
        let (original, variant) = self.aligned(profile)?;
        let physical = self.client(profile)?.to_owned();
        let key = (
            physical.clone(),
            original.command.clone(),
            variant.command.clone(),
        );
        let mut result = if let Some(previous) = self.comparisons.get(&key) {
            // Reuse only a completed comparison of these exact captured pixels.
            // Value validation stays fresh, and no future profile is claimed checked.
            let mut result = previous.clone();
            result["comparison_reused_from"] = previous["profile"].clone();
            result["profile"] = json!(profile);
            result["transport"]["reused_comparison"] = json!(true);
            result["transport"]["reused_image_pairs"] =
                previous["transport"]["requested_image_pairs"].clone();
            result["transport"]["requested_image_pairs"] = json!(0);
            result
        } else {
            let plan = self.plans[profile].clone();
            let captured = self.compare_snapshots(profile, &plan, &original, &variant, output)?;
            let options = visual_validation::VisualCheckOptions {
                profile: profile.into(),
                combined_report: Some(output.join("report.html")),
                ..Default::default()
            };
            visual_validation::write_captured_visuals(
                &plan,
                &self.original,
                output,
                &options,
                captured,
            )?;
            let mut result: Value =
                serde_json::from_slice(&fs::read(output.join("visual-result.json"))?)?;
            result["transport"] =
                serde_json::from_slice(&fs::read(output.join("transport.json"))?)?;
            result
        };
        result["capture_method"] = json!(if self.sweep_mode {
            "autonomous finalized PNG hash sweep; exact verified mismatch replay"
        } else if self.live.is_some() {
            "synchronized PNG uploads; host exact pixels"
        } else {
            "resident PNG SHA-256 then exact pixels"
        });
        result["shared_capture_client"] = json!(physical);
        result["capture_reused"] = json!(self.client(profile)? != profile);
        for sheet in result["sheets"]
            .as_array_mut()
            .context("missing visual sheets")?
        {
            if let Some(coverage) = sheet["index"]
                .as_u64()
                .and_then(|i| self.capture_coverage.get(&(i as usize)))
            {
                sheet["capture_coverage"] = coverage.clone();
                if coverage["omitted_tiles"].as_u64().unwrap_or(0) > 0 && sheet["status"] == "same"
                {
                    sheet["status"] = json!("sampled_same");
                }
            }
        }
        result["capture_policy"] = json!(if self.sweep_mode {
            "autonomous-selected-sweep-v1"
        } else {
            "tall-sheet-v1"
        });
        if self
            .capture_coverage
            .values()
            .any(|c| c["omitted_tiles"].as_u64().unwrap_or(0) > 0)
        {
            result["image_comparison"] =
                json!("selected ranges only on tall sheets; omitted ranges not visually checked");
        }
        fs::create_dir_all(output)?;
        fs::write(
            output.join("visual-result.json"),
            serde_json::to_vec_pretty(&result)?,
        )?;
        crate::regression::visual_report::write_visual_report(output, &result)?;
        self.comparisons
            .entry(key)
            .or_insert_with(|| result.clone());
        self.compared_originals.insert(profile.into(), original);
        Ok(result)
    }

    pub(in crate::regression) fn control_fingerprint(
        &self,
        profile: &str,
        primary: &Value,
    ) -> Result<String> {
        let original = self
            .compared_originals
            .get(profile)
            .context("missing primary original snapshot")?;
        let selected = select_mismatch_snapshot(original, primary)?;
        let bounds: Vec<_> = self.plans[profile].sheets.iter()
            .filter(|sheet| selected.tiles.iter().any(|tile|tile.sheet_index == sheet.sheet_index))
            .map(|sheet| json!({"sheet_index":sheet.sheet_index,"bottom_row":sheet.bottom_row,
                "row_height_bottom_row":sheet.row_height_bottom_row,"capture_right_column":sheet.capture_right_column}))
            .collect();
        fingerprint_control_snapshot(&selected, &bounds)
    }

    pub(in crate::regression) fn compare_control(
        &mut self,
        profile: &str,
        control: &Path,
        primary: &Value,
        output: &Path,
    ) -> Result<Value> {
        let original = self
            .compared_originals
            .get(profile)
            .context("missing primary original snapshot")?;
        let left = select_mismatch_snapshot(original, primary)?;
        let selected = &left.tiles;
        let sheets: BTreeSet<_> = selected.iter().map(|tile| tile.sheet_index).collect();
        let mut plan = plan_pair(&self.original, control)?;
        plan.sheets.retain(|s| sheets.contains(&s.sheet_index));
        for sheet in &mut plan.sheets {
            let base = self.plans[profile]
                .sheets
                .iter()
                .find(|s| s.sheet_index == sheet.sheet_index)
                .unwrap();
            ensure!(
                sheet.bottom_row <= base.bottom_row
                    && sheet.capture_right_column <= base.capture_right_column,
                "whitespace control extends beyond primary capture bounds"
            );
            sheet.bottom_row = base.bottom_row;
            sheet.row_height_bottom_row = base.row_height_bottom_row;
            sheet.capture_right_column = base.capture_right_column;
            sheet.shrunk_config["bottom_row"] = json!(base.bottom_row);
            sheet.shrunk_config["capture_right_column"] = json!(base.capture_right_column);
            sheet.shrunk_config["capture_end_column"] = json!(
                selected
                    .iter()
                    .find(|tile| tile.sheet_index == sheet.sheet_index)
                    .unwrap()
                    .capture_right_column
            );
            sheet.shrunk_config["tile_rows"] = json!(selected
                .iter()
                .filter(|t| t.sheet_index == sheet.sheet_index)
                .map(|t| t.core_rows)
                .collect::<Vec<_>>());
        }
        let client = format!("control-{profile}");
        let control_sha = artifact_cache::hash_file(control)?;
        let shared = self
            .reusable_clients
            .get(&control_sha)
            .filter(|physical| !self.physical_closed.contains(*physical))
            .cloned();
        let reused = shared.is_some();
        let right = if let Some(physical) = shared {
            let captured = self.snapshot(&physical)?;
            let selected = select_existing_snapshot(&captured, &left)?;
            self.aliases.insert(client.clone(), physical);
            selected
        } else {
            let configs: Vec<_> = plan
                .sheets
                .iter()
                .map(|sheet| sheet.shrunk_config.clone())
                .collect();
            self.aliases.insert(client.clone(), client.clone());
            self.configs.insert(client.clone(), configs.clone());
            publish(
                &self
                    .directory
                    .join("clients")
                    .join(format!("{client}.json")),
                &json!({
                    "client_id":client,"source_path":windows_path(control)?,"workbook":format!("{client}.xlsx"),"sheets":configs
                }),
            )?;
            self.recapture(&client, configs)?;
            self.snapshot(&client)?;
            if self.live.is_some() {
                self.capture_synchronized(&BTreeSet::from([client.clone()]))?;
            }
            self.snapshot(&client)?
        };
        let captured = self.compare_snapshots(&client, &plan, &left, &right, output)?;
        let options = visual_validation::VisualCheckOptions {
            profile: format!("{profile} without whitespace deletion"),
            combined_report: Some(output.join("report.html")),
            ..Default::default()
        };
        visual_validation::write_captured_visuals(
            &plan,
            &self.original,
            output,
            &options,
            captured,
        )?;
        let mut result: Value =
            serde_json::from_slice(&fs::read(output.join("visual-result.json"))?)?;
        result["checks_complete"] = json!(true);
        result["shared_capture_client"] = json!(self.client(&client)?);
        result["capture_reused"] = json!(reused);
        result["transport"] = serde_json::from_slice(&fs::read(output.join("transport.json"))?)?;
        let heights_changed = result["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["row_heights"].as_array().is_some_and(|a| !a.is_empty()));
        result["image_comparison"] = json!(if heights_changed {
            "primary mismatch tiles only; pixel comparison omitted on sheets with changed row heights"
        } else {
            "complete on primary mismatch tiles only"
        });
        result["checked_ranges"] = json!(left
            .tiles
            .iter()
            .map(|t| json!({"sheet_index":t.sheet_index,"range":t.range}))
            .collect::<Vec<_>>());
        for sheet in result["sheets"].as_array_mut().unwrap() {
            sheet["capture_scope"] = json!("primary mismatch tiles");
            sheet["checked_ranges"] = json!(left
                .tiles
                .iter()
                .filter(|t| sheet["index"] == t.sheet_index)
                .map(|t| &t.range)
                .collect::<Vec<_>>());
        }
        fs::write(
            output.join("visual-result.json"),
            serde_json::to_vec_pretty(&result)?,
        )?;
        self.close(&client)?;
        Ok(result)
    }

    pub(in crate::regression) fn close(&mut self, logical: &str) -> Result<()> {
        let client = self.client(logical)?.to_owned();
        self.closed.insert(logical.into());
        // Retain clients until the configured batch gate. The launcher closes a
        // KILL_ON_JOB_CLOSE job, including Office/WebView descendants. Whether
        // peers share those descendants is not established; see
        // docs/visual-validation/cleanup-isolation.md before changing this policy.
        if self.cleanup_barrier.is_some() {
            return Ok(());
        }
        // A logical profile can share the original, which must stay available for
        // whitespace controls. Other physical clients close after their last alias.
        if self
            .aliases
            .iter()
            .any(|(name, physical)| physical == &client && !self.closed.contains(name))
        {
            return Ok(());
        }
        if self.physical_closed.contains(&client) {
            return Ok(());
        }
        publish(
            &self.directory.join("close").join(format!("{client}.json")),
            &json!({"close":true}),
        )?;
        self.physical_closed.insert(client);
        Ok(())
    }

    pub(in crate::regression) fn finish(&mut self) -> Result<Value> {
        if let Some(live) = &mut self.live {
            live.complete()?;
        }
        let cleanup_wait = Instant::now();
        if let Some(barrier) = &self.cleanup_barrier {
            barrier.wait_in_worker()?;
        }
        let cleanup_wait_seconds = cleanup_wait.elapsed().as_secs_f64();
        if !self.finished {
            publish(
                &self.directory.join("finish.json"),
                &json!({"complete":true}),
            )?;
            let started = Instant::now();
            while self.child.try_wait()?.is_none() {
                ensure!(
                    started.elapsed() < Duration::from_secs(60),
                    "resident cleanup did not finish"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
            crate::regression::launcher_journal::mark_exited(&mut self.child)?;
            self.finished = true;
        }
        let mut metrics: Value =
            serde_json::from_slice(&fs::read(self.directory.join("metrics.json"))?)?;
        ensure!(
            metrics["status"] == "complete",
            "resident session did not finish: {}",
            metrics["error"]
        );
        metrics["capture_client_aliases"] = json!(self.aliases);
        metrics["batch_cleanup_wait_seconds"] = json!(cleanup_wait_seconds);
        metrics["pipeline_timings"] = serde_json::to_value(&self.timings)?;
        metrics["event_transport"] = self.events.snapshot();
        if let Some(live) = &self.live {
            metrics["capture_pipeline"] = live.capture_pipeline_metadata();
        }
        Ok(metrics)
    }
}

impl Drop for ResidentCapture {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(barrier) = &self.cleanup_barrier {
                if let Err(error) = barrier.wait_in_worker() {
                    eprintln!("Resident cleanup barrier ended: {error:#}");
                }
            }
            let _ = publish(
                &self.directory.join("finish.json"),
                &json!({"complete":false}),
            );
            let started = Instant::now();
            while matches!(self.child.try_wait(), Ok(None))
                && started.elapsed() < Duration::from_secs(35)
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            // The registered launcher owns a kill-on-close Windows job. Normal
            // cleanup above restores add-ins; the archive watchdog remains backup.
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
            if let Err(error) = crate::regression::launcher_journal::mark_exited(&mut self.child) {
                eprintln!("Could not retain resident launcher exit confirmation: {error}");
            }
        }
    }
}

fn same_ranges(left: &Snapshot, right: &Snapshot) -> bool {
    left.tiles.len() == right.tiles.len()
        && left.tiles.iter().zip(&right.tiles).all(|(a, b)| {
            a.sheet_index == b.sheet_index
                && a.sheet_name == b.sheet_name
                && a.range == b.range
                && a.core_rows == b.core_rows
        })
}

fn validate_snapshot(snapshot: &Snapshot, configs: &[Value]) -> Result<()> {
    let expected: BTreeSet<_> = configs.iter().filter_map(|c| c["index"].as_u64()).collect();
    let mut keys = BTreeSet::new();
    for tile in &snapshot.tiles {
        let config = configs
            .iter()
            .find(|s| s["index"] == tile.sheet_index)
            .context("unexpected captured sheet")?;
        let (first, last, start, end) = parse_range(&tile.range)?;
        ensure!(
            tile.sheet_name == config["name"]
                && start == 1
                && tile.capture_right_column <= MAX_CAPTURE_COLUMN
                && end == tile.capture_right_column + 1
                && u64::from(tile.capture_right_column)
                    <= config["capture_right_column"].as_u64().unwrap_or(0)
                && u64::from(tile.capture_right_column)
                    >= config["base_right_column"].as_u64().unwrap_or(0)
                && config["capture_end_column"]
                    .as_u64()
                    .is_none_or(|end| end == u64::from(tile.capture_right_column))
                && tile.core_rows == [first - 1, last]
                && tile.width > 0
                && tile.height > 0
                && tile.width <= 30000
                && tile.height <= 30000
                && u64::from(tile.width) * u64::from(tile.height) <= 64_000_000
                && tile.sha256.len() == 64
                && tile.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid resident tile metadata"
        );
        ensure!(
            keys.insert((tile.sheet_index, tile.tile_index)),
            "duplicate resident tile"
        );
        // The fallback explains a larger indivisible row image; it never
        // authorizes pixels beyond the hard limit or the declared tile budget.
        let pixels = tile
            .capture_pixels
            .max(u64::from(tile.width) * u64::from(tile.height));
        ensure!(
            pixels <= 64_000_000
                && tile.pixel_budget <= 64_000_000
                && (tile.pixel_budget == 0 || pixels <= tile.pixel_budget)
                && match tile.capture_budget_fallback.as_deref() {
                    None => true,
                    Some("single-visible-row") => tile.pixel_budget == 32_000_000,
                    Some(_) => false,
                },
            "invalid resident tile pixel budget"
        );
    }
    ensure!(
        snapshot
            .tiles
            .iter()
            .map(|t| t.sheet_index as u64)
            .collect::<BTreeSet<_>>()
            == expected,
        "resident capture omitted sheets"
    );
    for config in configs {
        let index = config["index"].as_u64().unwrap() as usize;
        let tiles: Vec<_> = snapshot
            .tiles
            .iter()
            .filter(|t| t.sheet_index == index)
            .collect();
        ensure!(
            tiles.iter().enumerate().all(|(i, t)| t.tile_index == i),
            "resident tile indices unordered"
        );
        ensure!(
            tiles
                .iter()
                .all(|tile| tile.capture_right_column == tiles[0].capture_right_column),
            "resident capture width changed within a sheet"
        );
        if let Some(rows) = config["tile_rows"].as_array() {
            let requested: Vec<[u32; 2]> = rows
                .iter()
                .cloned()
                .map(serde_json::from_value)
                .collect::<std::result::Result<_, _>>()?;
            ensure!(
                tiles.iter().map(|t| t.core_rows).collect::<Vec<_>>() == requested,
                "resident targeted capture coverage differs"
            );
        } else {
            ensure!(
                tiles.first().unwrap().core_rows[0] == 0
                    && tiles.last().unwrap().core_rows[1] as u64
                        == config["bottom_row"].as_u64().unwrap() + 1
                    && tiles
                        .windows(2)
                        .all(|p| p[0].core_rows[1] == p[1].core_rows[0]),
                "resident capture has gaps"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn publication_keeps_complete_state_while_a_windows_reader_blocks_replacement() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;

        let root =
            artifact_cache::unique_path(&std::env::temp_dir(), "resident-state-publication")?;
        let target = root.join("state.json");
        let before = json!({"status":"capturing","revision":1,"tiles":["first"]});
        let after = json!({"status":"complete","revision":2,"tiles":["first","second"]});
        publish(&target, &before)?;
        // Match a reader that permits reads but omits FILE_SHARE_DELETE.
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&target)?;
        let writer_target = target.clone();
        let writer_value = after.clone();
        let writer = std::thread::spawn(move || publish(&writer_target, &writer_value));
        let deadline = Instant::now() + Duration::from_secs(3);
        while fs::read_dir(&root)?.count() == 1
            && !writer.is_finished()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(300));
        let completed_before_release = writer.is_finished();
        let during: Value = serde_json::from_slice(&fs::read(&target)?)?;
        drop(reader);
        writer.join().expect("state publisher panicked")?;

        assert!(
            !completed_before_release,
            "publisher must wait for the temporary reader"
        );
        assert_eq!(
            during, before,
            "readers must retain the previous complete JSON"
        );
        assert_eq!(serde_json::from_slice::<Value>(&fs::read(&target)?)?, after);
        assert_eq!(
            fs::read_dir(&root)?.count(),
            1,
            "publication left a pending file"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn config() -> Vec<Value> {
        vec![json!({"index":1,"name":"Data","bottom_row":19,"capture_right_column":1})]
    }

    fn snapshot(spans: &[[u32; 2]]) -> Snapshot {
        Snapshot {
            heights: Vec::new(),
            command: "000001".into(),
            tiles: spans
                .iter()
                .enumerate()
                .map(|(index, rows)| Tile {
                    sheet_index: 1,
                    sheet_name: "Data".into(),
                    tile_index: index,
                    range: format!("A{}:B{}", rows[0] + 1, rows[1]),
                    core_rows: *rows,
                    capture_right_column: 1,
                    width: 40,
                    height: 60,
                    sha256: "a".repeat(64),
                    x: 0,
                    y: 0,
                    capture_pixels: 0,
                    pixel_budget: 0,
                    capture_budget_fallback: None,
                })
                .collect(),
        }
    }

    #[test]
    fn tile_budget_metadata_roundtrips_without_bypassing_hard_limits() -> Result<()> {
        let mut plan = snapshot(&[[0, 20]]);
        plan.tiles[0].capture_pixels = 5_000_000;
        plan.tiles[0].pixel_budget = 32_000_000;
        plan.tiles[0].capture_budget_fallback = Some("single-visible-row".into());
        let tile: Tile = serde_json::from_value(serde_json::to_value(&plan.tiles[0])?)?;
        assert_eq!(tile.pixel_budget, 32_000_000);
        assert_eq!(
            tile.capture_budget_fallback.as_deref(),
            Some("single-visible-row")
        );
        validate_snapshot(&plan, &config())?;
        plan.tiles[0].capture_pixels = 32_000_001;
        assert!(validate_snapshot(&plan, &config()).is_err());
        plan.tiles[0].capture_pixels = 5_000_000;
        plan.tiles[0].pixel_budget = 64_000_000;
        assert!(validate_snapshot(&plan, &config()).is_err());
        plan.tiles[0].pixel_budget = 4_000_000;
        plan.tiles[0].capture_budget_fallback = None;
        assert!(validate_snapshot(&plan, &config()).is_err());
        plan.tiles[0].capture_pixels = 48_000_000;
        plan.tiles[0].pixel_budget = 48_000_000;
        validate_snapshot(&plan, &config())?;
        plan.tiles[0].capture_pixels = 64_000_001;
        assert!(validate_snapshot(&plan, &config()).is_err());
        Ok(())
    }

    #[test]
    fn complete_capture_rejects_missing_overlapping_and_unordered_coverage() -> Result<()> {
        validate_snapshot(&snapshot(&[[0, 10], [10, 20]]), &config())?;
        for spans in [
            vec![[0, 10]],
            vec![[0, 10], [11, 20]],
            vec![[0, 10], [9, 20]],
            vec![[10, 20], [0, 10]],
            vec![[0, 10], [10, 21]],
        ] {
            assert!(
                validate_snapshot(&snapshot(&spans), &config()).is_err(),
                "accepted {spans:?}"
            );
        }
        let mut omitted_sheet = config();
        omitted_sheet
            .push(json!({"index":2,"name":"Other","bottom_row":19,"capture_right_column":1}));
        assert!(validate_snapshot(&snapshot(&[[0, 20]]), &omitted_sheet).is_err());
        assert!(validate_snapshot(&snapshot(&[]), &config()).is_err());
        Ok(())
    }

    #[test]
    fn targeted_capture_checks_exact_requested_spans_without_claiming_gaps() -> Result<()> {
        let mut selected = config();
        selected[0]["tile_rows"] = json!([[4, 7], [12, 16]]);
        validate_snapshot(&snapshot(&[[4, 7], [12, 16]]), &selected)?;
        for spans in [
            vec![[4, 7]],
            vec![[4, 8], [12, 16]],
            vec![[4, 7], [7, 12], [12, 16]],
            vec![[0, 20]],
        ] {
            assert!(
                validate_snapshot(&snapshot(&spans), &selected).is_err(),
                "accepted {spans:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn matching_hashes_do_not_match_different_ranges_or_sheets() {
        let original = snapshot(&[[0, 10], [10, 20]]);
        assert!(!same_ranges(&original, &snapshot(&[[0, 9], [9, 20]])));
        let mut changed = original.clone();
        changed.tiles[0].sheet_index = 2;
        assert!(!same_ranges(&original, &changed));
        changed = original.clone();
        changed.tiles[0].sheet_name = "Other".into();
        assert!(!same_ranges(&original, &changed));
        changed = original.clone();
        changed.tiles[0].core_rows[1] = 9;
        assert!(!same_ranges(&original, &changed));
        changed = original.clone();
        changed.tiles.swap(0, 1);
        assert!(!same_ranges(&original, &changed));
    }

    #[test]
    fn corresponding_ranges_allow_client_local_capture_and_tile_ids() {
        let original = snapshot(&[[4, 7], [12, 16]]);
        let mut control = original.clone();
        control.command = "000007".into();
        // A subset of the original retains its full-capture ordinal, while a
        // control numbers its selected tiles from zero. Correspondence is by range.
        control.tiles[0].tile_index = 8;
        control.tiles[1].tile_index = 12;
        assert!(same_ranges(&original, &control));
    }

    #[test]
    fn capture_metadata_rejects_invalid_dimensions_hashes_and_tile_ids() {
        let good = snapshot(&[[0, 10], [10, 20]]);
        for (width, height) in [(0, 60), (40, 0), (30_001, 1), (1, 30_001), (8_001, 8_000)] {
            let mut bad = good.clone();
            bad.tiles[0].width = width;
            bad.tiles[0].height = height;
            assert!(
                validate_snapshot(&bad, &config()).is_err(),
                "accepted {width}x{height}"
            );
        }
        for hash in [String::new(), "a".repeat(63), "g".repeat(64)] {
            let mut bad = good.clone();
            bad.tiles[0].sha256 = hash;
            assert!(validate_snapshot(&bad, &config()).is_err());
        }
        for ordinal in [0, 2] {
            let mut bad = good.clone();
            bad.tiles[1].tile_index = ordinal;
            assert!(validate_snapshot(&bad, &config()).is_err());
        }
        let mut bad = good.clone();
        bad.tiles[0].range = "A1:C10".into();
        assert!(validate_snapshot(&bad, &config()).is_err());
        bad = good;
        bad.tiles[0].sheet_name = "Other".into();
        assert!(validate_snapshot(&bad, &config()).is_err());
    }

    #[test]
    fn aliases_share_only_certified_identical_bytes_and_preserve_original_identity() {
        let aliases = aliases_for_certificates([
            ("original", "a", true),
            ("default", "a", true),
            ("maximum-cleaning", "b", true),
            ("too-aggressive-cleaning", "b", true),
        ]);
        assert_eq!(aliases["original"], "original");
        assert_eq!(aliases["default"], "original");
        assert_eq!(aliases["maximum-cleaning"], "maximum-cleaning");
        assert_eq!(aliases["too-aggressive-cleaning"], "maximum-cleaning");
        let unsafe_aliases = aliases_for_certificates([
            ("original", "a", false),
            ("default", "a", false),
            ("maximum-cleaning", "b", false),
            ("too-aggressive-cleaning", "b", false),
        ]);
        assert!(unsafe_aliases
            .iter()
            .all(|(logical, physical)| logical == physical));
    }

    #[test]
    fn capture_reuse_rejects_volatile_unknown_or_empty_runtime_certificates() -> Result<()> {
        let safe: artifact_cache::PackageManifest = serde_json::from_value(json!({
            "archive_sha256":"a", "package_sha256":"b", "parts":{
                "xl/worksheets/sheet1.xml":{"bytes":1,"sha256":"c","visual_context":false,
                    "visual_runtime":false,"recalc_cacheable":true,"has_formula":false}
            }
        }))?;
        assert!(reusable_capture_manifest(&safe));
        for (runtime, cacheable) in [(true, true), (false, false)] {
            let mut unsafe_manifest = safe.clone();
            let part = unsafe_manifest.parts.values_mut().next().unwrap();
            part.visual_runtime = runtime;
            part.recalc_cacheable = cacheable;
            assert!(!reusable_capture_manifest(&unsafe_manifest));
        }
        let mut empty = safe;
        empty.parts.clear();
        assert!(!reusable_capture_manifest(&empty));
        Ok(())
    }

    #[test]
    fn all_initial_clients_negotiate_one_union_and_keep_existing_fine_partitions() {
        let a = snapshot(&[[0, 10], [10, 20]]);
        let b = snapshot(&[[0, 5], [5, 15], [15, 20]]);
        let fine = snapshot(&[[0, 5], [5, 10], [10, 15], [15, 20]]);
        let rows = union_tile_rows([&a, &b, &fine]);
        assert_eq!(rows[&1], vec![[0, 5], [5, 10], [10, 15], [15, 20]]);
        assert!(!has_tile_rows(&a, &rows));
        assert!(!has_tile_rows(&b, &rows));
        assert!(has_tile_rows(&fine, &rows));
        let already_same = union_tile_rows([&a, &a]);
        assert!(has_tile_rows(&a, &already_same));
    }

    #[test]
    fn actual_capture_boundary_may_be_smaller_than_the_safety_bound() -> Result<()> {
        let mut configs = config();
        configs[0]["base_right_column"] = json!(0);
        configs[0]["capture_right_column"] = json!(10);
        let mut offered = snapshot(&[[0, 10], [10, 20]]);
        validate_snapshot(&offered, &configs)?;
        configs[0]["capture_end_column"] = json!(2);
        assert!(validate_snapshot(&offered, &configs).is_err());
        for tile in &mut offered.tiles {
            tile.capture_right_column = 2;
            tile.range = format!("A{}:C{}", tile.core_rows[0] + 1, tile.core_rows[1]);
        }
        validate_snapshot(&offered, &configs)?;
        configs[0]["capture_end_column"] = Value::Null;
        offered.tiles[1].capture_right_column = 1;
        offered.tiles[1].range = "A11:B20".into();
        assert!(validate_snapshot(&offered, &configs).is_err());
        offered.tiles[0].capture_right_column = u32::MAX;
        assert!(validate_snapshot(&offered, &configs).is_err());
        Ok(())
    }

    #[test]
    fn control_fingerprint_tracks_exact_selected_pixels_and_geometry() -> Result<()> {
        let original = snapshot(&[[0, 10], [10, 20]]);
        let primary = json!({"sheets":[{"index":1,"row_heights":[],
            "image_differences":[{"range":"A11:B20"},{"range":"A11:B20"}]}]});
        let selected = select_mismatch_snapshot(&original, &primary)?;
        assert_eq!(selected.tiles.len(), 1);
        assert_eq!(selected.tiles[0].tile_index, 1);
        assert_eq!(selected.command, original.command);
        let fingerprint = fingerprint_control_snapshot(&selected, &[])?;
        let mut changed = original.clone();
        changed.command = "000009".into();
        changed.tiles[0].sha256 = "b".repeat(64);
        assert_eq!(
            fingerprint_control_snapshot(&select_mismatch_snapshot(&changed, &primary)?, &[])?,
            fingerprint
        );
        changed.tiles[1].sha256 = "b".repeat(64);
        assert_ne!(
            fingerprint_control_snapshot(&select_mismatch_snapshot(&changed, &primary)?, &[])?,
            fingerprint
        );
        let mut different_size = selected.clone();
        different_size.tiles[0].width += 1;
        assert_ne!(
            fingerprint_control_snapshot(&different_size, &[])?,
            fingerprint
        );
        assert_ne!(
            fingerprint_control_snapshot(&selected, &[json!({"bottom_row":30})])?,
            fingerprint
        );
        let missing = json!({"sheets":[{"index":1,"image_differences":[{"range":"A10:B20"}]}]});
        assert!(select_mismatch_snapshot(&original, &missing).is_err());
        Ok(())
    }

    #[test]
    fn reused_control_keeps_physical_command_and_tile_ids_for_only_requested_ranges() -> Result<()>
    {
        let mut captured = snapshot(&[[0, 5], [5, 10], [10, 15], [15, 20]]);
        captured.command = "000008".into();
        let requested = snapshot(&[[5, 10], [15, 20]]);
        let selected = select_existing_snapshot(&captured, &requested)?;
        assert_eq!(selected.command, "000008");
        assert_eq!(
            selected
                .tiles
                .iter()
                .map(|tile| tile.tile_index)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert!(same_ranges(&requested, &selected));
        let mut wrong_end = requested.clone();
        wrong_end.tiles[0].capture_right_column += 1;
        assert!(select_existing_snapshot(&captured, &wrong_end).is_err());
        assert!(select_existing_snapshot(&captured, &snapshot(&[[4, 10]])).is_err());
        Ok(())
    }

    #[test]
    fn a_unique_safe_input_remains_available_for_a_later_control() -> Result<()> {
        use std::io::Write;
        let root =
            artifact_cache::unique_path(&std::env::temp_dir(), "resident-client-certificates")?;
        fs::create_dir_all(&root)?;
        let mut inputs = Vec::new();
        for (name, formula) in [
            ("original", "1+2"),
            ("default", "2+3"),
            ("maximum-cleaning", "NOW()"),
        ] {
            let path = root.join(format!("{name}.xlsx"));
            let mut zip = zip::ZipWriter::new(fs::File::create(&path)?);
            zip.start_file(
                "xl/worksheets/sheet1.xml",
                zip::write::FileOptions::default(),
            )?;
            zip.write_all(format!("<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><f>{formula}</f><v>3</v></c></row></sheetData></worksheet>").as_bytes())?;
            zip.finish()?;
            inputs.push((name.to_owned(), path));
        }
        let clients = capture_clients(&inputs)?;
        assert_eq!(
            clients.reusable[&artifact_cache::hash_file(&inputs[1].1)?],
            "default"
        );
        assert!(!clients
            .reusable
            .contains_key(&artifact_cache::hash_file(&inputs[2].1)?));
        assert!(clients
            .aliases
            .iter()
            .all(|(logical, physical)| logical == physical));
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
