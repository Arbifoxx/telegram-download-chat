use crate::protocol::{
    AuthBundle, Command, DcOption, DownloadJob, Event, ExportedAuth, RefreshDcAuthCommand,
    RefreshFileReferenceCommand, StartRunCommand,
};
use crate::state::{PartialState, StateError};
use base64::Engine as _;
use grammers_client::media::Downloadable;
use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::storages::MemorySession;
use grammers_session::types::DcOption as SessionDcOption;
use grammers_session::SessionData;
use grammers_tl_types as tl;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::sleep;

const CHUNK_SIZE: u64 = 128 * 1024;
const LARGE_FILE_THRESHOLD: u64 = 64 * 1024 * 1024;
const DEFAULT_SMALL_INFLIGHT: usize = 4;
const DEFAULT_LARGE_INFLIGHT: usize = 8;
const WINDOW_INTERVAL: Duration = Duration::from_secs(1);
const PAUSE_POLL_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("state error: {0}")]
    State(#[from] StateError),
    #[error("channel closed")]
    ChannelClosed,
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("invalid auth key length {0}")]
    InvalidAuthKeyLength(usize),
    #[error("invalid dc ip {0}")]
    InvalidDcIp(String),
    #[error("unsupported location type {0}")]
    UnsupportedLocation(String),
    #[error("missing field {0}")]
    MissingField(&'static str),
    #[error("grammers error: {0}")]
    Grammers(String),
}

#[derive(Clone)]
struct ControlState {
    paused: Arc<std::sync::atomic::AtomicBool>,
    stop_requested: Arc<std::sync::atomic::AtomicBool>,
    file_reference_responses: Arc<Mutex<HashMap<String, RefreshFileReferenceCommand>>>,
    dc_auth_responses: Arc<Mutex<HashMap<i32, RefreshDcAuthCommand>>>,
    file_reference_notify: Arc<Notify>,
    dc_auth_notify: Arc<Notify>,
}

impl ControlState {
    fn new() -> Self {
        Self {
            paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stop_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            file_reference_responses: Arc::new(Mutex::new(HashMap::new())),
            dc_auth_responses: Arc::new(Mutex::new(HashMap::new())),
            file_reference_notify: Arc::new(Notify::new()),
            dc_auth_notify: Arc::new(Notify::new()),
        }
    }

    fn is_paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn is_stop_requested(&self) -> bool {
        self.stop_requested
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn wait_if_paused(&self) {
        while self.is_paused() && !self.is_stop_requested() {
            sleep(PAUSE_POLL_INTERVAL).await;
        }
    }
}

#[derive(Clone)]
struct EventWriter {
    writer: Arc<Mutex<BufWriter<tokio::io::Stdout>>>,
}

impl EventWriter {
    fn new() -> Self {
        Self {
            writer: Arc::new(Mutex::new(BufWriter::new(tokio::io::stdout()))),
        }
    }

    async fn emit(&self, event: &Event) -> Result<(), RunnerError> {
        let mut writer = self.writer.lock().await;
        let line = serde_json::to_vec(event)?;
        writer.write_all(&line).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }
}

pub struct Runner {
    control: ControlState,
    events: EventWriter,
}

impl Runner {
    pub fn stdio() -> Self {
        Self {
            control: ControlState::new(),
            events: EventWriter::new(),
        }
    }

    pub async fn run(&mut self) -> Result<(), RunnerError> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Command>();
        let reader_handle = spawn_command_reader(tx);
        let mut active_run: Option<JoinHandle<Result<(), RunnerError>>> = None;

