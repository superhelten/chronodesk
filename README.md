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

While locked the overlay ignores the mouse entirely, so the tray icon is the way back.
Locking is disabled if the tray icon could not be created, and the app always starts unlocked.

Settings (mode, timer length, size, backdrop, chroma, seconds, always-on-top) and the window
position are saved to `%APPDATA%\chronodesk\data\app.ron`.

## Layout

- `src/main.rs` – window setup (transparent, borderless, always-on-top, hidden from taskbar)
- `src/app.rs` – state, rendering, window sizing, repaint scheduling
- `src/tray.rs` – tray icon and the shared native menu; events reach egui via a channel + `request_repaint`
- `src/timer.rs` – pure stopwatch/countdown logic and formatting (unit-tested)
- `src/icon.rs` – procedurally drawn clock icon (no asset files)

## Performance notes

- Repaints only when the display changes: 1 frame/s for the clock (landing just after each second),
  10 frames/s for a running stopwatch, none while paused. Idle CPU is below Windows' accounting resolution.
- Memory: ~40 MB private working set / ~65 MB private bytes, almost all of it the graphics driver.
  glow was chosen after measuring: wgpu used 130–310 MB private working set.
