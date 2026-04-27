use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub protocol_version: u32,
    pub transport_ready: bool,
    pub resume_ready: bool,
    pub commands: Vec<&'static str>,
    pub events: Vec<&'static str>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    StartRun(StartRunCommand),
    Pause,
    Resume,
    Stop,
    RefreshFileReference(RefreshFileReferenceCommand),
    RefreshDcAuth(RefreshDcAuthCommand),
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartRunCommand {
    pub protocol_version: u32,
    pub run_id: String,
    #[serde(default)]
    pub settings: RunSettings,
    pub auth_bundle: AuthBundle,
    #[serde(default)]
    pub jobs: Vec<DownloadJob>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunSettings {
    #[serde(default)]
    pub download_concurrency: usize,
    #[serde(default)]
    pub large_file_concurrency: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthBundle {
    pub api_id: i32,
    pub api_hash: String,
    #[serde(default)]
    pub current_dc_id: i32,
    #[serde(default)]
    pub self_id: Option<i64>,
    #[serde(default)]
    pub self_name: Option<String>,
    #[serde(default)]
    pub dc_options: Vec<DcOption>,
    #[serde(default)]
    pub exported_auth: HashMap<String, ExportedAuth>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DcOption {
    pub id: i32,
    pub ip_address: String,
    pub port: i32,
    #[serde(default)]
    pub ipv6: bool,
    #[serde(default)]
    pub media_only: bool,
    #[serde(default)]
    pub cdn: bool,
    #[serde(default)]
    pub tcpo_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportedAuth {
    pub id: i32,
    pub bytes_b64: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadJob {
    pub file_id: String,
    pub message_id: String,
    pub filename: String,
    pub category: String,
    pub final_path: String,
    pub temp_path: String,
    pub state_path: String,
    #[serde(default)]
    pub expected_size: u64,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub skip_if_complete: bool,
    #[serde(default)]
    pub resume_if_partial: bool,
    #[serde(default)]
    pub dc_id: i32,
    #[serde(default)]
    pub location: Value,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub input_chat: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefreshFileReferenceCommand {
    pub file_id: String,
    pub ok: bool,
    #[serde(default)]
    pub location: Option<Value>,
    #[serde(default)]
    pub dc_id: Option<i32>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefreshDcAuthCommand {
    pub dc_id: i32,
    pub ok: bool,
    #[serde(default)]
    pub auth: Option<ExportedAuth>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    RunStarted {
        run_id: String,
        file_count: usize,
    },
    FileStarted {
        file_id: String,
        message_id: String,
        filename: String,
        expected_size: u64,
    },
    FileProgress {
        file_id: String,
        message_id: String,
        filename: String,
        bytes_done: u64,
        expected_size: u64,
    },
    FileCompleted {
        file_id: String,
        message_id: String,
        filename: String,
        attachment_path: String,
    },
    FileSkipped {
        file_id: String,
        message_id: String,
        filename: String,
        attachment_path: String,
    },
    FileRestarted {
        file_id: String,
        message_id: String,
        filename: String,
    },
    FileError {
        file_id: String,
        message_id: String,
        filename: String,
        message: String,
    },
    TransportWindow {
        file_id: String,
        filename: String,
        inflight: usize,
        mbps: f64,
        parts: usize,
        progress: u64,
        total: u64,
    },
    TransportStall {
        file_id: String,
        filename: String,
        inflight: usize,
        progress: u64,
        total: u64,
        stalled_ms: u64,
    },
    RequestFileReferenceRefresh {
        file_id: String,
    },
    RequestDcAuthRefresh {
        dc_id: i32,
    },
    RunSummary {
        completed: usize,
        skipped: usize,
        failed: usize,
    },
    FatalError {
        message: String,
    },
}
