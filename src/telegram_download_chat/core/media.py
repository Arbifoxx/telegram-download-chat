"""Media download functionality for Telegram messages."""

import asyncio
import json
import os
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional

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

_PARALLEL_THRESHOLD = 5 * 1024 * 1024  # parallelize files >= 5 MB
_CHUNK_SIZE = 512 * 1024               # 512 KB per request (Telegram's standard block)

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

    # ------------------------------------------------------------------
    # Download methods
    # ------------------------------------------------------------------

    async def download_message_media(
        self,
        message: Any,
        attachments_dir: Path,
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

        # Skip if already downloaded from a previous run
        if download_to.exists():
            self.logger.debug(f"Skipping already-downloaded: {download_to}")
            return download_to

        download_to.parent.mkdir(parents=True, exist_ok=True)

        settings = getattr(self, "config", {}).get("settings", {})
        max_retries = settings.get("max_retries", 5)
        num_workers = settings.get("download_workers", 4)

        use_parallel = (
            isinstance(media, MessageMediaDocument)
            and isinstance(media.document, Document)
            and media.document.size >= _PARALLEL_THRESHOLD
        )

        for attempt in range(max_retries + 1):
            try:
                if self._serialize_synthetic_media(media, download_to):
                    return download_to

                if use_parallel:
                    result = await self._download_parallel(
                        message,
                        media.document.size,
                        download_to,
                        num_workers,
                    )
                else:
                    result = await self.client.download_media(
                        message, file=download_to
                    )
                    result = Path(result) if result else None

                if result:
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
                await asyncio.sleep(1)

            except FloodWaitError as e:
                wait = e.seconds + 1
                self.logger.info(
                    f"Flood-wait {wait}s for message {message_id} "
                    f"(attempt {attempt + 1}/{max_retries + 1}), sleeping..."
                )
                await asyncio.sleep(wait)

            except Exception as e:
                if attempt < max_retries:
                    backoff = 2 ** attempt
                    self.logger.warning(
                        f"Media download attempt {attempt + 1}/{max_retries + 1} "
                        f"failed for message {message_id}: {e}. "
                        f"Retrying in {backoff}s..."
                    )
                    await asyncio.sleep(backoff)
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
    ) -> Dict[str, str]:
        """Download media from all messages concurrently (up to 5 at a time).

        Returns dict mapping str(message_id) -> relative path (from attachments_dir)
        for each successfully downloaded file.
        """
        settings = getattr(self, "config", {}).get("settings", {})
        CONCURRENCY = settings.get("download_concurrency", 5)
        ERROR_THRESHOLD = settings.get("media_error_threshold", 5)

        semaphore = asyncio.Semaphore(CONCURRENCY)
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
            await resume_event.wait()
            if self._stop_requested:
                return
            async with semaphore:
                if self._stop_requested:
                    return
                path = await self.download_message_media(msg, attachments_dir)
                msg_id = str(
                    getattr(msg, "id", None)
                    or (msg.get("id") if isinstance(msg, dict) else None)
                    or ""
                )
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

    async def _download_parallel(
        self,
        message: Any,
        file_size: int,
        dest_path: Path,
        num_workers: int,
    ) -> Optional[Path]:
        """Download a file with a sliding window of concurrent chunk requests.

        Keeps exactly num_workers chunk requests in flight at all times. As
        soon as any request completes the next chunk fires immediately, so
        completions are always staggered in real time. This is robust against
        individual connection resets (e.g. exported DC auth expiry after ~5
        min): the other in-flight requests keep the pipe full while one slot
        reconnects, so no synchronised stall can form.
        """
        num_chunks = (file_size + _CHUNK_SIZE - 1) // _CHUNK_SIZE

        with open(dest_path, "wb") as f:
            f.seek(file_size - 1)
            f.write(b"\x00")

        fh = open(dest_path, "r+b")

        async def fetch_and_write(idx: int) -> None:
            offset = idx * _CHUNK_SIZE
            async for chunk in self.client.iter_download(
                message,
                offset=offset,
                request_size=_CHUNK_SIZE,
            ):
                fh.seek(offset)
                fh.write(chunk)
                return

        try:
            pending: set = set()
            next_idx = 0

            while next_idx < min(num_workers, num_chunks):
                pending.add(asyncio.create_task(fetch_and_write(next_idx)))
                next_idx += 1

            while pending:
                done, pending = await asyncio.wait(
                    pending, return_when=asyncio.FIRST_COMPLETED
                )
                for task in done:
                    exc = task.exception()
                    if exc:
                        raise exc
                    if next_idx < num_chunks:
                        pending.add(asyncio.create_task(fetch_and_write(next_idx)))
                        next_idx += 1
        except Exception:
            fh.close()
            dest_path.unlink(missing_ok=True)
            raise

        fh.close()
        return dest_path

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
