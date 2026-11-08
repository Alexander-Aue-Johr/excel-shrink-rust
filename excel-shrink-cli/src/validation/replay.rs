//! Bounded, independently decodable progress checkpoints for future static playback.
use anyhow::{ensure, Result};
use flate2::{write::GzEncoder, Compression};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf, time::Instant};

const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_CHUNK_BYTES: usize = 512 * 1024;
const MAX_CHUNK_EVENTS: usize = 32;

pub(super) struct Recorder {
    root: PathBuf,
    started: Instant,
    last_at_ms: u64,
    sequence: u64,
    pending: Vec<Value>,
    pending_bytes: usize,
    chunks: Vec<Value>,
}

impl Recorder {
    pub(super) fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir(&root)?;
        let recorder = Self {
            root,
            started: Instant::now(),
            last_at_ms: 0,
            sequence: 0,
            pending: Vec::new(),
            pending_bytes: 0,
            chunks: Vec::new(),
        };
        recorder.publish_manifest(false)?;
        Ok(recorder)
    }

    /// Each observation is a complete checkpoint, so seeking does not require earlier chunks.
    pub(super) fn record(&mut self, state: Value) -> Result<()> {
        self.record_at(self.started.elapsed().as_millis() as u64, state)
    }

    fn record_at(&mut self, at_ms: u64, state: Value) -> Result<()> {
        ensure!(at_ms >= self.last_at_ms, "Replay clock moved backwards");
        let event = json!({"sequence":self.sequence,"at_ms":at_ms,"state":state});
        let bytes = serde_json::to_vec(&event)?.len();
        ensure!(
            bytes <= MAX_EVENT_BYTES,
            "Replay checkpoint exceeds its size limit"
        );
        if self.pending_bytes + bytes > MAX_CHUNK_BYTES {
            self.flush()?;
        }
        self.pending.push(event);
        self.pending_bytes += bytes;
        self.sequence += 1;
        self.last_at_ms = at_ms;
        if self.pending.len() >= MAX_CHUNK_EVENTS {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let file = format!("chunk-{:06}.json.gz", self.chunks.len());
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        serde_json::to_writer(&mut encoder, &self.pending)?;
        encoder.flush()?;
        let compressed = encoder.finish()?;
        crate::regression::artifact_cache::atomic_write(&self.root.join(&file), &compressed)?;
        let entry = json!({
            "file":file,"sha256":format!("{:x}",Sha256::digest(&compressed)),
            "compressed_bytes":compressed.len(),"events":self.pending.len(),
            "first_sequence":self.pending[0]["sequence"],
            "first_at_ms":self.pending[0]["at_ms"],"last_at_ms":self.last_at_ms,
        });
        let mut index = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("chunks.jsonl"))?;
        serde_json::to_writer(&mut index, &entry)?;
        index.write_all(b"\n")?;
        index.sync_data()?;
        self.chunks.push(entry);
        self.pending.clear();
        self.pending_bytes = 0;
        Ok(())
    }

    fn publish_manifest(&self, complete: bool) -> Result<()> {
        let value = json!({
            "version":1,"channel":"run-progress","complete":complete,
            "duration_ms":self.last_at_ms,"events":self.sequence,
            "encoding":"gzip-json-checkpoints","clock":"elapsed-monotonic-milliseconds",
            "max_chunk_events":MAX_CHUNK_EVENTS,"max_chunk_bytes":MAX_CHUNK_BYTES + 64,
            "chunks":self.chunks,
            "interrupted_chunk_index":"chunks.jsonl",
            "availability":{
                "run_progress":"recorded at run orchestration progress updates",
                "process_sheet_capture_heartbeat":"not recorded by this channel",
                "interrupted_tail":"up to 31 unflushed checkpoints may be unavailable"
            }
        });
        crate::regression::artifact_cache::atomic_write(
            &self.root.join("manifest.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        self.flush()?;
        self.publish_manifest(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn checkpoints_roll_into_independent_bounded_chunks() -> Result<()> {
        let root = std::env::temp_dir().join(format!("replay-{}", uuid::Uuid::new_v4()));
        let mut recorder = Recorder::new(root.clone())?;
        for index in 0..70 {
            recorder.record_at(index * 10, json!({"completed":index}))?;
        }
        recorder.finish()?;
        let manifest: Value = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
        assert_eq!(manifest["complete"], true);
        assert_eq!(manifest["chunks"].as_array().unwrap().len(), 3);
        let mut events = Vec::new();
        for chunk in manifest["chunks"].as_array().unwrap() {
            let bytes = fs::read(root.join(chunk["file"].as_str().unwrap()))?;
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), chunk["sha256"]);
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut decoded)?;
            assert!(decoded.len() <= MAX_CHUNK_BYTES + 64);
            let observations: Vec<Value> = serde_json::from_slice(&decoded)?;
            assert!(observations.len() <= MAX_CHUNK_EVENTS);
            events.extend(observations);
        }
        assert_eq!(events.len(), 70);
        assert_eq!(events[48]["state"]["completed"], 48);
        Ok(())
    }

    #[test]
    fn invalid_clock_or_oversized_checkpoint_leaves_the_sequence_unchanged() -> Result<()> {
        let root = std::env::temp_dir().join(format!("replay-reject-{}", uuid::Uuid::new_v4()));
        let mut recorder = Recorder::new(root)?;
        recorder.record_at(10, json!({"completed":1}))?;
        assert!(recorder.record_at(9, Value::Null).is_err());
        assert!(recorder
            .record_at(11, json!("x".repeat(MAX_EVENT_BYTES)))
            .is_err());
        assert_eq!(recorder.sequence, 1);
        recorder.record_at(11, json!({"completed":2}))?;
        recorder.finish()?;
        Ok(())
    }

    #[test]
    fn byte_budget_flushes_and_interrupted_chunks_keep_a_durable_index() -> Result<()> {
        let root = std::env::temp_dir().join(format!("replay-bytes-{}", uuid::Uuid::new_v4()));
        let mut recorder = Recorder::new(root.clone())?;
        for index in 0..20 {
            recorder.record_at(index, json!({"detail":"x".repeat(60 * 1024)}))?;
        }
        assert!(recorder.chunks.len() >= 2);
        assert!(recorder.pending_bytes <= MAX_CHUNK_BYTES);
        drop(recorder);
        let index = fs::read_to_string(root.join("chunks.jsonl"))?;
        assert!(index.lines().count() >= 2);
        let manifest: Value = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
        assert_eq!(manifest["complete"], false);
        Ok(())
    }
}
