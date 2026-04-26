import asyncio
import logging
from pathlib import Path
from unittest.mock import AsyncMock

try:
    import pytest
except ModuleNotFoundError:  # pragma: no cover - local fallback for ad-hoc execution
    class _PytestFallback:
        class mark:
            @staticmethod
            def asyncio(func):
                return func

    pytest = _PytestFallback()

from telegram_download_chat.core import media as media_module
from telegram_download_chat.core.media import (
    MediaMixin,
    _CHUNK_SIZE,
    _LARGE_FILE_THRESHOLD,
)
from telegram_download_chat.core.render import _attachment_meta_from_message


class DummyDownloader(MediaMixin):
    def __init__(self):
        self.logger = logging.getLogger("test-media")
        self.config = {"settings": {}}
        self.client = None
        self._stop_requested = False
        self._stop_file = None
        self._pause_file = None
        self._manual_pause_logged = False


@pytest.mark.asyncio
async def test_large_media_download_uses_parallel_direct_stripes(tmp_path):
    downloader = DummyDownloader()
    payload = bytes(range(256)) * ((_CHUNK_SIZE * 4) // 256)
    downloader.client = object()
    downloader.config["settings"]["large_file_workers"] = 2

    calls = []

    class FakeDirectDownloadIter:
        def __init__(
            self,
            client,
            limit,
            *,
            file,
            dc_id,
            offset,
            stride,
            chunk_size,
            request_size,
            file_size,
            msg_data,
        ):
            calls.append(
                {
                    "offset": offset,
                    "stride": stride,
                    "limit": limit,
                    "chunk_size": chunk_size,
                    "request_size": request_size,
                    "file_size": file_size,
                }
            )
            self.offset = offset
            self.stride = stride
            self.limit = limit
            self.chunk_size = chunk_size
            self.payload = payload

        def __aiter__(self):
            async def iterator():
                pos = self.offset
                emitted = 0
                while pos < len(self.payload) and emitted < self.limit:
                    yield self.payload[pos : pos + self.chunk_size]
                    pos += self.stride
                    emitted += 1

            return iterator()

        async def close(self):
            return None

    class FakeFileInfo:
        def __init__(self, size):
            self.location = object()
            self.dc_id = 4
            self.size = size

    original_direct_iter = media_module._DirectDownloadIter
    original_get_file_info = media_module.utils._get_file_info
    media_module._DirectDownloadIter = FakeDirectDownloadIter
    media_module.utils._get_file_info = lambda _: FakeFileInfo(len(payload))

    try:
        dest = tmp_path / "file.bin"
        temp = downloader._get_partial_media_path(dest)
        resume_event = asyncio.Event()
        resume_event.set()

        result = await downloader._download_large_media(
            message=object(),
            file_size=len(payload),
            temp_path=temp,
            dest_path=dest,
            filename="file.bin",
            resume_event=resume_event,
        )
    finally:
        media_module._DirectDownloadIter = original_direct_iter
        media_module.utils._get_file_info = original_get_file_info

    assert result == dest
    assert dest.read_bytes() == payload
    assert not temp.exists()
    assert len(calls) == 2
    assert {call["offset"] for call in calls} == {0, _CHUNK_SIZE}
    assert all(call["stride"] == _CHUNK_SIZE * 2 for call in calls)


@pytest.mark.asyncio
async def test_small_media_download_uses_direct_512kb_chunks(tmp_path):
    downloader = DummyDownloader()
    payload = bytes(range(256)) * ((_CHUNK_SIZE * 2) // 256)
    downloader.client = object()

    calls = []

    class FakeDirectDownloadIter:
        def __init__(
            self,
            client,
            limit,
            *,
            file,
            dc_id,
            offset,
            stride,
            chunk_size,
            request_size,
            file_size,
            msg_data,
        ):
            calls.append(
                {
                    "limit": limit,
                    "offset": offset,
                    "stride": stride,
                    "chunk_size": chunk_size,
                    "request_size": request_size,
                    "file_size": file_size,
                }
            )
            self.payload = payload
            self.chunk_size = chunk_size

        def __aiter__(self):
            async def iterator():
                pos = 0
                while pos < len(self.payload):
                    yield self.payload[pos : pos + self.chunk_size]
                    pos += self.chunk_size

            return iterator()

        async def close(self):
            return None

    class FakeFileInfo:
        def __init__(self, size):
            self.location = object()
            self.dc_id = 4
            self.size = size

    original_direct_iter = media_module._DirectDownloadIter
    original_get_file_info = media_module.utils._get_file_info
    media_module._DirectDownloadIter = FakeDirectDownloadIter
    media_module.utils._get_file_info = lambda _: FakeFileInfo(len(payload))

    try:
        dest = tmp_path / "small.bin"
        temp = downloader._get_partial_media_path(dest)
        resume_event = asyncio.Event()
        resume_event.set()

        result = await downloader._download_small_media(
            message=object(),
            media=object(),
            file_size=len(payload),
            temp_path=temp,
            final_path=dest,
            filename="small.bin",
            resume_event=resume_event,
        )
    finally:
        media_module._DirectDownloadIter = original_direct_iter
        media_module.utils._get_file_info = original_get_file_info

    assert result == dest
    assert dest.read_bytes() == payload
    assert len(calls) == 1
    assert calls[0]["offset"] == 0
    assert calls[0]["stride"] == _CHUNK_SIZE
    assert calls[0]["chunk_size"] == _CHUNK_SIZE
    assert calls[0]["request_size"] == _CHUNK_SIZE


@pytest.mark.asyncio
async def test_manual_pause_blocks_queued_media_downloads(tmp_path):
    downloader = DummyDownloader()
    downloader.config["settings"]["download_concurrency"] = 1
    downloader.config["settings"]["media_error_threshold"] = 999

    pause_file = tmp_path / "manual.pause"
    downloader._pause_file = pause_file

    first_started = asyncio.Event()
    first_release = asyncio.Event()
    second_started = asyncio.Event()

    async def fake_download_message_media(msg, attachments_dir, resume_event=None):
        if msg["id"] == 1:
            first_started.set()
            await first_release.wait()
        else:
            second_started.set()
        return attachments_dir / "documents" / f"{msg['id']}.bin"

    downloader.download_message_media = AsyncMock(side_effect=fake_download_message_media)

    task = asyncio.create_task(
        downloader.download_all_media(
            messages=[{"id": 1}, {"id": 2}],
            attachments_dir=tmp_path,
        )
    )

    await asyncio.wait_for(first_started.wait(), timeout=1)
    pause_file.touch()
    first_release.set()

    await asyncio.sleep(0.3)
    assert not second_started.is_set()

    pause_file.unlink()
    results = await asyncio.wait_for(task, timeout=1)

    assert second_started.is_set()
    assert results == {
        "1": "documents/1.bin",
        "2": "documents/2.bin",
    }


@pytest.mark.asyncio
async def test_large_file_concurrency_is_capped(tmp_path):
    downloader = DummyDownloader()
    downloader.config["settings"].update(
        {
            "download_concurrency": 5,
            "large_file_concurrency": 2,
            "media_error_threshold": 999,
        }
    )

    active_large = 0
    max_active_large = 0
    release = asyncio.Event()

    async def fake_download_message_media(msg, attachments_dir, resume_event=None):
        nonlocal active_large, max_active_large
        active_large += 1
        max_active_large = max(max_active_large, active_large)
        await release.wait()
        active_large -= 1
        return attachments_dir / "documents" / f"{msg['id']}.bin"

    downloader.download_message_media = AsyncMock(side_effect=fake_download_message_media)
    downloader._get_media_file_size = lambda media: _CHUNK_SIZE * 400

    task = asyncio.create_task(
        downloader.download_all_media(
            messages=[
                {"id": 1, "media": object()},
                {"id": 2, "media": object()},
                {"id": 3, "media": object()},
            ],
            attachments_dir=tmp_path,
        )
    )

    await asyncio.sleep(0.2)
    assert max_active_large == 2

    release.set()
    await asyncio.wait_for(task, timeout=1)


@pytest.mark.asyncio
async def test_existing_wrong_size_file_is_re_downloaded(tmp_path):
    downloader = DummyDownloader()
    downloader.config["settings"]["max_retries"] = 0

    media = object()
    final_path = tmp_path / "documents" / "1_file.bin"
    final_path.parent.mkdir(parents=True, exist_ok=True)
    final_path.write_bytes(b"bad")

    downloader.get_filename = lambda _: "file.bin"
    downloader._get_media_category = lambda _: "documents"
    downloader._get_media_file_size = lambda _: 10
    downloader._serialize_synthetic_media = lambda *_: False
    downloader._download_small_media = AsyncMock(return_value=final_path)

    result = await downloader.download_message_media(
        {"id": 1, "media": media},
        tmp_path,
    )

    assert result == final_path
    downloader._download_small_media.assert_awaited_once()


@pytest.mark.asyncio
async def test_partial_media_is_restarted_when_not_overwriting(tmp_path):
    downloader = DummyDownloader()
    downloader.config["settings"]["max_retries"] = 0

    media = object()
    final_path = tmp_path / "documents" / "1_file.bin"
    part_path = downloader._get_partial_media_path(final_path)
    part_path.parent.mkdir(parents=True, exist_ok=True)
    part_path.write_bytes(b"partial")

    downloader.get_filename = lambda _: "file.bin"
    downloader._get_media_category = lambda _: "documents"
    downloader._get_media_file_size = lambda _: 10
    downloader._serialize_synthetic_media = lambda *_: False
    downloader._download_small_media = AsyncMock(return_value=final_path)

    result = await downloader.download_message_media(
        {"id": 1, "media": media},
        tmp_path,
        overwrite_existing_files=False,
    )

    assert result == final_path
    assert not part_path.exists()
    downloader._download_small_media.assert_awaited_once()


@pytest.mark.asyncio
async def test_complete_media_is_re_downloaded_when_overwriting(tmp_path):
    downloader = DummyDownloader()
    downloader.config["settings"]["max_retries"] = 0

    media = object()
    final_path = tmp_path / "documents" / "1_file.bin"
    final_path.parent.mkdir(parents=True, exist_ok=True)
    final_path.write_bytes(b"0123456789")

    downloader.get_filename = lambda _: "file.bin"
    downloader._get_media_category = lambda _: "documents"
    downloader._get_media_file_size = lambda _: 10
    downloader._serialize_synthetic_media = lambda *_: False
    downloader._download_small_media = AsyncMock(return_value=final_path)

    result = await downloader.download_message_media(
        {"id": 1, "media": media},
        tmp_path,
        overwrite_existing_files=True,
    )

    assert result == final_path
    downloader._download_small_media.assert_awaited_once()


@pytest.mark.asyncio
async def test_files_over_threshold_use_striped_large_downloads(tmp_path):
    downloader = DummyDownloader()
    downloader.config["settings"]["max_retries"] = 0

    media = object()
    final_path = tmp_path / "archives" / "1_big.bin"

    downloader.get_filename = lambda _: "big.bin"
    downloader._get_media_category = lambda _: "archives"
    downloader._get_media_file_size = lambda _: _LARGE_FILE_THRESHOLD
    downloader._serialize_synthetic_media = lambda *_: False
    downloader._download_small_media = AsyncMock(return_value=final_path)
    downloader._download_large_media = AsyncMock(return_value=final_path)

    result = await downloader.download_message_media(
        {"id": 1, "media": media},
        tmp_path,
        overwrite_existing_files=False,
    )

    assert result == final_path
    downloader._download_large_media.assert_awaited_once()
    downloader._download_small_media.assert_not_awaited()


def test_attachment_meta_is_inferred_without_downloaded_file():
    msg = {
        "media": {
            "_": "MessageMediaDocument",
            "document": {
                "size": 110100480,
                "attributes": [
                    {"_": "DocumentAttributeFilename", "file_name": "1A420.tar.bz2"}
                ],
            },
        },
        "attachment_path": None,
    }

    meta = _attachment_meta_from_message(msg)

    assert meta["attachment_downloaded"] is False
    assert meta["media_category"] == "archives"
    assert meta["attachment_filename"] == "1A420.tar.bz2"
    assert meta["attachment_size_label"] == "105.0 MB"
