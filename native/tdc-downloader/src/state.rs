use crate::protocol::DownloadJob;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialState {
    pub file_id: String,
    pub message_id: String,
    pub final_path: String,
    pub temp_path: String,
    pub expected_size: u64,
    pub chunk_size: u64,
    #[serde(default)]
    pub completed_chunks: BTreeSet<u64>,
    #[serde(default)]
    pub dc_id: i32,
}

impl PartialState {
    pub fn from_job(job: &DownloadJob, chunk_size: u64) -> Self {
        Self {
            file_id: job.file_id.clone(),
            message_id: job.message_id.clone(),
            final_path: job.final_path.clone(),
            temp_path: job.temp_path.clone(),
            expected_size: job.expected_size,
            chunk_size,
            completed_chunks: BTreeSet::new(),
            dc_id: job.dc_id,
        }
    }

    pub fn load(path: &Path) -> Result<Option<Self>, StateError> {
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path)?;
        let state = serde_json::from_slice(&bytes)?;
        Ok(Some(state))
    }

    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::write(path, bytes)?;
        Ok(())
    }

    pub fn is_compatible_with(&self, job: &DownloadJob) -> bool {
        self.file_id == job.file_id
            && self.message_id == job.message_id
            && self.final_path == job.final_path
            && self.temp_path == job.temp_path
            && self.expected_size == job.expected_size
    }

    pub fn mark_chunk_complete(&mut self, offset: u64) {
        let index = offset / self.chunk_size;
        self.completed_chunks.insert(index);
    }
}
