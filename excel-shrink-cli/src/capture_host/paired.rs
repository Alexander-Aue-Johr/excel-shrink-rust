use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{watch, Mutex};
use tokio::time::timeout;

const PEER_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HEIGHT_RESPONSE: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Side {
    Original,
    Shrunk,
}

impl Side {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "original" => Ok(Self::Original),
            "shrunk" => Ok(Self::Shrunk),
            _ => bail!("Invalid capture side."),
        }
    }
}

#[derive(Clone)]
struct Completion(watch::Sender<Option<Result<Value, String>>>);

impl Completion {
    fn new() -> Self {
        Self(watch::channel(None).0)
    }

    fn publish(&self, result: &Result<Value>) {
        self.0.send_replace(Some(match result {
            Ok(value) => Ok(value.clone()),
            Err(error) => Err(error.to_string()),
        }));
    }

    fn stop(&self) {
        if self.0.borrow().is_none() {
            self.0
                .send_replace(Some(Err("Paired capture stopped.".into())));
        }
    }

    async fn wait(&self) -> Result<Value> {
        let mut receiver = self.0.subscribe();
        timeout(PEER_TIMEOUT, async {
            loop {
                if let Some(result) = receiver.borrow().clone() {
                    return result.map_err(anyhow::Error::msg);
                }
                receiver.changed().await.context("Paired exchange closed")?;
            }
        })
        .await
        .context("Timed out waiting for the paired workbook")?
    }
}

// The first client waits; the second performs the shared operation. Neither
// may start the next exchange until the consumer has acknowledged this one.
struct Exchange<T> {
    first: Option<(Side, T)>,
    processing: bool,
    completion: Completion,
}

impl<T> Exchange<T> {
    fn new() -> Self {
        Self {
            first: None,
            processing: false,
            completion: Completion::new(),
        }
    }

    fn arrive(&mut self, side: Side, value: T) -> Result<(Completion, Option<(T, T)>)> {
        ensure!(
            !self.processing,
            "Paired exchange already received both clients."
        );
        if let Some((first_side, _)) = &self.first {
            ensure!(*first_side != side, "Duplicate paired submission.");
        }
        let pair = match self.first.take() {
            None => {
                self.first = Some((side, value));
                None
            }
            Some((_, other)) => {
                self.processing = true;
                Some(if side == Side::Original {
                    (value, other)
                } else {
                    (other, value)
                })
            }
        };
        Ok((self.completion.clone(), pair))
    }
}

struct Image {
    metadata: Value,
    bytes: Vec<u8>,
}

struct State {
    sheet_ordinal: usize,
    tile_index: i64,
    capture_end_column: Option<i64>,
    started: HashSet<Side>,
    failed: bool,
    geometry: Exchange<Value>,
    heights: Exchange<Value>,
    image: Exchange<Image>,
    sheet: Exchange<()>,
}

impl State {
    fn new() -> Self {
        Self {
            sheet_ordinal: 0,
            tile_index: 0,
            capture_end_column: None,
            started: HashSet::new(),
            failed: false,
            geometry: Exchange::new(),
            heights: Exchange::new(),
            image: Exchange::new(),
            sheet: Exchange::new(),
        }
    }

    fn stop(&mut self) {
        self.failed = true;
        self.geometry.completion.stop();
        self.heights.completion.stop();
        self.image.completion.stop();
        self.sheet.completion.stop();
    }
}

pub(super) struct PairedCapture {
    run_id: String,
    sheets: Vec<i64>,
    state: Mutex<State>,
    pipe: Mutex<Pipe>,
}

impl PairedCapture {
    pub fn new(name: &str, run_id: String, sheets: Vec<i64>) -> Result<Self> {
        Ok(Self {
            run_id,
            sheets,
            state: Mutex::new(State::new()),
            pipe: Mutex::new(Pipe::new(name)?),
        })
    }

