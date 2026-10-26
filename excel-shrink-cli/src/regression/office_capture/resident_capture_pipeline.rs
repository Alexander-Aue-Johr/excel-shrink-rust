//! One common batch of lookahead, independently dispatched to each physical client.
use super::*;
use processing::{BatchSummary, Event, ImageProcessor, Upload};
use std::ops::Range;

const LOOKAHEAD_BATCHES: usize = 2;

struct Dispatch {
    positions: Range<usize>,
    capture_batches: Vec<usize>,
}

/// Credits cover issued work until comparison, including uploaded images and
/// queued decoding. A slow peer therefore cannot grow another peer's backlog.
struct CaptureWindow {
    batches: Vec<Range<usize>>,
    pixels: BTreeMap<String, Vec<u64>>,
    next: BTreeMap<String, usize>,
    reserved: BTreeMap<String, BTreeSet<usize>>,
    compared: BTreeSet<usize>,
    frontier: usize,
}

impl CaptureWindow {
    fn new(
        reference: &Snapshot,
        snapshots: &BTreeMap<String, Snapshot>,
        clients: &BTreeSet<String>,
        requested: usize,
    ) -> Result<Self> {
        let mut pixels = BTreeMap::new();
        for client in clients {
            let snapshot = snapshots
                .get(client)
                .context("capture client has no planned snapshot")?;
            ensure!(
                snapshot.tiles.len() == reference.tiles.len(),
                "capture plans have different lengths"
            );
            let costs: Vec<_> = snapshot.tiles.iter().map(tile_pixels).collect();
            ensure!(
                costs.iter().all(|pixels| *pixels <= MAX_PENDING_PIXELS),
                "capture tile exceeds the 64 megapixel window"
            );
            pixels.insert(client.clone(), costs);
        }
        let mut batches = Vec::new();
        let mut start = 0;
        while start < reference.tiles.len() {
            let end = batch_end(reference, snapshots, clients, start, requested);
            batches.push(start..end);
            start = end;
        }
        Ok(Self {
            batches,
            pixels,
            next: clients.iter().map(|client| (client.clone(), 0)).collect(),
            reserved: clients
                .iter()
                .map(|client| (client.clone(), BTreeSet::new()))
                .collect(),
            compared: BTreeSet::new(),
            frontier: 0,
        })
    }

    fn admit(&mut self, client: &str, reference: &Snapshot, paused: bool) -> Option<Dispatch> {
        if paused {
            return None;
        }
        let first = self.next[client];
        let limit = self.batches.len().min(self.frontier + LOOKAHEAD_BATCHES);
        if first >= limit {
            return None;
        }
        let start = self.batches[first].start;
        let sheet = reference.tiles[start].sheet_index;
        let reserved = self.reserved.get_mut(client).unwrap();
        let costs = &self.pixels[client];
        let mut count = reserved.len();
        let mut pixels: u64 = reserved.iter().map(|position| costs[*position]).sum();
        let mut ends = Vec::new();
        let mut next = first;
        while next < limit {
            let batch = &self.batches[next];
            let additional: u64 = batch.clone().map(|position| costs[position]).sum();
            if reference.tiles[batch.start].sheet_index != sheet
                || count + batch.len() > MAX_PENDING_TILES
                || pixels + additional > MAX_PENDING_PIXELS
            {
                break;
            }
            count += batch.len();
            pixels += additional;
            ends.push(batch.end - start);
            reserved.extend(batch.clone());
            next += 1;
        }
        let end = start + ends.last()?;
        *self.next.get_mut(client).unwrap() = next;
        Some(Dispatch {
            positions: start..end,
            capture_batches: ends,
        })
    }