        loop {
            tokio::select! {
                maybe_command = rx.recv() => {
                    match maybe_command {
                        Some(Command::StartRun(start)) => {
                            if active_run.is_some() {
                                self.events.emit(&Event::FatalError {
                                    message: "start_run received while another run is active".to_string(),
                                }).await?;
                                continue;
                            }
                            self.control.stop_requested.store(false, std::sync::atomic::Ordering::Relaxed);
                            let control = self.control.clone();
                            let events = self.events.clone();
                            active_run = Some(tokio::spawn(async move {
                                handle_start_run(start, control, events).await
                            }));
                        }
                        Some(Command::Pause) => {
                            self.control.paused.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        Some(Command::Resume) => {
                            self.control.paused.store(false, std::sync::atomic::Ordering::Relaxed);
                        }
                        Some(Command::Stop) => {
                            self.control.stop_requested.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        Some(Command::RefreshFileReference(response)) => {
                            self.control.file_reference_responses.lock().await.insert(response.file_id.clone(), response);
                            self.control.file_reference_notify.notify_waiters();
                        }
                        Some(Command::RefreshDcAuth(response)) => {
                            self.control.dc_auth_responses.lock().await.insert(response.dc_id, response);
                            self.control.dc_auth_notify.notify_waiters();
                        }
                        None => {
                            break;
                        }
                    }
                }
                result = async {
                    match &mut active_run {
                        Some(handle) => Some(handle.await),
                        None => None,
                    }
                }, if active_run.is_some() => {
                    if let Some(join_result) = result {
                        active_run = None;
                        join_result.map_err(|error| RunnerError::Grammers(error.to_string()))??;
                    }
                }
            }
        }

        if let Some(handle) = active_run {
            handle.await.map_err(|error| RunnerError::Grammers(error.to_string()))??;
        }
        reader_handle
            .await
            .map_err(|error| RunnerError::Grammers(error.to_string()))??;
        Ok(())
    }
}

fn spawn_command_reader(tx: mpsc::UnboundedSender<Command>) -> JoinHandle<Result<(), RunnerError>> {
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin).lines();
        while let Some(line) = reader.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let command: Command = serde_json::from_str(&line)?;
            if tx.send(command).is_err() {
                break;
            }
        }
        Ok(())
    })
}

#[derive(Clone)]
struct NativeClientContext {
    client: Client,
    handle: grammers_mtsender::SenderPoolFatHandle,
    home_dc_id: i32,
    imported_dcs: Arc<Mutex<BTreeSet<i32>>>,
    exported_auth: HashMap<i32, ExportedAuth>,
}

impl NativeClientContext {
    async fn from_auth_bundle(auth_bundle: &AuthBundle) -> Result<Self, RunnerError> {
        let mut session_data = SessionData::default();
        session_data.home_dc = auth_bundle.current_dc_id;
        let mut dc_map = session_data.dc_options;

        for option in &auth_bundle.dc_options {
            let entry = dc_map.entry(option.id).or_insert_with(|| default_dc_option(option.id));
            update_dc_option(entry, option)?;
        }

        let home_auth_key = base64::engine::general_purpose::STANDARD
            .decode(&auth_bundle.home_auth_key_b64)?;
        if home_auth_key.len() != 256 {
            return Err(RunnerError::InvalidAuthKeyLength(home_auth_key.len()));
        }
        let mut key = [0u8; 256];
        key.copy_from_slice(&home_auth_key);

        let home_entry = dc_map
            .entry(auth_bundle.current_dc_id)
            .or_insert_with(|| default_dc_option(auth_bundle.current_dc_id));
        home_entry.auth_key = Some(key);

        session_data.dc_options = dc_map;
        let session = Arc::new(MemorySession::from(session_data));
        let SenderPool { runner, handle, .. } = SenderPool::new(Arc::clone(&session), auth_bundle.api_id);
        tokio::spawn(runner.run());
        let client = Client::new(handle.clone());

        Ok(Self {
            client,
            handle,
            home_dc_id: auth_bundle.current_dc_id,
            imported_dcs: Arc::new(Mutex::new(BTreeSet::new())),
            exported_auth: auth_bundle
                .exported_auth
                .iter()
                .filter_map(|(dc_id, auth)| dc_id.parse::<i32>().ok().map(|parsed| (parsed, auth.clone())))
                .collect(),
        })
    }

    async fn shutdown(&self) {
        let _ = self.handle.quit();
    }

    async fn ensure_dc_authorized(
        &self,
        dc_id: i32,
        control: &ControlState,
        events: &EventWriter,
    ) -> Result<(), RunnerError> {
        if dc_id == self.home_dc_id {
            return Ok(());
        }

        {
            let imported = self.imported_dcs.lock().await;
            if imported.contains(&dc_id) {
                return Ok(());
            }
        }

        let auth = if let Some(auth) = self.exported_auth.get(&dc_id) {
            auth.clone()
        } else {
            events
                .emit(&Event::RequestDcAuthRefresh { dc_id })
                .await?;
            wait_for_dc_auth(dc_id, control).await?
        };

        let bytes = base64::engine::general_purpose::STANDARD.decode(auth.bytes_b64)?;
        self.client
            .invoke_in_dc(
                dc_id,
                &tl::functions::auth::ImportAuthorization {
                    id: auth.id,
                    bytes,
                },
            )
            .await
            .map_err(|error| RunnerError::Grammers(error.to_string()))?;
        self.imported_dcs.lock().await.insert(dc_id);
        Ok(())
    }
}

