# ChronoDesk

Minimalist transparent desktop overlay: clock, stopwatch and timer. Rust + `eframe`/`egui` (glow renderer) + `tray-icon`.

## Build & run

```sh
cargo run --release          # target/release/chronodesk.exe (~5.5 MB, no console window)
cargo test                   # timer/stopwatch logic + menu-id parsing
```

Requires Rust 1.95+ (eframe 0.36).

## Using it

| Action | How |
| --- | --- |
| Move | Drag the overlay with the left mouse button |
| Menu | Right-click the overlay, or right-click the tray icon (same menu) |
| Lock / unlock click-through | **Left-click the tray icon** (or menu → *Locked*). The icon turns amber while locked |
| Start / pause, reset | Hover the overlay in Stopwatch/Timer mode, or `Space` / `R` when focused |
| Switch mode | Menu, or `1` Clock · `2` Stopwatch · `3` Timer |
| Timer duration | Menu → *Timer duration*, or scroll over an idle timer (±1 min per notch) |
| Streaming | Menu → *Chroma key background (#00FF00)* |
| Text over bright windows | Menu → *Text outline* (on by default; a dark halo keeps white text legible without a backdrop) |

While locked the overlay ignores the mouse entirely, so the tray icon is the way back.
Locking is disabled if the tray icon could not be created, and the app always starts unlocked.

## Configuration

Settings (mode, timer length, size, backdrop, chroma, seconds, always-on-top) and the window
position are saved to `%APPDATA%\chronodesk\data\app.ron` (`~/Library/Application Support/...`
on macOS), about 1.5 s after the last change and again on exit. The file is meant to be
readable and hand-editable.

The app owns this file rather than using eframe's persistence, to get two properties:

- **Atomic writes.** Saving writes `app.ron.tmp`, flushes it, then renames it over `app.ron`,
  so an interrupted write can never leave a truncated file.
- **Per-field tolerance.** The file is parsed untyped first and each field converted on its
  own, so one bad value falls back to its default while everything else — including the window
  position — is kept. Repairs are written back and reported on stderr.

Only a file that cannot be parsed at all, or one written by a newer `schema_version`, is
rejected; it is copied to `app.ron.bak` first. Files from the older eframe layout are migrated
automatically.

## Layout

- `src/main.rs` – window setup (transparent, borderless, always-on-top, hidden from taskbar)
- `src/app.rs` – state, rendering, window sizing, repaint scheduling
- `src/theme.rs` – visual tokens: colours, alphas and proportions
- `src/layout.rs` – derived state: metrics and glyph measurements, rebuilt only when the size or display scale changes
- `src/config.rs` – the config file: atomic saves, field-tolerant loading
- `src/tray.rs` – tray icon and the shared native menu; events reach egui via a channel + `request_repaint`
- `src/timer.rs` – pure stopwatch/countdown logic and formatting (unit-tested)
- `src/icon.rs` – procedurally drawn clock icon (no asset files)

## Performance notes

- Repaints only when the display changes: 1 frame/s for the clock (landing just after each second),
  10 frames/s for a running stopwatch, none while paused. Idle CPU is below Windows' accounting resolution.
- Memory: ~40 MB private working set / ~65 MB private bytes, almost all of it the graphics driver.
  glow was chosen after measuring: wgpu used 130–310 MB private working set.