    fn compared(&mut self, position: usize) -> Result<()> {
        ensure!(!self.compared.contains(&position), "tile compared twice");
        ensure!(
            self.reserved
                .values()
                .all(|positions| positions.contains(&position)),
            "comparison completed before every client was scheduled"
        );
        for positions in self.reserved.values_mut() {
            positions.remove(&position);
        }
        self.compared.insert(position);
        while self.frontier < self.batches.len()
            && self.batches[self.frontier]
                .clone()
                .all(|position| self.compared.contains(&position))
        {
            self.frontier += 1;
        }
        Ok(())
    }

    fn complete(&self) -> bool {
        self.frontier == self.batches.len()
    }
}

fn tile_pixels(tile: &Tile) -> u64 {
    tile.capture_pixels
        .max(tile.width as u64 * tile.height as u64)
}

struct Evidence {
    path: PathBuf,
    bytes: u64,
    response_received: bool,
    keep: Option<bool>,
}

/// Host result validation still needs each uploaded PNG. Retire it only after
/// both the comparison and the durable command response have completed.
fn retire_evidence(evidence: &mut BTreeMap<(String, usize), Evidence>) -> Result<()> {
    let finished: Vec<_> = evidence
        .iter()
        .filter(|(_, entry)| entry.response_received && entry.keep.is_some())
        .map(|(key, _)| key.clone())
        .collect();
    for key in finished {
        let entry = evidence.remove(&key).unwrap();
        if entry.keep == Some(false) {
            fs::remove_file(entry.path)?;
        }
    }
    Ok(())
}

fn merge_processing_stats(previous: &Value, current: &processing::ProcessorStats) -> Value {
    let mut clients = previous["clients"].as_object().cloned().unwrap_or_default();
    for (client, peak) in &current.clients {
        let previous = clients.get(client).unwrap_or(&Value::Null);
        let tiles = previous["tiles"]
            .as_u64()
            .unwrap_or(0)
            .max(peak.tiles as u64);
        let pixels = previous["pixels"].as_u64().unwrap_or(0).max(peak.pixels);
        clients.insert(client.clone(), json!({"tiles":tiles,"pixels":pixels}));
    }
    json!({
        "clients":clients,
        "cancelled":current.cancelled || previous["cancelled"].as_bool().unwrap_or(false)
    })
}

impl ResidentCapture {
    pub(super) fn capture_pipelined(&mut self, clients: &BTreeSet<String>) -> Result<()> {
        let control_only = !clients.contains("original");
        let reference_client = if control_only {
            clients.first().context("no live clients")?.as_str()
        } else {
            "original"
        };
        let reference = self.snapshot(reference_client)?;
        let live = self.live.as_mut().context("missing live capture state")?;
        if !control_only {
            live.plan(&reference)?;
        }
        let requested = live.batch_tiles();
        let controls = LiveControls::from_live(live);
        let window = CaptureWindow::new(&reference, &self.snapshots, clients, requested)?;
        let mut height_sheets = BTreeSet::new();
        if !control_only {
            let baseline: BTreeMap<_, _> = reference
                .heights
                .iter()
                .map(|sheet| Ok((sheet.sheet_index, serde_json::to_value(sheet)?)))
                .collect::<Result<_>>()?;
            for client in clients {
                for sheet in &self.snapshots[client].heights {
                    if baseline.get(&sheet.sheet_index) != Some(&serde_json::to_value(sheet)?) {
                        height_sheets.insert(sheet.sheet_index);
                    }
                }
            }
        }
        let pipeline = &mut live.state["capture_pipeline"];
        pipeline["mode"] = json!("bounded-overlap");
        pipeline["window_batches"] = json!(LOOKAHEAD_BATCHES);
        pipeline["max_pending_tiles_per_client"] = json!(MAX_PENDING_TILES);
        pipeline["max_pending_pixels_per_client"] = json!(MAX_PENDING_PIXELS);
        pipeline["max_pending_png_bytes_per_client"] = json!(MAX_PENDING_PNG_BYTES);
        live.save()?;
        let processor = ImageProcessor::start(
            self.live.take().unwrap(),
            clients.clone(),
            self.aliases.clone(),
            height_sheets,
            control_only,
            self.events.wake_sender(),
        )?;
        let result =
            self.run_capture_pipeline(clients, &reference, controls, window, &processor, requested);
        // Always join before cleanup can remove PNGs or close the group state.
        let (mut live, processed) = processor.finish(result.is_err())?;
        if let Ok(stats) = &processed {
            let previous = &live.state["capture_pipeline"]["processing"];
            live.state["capture_pipeline"]["processing"] = merge_processing_stats(previous, stats);
        }
        live.state["pipeline_timings"] = serde_json::to_value(&self.timings)?;
        live.state["event_transport"] = self.events.snapshot();
        if result.is_ok() && processed.is_ok() && !control_only {
            live.state["primary_complete"] = json!(true);
            live.state["status"] = json!("comparing");
        }
        self.live = Some(live);
        self.live.as_ref().unwrap().save()?;
        result?;
        processed?;
        Ok(())
    }

