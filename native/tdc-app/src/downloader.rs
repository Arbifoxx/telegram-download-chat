use std::collections::HashMap;
use std::collections::VecDeque;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine;
use chrono::{DateTime, Local, SecondsFormat};
use grammers_client::media::{Document, Downloadable, Media, Sticker};
use grammers_client::peer::Peer;
use grammers_client::Client;
use grammers_session::types::{PeerKind, PeerRef};
use grammers_session::storages::SqliteSession;
use grammers_session::Session;
use grammers_tl_types as tl;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use crate::config::{ActiveDownload, DownloadMode};
use crate::export::render_native_exports;
use crate::telegram;

const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct TelegramDownloadParams {
    pub api_id: String,
    pub api_hash: String,
    pub chat_input: String,
    pub output_path: String,
    pub overwrite_existing: bool,
    pub concurrent_downloads: u8,
    pub sort_descending: bool,
    pub html_export: bool,
    pub pdf_export: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DownloadSnapshot {
    pub files: Vec<ActiveDownload>,
    pub status_message: String,
    pub mode: DownloadMode,
    pub finished: bool,
}

#[derive(Debug, Clone)]
pub struct DownloadController {
    snapshot: Arc<Mutex<DownloadSnapshot>>,
    tx: mpsc::UnboundedSender<ControlCommand>,
}

#[derive(Debug, Clone)]
enum ControlCommand {
    Pause,
    Resume,
    Stop,
}

#[derive(Debug, Clone)]
struct JobContext {
    file_id: String,
    message_id: i32,
}

#[derive(Debug, Clone)]
struct RuntimeFile {
    file_id: String,
    filename: String,
    bytes_done: u64,
    expected_size: u64,
}

#[derive(Debug, Default)]
struct ScanResults {
    jobs: Vec<Value>,
    contexts: Vec<JobContext>,
    export_messages: Vec<Value>,
}

impl DownloadController {
    pub fn snapshot(&self) -> DownloadSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    pub fn pause(&self) {
        let _ = self.tx.send(ControlCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(ControlCommand::Resume);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(ControlCommand::Stop);
    }
}

pub async fn start_telegram_download(
    params: TelegramDownloadParams,
) -> Result<DownloadController, String> {
    if params.api_id.trim().is_empty() || params.api_hash.trim().is_empty() {
        return Err("Save API credentials first.".to_string());
    }
    if params.chat_input.trim().is_empty() {
        return Err("Enter a Telegram chat URL or username.".to_string());
    }
    if params.output_path.trim().is_empty() {
        return Err("Choose an output directory first.".to_string());
    }

    let snapshot = Arc::new(Mutex::new(DownloadSnapshot {
        status_message: "Preparing Telegram download...".to_string(),
        mode: DownloadMode::Running,
        finished: false,
        files: Vec::new(),
    }));
    let (tx, rx) = mpsc::unbounded_channel();
    let controller = DownloadController {
        snapshot: Arc::clone(&snapshot),
        tx,
    };

    tokio::spawn(async move {
        if let Err(error) = run_telegram_download(params, Arc::clone(&snapshot), rx).await {
            let mut state = snapshot.lock().unwrap();
            state.status_message = error;
            state.mode = DownloadMode::Stopped;
            state.finished = true;
            state.files.clear();
        }
    });

    Ok(controller)
}

async fn run_telegram_download(
    params: TelegramDownloadParams,
    snapshot: Arc<Mutex<DownloadSnapshot>>,
    mut rx: mpsc::UnboundedReceiver<ControlCommand>,
) -> Result<(), String> {
    set_status(&snapshot, "Connecting to Telegram...");
    let api_id = params
        .api_id
        .trim()
        .parse::<i32>()
        .map_err(|_| "API ID must be a number.".to_string())?;
    let client = telegram::connect_client(api_id, params.api_hash.trim()).await?;
    let me = client
        .get_me()
        .await
        .map_err(|error| format!("Failed to read Telegram session: {error}"))?;
    if !client
        .is_authorized()
        .await
        .map_err(|error| format!("Failed to verify Telegram session: {error}"))?
    {
        return Err("Sign in to Telegram before starting a download.".to_string());
    }

    set_status(&snapshot, "Resolving chat...");
    let (peer, peer_ref) = resolve_peer(&client, &params.chat_input).await?;
    let chat_title = peer_display_name(&peer);
    let output_root = PathBuf::from(params.output_path.trim());
    let output_json_path = output_root.join("output.json");
    let output_txt_path = output_root.join("output.txt");
    let output_html_path = output_root.join("output.html");
    let output_pdf_path = output_root.join("output.pdf");
    let attachments_dir = output_root.join("attachments");
    tokio::fs::create_dir_all(&attachments_dir)
        .await
        .map_err(|error| format!("Failed to create output directory: {error}"))?;

    set_status(&snapshot, "Scanning chat media...");
    let self_display_name = me.full_name();
    let mut scan = build_jobs(
        &client,
        peer_ref,
        &attachments_dir,
        &params,
        self_display_name.as_str(),
        Arc::clone(&snapshot),
    )
    .await?;
    if !params.sort_descending {
        scan.jobs.reverse();
        scan.export_messages.reverse();
    }
    write_message_exports(&scan.export_messages, &output_json_path, &output_txt_path).await?;
    if scan.jobs.is_empty() {
        finalize_attachment_paths(&mut scan.export_messages, &attachments_dir);
        write_message_exports(&scan.export_messages, &output_json_path, &output_txt_path).await?;
        if params.html_export || params.pdf_export {
            render_native_exports(
                &scan.export_messages,
                &attachments_dir,
                &chat_title,
                params.html_export.then_some(output_html_path.as_path()),
                params.pdf_export.then_some(output_pdf_path.as_path()),
            )
            .await?;
        }
        let mut state = snapshot.lock().unwrap();
        state.status_message = "Saved messages. No downloadable media found in this chat.".to_string();
        state.mode = DownloadMode::Stopped;
        state.finished = true;
        return Ok(());
    }
    let contexts_by_file_id = scan
        .contexts
        .into_iter()
        .map(|context| (context.file_id.clone(), context))
        .collect::<HashMap<_, _>>();

    set_status(&snapshot, "Preparing download transport...");
    let auth_bundle =
        build_auth_bundle(&client, api_id, &params.api_hash, &scan.jobs, me.full_name()).await?;
    let binary = locate_native_downloader_binary()
        .ok_or_else(|| "Rust downloader backend binary was not found.".to_string())?;

    let mut child = Command::new(binary)
        .arg("run")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to start downloader backend: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Downloader backend stdin was unavailable.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Downloader backend stdout was unavailable.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Downloader backend stderr was unavailable.".to_string())?;
    let stderr_tail = Arc::new(tokio::sync::Mutex::new(VecDeque::<String>::with_capacity(24)));
    let stderr_tail_reader = Arc::clone(&stderr_tail);
    tokio::spawn(async move {
        let mut stderr_reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = stderr_reader.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut tail = stderr_tail_reader.lock().await;
            if tail.len() == 24 {
                tail.pop_front();
            }
            tail.push_back(trimmed.to_string());
        }
    });

    let start_command = json!({
        "type": "start_run",
        "protocol_version": PROTOCOL_VERSION,
        "run_id": format!("tdc-app-{}", std::process::id()),
        "settings": {
            "download_concurrency": params.concurrent_downloads.clamp(1, 5),
        },
        "auth_bundle": auth_bundle,
        "jobs": scan.jobs,
    });
    send_command(&mut stdin, &start_command).await?;
    set_status(&snapshot, "Downloading media...");

    let mut active_files: Vec<RuntimeFile> = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    let mut paused = false;

    loop {
        tokio::select! {
            control = rx.recv() => {
                match control {
                    Some(ControlCommand::Pause) => {
                        send_command(&mut stdin, &json!({"type": "pause"})).await?;
                        paused = true;
                        let mut state = snapshot.lock().unwrap();
                        state.mode = DownloadMode::Paused;
                        state.status_message = "Paused.".to_string();
                    }
                    Some(ControlCommand::Resume) => {
                        send_command(&mut stdin, &json!({"type": "resume"})).await?;
                        paused = false;
                        let mut state = snapshot.lock().unwrap();
                        state.mode = DownloadMode::Running;
                        state.status_message = "Downloading media...".to_string();
                    }
                    Some(ControlCommand::Stop) => {
                        send_command(&mut stdin, &json!({"type": "stop"})).await?;
                    }
                    None => break,
                }
            }
            line = lines.next_line() => {
                let Some(line) = line.map_err(|error| format!("Failed to read downloader output: {error}"))? else {
                    break;
                };
                if line.trim().is_empty() {
                    continue;
                }
                let event: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                handle_event(
                    &client,
                    peer_ref,
                    &contexts_by_file_id,
                    &snapshot,
                    &mut stdin,
                    &mut active_files,
                    event,
                )
                .await?;
                if !paused {
                    snapshot.lock().unwrap().mode = DownloadMode::Running;
                }
            }
        }
    }

    let exit_status = child
        .wait()
        .await
        .map_err(|error| format!("Failed to wait for downloader backend: {error}"))?;
    let status = if exit_status.success() {
        if snapshot.lock().unwrap().status_message.is_empty() {
            "Telegram download finished.".to_string()
        } else {
            snapshot.lock().unwrap().status_message.clone()
        }
    } else {
        let current = snapshot.lock().unwrap().status_message.clone();
        let stderr_summary = backend_stderr_summary(&stderr_tail).await;
        if !stderr_summary.is_empty() {
            eprintln!("Downloader backend exited: {stderr_summary}");
        } else {
            eprintln!("Downloader backend exited with status {exit_status}.");
        }
        if current == "Preparing download transport..." || current == "Downloading media..." {
            String::new()
        } else {
            current
        }
    };

    finalize_attachment_paths(&mut scan.export_messages, &attachments_dir);
    write_message_exports(&scan.export_messages, &output_json_path, &output_txt_path).await?;
    if params.html_export || params.pdf_export {
        render_native_exports(
            &scan.export_messages,
            &attachments_dir,
            &chat_title,
            params.html_export.then_some(output_html_path.as_path()),
            params.pdf_export.then_some(output_pdf_path.as_path()),
        )
        .await?;
    }

    let mut state = snapshot.lock().unwrap();
    state.status_message = status;
    state.mode = DownloadMode::Stopped;
    state.finished = true;
    Ok(())
}

async fn handle_event(
    client: &Client,
    peer_ref: grammers_session::types::PeerRef,
    contexts: &HashMap<String, JobContext>,
    snapshot: &Arc<Mutex<DownloadSnapshot>>,
    stdin: &mut ChildStdin,
    active_files: &mut Vec<RuntimeFile>,
    event: Value,
) -> Result<(), String> {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "run_started" => {
            set_status(snapshot, "Downloader started.");
        }
        "file_started" => {
            let file_id = value_string(&event, "file_id");
            let filename = value_string(&event, "filename");
            let expected_size = value_u64(&event, "expected_size");
            upsert_file(active_files, RuntimeFile {
                file_id,
                filename,
                bytes_done: 0,
                expected_size,
            });
            publish_files(snapshot, active_files);
        }
        "file_progress" => {
            let file_id = value_string(&event, "file_id");
            let filename = value_string(&event, "filename");
            let bytes_done = value_u64(&event, "bytes_done");
            let expected_size = value_u64(&event, "expected_size");
            upsert_file(active_files, RuntimeFile {
                file_id,
                filename,
                bytes_done,
                expected_size,
            });
            publish_files(snapshot, active_files);
        }
        "file_completed" | "file_skipped" | "file_error" => {
            let file_id = value_string(&event, "file_id");
            active_files.retain(|file| file.file_id != file_id);
            publish_files(snapshot, active_files);
        }
        "request_file_reference_refresh" => {
            let file_id = value_string(&event, "file_id");
            let Some(context) = contexts.get(&file_id) else {
                send_command(
                    stdin,
                    &json!({
                        "type": "refresh_file_reference",
                        "file_id": file_id,
                        "ok": false,
                        "error": "unknown file id",
                    }),
                )
                .await?;
                return Ok(());
            };
            let refreshed = client
                .get_messages_by_id(peer_ref, &[context.message_id])
                .await
                .map_err(|error| format!("Failed to refresh Telegram message: {error}"))?;
            let Some(Some(message)) = refreshed.into_iter().next() else {
                send_command(
                    stdin,
                    &json!({
                        "type": "refresh_file_reference",
                        "file_id": file_id,
                        "ok": false,
                        "error": "message not found",
                    }),
                )
                .await?;
                return Ok(());
            };
            let Some(media) = message.media() else {
                send_command(
                    stdin,
                    &json!({
                        "type": "refresh_file_reference",
                        "file_id": file_id,
                        "ok": false,
                        "error": "message has no media",
                    }),
                )
                .await?;
                return Ok(());
            };
            send_command(
                stdin,
                &json!({
                    "type": "refresh_file_reference",
                    "file_id": file_id,
                    "ok": true,
                    "location": media_location_json(&media)?,
                    "dc_id": media_dc_id(&media),
                }),
            )
            .await?;
        }
        "request_dc_auth_refresh" => {
            let dc_id = value_i32(&event, "dc_id");
            let auth: tl::types::auth::ExportedAuthorization = client
                .invoke(&tl::functions::auth::ExportAuthorization { dc_id })
                .await
                .map_err(|error| format!("Failed to refresh exported auth: {error}"))?
                .into();
            send_command(
                stdin,
                &json!({
                    "type": "refresh_dc_auth",
                    "dc_id": dc_id,
                    "ok": true,
                    "auth": {
                        "id": auth.id,
                        "bytes_b64": base64::engine::general_purpose::STANDARD.encode(auth.bytes),
                    },
                }),
            )
            .await?;
        }
        "run_summary" => {
            let completed = value_u64(&event, "completed");
            let skipped = value_u64(&event, "skipped");
            let failed = value_u64(&event, "failed");
            active_files.clear();
            publish_files(snapshot, active_files);
            set_status(
                snapshot,
                &format!(
                    "Finished: {completed} completed, {skipped} skipped, {failed} failed."
                ),
            );
        }
        "fatal_error" => {
            let message = value_string(&event, "message");
            active_files.clear();
            publish_files(snapshot, active_files);
            set_status(snapshot, &message);
        }
        _ => {}
    }
    Ok(())
}

