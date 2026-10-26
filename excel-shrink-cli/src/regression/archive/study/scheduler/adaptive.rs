//! Estimate enforceable group budgets from live and completed memory samples.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::collections::VecDeque;

use super::super::control_room::DEFAULT_SYSTEM_RESERVE_MIB;
use super::super::prepared::PROFILES;
use super::MIB;

pub(super) const GIB: u64 = 1024 * 1024 * 1024;
const HISTORY_LENGTH: usize = 8;

#[derive(Clone, Copy, Debug)]
pub(super) struct Config {
    pub worker: u64,
    pub resident: u64,
    pub automation: u64,
    pub system_reserve: Option<u64>,
    pub helper_reserve: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Memory {
    pub physical: u64,
    pub commit: u64,
    pub total_physical: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    pub workers: usize,
    pub resident_limit: u64,
    pub group_reserve: u64,
    pub system_reserve: u64,
    pub helper_reserve: u64,
    pub reference_peak_bytes: Option<u64>,
    pub calibration_samples: usize,
}

pub(super) struct Controller {
    requested: usize,
    config: Config,
    width_ceiling: usize,
    normalized_peaks: VecDeque<u64>,
}

impl Controller {
    pub fn new(requested: usize, config: Config) -> Self {
        Self {
            requested,
            config,
            width_ceiling: 1,
            normalized_peaks: VecDeque::new(),
        }
    }

    pub fn plan(&self, memory: Memory, remaining: usize) -> Result<Plan> {
        self.plan_with_peak(memory, remaining, None)
    }

    // Existing jobs retain their original enforceable limits. Only commit can
    // credit their measured current usage: private commit is not physical RAM.
    pub fn plan_next(
        &self,
        memory: Memory,
        reserved_physical: u64,
        reserved_commit: u64,
        live_peak: Option<u64>,
    ) -> Result<Plan> {
        self.plan_with_peak(
            Memory {
                physical: memory.physical.saturating_sub(reserved_physical),
                commit: memory.commit.saturating_sub(reserved_commit),
                ..memory
            },
            1,
            live_peak,
        )
    }

    fn plan_with_peak(
        &self,
        memory: Memory,
        remaining: usize,
        live_peak: Option<u64>,
    ) -> Result<Plan> {
        ensure!(
            self.requested > 0,
            "comparison worker count must be positive"
        );
        ensure!(remaining > 0, "cannot schedule an empty comparison wave");
        ensure!(
            memory.total_physical > 0,
            "total physical RAM is unavailable"
        );
        ensure!(
            self.config.worker > 0 && self.config.resident >= GIB && self.config.automation > 0,
            "invalid comparison memory limits"
        );
        ensure!(
            self.config.helper_reserve >= GIB / 4
                && self.config.system_reserve.is_none_or(|bytes| {
                    (MIB..=65536 * MIB).contains(&bytes) && bytes % MIB == 0
                }),
            "comparison reserves require at least 256 MiB for helpers and 1..65536 whole MiB for the system"
        );
        let reference_peak_bytes = self.normalized_peaks.iter().copied().chain(live_peak).max();
        let resident_limit = reference_peak_bytes
            .map(|peak| estimated_limit(peak, self.config.resident))
            .unwrap_or(self.config.resident);
        // Rust retains its full allocation limit. COM and resident Excel run in
        // separate phases, so reserve the larger of their enforceable limits.
        let group_reserve = self
            .config
            .worker
            .checked_add(resident_limit.max(self.config.automation))
            .and_then(|bytes| bytes.checked_add(self.config.helper_reserve))
            .context("comparison memory reserve overflow")?;
        let system_reserve = self
            .config
            .system_reserve
            .unwrap_or(DEFAULT_SYSTEM_RESERVE_MIB * MIB);
        let available = memory.physical.min(memory.commit);
        let affordable = available.saturating_sub(system_reserve) / group_reserve;
        let workers = self
            .requested
            .min(self.width_ceiling)
            .min(remaining)
            .min(usize::try_from(affordable).unwrap_or(usize::MAX));
        ensure!(
            workers > 0,
            "Insufficient RAM for one resident comparison: physical {:.1} GiB, commit {:.1} GiB; group reserve {:.1} GiB plus system reserve {:.1} GiB required",
            memory.physical as f64 / GIB as f64,
            memory.commit as f64 / GIB as f64,
            group_reserve as f64 / GIB as f64,
            system_reserve as f64 / GIB as f64,
        );
        Ok(Plan {
            workers,
            resident_limit,
            group_reserve,
            system_reserve,
            helper_reserve: self.config.helper_reserve,
            reference_peak_bytes,
            calibration_samples: self.normalized_peaks.len(),
        })
    }

