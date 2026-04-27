"""Rust media backend bridge and manifest/export helpers."""

from __future__ import annotations

import asyncio
import base64
import json
import os
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

from telethon import functions, utils


def _native_json(value: Any) -> Any:
    """Convert Telethon/native objects into JSON-safe values."""
    if isinstance(value, bytes):
        return {"__bytes_b64__": base64.b64encode(value).decode("ascii")}
    if isinstance(value, dict):
        return {str(k): _native_json(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_native_json(v) for v in value]
    if hasattr(value, "to_dict"):
        return _native_json(value.to_dict())
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    return str(value)


def locate_native_downloader_binary() -> Optional[Path]:
    """Locate the Rust media backend binary if it exists."""
    override = os.environ.get("TDC_DOWNLOADER_BIN")
    if override:
        path = Path(override)
        if path.exists():
            return path

    suffix = ".exe" if sys.platform == "win32" else ""
    here = Path(__file__).resolve()
    for parent in here.parents:
        candidates = [
            parent / "native" / "tdc-downloader" / "target" / "debug" / f"tdc-downloader{suffix}",
            parent / "native" / "tdc-downloader" / "target" / "release" / f"tdc-downloader{suffix}",
        ]
        for candidate in candidates:
            if candidate.exists():
                return candidate
    return None


class NativeMediaBackend:
    """Bridge between Python/Telethon and the Rust downloader subprocess."""

    def __init__(self, downloader: Any):
        self.downloader = downloader
        self.logger = downloader.logger
        self._manual_pause_sent = False
        self._message_lookup: Dict[str, Any] = {}
        self._jobs_by_id: Dict[str, Dict[str, Any]] = {}
        self._results: Dict[str, str] = {}

    async def download_all_media(
        self,
        messages: List[Any],
        attachments_dir: Path,
        *,
        overwrite_existing_files: bool = False,
    ) -> Optional[Dict[str, str]]:
        """Attempt native downloads, or return None to fall back to Python."""
        backend_mode = (
            os.environ.get("TDC_MEDIA_BACKEND")
            or self.downloader.config.get("settings", {}).get("media_backend", "auto")
        ).lower()
        if backend_mode == "python":
            return None

        binary = locate_native_downloader_binary()
        if not binary:
            self.logger.debug("Rust media backend binary not found; using Python backend")
            return None

        capabilities = await self._read_capabilities(binary)
        if not capabilities:
            self.logger.warning(
                "Rust media backend did not report capabilities; using Python backend"
            )
            return None
        if not capabilities.get("transport_ready", False):
            self.logger.info(
                "Rust media backend is present but transport is not ready yet; "
                "using Python backend for downloads"
            )
            return None

        jobs = self._build_jobs(
            messages,
            attachments_dir,
            overwrite_existing_files=overwrite_existing_files,
        )
        if not jobs:
            return {}

        dc_ids = sorted({int(job["dc_id"]) for job in jobs if job.get("dc_id")})
        auth_bundle = await self._build_auth_bundle(dc_ids)
        run_command = {
            "type": "start_run",
            "protocol_version": 1,
            "run_id": f"tdc-{os.getpid()}",
            "settings": {
                "download_concurrency": int(
                    self.downloader.config.get("settings", {}).get(
                        "download_concurrency", 5
                    )
                    or 5
                ),
                "large_file_concurrency": int(
                    self.downloader.config.get("settings", {}).get(
                        "large_file_concurrency", 2
                    )
                    or 2
                ),
            },
            "auth_bundle": auth_bundle,
            "jobs": jobs,
        }
        self._jobs_by_id = {job["file_id"]: job for job in jobs}

        self.logger.info(
            f"Starting Rust media backend ({binary.name}) for {len(jobs)} files"
        )
        return await self._run_backend(binary, run_command)

    async def _read_capabilities(self, binary: Path) -> Optional[Dict[str, Any]]:
        process = await asyncio.create_subprocess_exec(
            str(binary),
            "capabilities",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await process.communicate()
        if process.returncode != 0:
            self.logger.warning(
                "Rust media backend capabilities probe failed: %s",
                stderr.decode("utf-8", errors="replace").strip(),
            )
            return None
        try:
            return json.loads(stdout.decode("utf-8"))
        except json.JSONDecodeError:
            self.logger.warning("Rust media backend capabilities output was invalid JSON")
            return None

    def _build_jobs(
        self,
        messages: List[Any],
        attachments_dir: Path,
        *,
        overwrite_existing_files: bool,
    ) -> List[Dict[str, Any]]:
        jobs: List[Dict[str, Any]] = []

        for message in messages:
            media = getattr(message, "media", None) or (
                message.get("media") if isinstance(message, dict) else None
            )
            if not media:
                continue

            message_id = str(
                getattr(message, "id", None)
                or (message.get("id") if isinstance(message, dict) else None)
                or ""
            )
            if not message_id:
                continue

            filename = self.downloader.get_filename(media)
            if not filename:
                continue

            category = self.downloader._get_media_category(media)
            final_path = attachments_dir / category / f"{message_id}_{filename}"
            temp_path = self.downloader._get_partial_media_path(final_path)
            file_size = int(self.downloader._get_media_file_size(media) or 0)

            try:
                download_source = self.downloader._get_direct_download_source(
                    message, media
                )
                file_info = utils._get_file_info(download_source)
            except Exception as exc:
                self.logger.debug(
                    "Unable to prepare native job for message %s: %s",
                    message_id,
                    exc,
                )
                continue

            file_id = f"{message_id}:{filename}"
            self._message_lookup[file_id] = message
            jobs.append(
                {
                    "file_id": file_id,
                    "message_id": message_id,
                    "filename": final_path.name,
                    "category": category,
                    "final_path": str(final_path),
                    "temp_path": str(temp_path),
                    "state_path": str(final_path.with_name(f"{final_path.name}.part.state.json")),
                    "expected_size": file_size,
                    "overwrite": overwrite_existing_files,
                    "skip_if_complete": not overwrite_existing_files,
                    "resume_if_partial": not overwrite_existing_files,
                    "dc_id": int(getattr(file_info, "dc_id", 0) or 0),
                    "location": _native_json(getattr(file_info, "location", None)),
                    "media_type": type(media).__name__,
                    "input_chat": _native_json(
                        getattr(message, "input_chat", None)
                    ),
                }
            )
        return jobs

    async def _build_auth_bundle(self, dc_ids: List[int]) -> Dict[str, Any]:
        settings = self.downloader.config.get("settings", {})
        config = await self.downloader.client(functions.help.GetConfigRequest())
        dc_options = []
        for option in config.dc_options:
            dc_options.append(
                {
                    "id": option.id,
                    "ip_address": option.ip_address,
                    "port": option.port,
                    "ipv6": bool(getattr(option, "ipv6", False)),
                    "media_only": bool(getattr(option, "media_only", False)),
                    "cdn": bool(getattr(option, "cdn", False)),
                    "tcpo_only": bool(getattr(option, "tcpo_only", False)),
                }
            )

        current_dc = int(getattr(self.downloader.client.session, "dc_id", 0) or 0)
        exported_auth = {}
        for dc_id in dc_ids:
            if dc_id == current_dc:
                continue
            auth = await self.downloader.client(
                functions.auth.ExportAuthorizationRequest(dc_id)
            )
            exported_auth[str(dc_id)] = {
                "id": auth.id,
                "bytes_b64": base64.b64encode(auth.bytes).decode("ascii"),
            }

        return {
            "api_id": int(settings.get("api_id")),
            "api_hash": settings.get("api_hash"),
            "current_dc_id": current_dc,
            "self_id": self.downloader._self_id,
            "self_name": self.downloader._self_name,
            "dc_options": dc_options,
            "exported_auth": exported_auth,
        }

    async def _run_backend(
        self,
        binary: Path,
        start_command: Dict[str, Any],
    ) -> Optional[Dict[str, str]]:
        process = await asyncio.create_subprocess_exec(
            str(binary),
            "run",
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )

        await self._send_command(process, start_command)
        fatal_error: Optional[str] = None

        assert process.stdout is not None
        while True:
            if self.downloader._stop_requested:
                await self._send_command(process, {"type": "stop"})

            await self._sync_pause_state(process)

            raw = await process.stdout.readline()
            if not raw:
                break

            line = raw.decode("utf-8", errors="replace").strip()
            if not line:
                continue

            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                self.logger.info(line)
                continue

            fatal_error = await self._handle_event(process, event) or fatal_error

        returncode = await process.wait()
        if returncode == 0 and fatal_error is None:
            return dict(self._results)

        if fatal_error:
            self.logger.warning(
                "Rust media backend failed (%s); falling back to Python backend",
                fatal_error,
            )
        else:
            self.logger.warning(
                "Rust media backend exited with code %s; falling back to Python backend",
                returncode,
            )
        return None

    async def _sync_pause_state(self, process: asyncio.subprocess.Process) -> None:
        pause_file = getattr(self.downloader, "_pause_file", None)
        should_pause = bool(pause_file and pause_file.exists())
        if should_pause and not self._manual_pause_sent:
            await self._send_command(process, {"type": "pause"})
            self._manual_pause_sent = True
        elif not should_pause and self._manual_pause_sent:
            await self._send_command(process, {"type": "resume"})
            self._manual_pause_sent = False

    async def _send_command(
        self,
        process: asyncio.subprocess.Process,
        command: Dict[str, Any],
    ) -> None:
        if process.stdin is None or process.stdin.is_closing():
            return
        payload = json.dumps(command, ensure_ascii=False) + "\n"
        process.stdin.write(payload.encode("utf-8"))
        await process.stdin.drain()

    async def _handle_event(
        self,
        process: asyncio.subprocess.Process,
        event: Dict[str, Any],
    ) -> Optional[str]:
        event_type = event.get("type", "")
        if event_type == "run_started":
            self.logger.info(
                f"RUST_MEDIA_RUN_STARTED:{event.get('run_id', '')}:{event.get('file_count', 0)}"
            )
        elif event_type == "file_started":
            filename = event.get("filename") or event.get("file_id", "")
            total = int(event.get("expected_size") or 0)
            self.logger.info(f"MEDIA_DOWNLOADING:{filename}:{total}")
            self.logger.info(f"MEDIA_FILE_PROGRESS:{filename}:0:{total}")
        elif event_type == "file_progress":
            filename = event.get("filename") or event.get("file_id", "")
            done = int(event.get("bytes_done") or 0)
            total = int(event.get("expected_size") or 0)
            self.logger.info(f"MEDIA_FILE_PROGRESS:{filename}:{done}:{total}")
        elif event_type == "file_completed":
            filename = event.get("filename") or event.get("file_id", "")
            final_rel = event.get("attachment_path")
            message_id = str(event.get("message_id") or "")
            if message_id and final_rel:
                self._results[message_id] = final_rel
            self.logger.info(f"MEDIA_DOWNLOADED:{filename}")
        elif event_type == "file_skipped":
            filename = event.get("filename") or event.get("file_id", "")
            final_rel = event.get("attachment_path")
            message_id = str(event.get("message_id") or "")
            if message_id and final_rel:
                self._results[message_id] = final_rel
            self.logger.info(f"MEDIA_DOWNLOADED:{filename}")
        elif event_type == "file_restarted":
            self.logger.info(
                f"MEDIA_RESTARTED:{event.get('filename') or event.get('file_id', '')}"
            )
        elif event_type == "transport_window":
            self.logger.info(
                "MEDIA_TRANSPORT_WINDOW:"
                f"{event.get('filename') or event.get('file_id', '')}:"
                f"inflight={event.get('inflight', 0)}:"
                f"mbps={event.get('mbps', 0)}:"
                f"parts={event.get('parts', 0)}:"
                f"progress={event.get('progress', 0)}:"
                f"total={event.get('total', 0)}"
            )
        elif event_type == "transport_stall":
            self.logger.info(
                "MEDIA_TRANSPORT_STALL:"
                f"{event.get('filename') or event.get('file_id', '')}:"
                f"inflight={event.get('inflight', 0)}:"
                f"progress={event.get('progress', 0)}:"
                f"stalled_ms={event.get('stalled_ms', 0)}:"
                f"total={event.get('total', 0)}"
            )
        elif event_type == "request_file_reference_refresh":
            await self._handle_file_reference_refresh(process, event)
        elif event_type == "request_dc_auth_refresh":
            await self._handle_dc_auth_refresh(process, event)
        elif event_type == "file_error":
            self.logger.warning(
                "Rust backend file error for %s: %s",
                event.get("filename") or event.get("file_id", ""),
                event.get("message", "unknown error"),
            )
        elif event_type == "run_summary":
            self.logger.info(
                "RUST_MEDIA_RUN_SUMMARY:"
                f"completed={event.get('completed', 0)}:"
                f"skipped={event.get('skipped', 0)}:"
                f"failed={event.get('failed', 0)}"
            )
        elif event_type == "fatal_error":
            return str(event.get("message") or "fatal error")
        return None

    async def _handle_file_reference_refresh(
        self,
        process: asyncio.subprocess.Process,
        event: Dict[str, Any],
    ) -> None:
        file_id = str(event.get("file_id") or "")
        message = self._message_lookup.get(file_id)
        if message is None:
            await self._send_command(
                process,
                {
                    "type": "refresh_file_reference",
                    "file_id": file_id,
                    "ok": False,
                    "error": "unknown file id",
                },
            )
            return

        refreshed = await self.downloader.client.get_messages(
            getattr(message, "input_chat", None),
            ids=[getattr(message, "id", None)],
        )
        refreshed_message = refreshed[0] if refreshed else None
        if refreshed_message is None:
            await self._send_command(
                process,
                {
                    "type": "refresh_file_reference",
                    "file_id": file_id,
                    "ok": False,
                    "error": "message not found",
                },
            )
            return

        media = getattr(refreshed_message, "media", None)
        source = self.downloader._get_direct_download_source(refreshed_message, media)
        info = utils._get_file_info(source)
        await self._send_command(
            process,
            {
                "type": "refresh_file_reference",
                "file_id": file_id,
                "ok": True,
                "location": _native_json(getattr(info, "location", None)),
                "dc_id": int(getattr(info, "dc_id", 0) or 0),
            },
        )

    async def _handle_dc_auth_refresh(
        self,
        process: asyncio.subprocess.Process,
        event: Dict[str, Any],
    ) -> None:
        dc_id = int(event.get("dc_id") or 0)
        auth = await self.downloader.client(
            functions.auth.ExportAuthorizationRequest(dc_id)
        )
        await self._send_command(
            process,
            {
                "type": "refresh_dc_auth",
                "dc_id": dc_id,
                "ok": True,
                "auth": {
                    "id": auth.id,
                    "bytes_b64": base64.b64encode(auth.bytes).decode("ascii"),
                },
            },
        )
