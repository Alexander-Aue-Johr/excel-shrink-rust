//! Decode, compare and publish images without delaying the capture coordinator.
use super::*;
use anyhow::anyhow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

pub(super) struct Upload {
    pub position: usize,
    pub reference: Tile,
    pub client: String,
    pub tile: Tile,
    pub path: PathBuf,
    pub png_sha256: String,
}

pub(super) enum Event {
    Decoded {
        position: usize,
        client: String,
        tile: Tile,
        path: PathBuf,
        decode_ms: f64,
        preview_ms: f64,
    },
    Compared {
        position: usize,
        keep: bool,
        compare_ms: f64,
    },
    Failed(String),
}

pub(super) struct BatchSummary {
    pub pixels: u64,
    pub core_pixels: u64,
    pub images: usize,
    pub bytes: u64,
    pub elapsed: Duration,
    pub requested: usize,
    pub actual: usize,
}

#[derive(Default, Serialize)]
pub(super) struct ClientPeak {
    pub tiles: usize,
    pub pixels: u64,
}

#[derive(Default, Serialize)]
pub(super) struct ProcessorStats {
    pub clients: BTreeMap<String, ClientPeak>,
    pub cancelled: bool,
}

enum Command {
    Begin(Vec<Tile>),
    Upload(Upload),
    Summary(BatchSummary),
}

type ProcessorResult = (LiveCapture, Result<ProcessorStats>);

pub(super) struct ImageProcessor {
    commands: Option<SyncSender<Command>>,
    events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<ProcessorResult>>,
}

impl ImageProcessor {
    pub(super) fn start(
        mut live: LiveCapture,
        clients: BTreeSet<String>,
        aliases: BTreeMap<String, String>,
        height_sheets: BTreeSet<usize>,
        control_only: bool,
        notifier: SyncSender<()>,
    ) -> Result<Self> {
        ensure!(!clients.is_empty(), "image processor needs capture clients");
        ensure!(
            control_only || aliases.values().all(|client| clients.contains(client)),
            "image processor aliases include an unknown capture client"
        );
        let window = MAX_PENDING_TILES * clients.len();
        // Queue entries contain paths and metadata, never decoded image buffers.
        // Extra command slots accommodate activity and throughput updates.
        let (commands, incoming) = mpsc::sync_channel(window * 3 + 4);
        let (outgoing, events) = mpsc::sync_channel(window * 2 + 1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker = thread::Builder::new()
            .name("resident-images".into())
            .spawn(move || {
                let mut processing = Processing {
                    clients,
                    aliases,
                    height_sheets,
                    control_only,
                    pending: BTreeMap::new(),
                    usage: BTreeMap::new(),
                    stats: ProcessorStats::default(),
                    outgoing,
                    notifier,
                };
                // Return ownership of the dashboard even after a processing
                // panic, so the coordinator can record the failed attempt.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    processing.run(&mut live, incoming, &worker_cancel)
                }))
                .unwrap_or_else(|_| Err(anyhow!("resident image processor panicked")));
                if let Err(error) = &result {
                    let _ = processing.emit(Event::Failed(format!("{error:#}")));
                }
                // Even if Failed could not fit in a full result queue, wake
                // the coordinator after disconnecting it so try_event reports
                // termination instead of waiting for another Excel event.
                drop(processing.outgoing);
                let _ = processing.notifier.try_send(());
                (live, result)
            })
            .context("start resident image processor")?;
        Ok(Self {
            commands: Some(commands),
            events,
            cancel,
            worker: Some(worker),
        })
    }

    pub(super) fn begin(&self, tiles: Vec<Tile>) -> Result<()> {
        self.send(Command::Begin(tiles))
    }

    pub(super) fn submit(&self, upload: Upload) -> Result<()> {
        self.send(Command::Upload(upload))
    }

    pub(super) fn record_batch(&self, summary: BatchSummary) -> Result<()> {
        self.send(Command::Summary(summary))
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands
            .as_ref()
            .context("image processor is already closing")?
            .try_send(command)
            .map_err(|error| anyhow!("queue resident image processing: {error}"))
    }

    pub(super) fn try_event(&self) -> Result<Option<Event>> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(anyhow!(
                "resident image processor stopped before capture finished"
            )),
        }
    }

    pub(super) fn finish(mut self, abort: bool) -> Result<ProcessorResult> {
        self.cancel.store(abort, Ordering::Release);
        // Disconnect wakes an idle receiver and lets a normal finish drain its
        // queued work. Cancellation skips queued work after the current image.
        self.commands.take();
        self.worker
            .take()
            .context("image processor already joined")?
            .join()
            .map_err(|_| anyhow!("resident image processor could not return its state"))
    }
}