async fn build_jobs(
    client: &Client,
    peer_ref: PeerRef,
    attachments_dir: &Path,
    params: &TelegramDownloadParams,
    self_display_name: &str,
    snapshot: Arc<Mutex<DownloadSnapshot>>,
) -> Result<ScanResults, String> {
    let mut scan = ScanResults::default();
    let mut messages = client.iter_messages(peer_ref);
    let mut scanned = 0usize;

    while let Some(message) = messages
        .next()
        .await
        .map_err(|error| format!("Failed to read Telegram messages: {error}"))?
    {
        scanned += 1;
        if scanned % 250 == 0 {
            set_status(&snapshot, &format!("Scanning media... {scanned} messages"));
        }
        let media = message.media();
        let mut predicted_attachment_path = None;

        if let Some(media) = media.as_ref() {
            if let Some(location) = media.to_raw_input_location() {
                let filename = media_filename(media, message.id());
                let final_path = attachments_dir.join(&filename);
                let state_path = final_path.with_file_name(format!("{filename}.part.state.json"));
                let temp_path = final_path.with_file_name(format!("{filename}.part"));
                let file_id = format!("{}:{filename}", message.id());

                scan.jobs.push(json!({
                    "file_id": file_id,
                    "message_id": message.id().to_string(),
                    "filename": filename,
                    "category": "attachments",
                    "final_path": final_path.to_string_lossy().to_string(),
                    "temp_path": temp_path.to_string_lossy().to_string(),
                    "state_path": state_path.to_string_lossy().to_string(),
                    "expected_size": media_expected_size(media),
                    "overwrite": params.overwrite_existing,
                    "skip_if_complete": !params.overwrite_existing,
                    "resume_if_partial": !params.overwrite_existing,
                    "dc_id": media_dc_id(media),
                    "location": location_to_json(&location)?,
                    "media_type": media_type_name(media),
                    "input_chat": Value::Null,
                }));
                scan.contexts.push(JobContext {
                    file_id,
                    message_id: message.id(),
                });
                predicted_attachment_path = Some(filename);
            }
        }

        scan.export_messages.push(export_message_json(
            &message,
            media.as_ref(),
            predicted_attachment_path.as_deref(),
            self_display_name,
        ));
    }

    Ok(scan)
}

