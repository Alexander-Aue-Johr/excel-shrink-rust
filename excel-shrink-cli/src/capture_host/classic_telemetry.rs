//! Observations from a classic Excel pair, kept outside its disposable workdir.
use super::super::{now_iso, now_ms, write_durable_bytes};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

pub(super) struct Telemetry {
    path: PathBuf,
    value: Value,
    profile: String,
}

impl Telemetry {
    pub fn new(path: PathBuf, attempt: &str, run: &str, profile: &str) -> Self {
        Self {
            path,
            value: json!({"attempt_id":attempt,"run_id":run,"capture_mode":"classic",
                "status":"running","client_reset_supported":false,"clients":{}}),
            profile: profile.to_owned(),
        }
    }

    pub fn observe(&mut self, data: &Value) -> Result<()> {
        ensure!(
            data["run_id"] == self.value["run_id"],
            "Capture run changed."
        );
        let side = data["side"].as_str().context("Missing capture side.")?;
        ensure!(
            matches!(side, "original" | "shrunk"),
            "Invalid capture side."
        );
        let client_id = if side == "original" {
            "original"
        } else {
            &self.profile
        };
        let time = now_iso();
        let millis = now_ms();
        let client = &mut self.value["clients"][client_id];
        if !client.is_object() {
            *client = json!({"status":"starting","stage":"starting","stage_since":time,
                "window_mode":"private_desktop","hide_excel":true,"heartbeat_count":0,
                "monitor":{"state":"starting","state_since":time,"events":[],
                    "uploaded_images":0,"completed_commands":0}});
        }
        if data["heartbeat"] == true {
            // Only an actual add-in request is a heartbeat. Launcher polling
            // and long-running Excel operations never manufacture one.
            client["last_heartbeat_at"] = json!(time);
            client["heartbeat_count"] = json!(client["heartbeat_count"].as_u64().unwrap_or(0) + 1);
        } else {
            // Requests already in flight when Excel exits cannot revive a
            // closed session. A new pair gets a new host and capture run id.
            if client["stage"] == "closed" {
                return Ok(());
            }
            let state = data["state"].as_str().context("Missing classic phase.")?;
            ensure!(
                matches!(
                    state,
                    "opening"
                        | "waiting_for_addin"
                        | "preparing_sheet"
                        | "checking_row_heights"
                        | "planning_sheet"
                        | "capturing"
                        | "waiting_for_peers"
                        | "providing_images"
                        | "finished"
                        | "closing"
                        | "closed"
                        | "failed"
                ),
                "Invalid classic phase."
            );
            if let Some(pid) = data["pid"].as_u64() {
                ensure!(
                    pid > 0 && pid <= u32::MAX as u64,
                    "Invalid Excel process id."
                );
                let started = data["process_started_unix_ms"]
                    .as_u64()
                    .context("Missing Excel process creation time.")?;
                client["pid"] = json!(pid);
                client["process_started_unix_ms"] = json!(started);
            }
            let monitor = &mut client["monitor"];
            if monitor["state"] != state {
                monitor["state_since"] = json!(time);
                client["stage_since"] = json!(time);
            }
            client["stage"] = json!(state);
            client["status"] = json!(state);
            let monitor = &mut client["monitor"];
            monitor["state"] = json!(state);
            monitor["wait_reason"] = data["wait_reason"].clone();
            let details = data["details"].as_object().cloned().unwrap_or_default();
            ensure!(
                serde_json::to_vec(&details)?.len() <= 4096,
                "Classic event details exceed the metadata budget."
            );
            if let Some(sheet) = details.get("sheet_name") {
                monitor["sheet_name"] = sheet.clone();
            }
            let kind = data["kind"].as_str().unwrap_or("classic_phase");
            let message = data["message"].as_str().unwrap_or(state);
            ensure!(
                kind.len() <= 80 && message.len() <= 1000,
                "Classic event exceeds the metadata budget."
            );
            let event = json!({"sequence":monitor["last_event"]["sequence"].as_u64().unwrap_or(0)+1,
                "at":time,"kind":kind,"message":message,"details":details});
            monitor["last_event"] = event.clone();
            if data["progress"] == true {
                monitor["last_progress_at"] = json!(time);
                monitor["last_success"] = event.clone();
            }
            if kind == "image_received" {
                monitor["uploaded_images"] =
                    json!(monitor["uploaded_images"].as_u64().unwrap_or(0) + 1);
            }
            let events = monitor["events"].as_array_mut().unwrap();
            events.push(event);
            if events.len() > 24 {
                events.remove(0);
            }
        }
        self.value["sampled_unix_ms"] = json!(millis);
        Ok(())
    }