impl Drop for ImageProcessor {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

type PeerImages = BTreeMap<String, (Tile, PathBuf, image::RgbaImage)>;

#[derive(Default)]
struct Usage {
    tiles: usize,
    pixels: u64,
}

struct Processing {
    clients: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    height_sheets: BTreeSet<usize>,
    control_only: bool,
    pending: BTreeMap<usize, PeerImages>,
    usage: BTreeMap<String, Usage>,
    stats: ProcessorStats,
    outgoing: SyncSender<Event>,
    notifier: SyncSender<()>,
}

impl Processing {
    fn run(
        &mut self,
        live: &mut LiveCapture,
        incoming: Receiver<Command>,
        cancel: &AtomicBool,
    ) -> Result<ProcessorStats> {
        while let Ok(command) = incoming.recv() {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            match command {
                Command::Begin(tiles) if !self.control_only => live.active(&tiles)?,
                Command::Begin(_) => {}
                Command::Upload(upload) => self.upload(live, upload)?,
                Command::Summary(summary) => live.record_batch(
                    summary.pixels,
                    summary.core_pixels,
                    summary.images,
                    summary.bytes,
                    summary.elapsed,
                    summary.requested,
                    summary.actual,
                    self.control_only,
                )?,
            }
        }
        self.stats.cancelled = cancel.load(Ordering::Acquire);
        ensure!(
            self.stats.cancelled || self.pending.is_empty(),
            "image processing finished with {} tiles still awaiting peers",
            self.pending.len()
        );
        Ok(std::mem::take(&mut self.stats))
    }

    fn upload(&mut self, live: &mut LiveCapture, mut upload: Upload) -> Result<()> {
        let started = Instant::now();
        ensure!(
            self.clients.contains(&upload.client),
            "image upload from unknown client {}",
            upload.client
        );
        ensure!(
            !self
                .pending
                .get(&upload.position)
                .is_some_and(|peers| peers.contains_key(&upload.client)),
            "duplicate image upload for {} at position {}",
            upload.client,
            upload.position
        );
        ensure!(
            fs::metadata(&upload.path)?.len() <= MAX_PENDING_PNG_BYTES,
            "resident PNG exceeds the encoded image limit"
        );
        ensure!(
            artifact_cache::hash_file(&upload.path)? == upload.png_sha256,
            "resident PNG checksum differs from its upload receipt"
        );
        let reader = image::ImageReader::open(&upload.path)?.with_guessed_format()?;
        ensure!(
            reader.format() == Some(image::ImageFormat::Png),
            "resident image is not a PNG"
        );
        let dimensions = reader.into_dimensions()?;
        ensure!(
            dimensions == (upload.tile.width, upload.tile.height),
            "resident PNG dimensions differ from its capture plan"
        );
        // Check dimensions before allocating decoded pixels, including for
        // malformed uploads whose small compressed representation hides a huge image.
        let pixels = pending_pixels(&upload.tile);
        self.reserve(&upload.client, pixels)?;
        let rgba = image::open(&upload.path)?.to_rgba8();
        upload.tile.sha256 = artifact_cache::sha256(rgba.as_raw());
        let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
        let preview_started = Instant::now();
        if !self.control_only {
            live.received(
                &upload.reference,
                &upload.client,
                &upload.tile,
                &rgba,
                &self.aliases,
            )?;
        }
        let preview_ms = preview_started.elapsed().as_secs_f64() * 1000.0;
        self.emit(Event::Decoded {
            position: upload.position,
            client: upload.client.clone(),
            tile: upload.tile.clone(),
            path: upload.path.clone(),
            decode_ms,
            preview_ms,
        })?;
        let peers = self.pending.entry(upload.position).or_default();
        peers.insert(upload.client, (upload.tile, upload.path, rgba));
        if peers.len() < self.clients.len() {
            return Ok(());
        }
        let peers = self.pending.remove(&upload.position).unwrap();
        let comparison_started = Instant::now();
        let keep = self.control_only
            || live.retain_tile(
                &upload.reference,
                &self.aliases,
                &peers,
                self.height_sheets.contains(&upload.reference.sheet_index),
            )?;
        let compare_ms = comparison_started.elapsed().as_secs_f64() * 1000.0;
        for (client, (tile, _, _)) in &peers {
            let usage = self.usage.get_mut(client).unwrap();
            usage.tiles -= 1;
            usage.pixels -= pending_pixels(tile);
        }
        drop(peers);
        self.emit(Event::Compared {
            position: upload.position,
            keep,
            compare_ms,
        })
    }

