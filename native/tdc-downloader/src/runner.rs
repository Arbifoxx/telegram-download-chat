use crate::protocol::{Command, DownloadJob, Event, StartRunCommand};
use crate::state::{PartialState, StateError};
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

const CHUNK_SIZE: u64 = 128 * 1024;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("state error: {0}")]
    State(#[from] StateError),
}

pub struct Runner<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    paused: bool,
    stop_requested: bool,
}

impl Runner<BufReader<io::Stdin>, BufWriter<io::Stdout>> {
    pub fn stdio() -> Self {
        Self {
            reader: BufReader::new(io::stdin()),
            writer: BufWriter::new(io::stdout()),
            paused: false,
            stop_requested: false,
        }
    }
}

impl<R: BufRead, W: Write> Runner<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            paused: false,
            stop_requested: false,
        }
    }

    pub fn run(&mut self) -> Result<(), RunnerError> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                break;
            }
            if line.trim().is_empty() {
                continue;
            }

            let command: Command = serde_json::from_str(&line)?;
            match command {
                Command::StartRun(start) => self.handle_start_run(start)?,
                Command::Pause => self.paused = true,
                Command::Resume => self.paused = false,
                Command::Stop => self.stop_requested = true,
                Command::RefreshFileReference(_) | Command::RefreshDcAuth(_) => {}
            }
        }
        Ok(())
    }

    fn handle_start_run(&mut self, start: StartRunCommand) -> Result<(), RunnerError> {
        self.emit(&Event::RunStarted {
            run_id: start.run_id.clone(),
            file_count: start.jobs.len(),
        })?;

        let completed = 0usize;
        let mut skipped = 0usize;
        let failed = 0usize;

        for job in &start.jobs {
            if self.stop_requested {
                break;
            }

            match self.preflight_job(job)? {
                PreflightOutcome::SkipExisting => {
                    skipped += 1;
                    self.emit(&Event::FileSkipped {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                        attachment_path: relative_attachment_path(job),
                    })?;
                }
                PreflightOutcome::Resume(_) => {
                    self.emit(&Event::FileStarted {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                        expected_size: job.expected_size,
                    })?;
                }
                PreflightOutcome::Restarted => {
                    self.emit(&Event::FileRestarted {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                    })?;
                    self.emit(&Event::FileStarted {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                        expected_size: job.expected_size,
                    })?;
                }
            }
        }

        self.emit(&Event::RunSummary {
            completed,
            skipped,
            failed,
        })?;
        self.emit(&Event::FatalError {
            message: "Native MTProto transport is not enabled in this build yet".to_string(),
        })?;
        Ok(())
    }

    fn preflight_job(&self, job: &DownloadJob) -> Result<PreflightOutcome, RunnerError> {
        let final_path = PathBuf::from(&job.final_path);
        let temp_path = PathBuf::from(&job.temp_path);
        let state_path = PathBuf::from(&job.state_path);

        if job.overwrite {
            remove_if_exists(&final_path)?;
            remove_if_exists(&temp_path)?;
            remove_if_exists(&state_path)?;
            return Ok(PreflightOutcome::Restarted);
        }

        if job.skip_if_complete && is_complete_file(&final_path, job.expected_size)? {
            return Ok(PreflightOutcome::SkipExisting);
        }

        if job.resume_if_partial {
            let loaded = PartialState::load(&state_path)?;
            if let Some(state) = loaded {
                if state.is_compatible_with(job) && temp_path.exists() {
                    return Ok(PreflightOutcome::Resume(state));
                }
            }

            if temp_path.exists() || state_path.exists() {
                remove_if_exists(&temp_path)?;
                remove_if_exists(&state_path)?;
                let state = PartialState::from_job(job, CHUNK_SIZE);
                state.save(&state_path)?;
                return Ok(PreflightOutcome::Restarted);
            }
        }

        let state = PartialState::from_job(job, CHUNK_SIZE);
        state.save(&state_path)?;
        Ok(PreflightOutcome::Resume(state))
    }

    fn emit(&mut self, event: &Event) -> Result<(), RunnerError> {
        serde_json::to_writer(&mut self.writer, event)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
}

enum PreflightOutcome {
    SkipExisting,
    Resume(PartialState),
    Restarted,
}

fn remove_if_exists(path: &Path) -> Result<(), RunnerError> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn is_complete_file(path: &Path, expected_size: u64) -> Result<bool, RunnerError> {
    if !path.exists() {
        return Ok(false);
    }
    if expected_size == 0 {
        return Ok(true);
    }
    let meta = fs::metadata(path)?;
    Ok(meta.len() == expected_size)
}

fn relative_attachment_path(job: &DownloadJob) -> String {
    format!("{}/{}", job.category, job.filename)
}