    pub async fn publish(&self) -> Result<()> {
        write_durable_bytes(&self.path, &serde_json::to_vec(&self.value)?, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_heartbeats_do_not_count_as_work_or_reset_phase_age() -> Result<()> {
        let mut telemetry =
            Telemetry::new(PathBuf::from("unused.json"), "attempt", "run", "default");
        telemetry.observe(
            &json!({"run_id":"run","side":"original","state":"capturing",
            "progress":true,"kind":"image_received","details":{"sheet_name":"S1"}}),
        )?;
        telemetry.value["clients"]["original"]["monitor"]["last_progress_at"] =
            json!("earlier-progress");
        telemetry.value["clients"]["original"]["monitor"]["state_since"] =
            json!("earlier-phase-start");
        let progress =
            telemetry.value["clients"]["original"]["monitor"]["last_progress_at"].clone();
        let since = telemetry.value["clients"]["original"]["monitor"]["state_since"].clone();
        telemetry.observe(&json!({"run_id":"run","side":"original","heartbeat":true}))?;
        let client = &telemetry.value["clients"]["original"];
        assert_eq!(client["heartbeat_count"], 1);
        assert_eq!(client["monitor"]["uploaded_images"], 1);
        assert_eq!(client["monitor"]["last_progress_at"], progress);
        assert_eq!(client["monitor"]["state_since"], since);
        assert_eq!(client["monitor"]["events"].as_array().unwrap().len(), 1);
        assert!(telemetry
            .observe(&json!({"run_id":"other","side":"original","heartbeat":true}))
            .is_err());
        Ok(())
    }

    #[test]
    fn delayed_requests_cannot_reopen_a_closed_excel_session() -> Result<()> {
        let mut telemetry =
            Telemetry::new(PathBuf::from("unused.json"), "attempt", "run", "default");
        telemetry.observe(
            &json!({"run_id":"run","side":"original","state":"finished","progress":true}),
        )?;
        telemetry.observe(&json!({"run_id":"run","side":"original","state":"closed"}))?;
        let monitor = telemetry.value["clients"]["original"]["monitor"].clone();
        telemetry.observe(&json!({"run_id":"run","side":"original","heartbeat":true}))?;
        telemetry.observe(
            &json!({"run_id":"run","side":"original","state":"capturing","progress":true}),
        )?;
        assert_eq!(telemetry.value["clients"]["original"]["stage"], "closed");
        assert_eq!(telemetry.value["clients"]["original"]["monitor"], monitor);
        assert_eq!(telemetry.value["clients"]["original"]["heartbeat_count"], 1);
        Ok(())
    }

    #[test]
    fn history_is_bounded_and_variants_keep_their_profile_name() -> Result<()> {
        let mut telemetry = Telemetry::new(
            PathBuf::from("unused.json"),
            "attempt",
            "run",
            "maximum-cleaning",
        );
        for _ in 0..100 {
            telemetry.observe(&json!({"run_id":"run","side":"shrunk","state":"planning_sheet"}))?;
        }
        assert_eq!(
            telemetry.value["clients"]["maximum-cleaning"]["monitor"]["events"]
                .as_array()
                .unwrap()
                .len(),
            24
        );
        assert!(telemetry.value["clients"]["original"].is_null());
        Ok(())
    }
}