async fn build_auth_bundle(
    client: &Client,
    api_id: i32,
    api_hash: &str,
    jobs: &[Value],
    self_name: String,
) -> Result<Value, String> {
    let session = SqliteSession::open(telegram::session_path())
        .await
        .map_err(|error| format!("Failed to open Telegram session: {error}"))?;
    let current_dc_id = session.home_dc_id();
    let home_auth_key = session
        .dc_option(current_dc_id)
        .and_then(|dc| dc.auth_key)
        .ok_or_else(|| "Telegram session is missing the home auth key.".to_string())?;

    let mut dc_ids = jobs
        .iter()
        .filter_map(|job| job.get("dc_id").and_then(Value::as_i64))
        .map(|dc_id| dc_id as i32)
        .collect::<Vec<_>>();
    dc_ids.sort_unstable();
    dc_ids.dedup();

    let exported_auth = build_exported_auth(client, current_dc_id, &dc_ids).await?;
    let dc_options = session_dc_options(&session);
    let me = client
        .get_me()
        .await
        .map_err(|error| format!("Failed to read Telegram user info: {error}"))?;

    Ok(json!({
        "api_id": api_id,
        "api_hash": api_hash,
        "current_dc_id": current_dc_id,
        "home_auth_key_b64": base64::engine::general_purpose::STANDARD.encode(home_auth_key),
        "self_id": me.id().bare_id(),
        "self_name": self_name,
        "dc_options": dc_options,
        "exported_auth": exported_auth,
    }))
}

