"""Media download functionality for Telegram messages."""

import asyncio
import json
import os
import tempfile
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

from telethon import utils
from telethon.client.downloads import _DirectDownloadIter
from telethon.errors import FileReferenceExpiredError, FloodWaitError
from telethon.tl.types import (
    Document,
    DocumentAttributeFilename,
    DocumentAttributeSticker,
    GeoPoint,
    MessageMediaContact,
    MessageMediaDice,
    MessageMediaDocument,
    MessageMediaGame,
    MessageMediaGeo,
    MessageMediaGeoLive,
    MessageMediaPhoto,
    MessageMediaPoll,
    MessageMediaVenue,
    MessageMediaWebPage,
    Photo,
    WebPage,
)

# ---------------------------------------------------------------------------
# Category constants — these become the subdirectory names under attachments/
# ---------------------------------------------------------------------------
_CAT_IMAGES = "images"
_CAT_VIDEOS = "videos"
_CAT_AUDIO = "audio"
_CAT_STICKERS = "stickers"
_CAT_DOCUMENTS = "documents"
_CAT_ARCHIVES = "archives"
_CAT_CONTACTS = "contacts"
_CAT_LOCATIONS = "locations"
_CAT_POLLS = "polls"
_CAT_OTHER = "other"

_LARGE_FILE_THRESHOLD = 100 * 1024 * 1024  # use striped downloads for files >= 100 MB
_CHUNK_SIZE = 512 * 1024               # 512 KB per request (Telegram's standard block)
_LARGE_FILE_WORKERS = 8

_ARCHIVE_MIMES = {
    "application/zip",
    "application/x-zip-compressed",
    "application/x-rar-compressed",
    "application/x-rar",
    "application/x-7z-compressed",
    "application/x-bzip2",
    "application/gzip",
    "application/x-tar",
}

_DOCUMENT_MIMES = {
    "application/pdf",
    "application/msword",
    "application/vnd.ms-excel",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/epub+zip",
    "application/x-mobipocket-ebook",
    "application/json",
    "application/xml",
}

_MIME_TO_EXT = {
    # Images
    "image/jpeg": ".jpg",
    "image/jpg": ".jpg",
    "image/png": ".png",
    "image/gif": ".gif",
    "image/webp": ".webp",
    "image/svg+xml": ".svg",
    "image/tiff": ".tiff",
    "image/bmp": ".bmp",
    "image/heic": ".heic",
    "image/heif": ".heif",
    "image/avif": ".avif",
    "image/jxl": ".jxl",
    "image/x-icon": ".ico",
    # Video
    "video/mp4": ".mp4",
    "video/webm": ".webm",
    "video/quicktime": ".mov",
    "video/x-matroska": ".mkv",
    "video/x-msvideo": ".avi",
    "video/x-flv": ".flv",
    "video/x-ms-wmv": ".wmv",
    "video/3gpp": ".3gp",
    "video/3gpp2": ".3g2",
    "video/ogg": ".ogv",
    "video/mpeg": ".mpg",
    # Audio
    "audio/mpeg": ".mp3",
    "audio/mp3": ".mp3",
    "audio/ogg": ".ogg",
    "audio/mp4": ".m4a",
    "audio/x-m4a": ".m4a",
    "audio/aac": ".aac",
    "audio/x-aac": ".aac",
    "audio/x-wav": ".wav",
    "audio/wav": ".wav",
    "audio/flac": ".flac",
    "audio/x-flac": ".flac",
    "audio/opus": ".opus",
    "audio/webm": ".weba",
    "audio/amr": ".amr",
    # Documents / text
    "application/pdf": ".pdf",
    "text/plain": ".txt",
    "text/csv": ".csv",
    "text/html": ".html",
    "text/css": ".css",
    "text/javascript": ".js",
    "application/javascript": ".js",
    "application/json": ".json",
    "application/xml": ".xml",
    "text/xml": ".xml",
    "text/markdown": ".md",
    "application/x-yaml": ".yaml",
    "text/yaml": ".yaml",
    "application/rtf": ".rtf",
    # Microsoft Office
    "application/msword": ".doc",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document": ".docx",
    "application/vnd.ms-excel": ".xls",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": ".xlsx",
    "application/vnd.ms-powerpoint": ".ppt",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation": ".pptx",
    # OpenDocument
    "application/vnd.oasis.opendocument.text": ".odt",
    "application/vnd.oasis.opendocument.spreadsheet": ".ods",
    "application/vnd.oasis.opendocument.presentation": ".odp",
    # E-books
    "application/epub+zip": ".epub",
    "application/x-mobipocket-ebook": ".mobi",
    # Archives
    "application/zip": ".zip",
    "application/x-zip-compressed": ".zip",
    "application/x-rar-compressed": ".rar",
    "application/x-rar": ".rar",
    "application/x-7z-compressed": ".7z",
    "application/x-bzip2": ".bz2",
    "application/gzip": ".gz",
    "application/x-tar": ".tar",
    # Executables / packages
    "application/vnd.android.package-archive": ".apk",
    "application/x-apple-diskimage": ".dmg",
    "application/x-ms-dos-executable": ".exe",
    "application/x-sh": ".sh",
    # Telegram-specific
    "application/x-tgsticker": ".tgs",
    # Database
    "application/x-sqlite3": ".db",
}