    pub fn validate(&self, data: &Value) -> Result<()> {
        ensure!(
            data["run_id"].as_str() == Some(&self.run_id),
            "Capture run changed."
        );
        Side::parse(data["side"].as_str().unwrap_or_default())?;
        Ok(())
    }

    fn check_sheet(&self, state: &State, data: &Value) -> Result<Side> {
        self.validate(data)?;
        ensure!(
            !state.failed
                && self
                    .sheets
                    .get(state.sheet_ordinal)
                    .is_some_and(|index| Some(*index) == data["sheet_index"].as_i64()),
            "Unexpected paired worksheet."
        );
        Side::parse(data["side"].as_str().unwrap_or_default())
    }

    pub async fn start_client(&self, side: &str) -> Result<bool> {
        let side = Side::parse(side)?;
        Ok(!self.state.lock().await.started.contains(&side))
    }

    pub async fn complete(&self) -> bool {
        self.state.lock().await.sheet_ordinal == self.sheets.len()
    }

    pub async fn heights(&self, data: Value) -> Result<Value> {
        let (ready, pair) = {
            let mut state = self.state.lock().await;
            let side = self.check_sheet(&state, &data)?;
            state.started.insert(side);
            state.heights.arrive(side, data)?
        };
        if let Some((original, shrunk)) = pair {
            let result = async {
                let metadata = json!({"kind":"heights", "sheet_index":original["sheet_index"]});
                let mut pipe = self.pipe.lock().await;
                pipe.frame(
                    &serde_json::to_vec(&metadata)?,
                    &serde_json::to_vec(&original["heights"])?,
                    &serde_json::to_vec(&shrunk["heights"])?,
                )
                .await?;
                pipe.height_response().await
            }
            .await;
            ready.publish(&result);
        }
        ready.wait().await
    }

    pub async fn geometry(&self, data: Value) -> Result<Value> {
        read_end_column(&data)?;
        read_rows(&data)?;
        let (ready, pair) = {
            let mut state = self.state.lock().await;
            let side = self.check_sheet(&state, &data)?;
            state.started.insert(side);
            state.geometry.arrive(side, data)?
        };
        if let Some((original, shrunk)) = pair {
            let result = negotiate_geometry(&original, &shrunk);
            if let Ok(response) = &result {
                let mut state = self.state.lock().await;
                state.capture_end_column = response["capture_right_column"].as_i64();
                state.tile_index = 0;
            }
            ready.publish(&result);
        }
        ready.wait().await
    }

    pub async fn image(&self, mut metadata: Value, bytes: Vec<u8>) -> Result<()> {
        let (ready, pair) = {
            let mut state = self.state.lock().await;
            let side = self.check_sheet(&state, &metadata)?;
            ensure!(
                metadata["tile_index"].as_i64() == Some(state.tile_index),
                "Unexpected paired image."
            );
            let end = state
                .capture_end_column
                .context("Paired image arrived before geometry.")?;
            if let Some((_, first)) = &state.image.first {
                ensure!(
                    first.metadata["range"] == metadata["range"]
                        && first.metadata["sheet_name"] == metadata["sheet_name"],
                    "Paired image identity differs."
                );
            }
            state.started.insert(side);
            metadata["capture_right_column"] = json!(end);
            state.image.arrive(side, Image { metadata, bytes })?
        };
        if let Some((original, shrunk)) = pair {
            let result = async {
                let mut pipe = self.pipe.lock().await;
                pipe.frame(
                    &serde_json::to_vec(&original.metadata)?,
                    &original.bytes,
                    &shrunk.bytes,
                )
                .await?;
                pipe.image_ack().await?;
                let mut state = self.state.lock().await;
                ensure!(!state.failed, "Paired capture stopped.");
                state.tile_index += 1;
                state.image = Exchange::new();
                Ok(json!(true))
            }
            .await;
            ready.publish(&result);
        }
        ready.wait().await?;
        Ok(())
    }