async fn build_exported_auth(
    client: &Client,
    current_dc_id: i32,
    dc_ids: &[i32],
) -> Result<Value, String> {
    let mut map = serde_json::Map::new();
    for &dc_id in dc_ids {
        if dc_id == current_dc_id {
            continue;
        }
        let auth: tl::types::auth::ExportedAuthorization = client
            .invoke(&tl::functions::auth::ExportAuthorization { dc_id })
            .await
            .map_err(|error| format!("Failed to export auth for dc {dc_id}: {error}"))?
            .into();
        map.insert(
            dc_id.to_string(),
            json!({
                "id": auth.id,
                "bytes_b64": base64::engine::general_purpose::STANDARD.encode(auth.bytes),
            }),
        );
    }
    Ok(Value::Object(map))
}

async fn write_message_exports(
    messages: &[Value],
    json_path: &Path,
    txt_path: &Path,
) -> Result<(), String> {
    let json_payload = serde_json::to_vec_pretty(messages)
        .map_err(|error| format!("Failed to serialize message export: {error}"))?;
    tokio::fs::write(json_path, json_payload)
        .await
        .map_err(|error| format!("Failed to write {}: {error}", json_path.display()))?;

    let txt_payload = format_messages_as_txt(messages);
    tokio::fs::write(txt_path, txt_payload)
        .await
        .map_err(|error| format!("Failed to write {}: {error}", txt_path.display()))?;
    Ok(())
}

