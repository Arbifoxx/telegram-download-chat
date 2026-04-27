# Rust GUI Implementation Plan

## Goal

Replace the current Python GUI with a native Rust GUI built with `iced`, while keeping the already-working Rust downloader backend and preserving the Python-side non-GUI responsibilities that still make sense during migration.

Primary goals:

1. Include the downloader functions from the Python GUI.
2. Include the login/authentication flow from the Python GUI.
3. Do not include the current Convert functions/tab.
4. Match the visual direction of the attached reference (reference.png at the root of the project, /Users/arbifox/tdesktop/telegram-download-chat/reference.png):
   - dark charcoal surfaces
   - purple accent color
   - rounded panels and inputs
   - compact desktop utility layout
   - “cool audio-tool” style rather than generic business UI

## Product Scope

The Rust GUI should eventually become the main desktop app for:

- Telegram login/session management
- Chat/channel download configuration
- Starting/stopping/pausing/resuming downloads
- Showing file progress and overall progress
- Showing useful logs/status text
- Opening output/download folders

Out of scope for the Rust GUI first release:

- Convert tab / JSON conversion utilities
- Media transport/progress log toggles as user-facing controls
- HTML/PDF rendering inside Rust

HTML/PDF generation can continue to be handled by the existing Python/backend layer during migration, or later be moved behind a subprocess/API boundary if desired.

If it is possible, the user would like to have the project as a complete rust implementation, no python at all. Therefore, it is a priority to find a way to implement at least the HTML generation in rust. The user has noted that the PDF generation, while still a nice feature, is of less priority.

## Architecture Direction

### Target shape

- `iced` desktop GUI binary
- Rust downloader backend integrated directly or called through an internal Rust module boundary
- Rust auth/session management for Telegram
- Python no longer required for GUI interaction
- Python can remain temporarily available for legacy CLI/export paths during transition
- Python will eventually be completely phased out

### Recommended long-term structure

- `native/tdc-app/`
  - `src/main.rs`
  - `src/app/`
  - `src/theme/`
  - `src/downloads/`
  - `src/auth/`
  - `src/config/`
  - `src/widgets/`
- Reuse logic from `native/tdc-downloader/` where practical.
- Either:
  - merge downloader code into shared Rust modules, or
  - make `tdc-downloader` a library crate plus thin binary wrapper.

### Recommended Telegram stack

Use the same Rust MTProto stack already being used:

- `grammers-client`
- `grammers-session`
- `grammers-mtsender`
- `grammers-tl-types`

Reason:

- keeps one transport stack
- avoids Python/Telethon for GUI/auth
- aligns with the downloader already being improved

## UI Requirements

### Tabs / sections

Keep only:

- `Download`
- `Settings`
- optionally `About`

Do not include:

- `Convert`

### Download view

Must support:

- chat input
- output path picker
- sort order
- split output mode
- message limit
- date filters
- user filter
- overwrite toggle
- media toggle
- HTML toggle
- PDF toggle
- concurrent downloads dropdown
- start button
- stop button
- pause/resume button
- overall progress bar
- active file progress list
- log/status area

Remove these user-facing toggles:

- media progress logs
- media transport logs

These can remain internal debug features if needed, but should not be exposed in the new UI.

### Login flow

Must support:

- API ID / API hash configuration
- phone number entry
- login code entry
- 2FA password entry
- session persistence
- signed-in identity display
- logout / reset session action

### Style direction

Use the attached reference as the design anchor:

- low-gloss dark background
- soft borders
- purple accent buttons, toggles, highlights, and progress elements
- compact dense controls
- panelized utility-app layout
- subtle contrast between panels rather than flat black everywhere

Suggested tokens:

- background: near-black blue/charcoal
- panel: dark gray with slight warm/purple tint
- accent: medium lavender / violet
- accent-strong: brighter purple for primary actions
- text: off-white
- muted text: gray-lilac
- danger: saturated red for Stop
- success: restrained green

## Functional Phases

### Phase 1: Rust GUI shell

Deliver:

- new `iced` app crate
- top-level window
- theme system
- Download tab layout
- Settings tab layout
- static controls and placeholder state

Acceptance:

- app launches on Windows/macOS/Linux
- visual style is clearly aligned with the reference