    pub async fn sheet_done(&self, data: Value) -> Result<()> {
        let ready = {
            let mut state = self.state.lock().await;
            let side = self.check_sheet(&state, &data)?;
            let (ready, pair) = state.sheet.arrive(side, ())?;
            if pair.is_some() {
                state.sheet_ordinal += 1;
                state.capture_end_column = None;
                state.geometry = Exchange::new();
                state.heights = Exchange::new();
                state.sheet = Exchange::new();
                ready.publish(&Ok(json!(true)));
            }
            ready
        };
        ready.wait().await?;
        Ok(())
    }

    pub async fn finish(&self, error: bool) -> Result<()> {
        if error {
            self.state.lock().await.stop();
        }
        self.pipe.lock().await.finish().await
    }
}

fn read_end_column(data: &Value) -> Result<i64> {
    let end = data["end_column"]
        .as_i64()
        .context("Missing capture boundary.")?;
    let maximum = data["max_end_column"]
        .as_i64()
        .context("Missing column limit.")?;
    ensure!(
        end >= 0 && end <= maximum && maximum < 16_384,
        "Invalid paired capture column boundary."
    );
    Ok(end)
}

fn read_rows(data: &Value) -> Result<Vec<[i64; 2]>> {
    let rows: Vec<[i64; 2]> = serde_json::from_value(data["tile_rows"].clone())?;
    ensure!(
        !rows.is_empty() && rows.len() <= 4096,
        "Invalid tile count."
    );
    let mut previous_end = 0;
    for [start, end] in &rows {
        ensure!(
            *start >= previous_end && start < end && *end <= 1_048_576,
            "Invalid tile rows."
        );
        previous_end = *end;
    }
    Ok(rows)
}

fn negotiate_geometry(original: &Value, shrunk: &Value) -> Result<Value> {
    ensure!(
        original["max_end_column"] == shrunk["max_end_column"],
        "Paired capture column limits differ."
    );
    let end_column = read_end_column(original)?.max(read_end_column(shrunk)?);
    let left = read_rows(original)?;
    let right = read_rows(shrunk)?;
    let mut cuts: Vec<i64> = left.iter().chain(&right).flatten().copied().collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut shared = Vec::new();
    for span in cuts.windows(2) {
        let covers = |rows: &[[i64; 2]]| {
            rows.iter()
                .any(|row| row[0] <= span[0] && row[1] >= span[1])
        };
        let left_covers = covers(&left);
        ensure!(
            left_covers == covers(&right),
            "Paired worksheet row coverage differs."
        );
        if left_covers {
            shared.push([span[0], span[1]]);
        }
    }
    ensure!(
        !shared.is_empty() && shared.len() <= 4096,
        "Invalid paired tile count."
    );
    Ok(json!({"tile_rows":shared, "capture_right_column":end_column}))
}

#[cfg(windows)]
struct Pipe {
    stream: tokio::net::windows::named_pipe::NamedPipeServer,
    connected: bool,
}

#[cfg(windows)]
impl Pipe {
    fn new(name: &str) -> Result<Self> {
        use tokio::net::windows::named_pipe::ServerOptions;
        ensure!(
            !name.is_empty() && !name.contains(['\\', '/']),
            "Invalid pipe name."
        );
        let stream = ServerOptions::new()
            .first_pipe_instance(true)
            .max_instances(1)
            .create(format!(r"\\.\pipe\{name}"))?;
        Ok(Self {
            stream,
            connected: false,
        })
    }

    async fn connect(&mut self) -> Result<()> {
        if !self.connected {
            timeout(CONNECT_TIMEOUT, self.stream.connect())
                .await
                .context("Pipe connection timed out")??;
            self.connected = true;
        }
        Ok(())
    }