fn format_messages_as_txt(messages: &[Value]) -> String {
    let mut output = String::new();
    for message in messages {
        let date = message
            .get("date")
            .and_then(Value::as_str)
            .and_then(format_export_datetime)
            .unwrap_or_default();
        let sender = message
            .get("user_display_name")
            .and_then(Value::as_str)
            .unwrap_or("Unknown");
        let mut text = message
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        if text.trim().is_empty() {
            if let Some(service_text) = service_text_from_export_message(message) {
                text = service_text;
            } else if let Some(placeholder) = media_placeholder_from_export_message(message) {
                text = placeholder;
            }
        } else if let Some(placeholder) = media_placeholder_from_export_message(message) {
            text.push('\n');
            text.push_str(&placeholder);
        }

        if !date.is_empty() || !sender.is_empty() {
            output.push_str(&format!("{date} {sender}:\n{text}\n\n"));
        } else {
            output.push_str(&format!("{text}\n\n"));
        }
    }
    output
}

fn format_export_datetime(raw: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|date| date.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string())
}

fn service_text_from_export_message(message: &Value) -> Option<String> {
    let action = message.get("action")?.as_object()?;
    let action_name = action.get("_")?.as_str()?;
    let sender = message
        .get("user_display_name")
        .and_then(Value::as_str)
        .unwrap_or("Someone");
    let label = match action_name {
        "MessageActionChatAddUser" => "joined the group",
        "MessageActionChatDeleteUser" => "left the group",
        "MessageActionChatJoinedByLink" => "joined via invite link",
        "MessageActionChatEditTitle" => {
            if let Some(title) = action.get("title").and_then(Value::as_str) {
                return Some(format!("{sender} changed the group name to \"{title}\""));
            }
            "changed the group name"
        }
        "MessageActionChatEditPhoto" => "updated the group photo",
        "MessageActionChatCreate" => "created the group",
        "MessageActionPinMessage" => "pinned a message",
        "MessageActionChatMigrateTo" => "group was upgraded to a supergroup",
        "MessageActionChannelCreate" => "created the channel",
        "MessageActionPhoneCall" => "Phone call",
        "MessageActionGroupCall" => "Group call",
        "MessageActionInviteToGroupCall" => "was invited to a voice chat",
        "MessageActionContactSignUp" => "joined Telegram",
        "MessageActionHistoryClear" => "cleared the history",
        "MessageActionSetMessagesTTL" => "changed the auto-delete timer",
        "MessageActionScreenshotTaken" => "took a screenshot",
        _ => return None,
    };
    Some(format!("{sender} {label}"))
}

fn media_placeholder_from_export_message(message: &Value) -> Option<String> {
    let media = message.get("media")?.as_object()?;
    let media_type = media.get("_")?.as_str()?;
    match media_type {
        "MessageMediaPhoto" => Some("[photo]".to_string()),
        "MessageMediaDocument" => {
            let filename = message
                .get("attachment_path")
                .and_then(Value::as_str)
                .map(Path::new)
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .or_else(|| {
                    media.get("document")
                        .and_then(Value::as_object)
                        .and_then(|document| document.get("attributes"))
                        .and_then(Value::as_array)
                        .and_then(|attrs| {
                            attrs.iter().find_map(|attr| {
                                let attr = attr.as_object()?;
                                if attr.get("_").and_then(Value::as_str)
                                    == Some("DocumentAttributeFilename")
                                {
                                    attr.get("file_name").and_then(Value::as_str)
                                } else {
                                    None
                                }
                            })
                        })
                })
                .unwrap_or("attachment");
            Some(format!("[file={filename}]"))
        }
        "MessageMediaContact" => Some("[contact]".to_string()),
        "MessageMediaGeo" | "MessageMediaGeoLive" | "MessageMediaVenue" => {
            Some("[location]".to_string())
        }
        "MessageMediaPoll" => Some("[poll]".to_string()),
        _ => Some("[media]".to_string()),
    }
}

fn finalize_attachment_paths(messages: &mut [Value], attachments_dir: &Path) {
    for message in messages {
        let Some(attachment_path) = message
            .get("attachment_path")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if !attachments_dir.join(&attachment_path).exists() {
            if let Some(object) = message.as_object_mut() {
                object.insert("attachment_path".to_string(), Value::Null);
            }
        }
    }
}

