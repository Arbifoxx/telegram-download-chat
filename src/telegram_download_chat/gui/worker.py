"""Worker thread for handling background tasks."""
import logging
import os
import subprocess
import sys
from pathlib import Path
from typing import List

from PySide6.QtCore import QThread, Signal

from telegram_download_chat.paths import get_downloads_dir


class WorkerThread(QThread):
    """Worker thread for running command line tasks in the background."""

    log = Signal(str)
    progress = Signal(int, int)  # current, maximum
    status_update = Signal(str)  # parsed status for status bar
    finished = Signal(list, bool)  # files, was_stopped_by_user
    media_paused = Signal()
    media_resumed = Signal()
    media_manually_paused = Signal()
    file_downloading = Signal(str, int)  # filename, total_bytes
    file_progress = Signal(str, int, int)  # filename, bytes_done, total_bytes
    file_done = Signal(str)  # filename

    def __init__(self, cmd_args, output_dir):
        """Initialize the worker thread.

        Args:
            cmd_args: List of command line arguments
            output_dir: Directory where output files will be saved
        """
        super().__init__()
        self.cmd = cmd_args
        self.output_dir = output_dir
        self.current_max = 1000  # Initial maximum value
        self._is_running = True
        self._stopped_by_user = False
        self.process = None
        self._stop_file = None  # Path to stop file for inter-process communication
        self._pause_file: Path | None = None
        self._manual_pause_file: Path | None = None  # User-triggered pause

    def pause(self):
        """Manually pause media downloads."""
        if self._manual_pause_file:
            try:
                self._manual_pause_file.touch()
            except Exception:
                pass

    def resume(self):
        """Resume media downloads (rate-limit or manual pause)."""
        # Rate-limit pause file
        if self._pause_file and self._pause_file.exists():
            try:
                self._pause_file.unlink()
            except Exception:
                pass
        self._pause_file = None
        # Manual pause file
        if self._manual_pause_file and self._manual_pause_file.exists():
            try:
                self._manual_pause_file.unlink()
            except Exception:
                pass

    def stop(self):
        """Stop the worker thread gracefully."""
        self._is_running = False
        self._stopped_by_user = True
        if self.process:
            # Create a stop file to signal the process to stop gracefully
            if not self._stop_file:
                import tempfile

                self._stop_file = (
                    Path(tempfile.gettempdir()) / "telegram_download_stop.tmp"
                )
            try:
                self._stop_file.touch()
                self.log.emit("\nSending graceful shutdown signal...")
            except Exception:
                # Fallback to terminate if stop file creation fails
                self.process.terminate()

    def _parse_status(self, line):
        """Parse log line and emit status update for the status bar.

        Args:
            line: Output line from the command
        """
        lower = line.lower()
        if "fetched:" in lower:
            # Extract count from "Fetched: N"
            try:
                count = line.split("Fetched:")[1].strip().split()[0]
                self.status_update.emit(f"Fetched {count} messages")
            except (IndexError, ValueError):
                pass
        elif "saved" in lower and "messages to" in lower:
            try:
                # "Saved N messages to ..."
                parts = line.split("Saved")[1].strip().split()
                count = parts[0]
                self.status_update.emit(f"Saved {count} messages")
            except (IndexError, ValueError):
                pass
        elif "resuming download from" in lower:
            self.status_update.emit("Resuming download...")
        elif "flood" in lower and "wait" in lower:
            self.status_update.emit("Rate limited, waiting...")
        elif "downloading media" in lower:
            self.status_update.emit("Downloading media...")
        elif "media_paused:" in lower:
            marker = "MEDIA_PAUSED:"
            idx = line.upper().find(marker)
            if idx != -1:
                raw = line[idx + len(marker):]
                pause_path = raw.split()[0] if raw.split() else ""
                if pause_path:
                    self._pause_file = Path(pause_path)
            self.status_update.emit("Media downloads paused (rate limited)")
            self.media_paused.emit()
        elif "media_manually_paused" in lower:
            self.status_update.emit("Media downloads paused")
            self.media_manually_paused.emit()
        elif "media_resumed" in lower:
            self._pause_file = None
            self.status_update.emit("Media downloads resumed")
            self.media_resumed.emit()
        elif "media_downloading:" in lower:
            marker = "MEDIA_DOWNLOADING:"
            idx = line.upper().find(marker)
            if idx != -1:
                raw = line[idx + len(marker):]
                # Format: {filename}:{file_size} — split from right so filenames with colons work
                fname, _, size_str = raw.rpartition(":")
                if not fname:
                    fname = size_str
                    size_str = "0"
                try:
                    total = int(size_str)
                except ValueError:
                    total = 0
                if fname:
                    self.file_downloading.emit(fname, total)
                    self.file_progress.emit(fname, 0, total)
        elif "media_downloaded:" in lower:
            marker = "MEDIA_DOWNLOADED:"
            idx = line.upper().find(marker)
            if idx != -1:
                fname = line[idx + len(marker):].strip()
                if fname:
                    self.file_done.emit(fname)
        elif "media_file_progress:" in lower:
            marker = "MEDIA_FILE_PROGRESS:"
            idx = line.upper().find(marker)
            if idx != -1:
                raw = line[idx + len(marker):]
                # Format: {filename}:{bytes_done}:{total} — last two segments are numbers
                try:
                    right = raw.rsplit(":", 2)
                    if len(right) >= 3:
                        fname = right[0]
                        done = int(right[1])
                        total = int(right[2])
                        self.file_progress.emit(fname, done, total)
                except (ValueError, IndexError):
                    pass

    def _extract_progress(self, line):
        """Extract progress information from command output.

        Args:
            line: Output line from the command
        """
        try:
            # Look for progress information in the format: [current/max]
            if "[" in line and "]" in line and "/" in line:
                progress_part = line[line.find("[") + 1 : line.find("]")]
                if "/" in progress_part:
                    current, max_progress = progress_part.split("/")
                    try:
                        current = int(current.strip())
                        self._update_progress(current)
                    except (ValueError, TypeError):
                        pass
        except Exception as e:
            logging.debug(f"Error extracting progress: {e}")

    def _update_progress(self, current):
        """Update the progress bar with current progress.

        Args:
            current: Current progress value
        """
        new_max = self.current_max
        if current > self.current_max:
            if current <= 10000:
                new_max = 10000
            elif current <= 50000:
                new_max = 50000
            elif current <= 100000:
                new_max = 100000
            else:
                new_max = (current // 100000 + 1) * 100000

            if new_max != self.current_max:
                self.current_max = new_max

        self.progress.emit(current, self.current_max)

    def run(self):
        """Run the worker thread."""
        files = []

        import tempfile as _tempfile
        self._manual_pause_file = (
            Path(_tempfile.gettempdir()) / f"tdc_manual_pause_{os.getpid()}.tmp"
        )
        # Ensure it doesn't exist at start
        self._manual_pause_file.unlink(missing_ok=True)

        try:
            # Build the command using the module path directly
            cmd = [sys.executable, "-m", "telegram_download_chat"] + self.cmd
            cmd += ["--pause-file", str(self._manual_pause_file)]

            self.log.emit(f"Executing: {' '.join(cmd)}")

            # Start the process
            env = os.environ.copy()
            env.setdefault("PYTHONIOENCODING", "utf-8")

            self.process = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
                bufsize=1,  # Line buffered
                universal_newlines=True,
                env=env,
            )

            # Read output in real-time
            while self._is_running and self.process.poll() is None:
                line = self.process.stdout.readline()
                if not line:
                    break

                line = line.rstrip()
                self.log.emit(line)

                # Try to extract progress information from the output
                self._extract_progress(line)
                self._parse_status(line)

            # Read any remaining output
            if self.process.poll() is not None:
                for line in self.process.stdout:
                    line = line.rstrip()
                    if line:
                        self.log.emit(line)
                        self._extract_progress(line)
                        self._parse_status(line)

        except Exception as e:
            self.log.emit(f"Error in worker thread: {str(e)}")
            logging.error("Worker thread error", exc_info=True)
        finally:
            # Ensure process is terminated
            if (
                hasattr(self, "process")
                and self.process
                and self.process.poll() is None
            ):
                self.process.terminate()
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()

            # If we broke out of the loop because stop was requested
            if (
                hasattr(self, "process")
                and self.process
                and self.process.poll() is None
            ):
                # Wait for the process to stop
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()

            # After completion, collect files in output_dir
            if not self.output_dir:
                self.output_dir = get_downloads_dir()
            p = Path(self.output_dir)
            if p.exists():
                # Get list of files with full paths and sort by modification time, newest first
                all_files = []
                for ext in ("*.json", "*.txt"):
                    all_files.extend(f for f in p.rglob(ext) if f.is_file())
                files.extend(
                    str(f.absolute())
                    for f in sorted(
                        all_files, key=lambda x: x.stat().st_mtime, reverse=True
                    )
                )

            # Clean up stop file if it exists
            if self._stop_file and self._stop_file.exists():
                try:
                    self._stop_file.unlink()
                except Exception:
                    pass

            # Clean up manual pause file
            if self._manual_pause_file and self._manual_pause_file.exists():
                try:
                    self._manual_pause_file.unlink()
                except Exception:
                    pass

            # Emit finished signal with collected files
            self.finished.emit(files, self._stopped_by_user)