    // Observe the original resident records before classic fallback replaces
    // them. A smaller file is not evidence that its memory demand is smaller.
    pub fn observe(&mut self, records: &[Value]) {
        if records.is_empty() {
            return;
        }
        if records.iter().any(reduced_budget_failure) {
            self.normalized_peaks.clear();
            self.width_ceiling = 1;
            return;
        }
        let mut learned = 0;
        for record in records {
            if let Some(peak) = completed_peak(record) {
                self.normalized_peaks.push_back(peak);
                if self.normalized_peaks.len() > HISTORY_LENGTH {
                    self.normalized_peaks.pop_front();
                }
                learned += 1;
            }
        }
        self.width_ceiling = if learned == records.len() {
            records.len().saturating_add(1).min(self.requested)
        } else {
            // A failed or unmeasured group cannot justify adding another slot.
            self.width_ceiling.min(records.len()).max(1)
        };
    }
}

fn primary_complete(record: &Value) -> bool {
    record["results"].as_array().is_some_and(|profiles| {
        PROFILES.iter().all(|name| {
            profiles
                .iter()
                .any(|profile| profile["profile"] == *name && profile["checks_complete"] == true)
        })
    })
}

fn completed_peak(record: &Value) -> Option<u64> {
    let control_status = record["whitespace_diagnosis"]["status"].as_str();
    let controls_complete = control_status == Some("complete");
    let controls_not_needed = record["status"] == "unchanged"
        && matches!(control_status, None | Some("not_investigated"));
    if record["capture_mode"] != "resident"
        || record["complete"] != true
        || !matches!(record["status"].as_str(), Some("unchanged" | "differences"))
        || !primary_complete(record)
        || record["resident_capture"]["status"] != "complete"
        || !(controls_complete || controls_not_needed)
    {
        return None;
    }
    let metrics = &record["resident_capture"];
    let peak = metrics["peak_job_bytes"]
        .as_u64()
        .filter(|&peak| peak > 0)?;
    let clients = metrics["clients"].as_object()?.len();
    if clients == 0 {
        return None;
    }
    // Identical profiles can share a single Excel process. Leave room for four
    // primary clients and two controls even when the calibration used aliases.
    normalized_peak(peak, clients)
}

pub(super) fn normalized_peak(peak: u64, clients: usize) -> Option<u64> {
    if peak == 0 || clients == 0 {
        return None;
    }
    let numerator = u128::from(peak) * clients.max(6) as u128;
    Some(ceil_div(numerator, clients as u128).min(u128::from(u64::MAX)) as u64)
}

fn reduced_budget_failure(record: &Value) -> bool {
    if record["capture_mode"] != "resident"
        || record["status"] != "skipped"
        || primary_complete(record)
        || record["comparison_batch"]["adaptive"] != true
    {
        return false;
    }
    let batch = &record["comparison_batch"];
    let reduced = match (
        batch["resident_memory_limit_mib"].as_u64(),
        batch["configured_resident_memory_mib"].as_u64(),
    ) {
        (Some(effective), Some(configured)) => effective > 0 && effective < configured,
        _ => false,
    };
    let deterministic = std::iter::once(record["error"].as_str())
        .chain(
            record["results"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|profile| profile["checks_complete"] != true)
                .flat_map(|profile| profile["failures"].as_array().into_iter().flatten())
                .map(Value::as_str),
        )
        .flatten()
        .any(|message| {
            let lower = message.to_ascii_lowercase();
            lower.contains("drawing boundary is unresolved")
                || lower.contains("drawing anchor exceeds capture boundary")
        });
    reduced && !deterministic
}

fn ceil_div(value: u128, divisor: u128) -> u128 {
    value / divisor + u128::from(value % divisor != 0)
}

fn estimated_limit(peak: u64, configured: u64) -> u64 {
    // Keep the fractional 1.5 multiplier until rounding to avoid truncation or
    // overflow near a GiB boundary, including corrupted very large counters.
    let numerator = u128::from(peak) * 3 + u128::from(GIB) * 2;
    let rounded = ceil_div(numerator, u128::from(GIB) * 2) * u128::from(GIB);
    rounded.min(u128::from(configured)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn controller() -> Controller {
        Controller::new(
            8,
            Config {
                worker: 8 * GIB,
                resident: 16 * GIB,
                automation: 4 * GIB,
                system_reserve: None,
                helper_reserve: GIB,
            },
        )
    }

    fn memory() -> Memory {
        Memory {
            physical: 256 * GIB,
            commit: 256 * GIB,
            total_physical: 256 * GIB,
        }
    }

    fn completed(peak: u64, clients: usize) -> Value {
        let clients: serde_json::Map<String, Value> = (0..clients)
            .map(|index| (format!("client-{index}"), json!({"startup_seconds":1.0})))
            .collect();
        json!({
            "capture_mode":"resident","complete":true,"status":"unchanged",
            "source_bytes":100,
            "results":PROFILES.iter().map(|name| json!({"profile":name,"checks_complete":true})).collect::<Vec<_>>(),
            "resident_capture":{"status":"complete","peak_job_bytes":peak,"clients":clients},
            "comparison_batch":{"adaptive":true,"resident_memory_limit_mib":4096,"configured_resident_memory_mib":16384}
        })
    }

    fn failed(message: &str) -> Value {
        let mut record = completed(GIB, 6);
        record["status"] = json!("skipped");
        record["error"] = json!(message);
        record["results"][0]["checks_complete"] = json!(false);
        record
    }

    #[test]
    fn grows_one_slot_per_successful_wave_and_can_exceed_three_groups() -> Result<()> {
        let mut controller = controller();
        let initial = controller.plan(memory(), 100)?;
        assert_eq!(initial.resident_limit, 16 * GIB);
        assert_eq!(initial.reference_peak_bytes, None);
        assert_eq!(initial.calibration_samples, 0);
        for width in 1..=8 {
            assert_eq!(controller.plan(memory(), 100)?.workers, width);
            controller.observe(&vec![completed(GIB, 6); width]);
        }
        assert_eq!(controller.plan(memory(), 100)?.workers, 8);
        assert_eq!(controller.plan(memory(), 100)?.calibration_samples, 8);
        assert_eq!(
            controller.plan(memory(), 100)?.reference_peak_bytes,
            Some(GIB)
        );
        assert_eq!(controller.plan(memory(), 2)?.workers, 2);
        controller.observe(&vec![completed(GIB, 6); 2]);
        assert_eq!(controller.plan(memory(), 100)?.workers, 3);
        Ok(())
    }

    #[test]
    fn explicit_ceilings_above_sixteen_keep_memory_and_growth_limits() -> Result<()> {
        let plentiful = Memory {
            physical: 1024 * GIB,
            commit: 1024 * GIB,
            total_physical: 1024 * GIB,
        };
        for requested in [24, 256, usize::MAX] {
            let mut controller = Controller::new(requested, controller().config);
            for width in 1..=24 {
                assert_eq!(controller.plan(plentiful, 100)?.workers, width);
                controller.observe(&vec![completed(GIB, 6); width]);
            }
            assert_eq!(controller.plan(plentiful, 100)?.workers, requested.min(25));
            assert_eq!(controller.plan(plentiful, 2)?.workers, 2);
            let low_commit = Memory {
                commit: 40 * GIB - 1,
                ..plentiful
            };
            assert_eq!(controller.plan(low_commit, 100)?.workers, 2);
            let low_ram = Memory {
                physical: 40 * GIB - 1,
                ..plentiful
            };
            assert_eq!(controller.plan(low_ram, 100)?.workers, 2);
        }
        Ok(())
    }

    #[test]
    fn rounds_up_normalizes_clients_and_bounds_effective_limits() -> Result<()> {
        assert_eq!(estimated_limit(2 * GIB, 16 * GIB), 4 * GIB);
        assert_eq!(estimated_limit(2 * GIB + 1, 16 * GIB), 5 * GIB);
        assert_eq!(estimated_limit(u64::MAX, 16 * GIB), 16 * GIB);
        assert_eq!(completed_peak(&completed(u64::MAX, 1)), Some(u64::MAX));
        for (clients, expected) in [(1, 16), (3, 7), (6, 4), (8, 4)] {
            let mut controller = controller();
            controller.observe(&[completed(2 * GIB, clients)]);
            assert_eq!(
                controller.plan(memory(), 100)?.resident_limit,
                expected * GIB
            );
        }
        let mut controller = controller();
        controller.observe(&[completed(1, 6)]);
        let plan = controller.plan(memory(), 100)?;
        assert_eq!(plan.resident_limit, 2 * GIB);
        assert_eq!(plan.group_reserve, 13 * GIB); // COM still reserves its full 4 GiB.
        controller.config.worker = u64::MAX;
        assert!(controller
            .plan(memory(), 100)
            .unwrap_err()
            .to_string()
            .contains("overflow"));
        Ok(())
    }

    #[test]
    fn missing_or_partial_metrics_cannot_calibrate_or_increase_width() -> Result<()> {
        let valid = completed(GIB, 6);
        let mut invalid = Vec::new();
        for (field, value) in [
            ("peak_job_bytes", Value::Null),
            ("peak_job_bytes", json!(0)),
            ("clients", json!({})),
            ("clients", Value::Null),
            ("status", json!("failed")),
            ("status", json!("control failed")),
        ] {
            let mut record = valid.clone();
            record["resident_capture"][field] = value;
            invalid.push(record);
        }
        let mut incomplete = valid.clone();
        incomplete["complete"] = json!(false);
        invalid.push(incomplete);
        let mut partial = valid.clone();
        partial["results"][0]["checks_complete"] = json!(false);
        invalid.push(partial);
        let mut incomplete_control = valid.clone();
        incomplete_control["whitespace_diagnosis"] = json!({"status":"failed"});
        invalid.push(incomplete_control);
        let mut package_only = valid.clone();
        package_only["capture_mode"] = json!("package-audit");
        package_only["package_identity"] = json!({"status":"equivalent"});
        invalid.push(package_only);
        for record in invalid {
            let mut controller = controller();
            controller.observe(&[record]);
            let plan = controller.plan(memory(), 100)?;
            assert_eq!(plan.workers, 1);
            assert_eq!(plan.resident_limit, 16 * GIB);
        }
        Ok(())
    }

    #[test]
    fn whitespace_completion_is_required_only_when_controls_are_needed() -> Result<()> {
        for (status, diagnosis, learns) in [
            ("unchanged", "not_investigated", true),
            ("differences", "failed", false),
            ("differences", "not_investigated", false),
            ("differences", "complete", true),
        ] {
            let mut record = completed(GIB, 6);
            record["status"] = json!(status);
            record["whitespace_diagnosis"] = json!({"status":diagnosis});
            let mut controller = controller();
            controller.observe(&[record]);
            let plan = controller.plan(memory(), 100)?;
            assert_eq!(plan.calibration_samples, usize::from(learns));
            assert_eq!(plan.workers, if learns { 2 } else { 1 });
            assert_eq!(plan.resident_limit, if learns { 3 * GIB } else { 16 * GIB });
        }
        let mut missing = completed(GIB, 6);
        missing["status"] = json!("differences");
        assert_eq!(completed_peak(&missing), None);
        Ok(())
    }

    #[test]
    fn uses_last_eight_successful_peaks_without_inferring_from_file_size() -> Result<()> {
        let mut controller = controller();
        let mut large = completed(6 * GIB, 6);
        large["source_bytes"] = json!(1);
        controller.observe(&[large]);
        for _ in 0..7 {
            let mut small = completed(GIB, 6);
            small["source_bytes"] = json!(u64::MAX);
            controller.observe(&[small]);
        }
        assert_eq!(controller.plan(memory(), 100)?.resident_limit, 10 * GIB);
        controller.observe(&[completed(GIB, 6)]);
        assert_eq!(controller.plan(memory(), 100)?.resident_limit, 3 * GIB);
        Ok(())
    }

    #[test]
    fn reduced_cap_failure_resets_learning_but_boundary_failure_only_holds_width() -> Result<()> {
        for message in [
            "Drawing boundary is unresolved",
            "drawing anchor exceeds capture boundary",
        ] {
            let mut controller = controller();
            controller.observe(&[completed(GIB, 6)]);
            controller.observe(&[completed(GIB, 6), failed(message)]);
            let plan = controller.plan(memory(), 100)?;
            assert_eq!(plan.workers, 2);
            assert_eq!(plan.resident_limit, 3 * GIB);
        }
        let mut controller = controller();
        controller.observe(&[completed(GIB, 6)]);
        controller.observe(&[completed(GIB, 6), failed("resident Excel process exited")]);
        let plan = controller.plan(memory(), 100)?;
        assert_eq!(plan.workers, 1);
        assert_eq!(plan.resident_limit, 16 * GIB);
        Ok(())
    }

    #[test]
    fn commit_headroom_and_enforced_limits_bound_every_wave() -> Result<()> {
        let mut controller = controller();
        controller.observe(&[completed(GIB, 6)]);
        controller.observe(&vec![completed(GIB, 6); 2]);
        let mut available = memory();
        available.commit = 27 * GIB;
        let plan = controller.plan(available, 100)?;
        assert_eq!(plan.workers, 2);
        assert_eq!(plan.system_reserve, GIB);
        assert!(plan.workers as u64 * plan.group_reserve + plan.system_reserve <= available.commit);
        available.commit -= 1;
        assert_eq!(controller.plan(available, 100)?.workers, 1);
        available.commit = 14 * GIB - 1;
        assert!(controller
            .plan(available, 100)
            .unwrap_err()
            .to_string()
            .contains("Insufficient RAM"));
        available = memory();
        available.physical = 0;
        assert!(controller.plan(available, 100).is_err());
        Ok(())
    }

    #[test]
    fn default_system_reserve_is_1024_mib_on_every_machine() -> Result<()> {
        for total_physical in [8 * GIB, 40 * GIB, 64 * GIB, u64::MAX] {
            let available = Memory {
                total_physical,
                ..memory()
            };
            assert_eq!(controller().plan(available, 1)?.system_reserve, GIB);
        }
        Ok(())
    }

    #[test]
    fn system_reserve_accepts_whole_mib_without_changing_group_limits() -> Result<()> {
        let mut controller = controller();
        let original = controller.plan(memory(), 1)?;
        for mib in [1, 512, 1024, 65536] {
            controller.config.system_reserve = Some(mib * MIB);
            let plan = controller.plan(memory(), 1)?;
            assert_eq!(plan.system_reserve, mib * MIB);
            assert_eq!(plan.group_reserve, original.group_reserve);
            assert_eq!(plan.resident_limit, original.resident_limit);
            assert_eq!(plan.helper_reserve, original.helper_reserve);
        }
        for bytes in [0, MIB - 1, 65536 * MIB + 1] {
            controller.config.system_reserve = Some(bytes);
            assert!(controller.plan(memory(), 1).is_err());
        }
        Ok(())
    }

    #[test]
    fn explicit_small_machine_reserves_fit_exactly_without_lowering_worker_limits() -> Result<()> {
        let configured = Controller::new(
            1,
            Config {
                worker: GIB / 2,
                resident: 3 * GIB / 2,
                automation: GIB / 2,
                system_reserve: Some(3 * GIB / 2),
                helper_reserve: GIB / 2,
            },
        );
        let mut available = Memory {
            physical: 4 * GIB,
            commit: 4 * GIB,
            total_physical: 8 * GIB,
        };
        let plan = configured.plan(available, 10)?;
        assert_eq!(plan.workers, 1);
        assert_eq!(plan.resident_limit, 3 * GIB / 2);
        assert_eq!(plan.helper_reserve, GIB / 2);
        assert_eq!(plan.system_reserve, 3 * GIB / 2);
        assert_eq!(plan.group_reserve + plan.system_reserve, 4 * GIB);
        assert!(controller().plan(available, 10).is_err());
        available.physical -= 1;
        assert!(configured.plan(available, 10).is_err());
        available.physical += 1;
        available.commit -= 1;
        assert!(configured.plan(available, 10).is_err());
        Ok(())
    }
}
