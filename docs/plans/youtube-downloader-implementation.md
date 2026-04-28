# YouTube Downloader Implementation Notes

## Scope

Build the native Rust YouTube downloader in the `tdc-app` GUI using a `yt-dlp` Rust crate/backend path.

The current UI work is only a mock. Real fetching, preview loading, and downloading still need to be wired.

## UI Requirements

1. URL box at the top, matching the Telegram downloader style.
2. Under the URL box, a `Preview` panel with an empty gray rectangle.
3. Under the preview, a `Quality` dropdown with the label above the selector.
4. The `Quality` dropdown must start empty and should only be populated after video information is successfully fetched.
5. Under that, a `Format` dropdown with:
   - `mp4`
   - `mkv`
   - `webm`
   - `mp3`
6. Under that, a `Codec` dropdown with:
   - `VP9`
   - `VP8`
   - `AVC`
   - `HEVC`
   - `AV1`
7. The `Codec` dropdown should be empty when `mp3` is selected.
8. Under that, a `Cookies` dropdown with:
   - `None`
   - `Brave`
   - `Chrome`
   - `Chromium`
   - `Edge`
   - `Opera`
   - `Vivaldi`
   - `Whale`
   - `Firefox`
   - `Safari`
9. Under that, an output directory box.
10. Save output using:
   - `[Video title] - [Video Author].[format]`

## Button Behavior

### Initial state

- Bottom button text: `Get Video Information`

### While fetching metadata

- Button text: `Fetching...`
- Preview box text: `Fetching...`

### On fetch failure

- Preview box text: `Failed to get video`

### On fetch success

- Replace the gray preview rectangle contents with the video thumbnail.
- Populate the `Quality` dropdown with the available qualities for the fetched video.
- Bottom button text changes to `Download Video`.

### While downloading

- A Telegram-style progress indicator appears between the form controls and the bottom button.
- Bottom button text changes to `Downloading...`

## Download Rules

Use the equivalent of these yt-dlp CLI options:

- `--concurrent-fragments 5`
- `--embed-subs`
- `--embed-thumbnail`
- `--embed-metadata`
- `--embed-chapters`
- `--embed-info-json`

Also support:

- `--cookies-from-browser`
- `--format`

Always use the best quality audio track available.

## Backend Notes

- Use a Rust `yt-dlp` crate / wrapper path rather than shelling out from the GUI layer if practical.
- If a clean crate integration is not good enough, use a thin Rust backend module that shells out to `yt-dlp` as a fallback while preserving the same UI contract.
- Cookies dropdown should map directly onto yt-dlp browser-cookie import behavior.
- The preview step should fetch:
  - title
  - author/uploader
  - thumbnail URL
  - available formats/stream metadata if needed later

## Suggested Wiring Steps

1. Add real YouTube metadata fetch state:
   - idle
   - fetching
   - ready
   - failed
2. Replace the gray preview box with a loaded thumbnail image on success.
3. Add real format/codec validation:
   - disable codec when `mp3`
   - keep audio best-quality behavior
4. Add download progress events and bind them to the existing progress row style used in Telegram.
5. Add final output filename templating:
   - `%(title)s - %(uploader)s.%(ext)s`
