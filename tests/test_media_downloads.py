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

from telegram_download_chat.core.media import MediaMixin, _CHUNK_SIZE


class DummyDownloader(MediaMixin):
    def __init__(self):
        self.logger = logging.getLogger("test-media")
        self.config = {"settings": {}}
        self.client = None
        self._stop_requested = False
        self._stop_file = None
        self._pause_file = None
        self._manual_pause_logged = False


class FakeClient:
    def __init__(self, payload: bytes):
        self.payload = payload
        self.calls = []

    async def iter_download(
        self,
        message,
        *,
        offset=0,
        stride=None,
        limit=None,
        chunk_size=None,
        request_size=None,
        file_size=None,
        dc_id=None,
    ):
        self.calls.append(
            {
                "offset": offset,
                "stride": stride,
                "limit": limit,
                "chunk_size": chunk_size,
                "request_size": request_size,
                "file_size": file_size,
            }
        )
        step = stride or request_size or _CHUNK_SIZE
        chunk_len = chunk_size or request_size or _CHUNK_SIZE
        emitted = 0
        pos = offset

        while pos < len(self.payload) and (limit is None or emitted < limit):
            yield self.payload[pos : pos + chunk_len]
            pos += step
            emitted += 1


@pytest.mark.asyncio
async def test_parallel_media_download_reuses_streams(tmp_path):
    downloader = DummyDownloader()
    payload = bytes(range(256)) * ((_CHUNK_SIZE * 4) // 256)
    downloader.client = FakeClient(payload)

    dest = tmp_path / "file.bin"
    resume_event = asyncio.Event()
    resume_event.set()

    result = await downloader._download_parallel(
        message=object(),
        file_size=len(payload),
        dest_path=dest,
        num_workers=2,
        filename="file.bin",
        resume_event=resume_event,
    )

    assert result == dest
    assert dest.read_bytes() == payload
    assert len(downloader.client.calls) == 2
    assert {call["offset"] for call in downloader.client.calls} == {0, _CHUNK_SIZE}
    assert all(call["stride"] == _CHUNK_SIZE * 2 for call in downloader.client.calls)


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
    downloader._get_media_file_size = lambda media: _CHUNK_SIZE * 20

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
