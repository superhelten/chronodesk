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
| Streaming | Menu → *Appearance* → *Chroma key background (#00FF00)* |
| Text over bright windows | Menu → *Appearance* → *Text outline* (on by default; a dark halo keeps white text legible without a backdrop) |
| 12-hour clock, date line | Menu → *Appearance* → *12-hour clock*, *Show date*. AM/PM sits on the caption line; with the date hidden the overlay shrinks to the time alone |
| Digital font | Menu → *Appearance* → *Digital font*: seven-segment digits drawn as polygons, no font file involved. The caption stays in the typeface, like the printed labels on a real display |
| Colours | Menu → *Appearance* → *Colours*: Default, Warm, Cool or Amber. Only the readout colours change; halo, backdrop and controls keep their contrast |
| Night mode | Menu → *Appearance* → *Night mode*: Off, On, or Auto between `night_from` and `night_to` (22:00–07:00 by default). Dims the readout to `night_dim` (0.7; never below 0.6) |

While locked the overlay ignores the mouse entirely, so the tray icon is the way back.
Locking is disabled if the tray icon could not be created, and the app always starts unlocked.

## Configuration

Settings (mode, timer length, size, font, backdrop, chroma, seconds, clock format, date line, colours,
night mode and its schedule, always-on-top) and the window position are saved to `%APPDATA%\chronodesk\data\app.ron` (`~/Library/Application Support/...`
on macOS), about 1.5 s after the last change and again on exit. The file is meant to be
readable and hand-editable. Setting `CHRONODESK_CONFIG` to a path names the file outright,
which is how the test scripts stay out of the config you actually use.

The app owns this file rather than using eframe's persistence, to get two properties:

- **Atomic writes.** Saving writes `app.ron.tmp`, flushes it, then renames it over `app.ron`,
  so an interrupted write can never leave a truncated file.
- **Per-field tolerance.** The file is parsed untyped first and each field converted on its
  own, so one bad value falls back to its default while everything else — including the window
  position — is kept. Repairs are written back and reported on stderr.

Only a file that cannot be parsed at all, or one written by a newer `schema_version`, is
rejected; it is copied to `app.ron.bak` first. Files from the older eframe layout are migrated
automatically.

### Window placement

A saved position only means something against the monitors attached right now, so it is checked
at startup, once the overlay's real size is known:

- **Nothing visible on any monitor** — typically the screen it lived on has been unplugged —
  and it goes to the top-right of the primary monitor, inset by 24 pt.
- **Visible but hanging over the taskbar or a screen edge**, and it is nudged back inside that
  monitor's work area, as close to where you left it as possible.
- Otherwise it is left alone, including on a secondary monitor or one at negative coordinates.

The new position is written back to `app.ron` immediately, so the next launch starts from a place
that exists. Windows would clamp such a window on-screen by itself; this decides *where* instead
of accepting whatever corner that lands in.

The margin is in points and scaled by the target monitor's DPI factor, and the work area comes
from the OS with the taskbar already excluded in pixels, so the overlay clears the tray at 100 %,
150 % or any other scaling. The arithmetic is done in physical pixels throughout — the only
space in which several monitors at different scalings share one coordinate system.

## Test instrumentation

Driving the overlay with a real mouse is unreliable — the pointer belongs to whoever is using the
machine, and since the window is transparent a screenshot of it also captures whatever moves
behind it. So the app can be driven from inside instead:

```sh
cargo build --release --features instrument
target/release/chronodesk --instrument     # prints its port, also written to %TEMP%\chronodesk-instrument.port
pwsh -File scripts/instrument-test.ps1     # PowerShell 7: the script is UTF-8 without a BOM
```

With the feature *and* the flag, the app listens on a loopback port and takes line commands:
`cmd <menu-id>` (the same ids the menus use), `hover <x> <y>` / `hover off`, `click <x> <y>`,
`place <x> <y>` / `placement`, `stats` / `stats reset`, `quit`. Synthetic pointer events are injected into egui's raw input, so
hovering and clicking take the same path as a real mouse.

`stats` reports frames, fps, mean/max time of the app's `ui()` pass, the gap between frames and a
breakdown of why each frame was drawn (`tick`, `input`, `config`, `other`).

`place` re-runs the placement check with a synthetic saved position, as if it had just been read
from the config file, and `placement` reports what was decided and against which monitors — so a
position left behind by a disconnected screen is reproducible without unplugging one:

```sh
pwsh -File scripts/placement-test.ps1
```

That script seeds a scratch `app.ron` with a position outside every screen, points the app at it
with `CHRONODESK_CONFIG`, and checks the overlay against the work area it reads from Windows
itself rather than against the app's own numbers.

Without the feature the flag only prints a notice: there is no listener, no thread and no timers.

## Layout

- `src/main.rs` – window setup (transparent, borderless, always-on-top, hidden from taskbar)
- `src/app.rs` – state, rendering, window sizing, repaint scheduling
- `src/theme.rs` – visual tokens: colours, alphas and proportions
- `src/layout.rs` – derived state: metrics and glyph measurements, rebuilt only when the size or display scale changes
- `src/config.rs` – the config file: atomic saves, field-tolerant loading
- `src/placement.rs` – validating the saved position against the attached monitors and their work areas
- `src/tray.rs` – tray icon and the shared native menu; events reach egui via a channel + `request_repaint`
- `src/timer.rs` – pure stopwatch/countdown logic and formatting (unit-tested)
- `src/icon.rs` – procedurally drawn clock icon (no asset files)

## Performance notes

- Repaints only when the display changes: 1 frame/s for the clock (landing just after each second),
  10 frames/s for a running stopwatch, none while paused. Idle CPU is below Windows' accounting resolution.
- Memory: ~40 MB private working set / ~65 MB private bytes, almost all of it the graphics driver.
  glow was chosen after measuring: wgpu used 130–310 MB private working set.
