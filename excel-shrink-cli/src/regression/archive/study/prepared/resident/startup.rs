//! Native startup measurements are distinct from historical COM Open timings.
use serde_json::{json, Value};

const METHOD: &str = "launch-to-addin-ready-v1";

pub(crate) fn record(record: &mut Value, metrics: &Value) {
    let reference = record["provenance"]["comparison"].clone();
    let original = sample(metrics, "original");
    for profile in record["results"].as_array_mut().into_iter().flatten() {
        let Some(name) = profile["profile"].as_str() else { continue };
        let shrunk = sample(metrics, name);
        if original.is_null() && shrunk.is_null() {
            continue;
        }
        profile["native_startup"] = json!({
            "method":METHOD, "provenance":reference,
            "attempt_id":metrics["attempt_id"],
            "original":original, "shrunk":shrunk,
            "shared_client":!original.is_null() && !shrunk.is_null()
                && original["client_id"] == shrunk["client_id"]
        });
    }
}

fn sample(metrics: &Value, logical: &str) -> Value {
    let physical = metrics["capture_client_aliases"][logical].as_str().unwrap_or(logical);
    let client = &metrics["clients"][physical];
    if client["startup_measurement_method"] != METHOD
        || !client["startup_seconds"].as_f64().is_some_and(|v| v.is_finite() && v >= 0.0)
        || client["startup_sampled_unix_ms"].as_u64().is_none()
        || client["ready_at"].as_str().is_none()
        || client["pid"].as_u64().is_none()
        || client["process_started_unix_ms"].as_u64().is_none()
    {
        return Value::Null;
    }
    json!({"client_id":physical, "seconds":client["startup_seconds"],
        "window_mode":client["window_mode"], "hide_excel":client["hide_excel"],
        "private_bytes":client["startup_memory_private_bytes"],
        "working_set_bytes":client["startup_memory_working_set_bytes"],
        "pid":client["pid"], "process_started_unix_ms":client["process_started_unix_ms"],
        "ready_at":client["ready_at"], "sampled_unix_ms":client["startup_sampled_unix_ms"]})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(pid: u64, seconds: f64) -> Value {
        json!({"startup_measurement_method":METHOD,"startup_seconds":seconds,
            "startup_memory_private_bytes":200,"startup_memory_working_set_bytes":100,
            "pid":pid,"process_started_unix_ms":1000,"ready_at":"2026-10-25T10:00:00Z",
            "startup_sampled_unix_ms":2000,"window_mode":"private_desktop","hide_excel":true})
    }

    #[test]
    fn native_metrics_keep_physical_alias_and_attempt_without_relabelling_com_values() {
        let mut record_value = json!({"results":[{"profile":"default","original_open_seconds":20.0}],
            "provenance":{"comparison":{"run_id":"run","machine_id":"tower"}}});
        let metrics = json!({"attempt_id":"attempt-two","clients":{"original":client(10,3.0)},
            "capture_client_aliases":{"original":"original","default":"original"}});
        record(&mut record_value, &metrics);
        let profile = &record_value["results"][0];
        assert_eq!(profile["original_open_seconds"],20.0);
        assert!(profile["shrunk_open_seconds"].is_null());
        assert!(profile["open_measurement_provenance"].is_null());
        assert_eq!(profile["native_startup"]["original"],profile["native_startup"]["shrunk"]);
        assert_eq!(profile["native_startup"]["shared_client"],true);
        assert_eq!(profile["native_startup"]["provenance"]["machine_id"],"tower");
        assert_eq!(profile["native_startup"]["attempt_id"],"attempt-two");
        assert_eq!(profile["native_startup"]["original"]["window_mode"], "private_desktop");
        assert_eq!(profile["native_startup"]["original"]["hide_excel"], true);
    }

    #[test]
    fn partial_or_legacy_startup_does_not_invent_a_ready_sample() {
        let mut record_value = json!({"results":[{"profile":"default"}]});
        let mut metrics = json!({"clients":{"original":client(10,3.0),"default":{"startup_seconds":4}}});
        record(&mut record_value, &metrics);
        assert_eq!(record_value["results"][0]["native_startup"]["original"]["seconds"],3.0);
        assert!(record_value["results"][0]["native_startup"]["shrunk"].is_null());
        metrics["clients"]["default"] = client(11,2.0);
        record(&mut record_value, &metrics);
        assert_eq!(record_value["results"][0]["native_startup"]["shrunk"]["seconds"],2.0);
        assert_eq!(record_value["results"][0]["native_startup"]["shared_client"],false);
    }
}