### Phase 2: Settings/config persistence

Deliver:

- Rust config storage
- save/load of:
  - API credentials
  - output path
  - toggles
  - concurrency
  - sort/split/filter options

Acceptance:

- app restores prior state on reopen

### Phase 3: Rust auth flow

Deliver:

- Telegram login flow in Rust using `grammers`
- phone/code/password screens or modal workflow
- persisted session
- current account display

Acceptance:

- user can log in and reopen app still authenticated

### Phase 4: Downloader integration

Deliver:

- connect GUI actions to Rust downloader logic
- start/stop/pause/resume
- active transfer list
- overall progress
- file progress bars
- log output panel

Acceptance:

- GUI can run real downloads without Python GUI

### Phase 5: Export integration

Deliver one of these approaches:

Option A:

- keep Python export pipeline for HTML/PDF by invoking existing Python backend after message/media work

Option B:

- move HTML/PDF generation later into Rust or into a cleaner subprocess boundary

Recommendation:

- do Option A first for speed and lower risk

Acceptance:

- HTML/PDF still available from the Rust GUI workflow

### Phase 6: Packaging

Deliver:

- single desktop binary/app packaging flow per platform
- Windows packaging first
- then macOS
- then Linux

Acceptance:

- end user can run the Rust GUI without launching Python manually

## Recommended Implementation Details

### State model

Use a central application state struct:

- auth state
- settings state
- download state
- active jobs
- logs
- modal/dialog state

Suggested top-level enums:

- `Screen` or `Tab`
- `AuthState`
- `DownloadState`
- `Message`

### Async model

Use `iced` tasks/subscriptions for:

- auth progress
- downloader events
- periodic UI refresh
- filesystem/dialog callbacks

Keep network/download work off the UI thread.

### Downloader event model

Expose structured events from the Rust download layer:

- run started
- file started
- file progress
- file completed
- file skipped
- file restarted
- file error
- run summary
- status/log events

The GUI should consume typed events, not parse log strings.

### Session/config files

Store:

- config in app-data directory
- Rust Telegram session in app-data directory
- logs optionally in app-data directory

Keep paths platform-native.

## Migration Strategy

### Step 1

Build the new Rust GUI as a second app without removing the Python GUI.

### Step 2

Reach feature parity for:

- login
- download configuration
- media download workflow

### Step 3

Reuse existing export flow temporarily for HTML/PDF.

### Step 4

Once stable, make the Rust GUI the default packaged desktop app.

### Step 5

Deprecate the Python GUI after enough real-world validation.

## Concrete TODO List

1. Create `native/tdc-app/` crate using `iced`.
2. Add app theme module with reference-inspired colors, spacing, border radius, and button styles.
3. Build Download tab without Convert tab.
4. Port persisted settings from Python GUI to Rust config.
5. Implement Rust login flow using `grammers`.
6. Refactor `tdc-downloader` into reusable library modules where needed.
7. Connect GUI actions to downloader start/stop/pause/resume.
8. Add file list + progress bars + overall progress.
9. Add log/status pane.
10. Add output-folder open action.
11. Bridge HTML/PDF export through existing Python path for first release.
12. Package Windows build as the first supported desktop target.

## Design Notes

The new Rust GUI should not look like a generic settings form.
It should feel like a purpose-built desktop tool:

- stronger visual identity
- compact but readable layout
- visibly native performance
- clear active/download state
- polished dark theme with purple accents

If there is a choice between “plain but fast to implement” and “stylized but still practical,” choose the stylized direction as long as usability stays high.

## Known Risks

- Rust auth/session flow may take longer than layout/theming.
- HTML/PDF may stay Python-backed longer than expected.
- `iced` desktop polish can require custom styling work.
- cross-platform file dialogs and packaging may differ by OS.
- downloader and GUI should be refactored into shared Rust modules to avoid duplication.

## Tomorrow’s Recommended Starting Point

1. Create `native/tdc-app/` crate.
2. Implement the themed shell window and Download tab only.
3. Reuse the current Python GUI field set minus Convert and minus media log toggles.
4. Add config persistence.
5. Then begin Rust auth integration.

That sequence gets visible momentum quickly while preserving a clean path toward full replacement.