    fn reserve(&mut self, client: &str, pixels: u64) -> Result<()> {
        let usage = self.usage.entry(client.to_owned()).or_default();
        ensure!(
            usage.tiles < MAX_PENDING_TILES
                && pixels <= MAX_PENDING_PIXELS.saturating_sub(usage.pixels),
            "resident image backlog exceeds the 16 tile / 64 megapixel limit for {client}"
        );
        usage.tiles += 1;
        usage.pixels += pixels;
        let peak = self.stats.clients.entry(client.to_owned()).or_default();
        peak.tiles = peak.tiles.max(usage.tiles);
        peak.pixels = peak.pixels.max(usage.pixels);
        Ok(())
    }

    fn emit(&self, event: Event) -> Result<()> {
        // Never block shutdown on a coordinator that has stopped receiving.
        // The scheduler must drain at most two events per outstanding image.
        self.outgoing
            .try_send(event)
            .map_err(|error| anyhow!("publish resident image processing result: {error}"))?;
        let _ = self.notifier.try_send(());
        Ok(())
    }
}

fn pending_pixels(tile: &Tile) -> u64 {
    (u64::from(tile.width) * u64::from(tile.height)).max(tile.capture_pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        live: Option<LiveCapture>,
    }

    impl Fixture {
        fn new(tile_count: usize) -> Result<Self> {
            let root = artifact_cache::unique_path(&std::env::temp_dir(), "resident-processing")?;
            let mut live = LiveCapture {
                root: root.clone(),
                directory: root.join("groups/fixture"),
                id: "fixture".into(),
                state: json!({"sheets":[]}),
                controls: RefCell::new(json!({"paused":false})),
            };
            fs::create_dir_all(live.images_directory())?;
            live.plan(&Snapshot {
                heights: vec![],
                command: "plan".into(),
                tiles: (0..tile_count).map(tile).collect(),
            })?;
            Ok(Self {
                root,
                live: Some(live),
            })
        }

        fn start(&mut self, clients: &[&str], height_sheets: &[usize]) -> Result<ImageProcessor> {
            let aliases = clients
                .iter()
                .map(|client| (client.to_string(), client.to_string()))
                .collect();
            let (notify, _) = mpsc::sync_channel(1);
            ImageProcessor::start(
                self.live.take().unwrap(),
                clients.iter().map(|client| client.to_string()).collect(),
                aliases,
                height_sheets.iter().copied().collect(),
                false,
                notify,
            )
        }

        fn upload(&self, position: usize, client: &str, changed: bool) -> Result<Upload> {
            let path = self.root.join(format!("{client}-{position}.png"));
            let mut image = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255]));
            if changed {
                image.put_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
            }
            save_png(&image, &path)?;
            Ok(Upload {
                position,
                reference: tile(position),
                client: client.into(),
                tile: tile(position),
                png_sha256: artifact_cache::hash_file(&path)?,
                path,
            })
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn tile(position: usize) -> Tile {
        Tile {
            sheet_index: 1,
            sheet_name: "Sheet".into(),
            tile_index: position,
            range: format!("A{}:B{}", position + 1, position + 1),
            core_rows: [position as u32, position as u32 + 1],
            capture_right_column: 1,
            width: 2,
            height: 2,
            x: 0,
            y: position as u32 * 2,
            capture_pixels: 4,
            sha256: String::new(),
        }
    }