class MediaMixin:
    """Mixin class for downloading media from Telegram messages."""

    # ------------------------------------------------------------------
    # Public helpers
    # ------------------------------------------------------------------

    def get_filename(self, media: Any) -> Optional[str]:
        """Return a filename for the given media object, or None if not downloadable."""
        if isinstance(media, MessageMediaPhoto):
            photo = media.photo
            if isinstance(photo, Photo):
                return f"{photo.id}.jpg"

        elif isinstance(media, MessageMediaDocument):
            doc = media.document
            if isinstance(doc, Document):
                for attr in doc.attributes:
                    if isinstance(attr, DocumentAttributeFilename):
                        # Sanitize: strip path components to prevent traversal
                        return Path(attr.file_name).name or f"{doc.id}.bin"
                return f"{doc.id}{self._get_extension_from_mime(doc.mime_type)}"

        elif isinstance(media, MessageMediaWebPage):
            webpage = media.webpage
            if isinstance(webpage, WebPage):
                if webpage.document and isinstance(webpage.document, Document):
                    doc = webpage.document
                    for attr in doc.attributes:
                        if isinstance(attr, DocumentAttributeFilename):
                            return Path(attr.file_name).name or f"{doc.id}.bin"
                    return f"{doc.id}{self._get_extension_from_mime(doc.mime_type)}"
                elif webpage.photo and isinstance(webpage.photo, Photo):
                    return f"{webpage.photo.id}.jpg"
            return None

        elif isinstance(media, MessageMediaContact):
            identifier = media.user_id or media.phone_number or "unknown"
            return f"contact_{identifier}.vcf"

        elif isinstance(media, MessageMediaGeo):
            geo = media.geo
            if isinstance(geo, GeoPoint):
                return f"location_{geo.lat:.6f}_{geo.long:.6f}.json"
            return None

        elif isinstance(media, MessageMediaGeoLive):
            geo = media.geo
            if isinstance(geo, GeoPoint):
                return f"live_location_{geo.lat:.6f}_{geo.long:.6f}.json"
            return None

        elif isinstance(media, MessageMediaVenue):
            vid = getattr(media, "venue_id", None) or "unknown"
            return f"venue_{vid}.json"

        elif isinstance(media, MessageMediaPoll):
            poll_id = getattr(media.poll, "id", "unknown")
            return f"poll_{poll_id}.json"

        elif isinstance(media, MessageMediaDice):
            char = media.emoticon[0] if media.emoticon else "unknown"
            return (
                f"dice_{ord(char):x}_{media.value}.json"
                if char != "unknown"
                else f"dice_unknown_{media.value}.json"
            )

        elif isinstance(media, MessageMediaGame):
            game_id = getattr(media.game, "id", "unknown")
            return f"game_{game_id}.json"

        return None

    def get_predicted_attachment_path(
        self,
        media: Any,
        message_id: str,
        attachments_dir: Path,
    ) -> Optional[str]:
        """Return the relative path (from attachments_dir) where media will be saved.

        Structure: <category>/<message_id>_<filename>
        """
        filename = self.get_filename(media)
        if not filename:
            return None
        category = self._get_media_category(media)
        return f"{category}/{message_id}_{filename}"

    def _get_media_file_size(self, media: Any) -> int:
        """Return the media size in bytes when known, otherwise 0."""
        if isinstance(media, MessageMediaDocument) and isinstance(media.document, Document):
            return int(getattr(media.document, "size", 0) or 0)

        if isinstance(media, MessageMediaWebPage):
            webpage = media.webpage
            if (
                isinstance(webpage, WebPage)
                and webpage.document
                and isinstance(webpage.document, Document)
            ):
                return int(getattr(webpage.document, "size", 0) or 0)

        return 0

    # ------------------------------------------------------------------
    # Download methods
    # ------------------------------------------------------------------

    async def download_message_media(
        self,
        message: Any,
        attachments_dir: Path,
        resume_event: Optional[asyncio.Event] = None,
        overwrite_existing_files: bool = False,
    ) -> Optional[Path]:
        """Download media from a single message into a category subdirectory.

        Directory structure: attachments_dir/<category>/<message_id>_<filename>

        Returns the path to the saved file, or None on failure/skip.
        """
        media = getattr(message, "media", None) or (
            message.get("media") if isinstance(message, dict) else None
        )
        if not media:
            return None

        filename = self.get_filename(media)
        if not filename:
            return None

        message_id = str(
            getattr(message, "id", None)
            or (message.get("id") if isinstance(message, dict) else None)
            or ""
        )
        if not message_id:
            return None

        category = self._get_media_category(media)
        download_to = attachments_dir / category / f"{message_id}_{filename}"
        file_size = self._get_media_file_size(media)
        temp_path = self._get_partial_media_path(download_to)

        if overwrite_existing_files:
            download_to.unlink(missing_ok=True)
            temp_path.unlink(missing_ok=True)
        elif download_to.exists():
            if file_size > 0 and download_to.stat().st_size != file_size:
                self.logger.warning(
                    f"Existing media has wrong size, re-downloading: {download_to.name} "
                    f"(expected {file_size}, got {download_to.stat().st_size})"
                )
                download_to.unlink(missing_ok=True)
            else:
                self.logger.debug(f"Skipping already-downloaded: {download_to}")
                return download_to

        if temp_path.exists():
            self.logger.info(
                f"Discarding partial media and restarting from scratch: {temp_path.name}"
            )
            temp_path.unlink(missing_ok=True)

        download_to.parent.mkdir(parents=True, exist_ok=True)

        settings = getattr(self, "config", {}).get("settings", {})
        max_retries = settings.get("max_retries", 5)

        use_resumable_stream = file_size >= _LARGE_FILE_THRESHOLD

        unique_name = download_to.name  # "{message_id}_{filename}" — unique per message
        self.logger.info(f"MEDIA_DOWNLOADING:{unique_name}:{file_size}")

        for attempt in range(max_retries + 1):
            try:
                await self._wait_for_media_resume(resume_event)
                if self._stop_requested:
                    return None

                if self._serialize_synthetic_media(media, download_to):
                    return download_to

                if use_resumable_stream:
                    result = await self._download_large_media(
                        message,
                        file_size,
                        temp_path,
                        download_to,
                        unique_name,
                        resume_event=resume_event,
                    )
                else:
                    result = await self._download_small_media(
                        message,
                        file_size,
                        temp_path,
                        download_to,
                        unique_name,
                        resume_event=resume_event,
                    )

                if result:
                    self.logger.info(f"MEDIA_DOWNLOADED:{unique_name}")
                    self.logger.debug(
                        f"Downloaded media for message {message_id}: {result}"
                    )
                    return result
                else:
                    self.logger.warning(
                        f"Failed to download media for message {message_id}"
                    )
                    return None

            except FileReferenceExpiredError:
                peer_id = getattr(message, "peer_id", None)
                if peer_id is not None and attempt < max_retries:
                    try:
                        fresh = await self.client.get_messages(
                            peer_id, ids=[int(message_id)]
                        )
                        if fresh:
                            message = fresh[0]
                    except Exception:
                        pass
                await self._sleep_with_pause(1, resume_event)

            except FloodWaitError as e:
                wait = e.seconds + 1
                self.logger.info(
                    f"Flood-wait {wait}s for message {message_id} "
                    f"(attempt {attempt + 1}/{max_retries + 1}), sleeping..."
                )
                await self._sleep_with_pause(wait, resume_event)

            except Exception as e:
                if attempt < max_retries:
                    backoff = 2 ** attempt
                    self.logger.warning(
                        f"Media download attempt {attempt + 1}/{max_retries + 1} "
                        f"failed for message {message_id}: {e}. "
                        f"Retrying in {backoff}s..."
                    )
                    await self._sleep_with_pause(backoff, resume_event)
                else:
                    self.logger.warning(
                        f"Failed to download media for message {message_id} "
                        f"after {max_retries + 1} attempts: {e}"
                    )
                    return None

        self.logger.warning(
            f"Gave up downloading media for message {message_id} after flood waits"
        )
        return None

    def _get_partial_media_path(self, final_path: Path) -> Path:
        """Return the on-disk temp file path used for atomic/resumable downloads."""
        return final_path.with_name(f"{final_path.name}.part")

    def _emit_media_progress(
        self,
        filename: str,
        bytes_done: int,
        total_bytes: int,
        *,
        force: bool = False,
    ) -> None:
        """Throttle progress log emission to avoid stdout/UI backpressure."""
        if total_bytes <= 0:
            self.logger.info(
                f"MEDIA_FILE_PROGRESS:{filename}:{int(bytes_done)}:{int(total_bytes)}"
            )
            return

        state = getattr(self, "_media_progress_state", {})
        now = time.monotonic()
        pct = int((bytes_done / total_bytes) * 100)
        last = state.get(filename, {"time": 0.0, "bytes": -1, "pct": -1})

        should_emit = force or (
            bytes_done <= 0
            or bytes_done >= total_bytes
            or pct >= last["pct"] + 1
            or bytes_done >= last["bytes"] + (4 * 1024 * 1024)
            or (now - last["time"]) >= 0.5
        )

        if should_emit:
            self.logger.info(
                f"MEDIA_FILE_PROGRESS:{filename}:{int(bytes_done)}:{int(total_bytes)}"
            )
            state[filename] = {"time": now, "bytes": bytes_done, "pct": pct}
            self._media_progress_state = state

    def _finalize_media_download(
        self,
        temp_path: Path,
        final_path: Path,
        expected_size: int = 0,
    ) -> Optional[Path]:
        """Validate and atomically move a finished temp file into place."""
        if not temp_path.exists():
            return None

        if expected_size > 0:
            actual_size = temp_path.stat().st_size
            if actual_size != expected_size:
                raise IOError(
                    f"Downloaded size mismatch for {final_path.name}: "
                    f"expected {expected_size}, got {actual_size}"
                )

        temp_path.replace(final_path)
        if hasattr(self, "_media_progress_state"):
            self._media_progress_state.pop(final_path.name, None)
        return final_path

    async def _download_small_media(
        self,
        message: Any,
        file_size: int,
        temp_path: Path,
        final_path: Path,
        filename: str,
        resume_event: Optional[asyncio.Event] = None,
    ) -> Optional[Path]:
        """Download a smaller binary media file atomically via Telethon."""
        temp_path.parent.mkdir(parents=True, exist_ok=True)
        temp_path.unlink(missing_ok=True)

        def progress_callback(done: int, total: int) -> None:
            resolved_total = int(total or file_size or 0)
            self._emit_media_progress(filename, int(done), resolved_total)

        result = await self.client.download_media(
            message,
            file=temp_path,
            progress_callback=progress_callback,
        )
        if not result:
            return None

        if file_size > 0:
            self._emit_media_progress(filename, file_size, file_size, force=True)
        return self._finalize_media_download(temp_path, final_path, file_size)

    async def _wait_for_media_resume(
        self, resume_event: Optional[asyncio.Event] = None
    ) -> None:
        """Block while automatic or manual media pause is active."""
        if resume_event is not None:
            await resume_event.wait()

        while not self._stop_requested:
            pause_file = getattr(self, "_pause_file", None)
            if not pause_file or not pause_file.exists():
                if getattr(self, "_manual_pause_logged", False):
                    self._manual_pause_logged = False
                    self.logger.info("MEDIA_RESUMED")
                return

            if not getattr(self, "_manual_pause_logged", False):
                self._manual_pause_logged = True
                self.logger.info("MEDIA_MANUALLY_PAUSED")

            await asyncio.sleep(0.2)
            if resume_event is not None:
                await resume_event.wait()

    async def _sleep_with_pause(
        self,
        seconds: float,
        resume_event: Optional[asyncio.Event] = None,
    ) -> None:
        """Sleep in short slices so pause/stop requests take effect quickly."""
        remaining = max(0.0, float(seconds))
        while remaining > 0 and not self._stop_requested:
            await self._wait_for_media_resume(resume_event)
            chunk = min(0.25, remaining)
            await asyncio.sleep(chunk)
            remaining -= chunk

    async def _media_pause(self, resume_event: asyncio.Event) -> None:
        pause_file = Path(tempfile.gettempdir()) / f"tdc_media_pause_{os.getpid()}.tmp"
        pause_file.touch()
        self.logger.info(
            f"MEDIA_PAUSED:{pause_file}  "
            "— too many download failures. "
            "Resume from the GUI or delete this file to continue."
        )
        try:
            while pause_file.exists() and not self._stop_requested:
                await asyncio.sleep(0.5)
        finally:
            pause_file.unlink(missing_ok=True)
        if not self._stop_requested:
            self.logger.info("MEDIA_RESUMED")
            resume_event.set()

    async def download_all_media(
        self,
        messages: List[Any],
        attachments_dir: Path,
        overwrite_existing_files: bool = False,
    ) -> Dict[str, str]:
        """Download media from all messages concurrently (up to 5 at a time).

        Returns dict mapping str(message_id) -> relative path (from attachments_dir)
        for each successfully downloaded file.
        """
        settings = getattr(self, "config", {}).get("settings", {})
        CONCURRENCY = settings.get("download_concurrency", 5)
        ERROR_THRESHOLD = settings.get("media_error_threshold", 5)
        LARGE_FILE_CONCURRENCY = max(
            1,
            min(
                CONCURRENCY,
                int(settings.get("large_file_concurrency", 2) or 2),
            ),
        )

        semaphore = asyncio.Semaphore(CONCURRENCY)
        large_file_semaphore = asyncio.Semaphore(LARGE_FILE_CONCURRENCY)
        resume_event = asyncio.Event()
        resume_event.set()

        results: Dict[str, str] = {}
        total = len(messages)
        completed = 0
        consecutive_errors = 0
        log_interval = max(1, min(50, total // 10))

        async def download_one(msg: Any) -> None:
            nonlocal completed, consecutive_errors
            if self._stop_requested:
                return
            await self._wait_for_media_resume(resume_event)
            if self._stop_requested:
                return
            async with semaphore:
                await self._wait_for_media_resume(resume_event)
                if self._stop_requested:
                    return
                media = getattr(msg, "media", None) or (
                    msg.get("media") if isinstance(msg, dict) else None
                )
                file_size = self._get_media_file_size(media) if media else 0

                if file_size >= _LARGE_FILE_THRESHOLD:
                    async with large_file_semaphore:
                        path = await self.download_message_media(
                            msg,
                            attachments_dir,
                            resume_event=resume_event,
                            overwrite_existing_files=overwrite_existing_files,
                        )
                else:
                    path = await self.download_message_media(
                        msg,
                        attachments_dir,
                        resume_event=resume_event,
                        overwrite_existing_files=overwrite_existing_files,
                    )
                msg_id = str(
                    getattr(msg, "id", None)
                    or (msg.get("id") if isinstance(msg, dict) else None)
                    or ""
                )
                if self._stop_requested:
                    return
                if path and msg_id:
                    try:
                        results[msg_id] = str(
                            path.relative_to(attachments_dir)
                        ).replace("\\", "/")
                    except ValueError:
                        results[msg_id] = str(path)
                    consecutive_errors = 0
                else:
                    consecutive_errors += 1
                    if consecutive_errors >= ERROR_THRESHOLD:
                        consecutive_errors = 0
                        resume_event.clear()
                        await self._media_pause(resume_event)

                completed += 1
                if completed % log_interval == 0 or completed == total:
                    pct = int(completed / total * 100)
                    self.logger.info(
                        f"Media download progress: {completed}/{total} ({pct}%)"
                    )

        gather_results = await asyncio.gather(
            *[download_one(msg) for msg in messages], return_exceptions=True
        )
        for r in gather_results:
            if isinstance(r, Exception):
                self.logger.warning(f"Media download task failed: {r}")
        self.logger.info(f"Downloaded {len(results)} media files to {attachments_dir}")
        return results

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    async def _download_large_media(
        self,
        message: Any,
        file_size: int,
        temp_path: Path,
        dest_path: Path,
        filename: str = "",
        resume_event: Optional[asyncio.Event] = None,
    ) -> Optional[Path]:
        """Download a large file with parallel striped chunk requests."""
        temp_path.parent.mkdir(parents=True, exist_ok=True)
        temp_path.unlink(missing_ok=True)

        with open(temp_path, "wb") as f:
            if file_size > 0:
                f.seek(file_size - 1)
                f.write(b"\x00")

        bytes_done = 0
        file_info = utils._get_file_info(message)
        input_location = file_info.location
        dc_id = file_info.dc_id
        msg_data = None
        if hasattr(message, "input_chat") and getattr(message, "input_chat", None):
            msg_data = (message.input_chat, message.id)

        num_chunks = max(1, (file_size + _CHUNK_SIZE - 1) // _CHUNK_SIZE)
        num_workers = max(
            1,
            min(
                num_chunks,
                int(
                    getattr(self, "config", {})
                    .get("settings", {})
                    .get("large_file_workers", _LARGE_FILE_WORKERS)
                    or _LARGE_FILE_WORKERS
                ),
            ),
        )
        stride = num_workers * _CHUNK_SIZE

        fh = open(temp_path, "r+b")

        async def fetch_and_write(worker_index: int) -> int:
            nonlocal bytes_done
            offset = worker_index * _CHUNK_SIZE
            written_total = 0

            while offset < file_size:
                remaining_chunks = max(1, (file_size - offset + stride - 1) // stride)
                await self._wait_for_media_resume(resume_event)
                stream = _DirectDownloadIter(
                    self.client,
                    remaining_chunks,
                    file=input_location,
                    dc_id=dc_id,
                    offset=offset,
                    stride=stride,
                    chunk_size=_CHUNK_SIZE,
                    request_size=_CHUNK_SIZE,
                    file_size=file_size,
                    msg_data=msg_data,
                )
                try:
                    async for chunk in stream:
                        await self._wait_for_media_resume(resume_event)
                        if self._stop_requested:
                            return written_total

                        fh.seek(offset)
                        fh.write(chunk)
                        offset += stride
                        written_total += len(chunk)
                        bytes_done += len(chunk)
                        self._emit_media_progress(filename, bytes_done, file_size)
                finally:
                    await stream.close()
                break

            return written_total

        try:
            workers = [
                asyncio.create_task(fetch_and_write(worker_index))
                for worker_index in range(num_workers)
            ]
            await asyncio.gather(*workers)
        except Exception:
            for task in locals().get("workers", []):
                task.cancel()
            if "workers" in locals():
                await asyncio.gather(*workers, return_exceptions=True)
            fh.close()
            raise

        fh.close()
        if self._stop_requested or bytes_done < file_size:
            return None
        self._emit_media_progress(filename, file_size, file_size, force=True)
        return self._finalize_media_download(temp_path, dest_path, file_size)

    def _get_media_category(self, media: Any) -> str:
        """Return the category subdirectory name for a media object."""
        if isinstance(media, MessageMediaPhoto):
            return _CAT_IMAGES

        elif isinstance(media, MessageMediaDocument):
            doc = media.document
            if isinstance(doc, Document):
                for attr in doc.attributes:
                    if isinstance(attr, DocumentAttributeSticker):
                        return _CAT_STICKERS
                return self._category_from_mime(doc.mime_type)

        elif isinstance(media, MessageMediaWebPage):
            webpage = media.webpage
            if isinstance(webpage, WebPage):
                if webpage.document and isinstance(webpage.document, Document):
                    for attr in webpage.document.attributes:
                        if isinstance(attr, DocumentAttributeSticker):
                            return _CAT_STICKERS
                    return self._category_from_mime(webpage.document.mime_type)
                elif webpage.photo:
                    return _CAT_IMAGES
            return _CAT_OTHER

        elif isinstance(media, MessageMediaContact):
            return _CAT_CONTACTS

        elif isinstance(
            media, (MessageMediaGeo, MessageMediaGeoLive, MessageMediaVenue)
        ):
            return _CAT_LOCATIONS

        elif isinstance(media, MessageMediaPoll):
            return _CAT_POLLS

        return _CAT_OTHER

    def _category_from_mime(self, mime_type: Optional[str]) -> str:
        """Map a MIME type string to a category name."""
        if not mime_type:
            return _CAT_OTHER
        if mime_type == "application/x-tgsticker":
            return _CAT_STICKERS
        if mime_type.startswith("image/"):
            return _CAT_IMAGES
        if mime_type.startswith("video/"):
            return _CAT_VIDEOS
        if mime_type.startswith("audio/"):
            return _CAT_AUDIO
        if mime_type.startswith("text/"):
            return _CAT_DOCUMENTS
        if mime_type in _ARCHIVE_MIMES:
            return _CAT_ARCHIVES
        if mime_type in _DOCUMENT_MIMES:
            return _CAT_DOCUMENTS
        return _CAT_OTHER

    def _get_extension_from_mime(self, mime_type: Optional[str]) -> str:
        """Derive a file extension from a MIME type string."""
        if not mime_type:
            return ".bin"
        return _MIME_TO_EXT.get(mime_type, ".bin")

    def _serialize_synthetic_media(self, media: Any, target_path: Path) -> bool:
        """Write vCard/JSON for non-binary media types directly to disk.

        Returns True if handled here (no Telethon download call needed).
        """
        target_path.parent.mkdir(parents=True, exist_ok=True)

        if isinstance(media, MessageMediaContact):
            if media.vcard:
                content = media.vcard
            else:
                content = (
                    "BEGIN:VCARD\nVERSION:3.0\n"
                    f"FN:{media.first_name} {media.last_name}\n"
                    f"TEL:{media.phone_number}\n"
                    "END:VCARD\n"
                )
            target_path.write_text(content, encoding="utf-8")
            return True

        elif isinstance(media, (MessageMediaGeo, MessageMediaGeoLive)):
            geo = media.geo
            if not isinstance(geo, GeoPoint):
                return False
            data: dict = {"lat": geo.lat, "long": geo.long}
            if isinstance(media, MessageMediaGeoLive):
                data["heading"] = getattr(media, "heading", None)
                data["period"] = media.period
            target_path.write_text(json.dumps(data, ensure_ascii=False, indent=2))
            return True

        elif isinstance(media, MessageMediaVenue):
            geo = media.geo
            data = {
                "title": media.title,
                "address": media.address,
                "provider": media.provider,
                "venue_id": media.venue_id,
                "venue_type": media.venue_type,
                "lat": geo.lat if isinstance(geo, GeoPoint) else None,
                "long": geo.long if isinstance(geo, GeoPoint) else None,
            }
            target_path.write_text(json.dumps(data, ensure_ascii=False, indent=2))
            return True

        elif isinstance(media, MessageMediaPoll):
            poll = media.poll
            results = media.results
            answers = []
            for ans in poll.answers or []:
                text_val = ans.text
                if hasattr(text_val, "text"):
                    text_val = text_val.text
                answers.append({"text": text_val, "option": ans.option.hex()})
            result_map = {}
            if results and results.results:
                for r in results.results:
                    result_map[r.option.hex()] = r.voters
            for ans in answers:
                ans["voters"] = result_map.get(ans["option"], None)
            question = poll.question
            if hasattr(question, "text"):
                question = question.text
            data = {
                "question": question,
                "answers": answers,
                "total_voters": getattr(results, "total_voters", None),
                "closed": poll.closed,
                "quiz": poll.quiz,
            }
            target_path.write_text(json.dumps(data, ensure_ascii=False, indent=2))
            return True

        elif isinstance(media, MessageMediaDice):
            data = {"emoticon": media.emoticon, "value": media.value}
            target_path.write_text(json.dumps(data, ensure_ascii=False, indent=2))
            return True

        elif isinstance(media, MessageMediaGame):
            game = media.game
            data = {
                "id": game.id,
                "short_name": game.short_name,
                "title": game.title,
                "description": game.description,
            }
            target_path.write_text(json.dumps(data, ensure_ascii=False, indent=2))
            return True

        return False