async fn handle_start_run(
    start: StartRunCommand,
    control: ControlState,
    events: EventWriter,
) -> Result<(), RunnerError> {
    events
        .emit(&Event::RunStarted {
            run_id: start.run_id.clone(),
            file_count: start.jobs.len(),
        })
        .await?;

    let client = NativeClientContext::from_auth_bundle(&start.auth_bundle).await?;
    let mut completed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for job in &start.jobs {
        if control.is_stop_requested() {
            break;
        }

        let preflight = preflight_job(job)?;
        match preflight {
            PreflightOutcome::SkipExisting => {
                skipped += 1;
                events
                    .emit(&Event::FileSkipped {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                        attachment_path: relative_attachment_path(job),
                    })
                    .await?;
            }
            PreflightOutcome::Resume(state) => {
                events
                    .emit(&Event::FileStarted {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                        expected_size: job.expected_size,
                    })
                    .await?;
                match download_job(
                    job.clone(),
                    state,
                    &start.auth_bundle,
                    &start.settings,
                    &client,
                    &control,
                    &events,
                )
                .await
                {
                    Ok(DownloadOutcome::Completed) => completed += 1,
                    Ok(DownloadOutcome::Stopped) => break,
                    Err(error) => {
                        failed += 1;
                        events
                            .emit(&Event::FileError {
                                file_id: job.file_id.clone(),
                                message_id: job.message_id.clone(),
                                filename: job.filename.clone(),
                                message: error.to_string(),
                            })
                            .await?;
                    }
                }
            }
            PreflightOutcome::Restarted(state) => {
                events
                    .emit(&Event::FileRestarted {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                    })
                    .await?;
                events
                    .emit(&Event::FileStarted {
                        file_id: job.file_id.clone(),
                        message_id: job.message_id.clone(),
                        filename: job.filename.clone(),
                        expected_size: job.expected_size,
                    })
                    .await?;
                match download_job(
                    job.clone(),
                    state,
                    &start.auth_bundle,
                    &start.settings,
                    &client,
                    &control,
                    &events,
                )
                .await
                {
                    Ok(DownloadOutcome::Completed) => completed += 1,
                    Ok(DownloadOutcome::Stopped) => break,
                    Err(error) => {
                        failed += 1;
                        events
                            .emit(&Event::FileError {
                                file_id: job.file_id.clone(),
                                message_id: job.message_id.clone(),
                                filename: job.filename.clone(),
                                message: error.to_string(),
                            })
                            .await?;
                    }
                }
            }
        }
    }

    client.shutdown().await;
    events
        .emit(&Event::RunSummary {
            completed,
            skipped,
            failed,
        })
        .await?;
    Ok(())
}

enum DownloadOutcome {
    Completed,
    Stopped,
}