fn session_dc_options(session: &SqliteSession) -> Vec<Value> {
    let mut results = Vec::new();
    for dc_id in 1..=10 {
        if let Some(dc) = session.dc_option(dc_id) {
            results.push(json!({
                "id": dc.id,
                "ip_address": dc.ipv4.ip().to_string(),
                "port": 443,
                "ipv6": false,
                "media_only": false,
                "cdn": false,
                "tcpo_only": false,
            }));
            let ipv6 = dc.ipv6.ip().to_string();
            if !ipv6.is_empty() && ipv6 != "::" {
                results.push(json!({
                    "id": dc.id,
                    "ip_address": ipv6,
                    "port": 443,
                    "ipv6": true,
                    "media_only": false,
                    "cdn": false,
                    "tcpo_only": false,
                }));
            }
        }
    }
    results
}

fn export_message_json(
    message: &grammers_client::message::Message,
    media: Option<&Media>,
    attachment_path: Option<&str>,
    self_display_name: &str,
) -> Value {
    json!({
        "id": message.id(),
        "date": message.date().to_rfc3339_opts(SecondsFormat::Secs, true),
        "out": message.outgoing(),
        "from_id": export_from_id_json(message),
        "user_display_name": export_sender_name(message, self_display_name),
        "message": message.text(),
        "edit_date": message.edit_date().map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true)),
        "action": export_action_json(message),
        "media": media.map(export_media_json),
        "attachment_path": attachment_path,
        "fwd_from": export_forward_json(message),
        "reply_to": export_reply_json(message),
    })
}

fn export_from_id_json(message: &grammers_client::message::Message) -> Value {
    let Some(sender_id) = message.sender_id() else {
        return Value::Null;
    };
    match sender_id.kind() {
        PeerKind::User | PeerKind::UserSelf => json!({"user_id": sender_id.bare_id()}),
        PeerKind::Chat => json!({"chat_id": sender_id.bare_id()}),
        PeerKind::Channel => json!({"channel_id": sender_id.bare_id()}),
    }
}

fn export_sender_name(message: &grammers_client::message::Message, self_display_name: &str) -> String {
    if let Some(sender) = message.sender() {
        return peer_display_name(sender);
    }
    if message.outgoing() && !self_display_name.is_empty() {
        return self_display_name.to_string();
    }
    "Unknown".to_string()
}

fn export_action_json(message: &grammers_client::message::Message) -> Option<Value> {
    match message.action()? {
        tl::enums::MessageAction::ChatAddUser(_) => Some(json!({"_": "MessageActionChatAddUser"})),
        tl::enums::MessageAction::ChatDeleteUser(_) => Some(json!({"_": "MessageActionChatDeleteUser"})),
        tl::enums::MessageAction::ChatJoinedByLink(_) => Some(json!({"_": "MessageActionChatJoinedByLink"})),
        tl::enums::MessageAction::ChatEditPhoto(_) => Some(json!({"_": "MessageActionChatEditPhoto"})),
        tl::enums::MessageAction::ChatCreate(_) => Some(json!({"_": "MessageActionChatCreate"})),
        tl::enums::MessageAction::PinMessage => Some(json!({"_": "MessageActionPinMessage"})),
        tl::enums::MessageAction::ChatMigrateTo(_) => Some(json!({"_": "MessageActionChatMigrateTo"})),
        tl::enums::MessageAction::ChannelCreate(_) => Some(json!({"_": "MessageActionChannelCreate"})),
        tl::enums::MessageAction::PhoneCall(_) => Some(json!({"_": "MessageActionPhoneCall"})),
        tl::enums::MessageAction::GroupCall(_) => Some(json!({"_": "MessageActionGroupCall"})),
        tl::enums::MessageAction::InviteToGroupCall(_) => {
            Some(json!({"_": "MessageActionInviteToGroupCall"}))
        }
        tl::enums::MessageAction::ContactSignUp => Some(json!({"_": "MessageActionContactSignUp"})),
        tl::enums::MessageAction::HistoryClear => Some(json!({"_": "MessageActionHistoryClear"})),
        tl::enums::MessageAction::SetMessagesTtl(_) => Some(json!({"_": "MessageActionSetMessagesTTL"})),
        tl::enums::MessageAction::ScreenshotTaken => Some(json!({"_": "MessageActionScreenshotTaken"})),
        tl::enums::MessageAction::ChatEditTitle(action) => Some(json!({
            "_": "MessageActionChatEditTitle",
            "title": action.title,
        })),
        _ => None,
    }
}

fn export_forward_json(message: &grammers_client::message::Message) -> Option<Value> {
    let header = message.forward_header()?;
    match header {
        tl::enums::MessageFwdHeader::Header(header) => Some(json!({
            "from_name": header.from_name,
        })),
    }
}

fn export_reply_json(message: &grammers_client::message::Message) -> Option<Value> {
    let header = message.reply_header()?;
    match header {
        tl::enums::MessageReplyHeader::Header(header) => Some(json!({
            "quote_text": header.quote_text,
        })),
        _ => None,
    }
}