    fn next_event(processor: &ImageProcessor) -> Result<Event> {
        processor
            .events
            .recv_timeout(Duration::from_secs(5))
            .context("wait for image processor test event")
    }

    #[test]
    fn compares_out_of_order_peers_and_retains_only_difference_evidence() -> Result<()> {
        let mut fixture = Fixture::new(2)?;
        let processor = fixture.start(&["original", "default"], &[])?;
        processor.begin(vec![tile(0), tile(1)])?;
        for (position, client, changed) in [
            (0, "original", false),
            (1, "original", false),
            (1, "default", true),
            (0, "default", false),
        ] {
            processor.submit(fixture.upload(position, client, changed)?)?;
        }
        let mut compared = Vec::new();
        let mut decoded = Vec::new();
        while compared.len() < 2 {
            match next_event(&processor)? {
                Event::Decoded {
                    position,
                    client,
                    tile,
                    path,
                    ..
                } => {
                    assert!(path.exists());
                    assert_eq!(
                        tile.sha256,
                        artifact_cache::sha256(image::open(path)?.to_rgba8().as_raw())
                    );
                    decoded.push((position, client));
                }
                Event::Compared { position, keep, .. } => compared.push((position, keep)),
                Event::Failed(error) => anyhow::bail!(error),
            }
        }
        assert_eq!(compared, [(1, true), (0, false)]);
        assert_eq!(decoded.len(), 4);
        let (live, stats) = processor.finish(false)?;
        let stats = stats?;
        assert_eq!(stats.clients["original"].tiles, 2);
        assert_eq!(stats.clients["original"].pixels, 8);
        assert_eq!(live.state["sheets"][0]["tiles"][0]["status"], "equal");
        assert_eq!(live.state["sheets"][0]["tiles"][1]["status"], "different");
        assert!(live.images_directory().join("s1-t1-default.png").exists());
        assert!(!live.images_directory().join("s1-t0-default.png").exists());
        Ok(())
    }

    #[test]
    fn equal_pixels_retain_source_when_row_heights_need_evidence() -> Result<()> {
        let mut fixture = Fixture::new(1)?;
        let processor = fixture.start(&["original"], &[1])?;
        processor.submit(fixture.upload(0, "original", false)?)?;
        assert!(matches!(next_event(&processor)?, Event::Decoded { .. }));
        assert!(matches!(
            next_event(&processor)?,
            Event::Compared { keep: true, .. }
        ));
        let (_, result) = processor.finish(false)?;
        result?;
        Ok(())
    }

    #[test]
    fn rejects_bad_png_receipt_and_returns_dashboard_ownership() -> Result<()> {
        let mut fixture = Fixture::new(1)?;
        let processor = fixture.start(&["original"], &[])?;
        let mut upload = fixture.upload(0, "original", false)?;
        upload.png_sha256 = "incorrect".into();
        processor.submit(upload)?;
        match next_event(&processor)? {
            Event::Failed(error) => assert!(error.contains("checksum")),
            _ => anyhow::bail!("invalid upload must not be published as decoded"),
        }
        let (live, result) = processor.finish(false)?;
        assert!(result.is_err());
        assert_eq!(live.id, "fixture");
        Ok(())
    }