    fn run_capture_pipeline(
        &mut self,
        clients: &BTreeSet<String>,
        reference: &Snapshot,
        controls: LiveControls,
        window: CaptureWindow,
        processor: &ImageProcessor,
        requested: usize,
    ) -> Result<()> {
        let mut progress = PipelineProgress {
            clients,
            reference,
            processor,
            window,
            pending: Vec::new(),
            starts: BTreeMap::new(),
            announced: BTreeSet::new(),
            evidence: BTreeMap::new(),
            summary: empty_summary(requested),
            last_dispatch_tiles: 0,
            clock: OpeningPause::new(
                self.cleanup_barrier
                    .as_ref()
                    .and_then(|barrier| barrier.pause_path.as_deref()),
            ),
            sampled: Instant::now(),
            paused_time: Duration::ZERO,
            reported_elapsed: Duration::ZERO,
            reported_frontier: 0,
        };
        progress.run(self, &controls)
    }
}

struct PipelineProgress<'a> {
    clients: &'a BTreeSet<String>,
    reference: &'a Snapshot,
    processor: &'a ImageProcessor,
    window: CaptureWindow,
    pending: Vec<PendingCapture>,
    starts: BTreeMap<String, usize>,
    announced: BTreeSet<usize>,
    evidence: BTreeMap<(String, usize), Evidence>,
    summary: BatchSummary,
    last_dispatch_tiles: usize,
    clock: OpeningPause,
    sampled: Instant,
    paused_time: Duration,
    reported_elapsed: Duration,
    reported_frontier: usize,
}