async fn download_job(
    job: DownloadJob,
    state: PartialState,
    _auth_bundle: &AuthBundle,
    settings: &crate::protocol::RunSettings,
    client: &NativeClientContext,
    control: &ControlState,
    events: &EventWriter,
) -> Result<DownloadOutcome, RunnerError> {
    let location = parse_input_location(&job.location)?;
    let large = job.expected_size >= LARGE_FILE_THRESHOLD;
    let requested_sessions = if large {
        settings.large_file_concurrency.max(1)
    } else {
        1
    };
    let requested_pipeline = if large { 4usize } else { 2usize };
    let inflight = if large {
        (requested_sessions * 2).clamp(DEFAULT_LARGE_INFLIGHT, 16)
    } else {
        DEFAULT_SMALL_INFLIGHT.clamp(1, 8)
    };

    events
        .emit(&Event::TransportPipeline {
            file_id: job.file_id.clone(),
            filename: job.filename.clone(),
            dc_id: job.dc_id,
            inflight,
            large,
            requested_pipeline,
            requested_sessions,
            worker: 0,
        })
        .await?;

    client
        .ensure_dc_authorized(job.dc_id, control, events)
        .await?;

    let file = prepare_temp_file(&job).await?;
    let file = Arc::new(Mutex::new(file));
    let total_chunks = total_chunks_for_size(job.expected_size);
    let completed_chunks = Arc::new(Mutex::new(state.completed_chunks.clone()));
    let state_holder = Arc::new(Mutex::new(state));
    let next_chunk = Arc::new(Mutex::new(0u64));
    let initial_bytes = {
        let guard = state_holder.lock().await;
        bytes_done_from_state(&guard, job.expected_size)
    };
    let progress = Arc::new(ProgressTracker::new(initial_bytes));

    let monitor = tokio::spawn(monitor_progress(
        job.file_id.clone(),
        job.message_id.clone(),
        job.filename.clone(),
        job.expected_size,
        inflight,
        Arc::clone(&progress),
        control.clone(),
        events.clone(),
    ));

    let mut workers = Vec::new();
    for _ in 0..inflight {
        let worker_job = job.clone();
        let worker_location = location.clone();
        let worker_client = client.client.clone();
        let worker_context = client.clone();
        let worker_control = control.clone();
        let worker_events = events.clone();
        let worker_file = Arc::clone(&file);
        let worker_chunks = Arc::clone(&completed_chunks);
        let worker_state = Arc::clone(&state_holder);
        let worker_next = Arc::clone(&next_chunk);
        let worker_progress = Arc::clone(&progress);

        workers.push(tokio::spawn(async move {
            worker_loop(
                worker_job,
                worker_location,
                total_chunks,
                worker_client,
                &worker_context,
                worker_control,
                worker_events,
                worker_file,
                worker_chunks,
                worker_state,
                worker_next,
                worker_progress,
            )
            .await
        }));
    }

    let mut first_error: Option<RunnerError> = None;
    for worker in workers {
        match worker.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(RunnerError::Grammers(error.to_string()));
                }
            }
        }
    }

    progress.mark_finished();
    let _ = monitor.await;

    if control.is_stop_requested() {
        return Ok(DownloadOutcome::Stopped);
    }
    if let Some(error) = first_error {
        return Err(error);
    }

    finalize_job(&job).await?;
    events
        .emit(&Event::FileCompleted {
            file_id: job.file_id.clone(),
            message_id: job.message_id.clone(),
            filename: job.filename.clone(),
            attachment_path: relative_attachment_path(&job),
        })
        .await?;
    Ok(DownloadOutcome::Completed)
}

#[allow(clippy::too_many_arguments)]
async fn worker_loop(
    mut job: DownloadJob,
    mut location: tl::enums::InputFileLocation,
    total_chunks: u64,
    client: Client,
    native_client: &NativeClientContext,
    control: ControlState,
    events: EventWriter,
    file: Arc<Mutex<tokio::fs::File>>,
    completed_chunks: Arc<Mutex<BTreeSet<u64>>>,
    state_holder: Arc<Mutex<PartialState>>,
    next_chunk: Arc<Mutex<u64>>,
    progress: Arc<ProgressTracker>,
) -> Result<(), RunnerError> {
    loop {
        control.wait_if_paused().await;
        if control.is_stop_requested() {
            break;
        }

        let chunk_index = {
            let mut cursor = next_chunk.lock().await;
            let mut chosen = None;
            while *cursor < total_chunks {
                let current = *cursor;
                *cursor += 1;
                if !completed_chunks.lock().await.contains(&current) {
                    chosen = Some(current);
                    break;
                }
            }
            chosen
        };

        let Some(chunk_index) = chunk_index else {
            break;
        };

        let offset = chunk_index * CHUNK_SIZE;
        progress.mark_inflight_start();
        let result = fetch_chunk(
            &client,
            native_client,
            &mut job,
            &mut location,
            offset,
            &control,
            &events,
        )
        .await;
        progress.mark_inflight_end();

        let bytes = match result {
            Ok(bytes) => bytes,
            Err(error) => return Err(error),
        };
        if bytes.is_empty() && control.is_stop_requested() {
            break;
        }

        {
            let mut temp = file.lock().await;
            use tokio::io::{AsyncSeekExt, AsyncWriteExt};
            temp.seek(std::io::SeekFrom::Start(offset)).await?;
            temp.write_all(&bytes).await?;
            temp.flush().await?;
        }

        {
            let mut completed = completed_chunks.lock().await;
            completed.insert(chunk_index);
        }
        {
            let mut state = state_holder.lock().await;
            state.mark_chunk_complete(offset);
            state.dc_id = job.dc_id;
            state.save(Path::new(&job.state_path))?;
        }
        progress.mark_bytes(bytes.len() as u64);

        if bytes.len() < CHUNK_SIZE as usize {
            break;
        }
    }

    Ok(())
}