    #[test]
    fn rejects_unexpected_png_dimensions_before_publishing_pixels() -> Result<()> {
        let mut fixture = Fixture::new(1)?;
        let processor = fixture.start(&["original"], &[])?;
        let mut upload = fixture.upload(0, "original", false)?;
        upload.tile.height = 3;
        processor.submit(upload)?;
        match next_event(&processor)? {
            Event::Failed(error) => assert!(error.contains("dimensions")),
            _ => anyhow::bail!("unexpected PNG dimensions must not be published"),
        }
        let (_, result) = processor.finish(false)?;
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn control_captures_do_not_replace_primary_dashboard_images() -> Result<()> {
        let mut fixture = Fixture::new(1)?;
        let (notify, _) = mpsc::sync_channel(1);
        let processor = ImageProcessor::start(
            fixture.live.take().unwrap(),
            BTreeSet::from(["control-default".into()]),
            BTreeMap::from([("original".into(), "original".into())]),
            BTreeSet::new(),
            true,
            notify,
        )?;
        processor.begin(vec![tile(0)])?;
        processor.submit(fixture.upload(0, "control-default", false)?)?;
        assert!(matches!(next_event(&processor)?, Event::Decoded { .. }));
        assert!(matches!(
            next_event(&processor)?,
            Event::Compared { keep: true, .. }
        ));
        let (live, result) = processor.finish(false)?;
        result?;
        assert_eq!(live.state["sheets"][0]["tiles"][0]["images"], json!({}));
        assert_eq!(live.state["sheets"][0]["tiles"][0]["status"], "pending");
        Ok(())
    }

    #[test]
    fn rejects_a_client_backlog_before_decoding_beyond_its_budget() -> Result<()> {
        let mut fixture = Fixture::new(2)?;
        let processor = fixture.start(&["original", "default"], &[])?;
        for position in 0..2 {
            let mut upload = fixture.upload(position, "original", false)?;
            upload.tile.capture_pixels = 40_000_000;
            processor.submit(upload)?;
        }
        assert!(matches!(next_event(&processor)?, Event::Decoded { .. }));
        match next_event(&processor)? {
            Event::Failed(error) => assert!(error.contains("64 megapixel limit")),
            _ => anyhow::bail!("oversized backlog must not decode another tile"),
        }
        let (_, result) = processor.finish(false)?;
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn graceful_finish_rejects_missing_peers_but_abort_can_leave_them_pending() -> Result<()> {
        for abort in [false, true] {
            let mut fixture = Fixture::new(1)?;
            let processor = fixture.start(&["original", "default"], &[])?;
            processor.submit(fixture.upload(0, "original", false)?)?;
            assert!(matches!(next_event(&processor)?, Event::Decoded { .. }));
            let (_, result) = processor.finish(abort)?;
            if abort {
                assert!(result?.cancelled);
            } else {
                assert!(result.err().unwrap().to_string().contains("awaiting peers"));
            }
        }
        Ok(())
    }

    #[test]
    fn abort_does_not_wait_for_the_coordinator_to_drain_a_full_queue() -> Result<()> {
        let mut fixture = Fixture::new(80)?;
        let processor = fixture.start(&["original"], &[])?;
        // This intentionally violates the scheduler window. Neither a full
        // result queue nor queued dashboard updates may prevent joining.
        for position in 0..80 {
            if processor
                .submit(fixture.upload(position, "original", false)?)
                .is_err()
            {
                break;
            }
        }
        let (live, _) = processor.finish(true)?;
        assert_eq!(live.id, "fixture");
        Ok(())
    }

    #[test]
    fn a_full_result_queue_terminates_without_needing_another_worker_event() -> Result<()> {
        let mut fixture = Fixture::new(17)?;
        let processor = fixture.start(&["original"], &[])?;
        // One client has 33 result slots. Its seventeenth complete tile must
        // fail explicitly, even when there is no room to send a Failed event.
        for position in 0..17 {
            processor.submit(fixture.upload(position, "original", false)?)?;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !processor.worker.as_ref().unwrap().is_finished() {
            ensure!(
                Instant::now() < deadline,
                "full result queue blocked shutdown"
            );
            thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..33 {
            ensure!(processor.try_event()?.is_some(), "missing buffered event");
        }
        assert!(processor.try_event().is_err());
        let (live, result) = processor.finish(false)?;
        assert_eq!(live.id, "fixture");
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("publish resident image processing result"));
        Ok(())
    }
}