impl PipelineProgress<'_> {
    fn run(&mut self, resident: &mut ResidentCapture, controls: &LiveControls) -> Result<()> {
        loop {
            let settings = controls.read();
            let paused = settings["paused"].as_bool().unwrap_or(true);
            let now = Instant::now();
            if paused {
                self.paused_time += now.duration_since(self.sampled);
            }
            self.sampled = now;

            let mut progressed = self.receive_processed(resident)?;
            retire_evidence(&mut self.evidence)?;
            progressed |= self.dispatch_ready(resident, paused)?;
            if let Some(event) = resident.poll_live_event(&mut self.pending, &settings)? {
                match event {
                    CaptureEvent::Receipt(index, offset, receipt) => {
                        self.receive_upload(resident, index, offset, receipt)?;
                    }
                    CaptureEvent::Response(index) => self.complete_command(resident, index)?,
                }
                progressed = true;
            }
            let finished = self.window.complete() && self.pending.is_empty();
            self.report_progress(finished)?;
            if finished {
                retire_evidence(&mut self.evidence)?;
                ensure!(
                    self.evidence.is_empty(),
                    "capture finished before evidence retirement"
                );
                return Ok(());
            }
            if !progressed {
                let waiting = Instant::now();
                resident.events.wait()?;
                resident.timings.wait_ms += waiting.elapsed().as_secs_f64() * 1000.0;
            }
        }
    }

    fn receive_processed(&mut self, resident: &mut ResidentCapture) -> Result<bool> {
        let mut progressed = false;
        while let Some(event) = self.processor.try_event()? {
            match event {
                Event::Decoded {
                    position,
                    client,
                    tile,
                    path,
                    decode_ms,
                    preview_ms,
                } => {
                    resident.timings.decode_ms += decode_ms;
                    resident.timings.preview_ms += preview_ms;
                    let snapshot = resident.snapshots.get_mut(&client).unwrap();
                    snapshot.tiles[position] = tile.clone();
                    resident.images.insert(
                        (
                            client,
                            snapshot.command.clone(),
                            tile.sheet_index,
                            tile.tile_index,
                        ),
                        path,
                    );
                }
                Event::Compared {
                    position,
                    keep,
                    compare_ms,
                } => {
                    resident.timings.compare_ms += compare_ms;
                    self.complete_comparison(resident, position, keep)?;
                }
                Event::Failed(error) => bail!("resident image processing: {error}"),
            }
            progressed = true;
        }
        Ok(progressed)
    }

    fn complete_comparison(
        &mut self,
        resident: &mut ResidentCapture,
        position: usize,
        keep: bool,
    ) -> Result<()> {
        self.window.compared(position)?;
        for client in self.clients {
            self.evidence
                .get_mut(&(client.clone(), position))
                .context("comparison has no upload receipt")?
                .keep = Some(keep);
            if !keep {
                let snapshot = &resident.snapshots[client];
                let tile = &snapshot.tiles[position];
                resident.images.remove(&(
                    client.clone(),
                    snapshot.command.clone(),
                    tile.sheet_index,
                    tile.tile_index,
                ));
            }
        }
        let tile = &self.reference.tiles[position];
        crate::regression::archive::work_state::event(
            "comparing_images",
            "tile_compared",
            "All profile images compared and retained",
            true,
            None,
            json!({"sheet_index":tile.sheet_index,"sheet_name":tile.sheet_name,"tile_index":tile.tile_index}),
        );
        Ok(())
    }

    fn dispatch_ready(&mut self, resident: &mut ResidentCapture, paused: bool) -> Result<bool> {
        let mut progressed = false;
        for client in self.clients {
            if self.pending.iter().any(|job| &job.client == client) {
                continue;
            }
            let Some(dispatch) = self.window.admit(client, self.reference, paused) else {
                continue;
            };
            let new_tiles: Vec<_> = dispatch
                .positions
                .clone()
                .filter(|position| self.announced.insert(*position))
                .map(|position| self.reference.tiles[position].clone())
                .collect();
            if !new_tiles.is_empty() {
                self.processor.begin(new_tiles)?;
            }
            let snapshot = &resident.snapshots[client];
            let capture = snapshot.command.clone();
            let tiles = snapshot.tiles[dispatch.positions.clone()].to_vec();
            let command = resident.issue(
                client,
                json!({"kind":"capture_images", "capture_id":capture,
                    "tiles":tiles,"capture_batches":dispatch.capture_batches}),
            )?;
            self.summary.actual = self.summary.actual.max(tiles.len());
            self.last_dispatch_tiles = tiles.len();
            self.starts.insert(client.clone(), dispatch.positions.start);
            self.pending.push(PendingCapture {
                client: client.clone(),
                capture,
                command,
                tiles,
                received: BTreeSet::new(),
                clock: None,
                user_pause: Duration::ZERO,
                sampled: Instant::now(),
            });
            progressed = true;
        }
        Ok(progressed)
    }

    fn receive_upload(
        &mut self,
        resident: &mut ResidentCapture,
        index: usize,
        offset: usize,
        receipt: Value,
    ) -> Result<()> {
        let job = &mut self.pending[index];
        let position = self.starts[&job.client] + offset;
        let tile = job.tiles[offset].clone();
        let metadata: Tile = serde_json::from_value(receipt.clone())?;
        validate_receipt(&tile, &metadata)?;
        let path = fs::canonicalize(receipt["image_path"].as_str().context("missing PNG path")?)?;
        ensure!(
            path.starts_with(resident.directory.join("images")),
            "image-first PNG outside session"
        );
        let bytes = fs::metadata(&path)?.len();
        let queued_bytes: u64 = self
            .evidence
            .iter()
            .filter(|((client, _), entry)| client == &job.client && entry.keep.is_none())
            .map(|(_, entry)| entry.bytes)
            .sum();
        ensure!(
            bytes <= MAX_PENDING_PNG_BYTES.saturating_sub(queued_bytes),
            "pending PNGs exceed the per-client 160 MiB budget"
        );
        self.evidence.insert(
            (job.client.clone(), position),
            Evidence {
                path: path.clone(),
                bytes,
                response_received: false,
                keep: None,
            },
        );
        self.processor.submit(Upload {
            position,
            reference: self.reference.tiles[position].clone(),
            client: job.client.clone(),
            tile: tile.clone(),
            path,
            png_sha256: metadata.sha256,
        })?;
        job.received.insert(tile.tile_index);
        resident.timings.images_received += 1;
        for (key, total) in [
            ("upload_receive_ms", &mut resident.timings.upload_receive_ms),
            (
                "upload_validate_publish_ms",
                &mut resident.timings.upload_validate_publish_ms,
            ),
        ] {
            if let Some(value) = receipt["timings"][key]
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
            {
                *total += value;
            }
        }
        self.summary.pixels += tile_pixels(&tile);
        self.summary.core_pixels += tile.width as u64 * tile.height as u64;
        self.summary.images += 1;
        self.summary.bytes += bytes;
        Ok(())
    }

    fn complete_command(&mut self, resident: &mut ResidentCapture, index: usize) -> Result<()> {
        let job = self.pending.remove(index);
        let start = self
            .starts
            .remove(&job.client)
            .context("command has no capture start position")?;
        let result = resident.wait(&job.client, &job.command)?;
        ensure!(
            result["images"]
                .as_array()
                .is_some_and(|images| images.len() == job.tiles.len())
                && job.received.len() == job.tiles.len(),
            "incomplete image-first batch receipts"
        );
        for position in start..start + job.tiles.len() {
            self.evidence
                .get_mut(&(job.client.clone(), position))
                .context("command response has no image receipt")?
                .response_received = true;
        }
        Ok(())
    }

    fn report_progress(&mut self, finished: bool) -> Result<()> {
        if self.window.frontier == self.reported_frontier && !finished {
            return Ok(());
        }
        let elapsed = self.clock.active_elapsed().saturating_sub(self.paused_time);
        let requested = self.summary.requested;
        let mut summary = std::mem::replace(&mut self.summary, empty_summary(requested));
        // Receipts and comparisons can flush after their dispatch interval.
        // Keep the most recent dispatch size when this interval started no work.
        if summary.actual == 0 {
            summary.actual = self.last_dispatch_tiles;
        }
        summary.elapsed = elapsed.saturating_sub(self.reported_elapsed);
        self.processor.record_batch(summary)?;
        self.reported_elapsed = elapsed;
        self.reported_frontier = self.window.frontier;
        Ok(())
    }
}

