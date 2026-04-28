# UI Design Brief

This file captures the user’s preferred UI direction for the native Rust app so future work can follow the same design language and avoid repeating layout mistakes.

## Overall Direction

The app should feel:

- compact
- polished
- clean
- smooth
- consistent
- intentional
- desktop-native

It should not feel:

- stretched
- oversized
- clunky
- improvised
- overly verbose
- placeholder-heavy
- “developer UI”

The user strongly prefers a small utility-app feel over a wide dashboard feel.

## Visual Style

Use the existing dark purple/charcoal direction established in the Rust shell and the reference image.

### Palette

- dark charcoal / near-black background
- soft dark panels
- purple accent for active states and primary actions
- muted lilac/gray supporting text
- restrained red for stop/danger

### Shape Language

- rounded panels
- rounded inputs
- rounded buttons
- rounded service tiles
- no inconsistent mix of square and rounded controls

The window itself should not look over-styled. The content can be rounded; the app should not feel like one giant inflated rounded card.

## Layout Philosophy

The UI should be:

- compact
- portrait-leaning
- dense without feeling cramped
- visually centered
- aligned on clear rails

Avoid:

- unnecessary empty space
- wide rows just because space is available
- overly tall sections
- giant buttons unless they are primary bottom actions
- cut-off controls caused by poor vertical budgeting

## Persistent App Structure

The app uses:

- a left vertical service rail
- a divider line between the service rail and active screen
- one active downloader screen at a time

The service rail is meant for:

- Telegram
- YouTube
- Spotify
- Archive.org
- Website copy
- Generic downloader

Only some services are active now, but all placeholders should visually belong to the same system.

## Service Rail Rules

The left service selectors should be:

- vertically centered
- consistently aligned
- slightly vertically rectangular rather than perfect squares
- visually even from top to bottom

They should not:

- drift off-center
- have inconsistent padding
- have one tile centered differently than the others
- look broken or misrendered

## Screen Structure Rules

Each downloader screen should reuse the same overall visual language:

- top title/header panel
- stacked input and option panels below
- bottom primary action area

When scrolling is needed:

- the service rail stays fixed
- the header/title stays fixed
- the bottom primary action area stays fixed
- only the middle content scrolls
- the scrollbar should not be visible

## Typography and Labels

Labels should be:

- short
- direct
- production-like

Avoid placeholder/development copy like:

- “Rust GUI shell ready”
- “Telegram auth and downloader wiring come next”
- any explanatory text that would not appear in a real finished app

The user prefers practical labels over descriptive filler.

## Alignment Rules

Controls must align across panels.

This is especially important for:

- checkbox text lining up with text inputs
- divider lines actually centering between left and right option groups
- dropdown labels aligning with their dropdowns
- file rows aligning with each other
- bottom buttons centering their text

If a divider exists, it should:

- be intentional
- be visually centered
- stop at the correct vertical boundary
- not float awkwardly
- not be too tall or too short

## Telegram Screen Requirements

The Telegram downloader screen should follow the compact mockup-driven structure.

### Core Elements

- `URL` field near the top
- `Output` field underneath
- `Options` panel
- `Files:` panel
- bottom action button area

### Options Panel

Left side:

- `Replace Files`
- `Output PDF`
- `Output HTML`

Divider:

- a real vertical line
- centered between left and right option groups

Right side:

- `Concurrent Streams` selector
- `Sort` selector
- `Credentials` button

Behavior:

- labels above the selectors
- credentials button spans the bottom of the options area

### Telegram Action Buttons

When idle:

- show one `Start` button

When active:

- show `Pause` and `Stop`

The button text must be centered properly.

### Files Panel

The files panel should:

- be compact
- not have useless empty space
- not include junk controls
- show clear progress rows

Progress rows should include:

- filename
- progress bar
- transferred size / total size / percent

## YouTube Screen Requirements

The YouTube screen should reuse the Telegram design language instead of inventing a totally different UI.

### Core Elements

- `URL`
- `Preview`
- `Quality`
- `Format`
- `Codec`
- `Cookies`
- `Output`
- progress area
- bottom action button

### Preview Panel

- titled `Preview`
- contains a gray rectangle before loading
- becomes thumbnail area later

### Selector Style

Use the same dropdown visual system as Telegram:

- label above dropdown
- compact spacing
- no giant empty blocks

## Sizing Rules

The user has repeatedly preferred a smaller window over a larger one.

The app should stay:

- compact
- not excessively wide
- not excessively tall
- visually efficient

If there is extra space, do not automatically stretch controls to fill it unless doing so improves the design.

## Interaction Design Notes

The user likes:

- clear single-purpose primary actions
- controls that visually match their purpose
- minimal friction
- no unnecessary tabs if one screen is enough

The user does not like:

- extra tabs that don’t add value
- filler labels
- settings separated out without a strong reason
- UI elements that exist “just because”

## Future-Service Rule

Even while only Telegram and YouTube are active, the app should feel like a cohesive multi-service downloader suite.

That means:

- shared layout system
- shared button language
- shared panel styling
- shared spacing rhythm

But each service screen can still have its own fitting workflow.

## Practical Standard For Future UI Work

Before changing the UI, future work should check:

1. Is this more compact or less compact?
2. Does this improve alignment or introduce drift?
3. Would this look like a real finished utility app?
4. Is this following the existing purple/charcoal style?
5. Is there filler text or filler structure that should be removed?
6. Is this adding empty space without purpose?
7. Does this match the user’s mockup/instructions literally where specified?

If unsure, default to:

- smaller
- simpler
- cleaner
- tighter
- more aligned
- more production-like