async fn fetch_chunk(
    client: &Client,
    native_client: &NativeClientContext,
    job: &mut DownloadJob,
    location: &mut tl::enums::InputFileLocation,
    offset: u64,
    control: &ControlState,
    events: &EventWriter,
) -> Result<Vec<u8>, RunnerError> {
    let mut dc_id = job.dc_id;
    loop {
        if control.is_stop_requested() {
            return Ok(Vec::new());
        }

        native_client
            .ensure_dc_authorized(dc_id, control, events)
            .await?;

        let request = tl::functions::upload::GetFile {
            precise: true,
            cdn_supported: false,
            location: location.clone(),
            offset: offset as i64,
            limit: CHUNK_SIZE as i32,
        };

        match client.invoke_in_dc(dc_id, &request).await {
            Ok(tl::enums::upload::File::File(file)) => return Ok(file.bytes),
            Ok(tl::enums::upload::File::CdnRedirect(_)) => {
                return Err(RunnerError::Grammers("cdn redirects are not supported yet".to_string()));
            }
            Err(grammers_mtsender::InvocationError::Rpc(error))
                if error.name == "AUTH_KEY_UNREGISTERED" =>
            {
                native_client.ensure_dc_authorized(dc_id, control, events).await?;
                continue;
            }
            Err(grammers_mtsender::InvocationError::Rpc(error))
                if error.name.contains("FILE_REFERENCE") =>
            {
                let refresh = request_file_reference_refresh(&job.file_id, control, events).await?;
                if !refresh.ok {
                    return Err(RunnerError::Grammers(
                        refresh.error.unwrap_or_else(|| "file reference refresh failed".to_string()),
                    ));
                }
                if let Some(new_location) = refresh.location {
                    *location = parse_input_location(&new_location)?;
                }
                if let Some(new_dc) = refresh.dc_id {
                    job.dc_id = new_dc;
                    dc_id = new_dc;
                }
                continue;
            }
            Err(grammers_mtsender::InvocationError::Rpc(error))
                if error.code == 303 && error.value.is_some() =>
            {
                dc_id = error.value.unwrap() as i32;
                job.dc_id = dc_id;
                continue;
            }
            Err(error) => return Err(RunnerError::Grammers(error.to_string())),
        }
    }
}

async fn request_file_reference_refresh(
    file_id: &str,
    control: &ControlState,
    events: &EventWriter,
) -> Result<RefreshFileReferenceCommand, RunnerError> {
    events
        .emit(&Event::RequestFileReferenceRefresh {
            file_id: file_id.to_string(),
        })
        .await?;
    loop {
        if let Some(response) = control
            .file_reference_responses
            .lock()
            .await
            .remove(file_id)
        {
            return Ok(response);
        }
        control.file_reference_notify.notified().await;
    }
}

async fn wait_for_dc_auth(
    dc_id: i32,
    control: &ControlState,
) -> Result<ExportedAuth, RunnerError> {
    loop {
        if let Some(response) = control.dc_auth_responses.lock().await.remove(&dc_id) {
            if !response.ok {
                return Err(RunnerError::Grammers(
                    response
                        .error
                        .unwrap_or_else(|| format!("dc auth refresh failed for dc {dc_id}")),
                ));
            }
            if let Some(auth) = response.auth {
                return Ok(auth);
            }
            return Err(RunnerError::Grammers(format!(
                "dc auth refresh returned no auth for dc {dc_id}"
            )));
        }
        control.dc_auth_notify.notified().await;
    }
}