fn empty_summary(requested: usize) -> BatchSummary {
    BatchSummary {
        pixels: 0,
        core_pixels: 0,
        images: 0,
        bytes: 0,
        elapsed: Duration::ZERO,
        requested,
        actual: 0,
    }
}

fn validate_receipt(expected: &Tile, received: &Tile) -> Result<()> {
    ensure!(
        received.sheet_index == expected.sheet_index
            && received.tile_index == expected.tile_index
            && received.range == expected.range
            && received.width == expected.width
            && received.height == expected.height
            && received.core_rows == expected.core_rows
            && received.capture_right_column == expected.capture_right_column
            && received.x == expected.x
            && received.y == expected.y
            && received.capture_pixels == expected.capture_pixels,
        "image-first receipt differs from negotiated tile"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processing_peaks_include_primary_and_control_phases() {
        let previous = json!({
            "clients":{
                "original":{"tiles":4,"pixels":32_000_000},
                "default":{"tiles":2,"pixels":16_000_000},
                "maximum-cleaning":{"tiles":3,"pixels":24_000_000},
                "too-aggressive-cleaning":{"tiles":4,"pixels":30_000_000}
            },
            "cancelled":false
        });
        let current = processing::ProcessorStats {
            clients: BTreeMap::from([
                (
                    "original".into(),
                    processing::ClientPeak {
                        tiles: 2,
                        pixels: 40_000_000,
                    },
                ),
                (
                    "control-default".into(),
                    processing::ClientPeak {
                        tiles: 1,
                        pixels: 8_000_000,
                    },
                ),
            ]),
            cancelled: true,
        };
        let merged = merge_processing_stats(&previous, &current);
        assert_eq!(merged["clients"].as_object().unwrap().len(), 5);
        assert_eq!(
            merged["clients"]["original"],
            json!({"tiles":4,"pixels":40_000_000})
        );
        for client in ["default", "maximum-cleaning", "too-aggressive-cleaning"] {
            assert_eq!(merged["clients"][client], previous["clients"][client]);
        }
        assert_eq!(merged["clients"]["control-default"]["pixels"], 8_000_000);
        assert_eq!(merged["cancelled"], true);
        assert_eq!(
            merge_processing_stats(&merged, &processing::ProcessorStats::default()),
            merged
        );
        let initial = merge_processing_stats(&Value::Null, &current);
        assert_eq!(initial["clients"].as_object().unwrap().len(), 2);
    }

    fn plans(cost: u64, count: usize) -> (Snapshot, BTreeMap<String, Snapshot>, BTreeSet<String>) {
        let reference = Snapshot {
            command: "plan".into(),
            heights: vec![],
            tiles: (0..count)
                .map(|position| Tile {
                    sheet_index: 0,
                    sheet_name: "Sheet".into(),
                    tile_index: position,
                    range: format!("A{}", position + 1),
                    core_rows: [position as u32, position as u32 + 1],
                    capture_right_column: 1,
                    width: 1,
                    height: 1,
                    x: 0,
                    y: position as u32,
                    capture_pixels: cost,
                    sha256: String::new(),
                })
                .collect(),
        };
        (
            reference.clone(),
            BTreeMap::from([
                ("original".into(), reference.clone()),
                ("variant".into(), reference),
            ]),
            BTreeSet::from(["original".into(), "variant".into()]),
        )
    }

    #[test]
    fn fast_client_advances_only_after_comparison_releases_a_common_batch() {
        let (reference, snapshots, clients) = plans(1_000_000, 12);
        let mut window = CaptureWindow::new(&reference, &snapshots, &clients, 2).unwrap();
        let first = window.admit("original", &reference, false).unwrap();
        assert_eq!(first.positions, 0..4);
        assert_eq!(first.capture_batches, vec![2, 4]);
        assert!(window.admit("original", &reference, false).is_none());
        assert!(window.compared(0).is_err());
        window.admit("variant", &reference, false).unwrap();
        window.compared(1).unwrap();
        assert!(window.admit("original", &reference, false).is_none());
        window.compared(0).unwrap();
        assert_eq!(
            window
                .admit("original", &reference, false)
                .unwrap()
                .positions,
            4..6
        );
        assert!(window.compared(0).is_err());
    }

    #[test]
    fn memory_credit_covers_all_uncompared_commands_and_pause_does_not_consume_it() {
        let (reference, snapshots, clients) = plans(32_000_000, 6);
        let mut window = CaptureWindow::new(&reference, &snapshots, &clients, 2).unwrap();
        assert!(window.admit("original", &reference, true).is_none());
        assert_eq!(
            window
                .admit("original", &reference, false)
                .unwrap()
                .positions,
            0..2
        );
        window.admit("variant", &reference, false).unwrap();
        assert!(window.admit("original", &reference, false).is_none());
        window.compared(0).unwrap();
        assert!(window.admit("original", &reference, false).is_none());
        window.compared(1).unwrap();
        assert_eq!(
            window
                .admit("original", &reference, false)
                .unwrap()
                .positions,
            2..4
        );
    }

    #[test]
    fn commands_respect_sheet_boundaries_and_the_sixteen_tile_limit() {
        let (mut reference, mut snapshots, clients) = plans(1, 40);
        reference.tiles[3].sheet_index = 1;
        for snapshot in snapshots.values_mut() {
            snapshot.tiles[3].sheet_index = 1;
        }
        let mut window = CaptureWindow::new(&reference, &snapshots, &clients, 4).unwrap();
        assert_eq!(
            window
                .admit("original", &reference, false)
                .unwrap()
                .positions,
            0..3
        );
        let (reference, snapshots, clients) = plans(1, 40);
        let mut window = CaptureWindow::new(&reference, &snapshots, &clients, 16).unwrap();
        assert_eq!(
            window
                .admit("original", &reference, false)
                .unwrap()
                .positions,
            0..16
        );
    }

    #[test]
    fn oversized_tile_fails_instead_of_waiting_forever_for_credit() {
        let (reference, snapshots, clients) = plans(64_000_001, 1);
        assert!(CaptureWindow::new(&reference, &snapshots, &clients, 4).is_err());
    }

    #[test]
    fn unequal_capture_plan_lengths_fail_before_batches_are_indexed() {
        let (reference, mut snapshots, clients) = plans(1, 4);
        snapshots.get_mut("variant").unwrap().tiles.pop();
        let error = CaptureWindow::new(&reference, &snapshots, &clients, 4)
            .err()
            .unwrap();
        assert!(error.to_string().contains("different lengths"));
    }

    #[test]
    fn equal_png_survives_until_host_result_validation_finishes() -> Result<()> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "pipeline-evidence")?;
        fs::create_dir_all(&directory)?;
        let path = directory.join("tile.png");
        fs::write(&path, b"evidence").unwrap();
        let key = ("original".into(), 0);
        let mut evidence = BTreeMap::from([(
            key.clone(),
            Evidence {
                path: path.clone(),
                bytes: 8,
                response_received: false,
                keep: Some(false),
            },
        )]);
        retire_evidence(&mut evidence).unwrap();
        assert!(path.is_file());
        evidence.get_mut(&key).unwrap().response_received = true;
        retire_evidence(&mut evidence).unwrap();
        assert!(evidence.is_empty());
        assert!(!path.exists());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn validated_png_survives_until_comparison_decides_its_retention() -> Result<()> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "pipeline-evidence")?;
        fs::create_dir_all(&directory)?;
        let path = directory.join("tile.png");
        fs::write(&path, b"evidence")?;
        let key = ("original".into(), 0);
        let mut evidence = BTreeMap::from([(
            key.clone(),
            Evidence {
                path: path.clone(),
                bytes: 8,
                response_received: true,
                keep: None,
            },
        )]);
        retire_evidence(&mut evidence)?;
        assert!(path.is_file());
        evidence.get_mut(&key).unwrap().keep = Some(false);
        retire_evidence(&mut evidence)?;
        assert!(evidence.is_empty());
        assert!(!path.exists());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn different_or_height_evidence_is_retained_after_both_steps_finish() -> Result<()> {
        let directory = artifact_cache::unique_path(&std::env::temp_dir(), "pipeline-evidence")?;
        fs::create_dir_all(&directory)?;
        let path = directory.join("tile.png");
        fs::write(&path, b"evidence")?;
        let mut evidence = BTreeMap::from([(
            ("original".into(), 0),
            Evidence {
                path: path.clone(),
                bytes: 8,
                response_received: true,
                keep: Some(true),
            },
        )]);
        retire_evidence(&mut evidence)?;
        assert!(evidence.is_empty());
        assert_eq!(fs::read(path)?, b"evidence");
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