fn export_media_json(media: &Media) -> Value {
    match media {
        Media::Photo(_) => json!({"_": "MessageMediaPhoto"}),
        Media::Document(document) => json!({
            "_": "MessageMediaDocument",
            "document": {
                "size": document.size(),
                "attributes": export_document_attributes(document),
            }
        }),
        Media::Sticker(sticker) => json!({
            "_": "MessageMediaDocument",
            "document": {
                "size": sticker.document.size(),
                "attributes": export_sticker_attributes(sticker),
            }
        }),
        Media::Contact(_) => json!({"_": "MessageMediaContact"}),
        Media::Poll(_) => json!({"_": "MessageMediaPoll"}),
        Media::Geo(_) => json!({"_": "MessageMediaGeo"}),
        Media::GeoLive(_) => json!({"_": "MessageMediaGeoLive"}),
        Media::Venue(_) => json!({"_": "MessageMediaVenue"}),
        _ => json!({"_": "MessageMediaUnsupported"}),
    }
}

fn export_document_attributes(document: &Document) -> Vec<Value> {
    let mut attributes = Vec::new();
    if let Some(filename) = document.name() {
        attributes.push(json!({
            "_": "DocumentAttributeFilename",
            "file_name": filename,
        }));
    }
    if document
        .mime_type()
        .map(|mime| mime.starts_with("video/"))
        .unwrap_or(false)
    {
        attributes.push(json!({"_": "DocumentAttributeVideo"}));
    }
    if document
        .mime_type()
        .map(|mime| mime.starts_with("audio/"))
        .unwrap_or(false)
    {
        attributes.push(json!({"_": "DocumentAttributeAudio"}));
    }
    attributes
}

fn export_sticker_attributes(sticker: &Sticker) -> Vec<Value> {
    let mut attributes = export_document_attributes(&sticker.document);
    if !attributes
        .iter()
        .any(|attr| attr.get("_").and_then(Value::as_str) == Some("DocumentAttributeSticker"))
    {
        attributes.push(json!({"_": "DocumentAttributeSticker"}));
    }
    attributes
}

async fn resolve_peer(client: &Client, raw_input: &str) -> Result<(Peer, PeerRef), String> {
    let normalized = raw_input
        .trim()
        .trim_start_matches('@')
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("t.me/")
        .trim_start_matches("telegram.me/")
        .trim_matches('/');
    let peer = client
        .resolve_username(normalized)
        .await
        .map_err(|error| format!("Failed to resolve Telegram chat: {error}"))?
        .ok_or_else(|| "Telegram chat could not be resolved.".to_string())?;
    let peer_ref = peer
        .to_ref()
        .await
        .ok_or_else(|| "Telegram chat reference could not be created.".to_string())?;
    Ok((peer, peer_ref))
}

fn peer_display_name(peer: &Peer) -> String {
    match peer {
        Peer::User(user) => {
            let full_name = user.full_name();
            if full_name.is_empty() {
                user.username().unwrap_or("Unknown").to_string()
            } else {
                full_name
            }
        }
        Peer::Group(group) => group.title().unwrap_or("Chat").to_string(),
        Peer::Channel(channel) => channel.title().to_string(),
    }
}

fn media_filename(media: &Media, message_id: i32) -> String {
    let base = match media {
        Media::Photo(_) => format!("photo_{message_id}.jpg"),
        Media::Document(document) => document
            .name()
            .map(sanitize_filename)
            .filter(|value: &String| !value.is_empty())
            .unwrap_or_else(|| fallback_document_name(document, message_id)),
        Media::Sticker(Sticker { document, .. }) => document
            .name()
            .map(sanitize_filename)
            .filter(|value: &String| !value.is_empty())
            .unwrap_or_else(|| format!("sticker_{message_id}.webp")),
        _ => format!("media_{message_id}.bin"),
    };
    format!("{}_{}", message_id, base)
}

fn fallback_document_name(document: &Document, message_id: i32) -> String {
    let extension = document
        .mime_type()
        .and_then(mime_extension)
        .unwrap_or("bin");
    format!("document_{message_id}.{extension}")
}

fn mime_extension(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "video/mp4" => Some("mp4"),
        "audio/mpeg" => Some("mp3"),
        "application/zip" => Some("zip"),
        _ => None,
    }
}

fn sanitize_filename(raw: &str) -> String {
    raw.chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => ch,
        })
        .collect()
}

fn media_type_name(media: &Media) -> &'static str {
    match media {
        Media::Photo(_) => "photo",
        Media::Document(_) => "document",
        Media::Sticker(_) => "sticker",
        Media::Contact(_) => "contact",
        Media::Poll(_) => "poll",
        Media::Geo(_) => "geo",
        Media::Dice(_) => "dice",
        Media::Venue(_) => "venue",
        Media::GeoLive(_) => "geo_live",
        Media::WebPage(_) => "webpage",
        _ => "media",
    }
}

fn media_expected_size(media: &Media) -> u64 {
    match media {
        Media::Photo(photo) => photo.size().map(|size| size as u64).unwrap_or(0),
        Media::Document(document) => document.size().map(|size| size as u64).unwrap_or(0),
        Media::Sticker(sticker) => sticker
            .document
            .size()
            .map(|size| size as u64)
            .unwrap_or(0),
        _ => media.size().map(|size| size as u64).unwrap_or(0),
    }
}

