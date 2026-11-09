use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExcelOpenMetrics {
    pub open_seconds: f64,
    pub working_set_bytes: u64,
    pub private_bytes: u64,
    pub calculation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macros_removed: Option<bool>,
    /// Conversion copies use exclusive access instead of legacy shared editing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_editing_removed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_seconds: Option<std::collections::BTreeMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excel_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workbooks_converted: Option<u64>,
}