    async fn frame(&mut self, metadata: &[u8], original: &[u8], shrunk: &[u8]) -> Result<()> {
        self.connect().await?;
        let lengths = [
            12 + metadata.len() + original.len() + shrunk.len(),
            metadata.len(),
            original.len(),
            shrunk.len(),
        ];
        timeout(PEER_TIMEOUT, async {
            for length in lengths {
                self.stream.write_i32_le(i32::try_from(length)?).await?;
            }
            self.stream.write_all(metadata).await?;
            self.stream.write_all(original).await?;
            self.stream.write_all(shrunk).await?;
            self.stream.flush().await?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("Timed out writing the comparison frame")?
    }

    async fn height_response(&mut self) -> Result<Value> {
        timeout(PEER_TIMEOUT, async {
            ensure!(
                self.stream.read_u8().await? == 3,
                "Row-height comparison failed."
            );
            let length = self.stream.read_i32_le().await?;
            ensure!(
                length >= 0 && length as usize <= MAX_HEIGHT_RESPONSE,
                "Invalid height response."
            );
            let mut body = vec![0; length as usize];
            self.stream.read_exact(&mut body).await?;
            let response = serde_json::from_slice(&body)?;
            ensure!(response != Value::Null, "Missing height response.");
            Ok(response)
        })
        .await
        .context("Timed out waiting for row-height comparison")?
    }

    async fn image_ack(&mut self) -> Result<()> {
        ensure!(
            timeout(PEER_TIMEOUT, self.stream.read_u8())
                .await
                .context("Pixel comparison timed out")??
                == 1,
            "Pixel comparison failed."
        );
        Ok(())
    }

    async fn finish(&mut self) -> Result<()> {
        self.connect().await?;
        timeout(PEER_TIMEOUT, async {
            self.stream.write_i32_le(0).await?;
            self.stream.flush().await
        })
        .await
        .context("Timed out writing the end frame")??;
        Ok(())
    }
}

#[cfg(not(windows))]
struct Pipe;

#[cfg(not(windows))]
impl Pipe {
    fn new(_: &str) -> Result<Self> {
        bail!("Paired Office capture requires Windows named pipes.")
    }
    async fn frame(&mut self, _: &[u8], _: &[u8], _: &[u8]) -> Result<()> {
        unreachable!()
    }
    async fn height_response(&mut self) -> Result<Value> {
        unreachable!()
    }
    async fn image_ack(&mut self) -> Result<()> {
        unreachable!()
    }
    async fn finish(&mut self) -> Result<()> {
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_preserves_both_boundaries_and_all_row_cuts() {
        let left =
            json!({"end_column":24,"max_end_column":65,"tile_rows":[[0,10],[10,20],[30,40]]});
        let right =
            json!({"end_column":30,"max_end_column":65,"tile_rows":[[0,15],[15,20],[30,40]]});
        let result = negotiate_geometry(&left, &right).unwrap();
        assert_eq!(
            result,
            json!({"capture_right_column":30,"tile_rows":[[0,10],[10,15],[15,20],[30,40]]})
        );
        let mut incomplete = right.clone();
        incomplete["tile_rows"] = json!([[0, 15], [30, 40]]);
        assert!(negotiate_geometry(&left, &incomplete).is_err());
    }

    #[test]
    fn rejects_invalid_column_bounds_and_overlapping_rows() {
        for (end, maximum) in [(-1, 65), (66, 65), (65, 16384)] {
            assert!(read_end_column(&json!({"end_column":end,"max_end_column":maximum})).is_err());
        }
        assert!(read_rows(&json!({"tile_rows":[[0,10],[9,20]]})).is_err());
    }

    #[tokio::test]
    async fn exchange_rejects_duplicate_and_unblocks_both_sides_on_failure() {
        let mut exchange = Exchange::new();
        let (first, pair) = exchange.arrive(Side::Original, 1).unwrap();
        assert!(pair.is_none());
        assert!(exchange.arrive(Side::Original, 2).is_err());
        let (second, pair) = exchange.arrive(Side::Shrunk, 3).unwrap();
        assert_eq!(pair, Some((1, 3)));
        assert!(exchange.arrive(Side::Original, 4).is_err());
        exchange.completion.stop();
        assert!(first.wait().await.is_err());
        assert!(second.wait().await.is_err());
    }
}