async fn prepare_temp_file(job: &DownloadJob) -> Result<tokio::fs::File, RunnerError> {
    if let Some(parent) = Path::new(&job.temp_path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&job.temp_path)
        .await?;
    if job.expected_size > 0 {
        file.set_len(job.expected_size).await?;
    }
    Ok(file)
}

async fn finalize_job(job: &DownloadJob) -> Result<(), RunnerError> {
    if let Some(parent) = Path::new(&job.final_path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(&job.temp_path, &job.final_path).await?;
    let state_path = Path::new(&job.state_path);
    if state_path.exists() {
        tokio::fs::remove_file(state_path).await?;
    }
    Ok(())
}

async fn monitor_progress(
    file_id: String,
    message_id: String,
    filename: String,
    total: u64,
    inflight: usize,
    progress: Arc<ProgressTracker>,
    control: ControlState,
    events: EventWriter,
) -> Result<(), RunnerError> {
    let mut last_bytes = progress.bytes_done();
    let mut last_tick = Instant::now();
    while !progress.is_finished() && !control.is_stop_requested() {
        sleep(WINDOW_INTERVAL).await;
        let current_bytes = progress.bytes_done();
        let bytes_delta = current_bytes.saturating_sub(last_bytes);
        let elapsed = last_tick.elapsed().as_secs_f64().max(0.001);
        let mbps = (bytes_delta as f64 * 8.0) / elapsed / 1_000_000.0;
        let completed_parts = (bytes_delta / CHUNK_SIZE) as usize;
        let inflight_now = progress.inflight();
        events
            .emit(&Event::FileProgress {
                file_id: file_id.clone(),
                message_id: message_id.clone(),
                filename: filename.clone(),
                bytes_done: current_bytes,
                expected_size: total,
            })
            .await?;
        if bytes_delta == 0 && inflight_now > 0 {
            events
                .emit(&Event::TransportStall {
                    file_id: file_id.clone(),
                    filename: filename.clone(),
                    inflight: inflight_now.max(inflight),
                    progress: current_bytes,
                    total,
                    stalled_ms: WINDOW_INTERVAL.as_millis() as u64,
                })
                .await?;
        }
        events
            .emit(&Event::TransportWindow {
                file_id: file_id.clone(),
                filename: filename.clone(),
                inflight: inflight_now.max(inflight),
                mbps,
                parts: completed_parts,
                progress: current_bytes,
                total,
            })
            .await?;
        last_bytes = current_bytes;
        last_tick = Instant::now();
    }
    Ok(())
}

struct ProgressTracker {
    bytes_done: std::sync::atomic::AtomicU64,
    inflight: std::sync::atomic::AtomicUsize,
    finished: std::sync::atomic::AtomicBool,
}

impl ProgressTracker {
    fn new(initial_bytes: u64) -> Self {
        Self {
            bytes_done: std::sync::atomic::AtomicU64::new(initial_bytes),
            inflight: std::sync::atomic::AtomicUsize::new(0),
            finished: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn mark_bytes(&self, bytes: u64) {
        self.bytes_done
            .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
    }

    fn bytes_done(&self) -> u64 {
        self.bytes_done.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn mark_inflight_start(&self) {
        self.inflight.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn mark_inflight_end(&self) {
        self.inflight.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn inflight(&self) -> usize {
        self.inflight.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn mark_finished(&self) {
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn is_finished(&self) -> bool {
        self.finished.load(std::sync::atomic::Ordering::Relaxed)
    }
}

enum PreflightOutcome {
    SkipExisting,
    Resume(PartialState),
    Restarted(PartialState),
}

fn preflight_job(job: &DownloadJob) -> Result<PreflightOutcome, RunnerError> {
    let final_path = PathBuf::from(&job.final_path);
    let temp_path = PathBuf::from(&job.temp_path);
    let state_path = PathBuf::from(&job.state_path);

    if job.overwrite {
        remove_if_exists(&final_path)?;
        remove_if_exists(&temp_path)?;
        remove_if_exists(&state_path)?;
        let state = PartialState::from_job(job, CHUNK_SIZE);
        state.save(&state_path)?;
        return Ok(PreflightOutcome::Restarted(state));
    }

    if job.skip_if_complete && is_complete_file(&final_path, job.expected_size)? {
        return Ok(PreflightOutcome::SkipExisting);
    }

    if job.resume_if_partial {
        if let Some(state) = PartialState::load(&state_path)? {
            if state.is_compatible_with(job) && temp_path.exists() {
                return Ok(PreflightOutcome::Resume(state));
            }
        }

        if temp_path.exists() || state_path.exists() {
            remove_if_exists(&temp_path)?;
            remove_if_exists(&state_path)?;
            let state = PartialState::from_job(job, CHUNK_SIZE);
            state.save(&state_path)?;
            return Ok(PreflightOutcome::Restarted(state));
        }
    }

    let state = PartialState::from_job(job, CHUNK_SIZE);
    state.save(&state_path)?;
    Ok(PreflightOutcome::Resume(state))
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

fn total_chunks_for_size(size: u64) -> u64 {
    if size == 0 {
        1
    } else {
        size.div_ceil(CHUNK_SIZE)
    }
}

fn bytes_done_from_state(state: &PartialState, expected_size: u64) -> u64 {
    let done = state.completed_chunks.len() as u64 * CHUNK_SIZE;
    if expected_size > 0 {
        done.min(expected_size)
    } else {
        done
    }
}

#[derive(Clone)]
struct RawLocation {
    location: tl::enums::InputFileLocation,
}

impl Downloadable for RawLocation {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(self.location.clone())
    }

    fn size(&self) -> Option<usize> {
        None
    }
}

fn parse_input_location(value: &serde_json::Value) -> Result<tl::enums::InputFileLocation, RunnerError> {
    let kind = value
        .get("_")
        .and_then(serde_json::Value::as_str)
        .ok_or(RunnerError::MissingField("_"))?;
    match kind {
        "InputDocumentFileLocation" => Ok(tl::types::InputDocumentFileLocation {
            id: get_i64(value, "id")?,
            access_hash: get_i64(value, "access_hash")?,
            file_reference: get_bytes(value, "file_reference")?,
            thumb_size: get_string(value, "thumb_size")?,
        }
        .into()),
        "InputPhotoFileLocation" => Ok(tl::types::InputPhotoFileLocation {
            id: get_i64(value, "id")?,
            access_hash: get_i64(value, "access_hash")?,
            file_reference: get_bytes(value, "file_reference")?,
            thumb_size: get_string(value, "thumb_size")?,
        }
        .into()),
        other => Err(RunnerError::UnsupportedLocation(other.to_string())),
    }
}

fn get_i64(value: &serde_json::Value, field: &'static str) -> Result<i64, RunnerError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_i64)
        .ok_or(RunnerError::MissingField(field))
}

fn get_string(value: &serde_json::Value, field: &'static str) -> Result<String, RunnerError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or(RunnerError::MissingField(field))
}

fn get_bytes(value: &serde_json::Value, field: &'static str) -> Result<Vec<u8>, RunnerError> {
    let Some(value) = value.get(field) else {
        return Err(RunnerError::MissingField(field));
    };
    if let Some(object) = value.as_object() {
        if let Some(encoded) = object.get("__bytes_b64__").and_then(serde_json::Value::as_str) {
            return Ok(base64::engine::general_purpose::STANDARD.decode(encoded)?);
        }
    }
    Err(RunnerError::MissingField(field))
}

fn default_dc_option(dc_id: i32) -> SessionDcOption {
    SessionDcOption {
        id: dc_id,
        ipv4: SocketAddrV4::new(Ipv4Addr::new(149, 154, 167, 50), 443),
        ipv6: SocketAddrV6::new(Ipv6Addr::LOCALHOST, 443, 0, 0),
        auth_key: None,
    }
}

fn update_dc_option(entry: &mut SessionDcOption, option: &DcOption) -> Result<(), RunnerError> {
    if option.ipv6 {
        let ipv6 = option
            .ip_address
            .parse::<Ipv6Addr>()
            .map_err(|_| RunnerError::InvalidDcIp(option.ip_address.clone()))?;
        entry.ipv6 = SocketAddrV6::new(ipv6, option.port as u16, 0, 0);
    } else {
        let ipv4 = option
            .ip_address
            .parse::<Ipv4Addr>()
            .map_err(|_| RunnerError::InvalidDcIp(option.ip_address.clone()))?;
        entry.ipv4 = SocketAddrV4::new(ipv4, option.port as u16);
    }
    Ok(())
}