fn media_dc_id(media: &Media) -> i32 {
    match media {
        Media::Photo(photo) => match photo.raw.photo.as_ref() {
            Some(tl::enums::Photo::Photo(raw)) => raw.dc_id,
            _ => 0,
        },
        Media::Document(document) => match document.raw.document.as_ref() {
            Some(tl::enums::Document::Document(raw)) => raw.dc_id,
            _ => 0,
        },
        Media::Sticker(sticker) => match sticker.document.raw.document.as_ref() {
            Some(tl::enums::Document::Document(raw)) => raw.dc_id,
            _ => 0,
        },
        _ => 0,
    }
}

fn location_to_json(location: &tl::enums::InputFileLocation) -> Result<Value, String> {
    match location {
        tl::enums::InputFileLocation::InputDocumentFileLocation(value) => Ok(json!({
            "_": "InputDocumentFileLocation",
            "id": value.id,
            "access_hash": value.access_hash,
            "file_reference": {
                "__bytes_b64__": base64::engine::general_purpose::STANDARD.encode(&value.file_reference),
            },
            "thumb_size": value.thumb_size,
        })),
        tl::enums::InputFileLocation::InputPhotoFileLocation(value) => Ok(json!({
            "_": "InputPhotoFileLocation",
            "id": value.id,
            "access_hash": value.access_hash,
            "file_reference": {
                "__bytes_b64__": base64::engine::general_purpose::STANDARD.encode(&value.file_reference),
            },
            "thumb_size": value.thumb_size,
        })),
        other => Err(format!("Unsupported Telegram media location: {other:?}")),
    }
}

fn media_location_json(media: &Media) -> Result<Value, String> {
    let location = media
        .to_raw_input_location()
        .ok_or_else(|| "Telegram media location is unavailable.".to_string())?;
    location_to_json(&location)
}

fn locate_native_downloader_binary() -> Option<PathBuf> {
    if let Ok(override_path) = env::var("TDC_DOWNLOADER_BIN") {
        let path = PathBuf::from(override_path);
        if path.exists() {
            return Some(path);
        }
    }

    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)?;
    [
        root.join("native/tdc-downloader/target/debug").join(format!("tdc-downloader{suffix}")),
        root.join("native/tdc-downloader/target/release").join(format!("tdc-downloader{suffix}")),
    ]
    .into_iter()
    .find(|candidate| candidate.exists())
}

async fn send_command(stdin: &mut ChildStdin, value: &Value) -> Result<(), String> {
    let payload = serde_json::to_string(value).map_err(|error| format!("Failed to encode command: {error}"))?;
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|error| format!("Failed to write to downloader backend: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("Failed to write newline to downloader backend: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("Failed to flush downloader backend input: {error}"))
}

fn upsert_file(files: &mut Vec<RuntimeFile>, next: RuntimeFile) {
    if let Some(existing) = files.iter_mut().find(|file| file.file_id == next.file_id) {
        *existing = next;
    } else {
        files.push(next);
    }
}

fn publish_files(snapshot: &Arc<Mutex<DownloadSnapshot>>, files: &[RuntimeFile]) {
    let mapped = files
        .iter()
        .map(|file| ActiveDownload {
            name: file.filename.clone(),
            progress: if file.expected_size == 0 {
                0.0
            } else {
                (file.bytes_done as f32 / file.expected_size as f32).clamp(0.0, 1.0)
            },
            transferred_label: format_transfer_label(file.bytes_done, file.expected_size),
        })
        .collect::<Vec<_>>();
    snapshot.lock().unwrap().files = mapped;
}

fn set_status(snapshot: &Arc<Mutex<DownloadSnapshot>>, message: &str) {
    snapshot.lock().unwrap().status_message = message.to_string();
}

fn format_transfer_label(done: u64, total: u64) -> String {
    if total == 0 {
        return format!("{} / ?   0%", format_bytes(done));
    }
    let percent = ((done as f64 / total as f64) * 100.0).round() as u64;
    format!("{} / {}   {}%", format_bytes(done), format_bytes(total), percent)
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.0} MB", value / MB)
    } else if value >= KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn value_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn value_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn value_i32(value: &Value, key: &str) -> i32 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0) as i32
}

async fn backend_stderr_summary(
    stderr_tail: &Arc<tokio::sync::Mutex<VecDeque<String>>>,
) -> String {
    let tail = stderr_tail.lock().await;
    let mut lines = tail
        .iter()
        .rev()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else if trimmed.len() > 220 {
                Some(format!("{}...", &trimmed[..220]))
            } else {
                Some(trimmed.to_string())
            }
        })
        .take(3)
        .collect::<Vec<_>>();
    lines.reverse();
    lines.join(" | ")
}
