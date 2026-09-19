# ChronoDesk

Minimalist transparent desktop overlay: clock, stopwatch and timer. Rust + `eframe`/`egui` (glow renderer) + `tray-icon`.

## Install

Download `ChronoDesk-Setup.exe` and run it. There is no wizard: it copies itself to
`%LOCALAPPDATA%\ChronoDesk\chronodesk.exe`, adds a Start menu shortcut and an entry under *Installed
apps*, and starts the overlay, which greets a first-time user with a card of tips. Nothing asks for
administrator rights, because everything it touches belongs to the user.

- **Run it again** at any time. With the same version installed it copies nothing and the running
  overlay outlines itself for a few seconds ("here I am"). With a newer one it asks the running overlay
  to quit, swaps the exe and starts the new one; settings and a running timer carry over.
- **Start with Windows** is not switched on for you; it is one click in the menu, and from the installed
  copy it names the fixed path. If an entry already exists (from a build folder, a copy in Downloads),
  the setup repoints it at the installed exe. An entry switched off in Task Manager stays off.
- **Uninstall** from *Settings → Apps → Installed apps*, or `chronodesk.exe --uninstall`. The exe, the
  shortcut, the registry entry and the folder go; your settings in `%APPDATA%\chronodesk` stay.

The setup is the app itself under another name: an exe whose file name contains `setup`, or any copy
run with `--install`, installs instead of showing a clock (`--quiet` suppresses the message boxes).
So there is no second program to build, and no installer that can drift out of step with the app:

```sh
pwsh -File scripts/package.ps1      # dist\ChronoDesk-Setup.exe + its SHA-256
```

The exe is not code-signed, so SmartScreen will say "unrecognised app" on a machine that has not
seen it before (*More info → Run anyway*).

## Build & run

```sh
cargo run --release          # target/release/chronodesk.exe (~5.5 MB, no console window)
cargo test                   # pure logic, plus the mutex and a scratch registry key on Windows
```

Requires Rust 1.95+ (eframe 0.36).

## Using it

| Action | How |
| --- | --- |
| First launch | A welcome card instead of the clock: where the menu is, the keys, and the looks worth knowing. Any click, key or menu choice dismisses it for good; menu → *Quick tips* shows it again |
| Move | Drag the overlay with the left mouse button |
| Menu | Right-click the overlay, or right-click the tray icon (same menu) |
| Lock / unlock click-through | **Left-click the tray icon** (or menu → *Locked*). The icon turns amber while locked |
| Start / pause, reset | Hover the overlay in Stopwatch/Timer mode, or `Space` / `R` when focused |
| Switch mode | Menu, or `1` Clock · `2` Stopwatch · `3` Timer · `4` Markets |
| Timer duration | Menu → *Timer duration*, or scroll over an idle timer (±1 min per notch) |
| Restarts | A stopwatch or timer that is running keeps running across a restart of the app, a logout or a reboot: the time it was away is counted, and a paused one comes back as it was. A timer that ran out in the meantime comes back as *TIME'S UP*, without a chime unless it finished within the last half minute |
| Timer sound | A finished timer blinks for half a minute and chimes three times, ten seconds apart, in whatever mode is showing. Menu → *Timer duration* → *Sound when finished* switches the chime off |
| Market board | Mode *Markets*: one row per exchange with its local time and a status dot (filled green = trading, amber = midday break, hollow = closed), and the next open or close across the board on the caption line. Menu → *Exchanges* picks the rows, *One line* lays them out as a strip, *Exchange codes* swaps city names for NYSE, LSE, OSE… |
| Streaming | Menu → *Appearance* → *Chroma key background (#00FF00)* |
| Text over bright windows | Menu → *Appearance* → *Text outline* (on by default; a dark halo keeps white text legible without a backdrop). Off under a backdrop or a chroma key, where a dark rim would only leave a fringe once the green is keyed out |
| 12-hour clock, date line | Menu → *Appearance* → *12-hour clock*, *Show date*. AM/PM sits on the caption line; with the date hidden the overlay shrinks to the time alone |
| Face | Menu → *Appearance* → *Face*: *Typeface*, *Seven-segment* or *Dot matrix* (a 5×7 grid of round LEDs, as on multi-zone and studio hardware clocks). Both LED faces are drawn as polygons, no font file involved, with the unlit diodes ghosted under every digit at about 8 % so the whole display is there. The caption stays in the typeface, like the printed labels on a real display |
| Seconds ring | Menu → *Appearance* → *Seconds ring*: sixty discrete LEDs along the window's outline, each in a dark socket, lit clockwise from the top as the seconds pass; the LED that just lit blooms, and every fifth position — where an hour hand would point — carries a red marker LED. Follows the clock, a running stopwatch, and a countdown (emptying with it) |
| Colours | Menu → *Appearance* → *Colours*: Default, Warm, Cool, Amber, or the industrial presets Green matrix, Red seven-segment, Yellow matrix and Studio (green time, red counters and ring). Only the readout colours change; halo, backdrop and controls keep their contrast |
| Start with Windows | Menu → *Start with Windows*. The check mark is read from the registry, so it means "this exe starts at the next login" and nothing less: an entry switched off in Task Manager, or one left behind by a copy that has since moved, shows as off, and one click puts it right |
| Night mode | Menu → *Appearance* → *Night mode*: Off, On, or Auto between `night_from` and `night_to` (22:00–07:00 by default). Dims the readout to `night_dim` (0.7; never below 0.6) |

While locked the overlay ignores the mouse entirely, so the tray icon is the way back.
Locking is disabled if the tray icon could not be created, and the app always starts unlocked.

### Market board

Nine exchanges are built in, west to east: New York (NYSE), London (LSE), Oslo (Euronext Oslo),
Frankfurt (Xetra), Mumbai (NSE), Shanghai (SSE), Hong Kong (HKEX), Tokyo (TSE) and Sydney (ASX).
The default board shows New York, London, Oslo, Tokyo and Sydney. Rows follow the 12/24-hour and
*Show seconds* settings; without seconds the board redraws once a minute.

Everything is computed locally, with no network and no time-zone database: each exchange is a
fixed offset plus one of three daylight-saving rules (US, EU, Australia), and a regular
Monday–Friday session with the midday break where there is one (Shanghai, Hong Kong, Tokyo).
Oslo's close is taken as 16:30, the end of the closing auction. Holidays and half-day closes
are **not** modelled: on a public holiday the dot is green when it shouldn't be. Should a
jurisdiction change its daylight-saving law, the rule in `src/tz.rs` changes with a rebuild,
exactly as a bundled database would.

Columns are sized for their widest possible values (two-digit hours, the longest countdown any
listed exchange can produce), so the window never resizes as the clocks tick. *One line* turns the
stack into a strip of modules, each with its label printed above its digits, the way a multi-zone
hardware clock is built; *Exchange codes* names the rows and the caption by the exchange instead of
the city. Labels are printed bold white whatever the palette, only the LEDs (times, status dots)
carry the colour, and faint hairlines separate the modules.

Translucent dressing — ghost segments, LED sockets, hairlines — is left out under a chroma key,
where anything half-transparent would key as a green tint.

### Studio look

*Seconds ring* wraps the readout in sixty LEDs along the window's rounded outline. The ring lights
clockwise from the top: one LED at :00, all sixty at :59. It follows the wall clock, a running
stopwatch (seconds of the current minute), or a countdown, where it empties with the digits. With
seconds hidden the ring still moves every second, so the clock is back to one frame per second; a
paused counter has nothing moving and stays asleep.

The *Studio* colour preset is the two-colour broadcast convention (Wharton's "GR" option): the
clock in green, the stopwatch, timer and ring in red. Every other preset draws the counters in the same colour as the
clock. A green preset over the chroma key is the user's choice, and is keyed out like anything green.

### One overlay per config file

Launching ChronoDesk a second time starts nothing new: the second copy sees the first, asks it to
show itself, and exits before it reads anything. The first takes the focus, wears an outline for
three seconds and has its position checked again, which brings it home if the screen it was on has
gone. A minimised overlay is restored first, from the listening thread, because a minimised window
draws no frames and so looks at nothing it is sent; with no taskbar button this is the way back. (It
should rarely get there: the window has no minimise style bit, so the system menu's *Minimise* and
Win+↓ do not apply to it.) The word is passed through a named event next to the guard's mutex: no socket, no polling, and
no frames while nothing is said. The installer asks a running overlay to quit the same way. Two copies sharing one `app.ron` would each save their own settings and window
position, with whichever quit last winning, behind two identical tray icons. The guard is a named
mutex keyed on the config file's path, not on the app, so an instance pointed elsewhere with
`CHRONODESK_CONFIG` runs alongside — which is what lets the test scripts run while your own overlay
stays up. Windows releases the mutex with the process, so a crash never leaves a stale lock.

*Start with Windows* writes the exe's quoted path to the per-user `Run` key (no admin rights, no
service, no scheduled task) and removes it again. The setting is deliberately **not** in `app.ron`:
the registry is the only record, because it can change behind the app's back — in Settings, in Task
Manager (which leaves the value and writes a veto under `Explorer\StartupApproved\Run`), or by
moving the exe. The menu re-reads it at most every ten seconds, on frames that are drawn anyway.
Neither feature exists on macOS yet.

### Timer sound

The chime is Windows' own notification sound (the scheme's *Asterisk*), asked for with one system
call: no audio file, no audio stack, and it follows the volume, mute and sound scheme already set —
with the scheme on *No sounds* the timer is silent. It plays when the countdown reaches zero and
again ten and twenty seconds later, inside the half minute the digits blink, then never again; *Reset*
or *Restart* stops it at once. A countdown left running behind another mode is still heard: its finish
is scheduled as a wake of its own, so even behind a paused stopwatch — which draws nothing — the whole
alarm costs about seven frames. With the sound off no wake is scheduled at all. macOS is silent for now.

## Configuration

Settings (mode, timer length, size, font, backdrop, chroma, seconds, clock format, date line, colours,
night mode and its schedule, the market board's rows, always-on-top) and the window position are saved to `%APPDATA%\chronodesk\data\app.ron` (`~/Library/Application Support/...`
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

`markets` is a list of exchange ids in display order (`["new-york", "london", "oslo", "tokyo", "sydney"]`),
tolerant per element: a misspelt id drops that row with a warning, a duplicate is collapsed, and an
empty list falls back to the default board. The *Exchanges* menu adds a row at its catalogue position
among whatever order the file holds, and never removes the last one. `board_layout` is `"vertical"`
or `"horizontal"`, `board_labels` is `"city"` or `"code"`, and `seconds_ring` and `timer_sound` are booleans.

`first_run` is `true` until the welcome card has been dismissed once, and only a launch that finds
no config file at all starts out with it set: a file from before the field existed belongs to
someone who knows the app. Set it to `true` by hand to see the card at the next launch.

`stopwatch` and `countdown` are the app's own notes, not settings: `None` when idle, otherwise
`Some((accumulated_ms: …, started_at_ms: …))` — what had been counted when it was last started, and
when that was, in milliseconds since the Unix epoch (`None` while paused). An `Instant` means nothing
to the next process, so a running counter is anchored to the wall clock, and the time until the next
launch is simply part of the run. They are written the moment a counter is started, paused or reset
rather than after the usual delay, because a logout or a power cut gives no notice; nothing is
written while a counter merely runs. Setting the PC's clock back while the app is closed counts as
no time away; setting it forward counts as time away.

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
`place <x> <y>` / `placement`, `stats` / `stats reset`, `state`, `passthrough on` / `off`, `quit`. Synthetic pointer events are injected into egui's raw input, so
hovering and clicking take the same path as a real mouse.

`state` reports what the last frame showed: mode, whether the welcome card is up (`welcome`) or the
overlay is outlined after another launch asked for it (`attention`), the window's size in points, both counters
(`sw_running`, `sw_ms`, `cd_running`, `cd_finished`, `cd_remaining_ms`) and, while the hover buttons
are drawn, their centres (`start_x`, `start_y`, `reset_x`, `reset_y`). The test script clicks the
button where the app says it drew it and then asks whether the stopwatch is running, instead of
deriving a pixel offset from the window rectangle and inferring the click from a frame rate.
A scripted instance (the feature *and* the flag) also starts without taking the focus and without a
tray icon: the keyboard belongs to whoever is using the machine, and Space, R and 1–4 mean something
here, while a second, identical tray icon invites a click that locks the test or opens a menu that
blocks its event loop.
`passthrough on` makes the window click-through for the run, so a real pointer resting on or
crossing the overlay can neither steal the synthetic hover nor add frames to a measurement;
injected events never pass through the OS and are unaffected. The script runs under its own
`CHRONODESK_CONFIG`, like the others, and leaves a running overlay and its config alone.

`stats` reports frames, fps, mean/max time of the app's `ui()` pass, the gap between frames and a
breakdown of why each frame was drawn (`tick`, `input`, `config`, `other`), plus two absolute counters
that survive a reset: layout `rebuilds=` and timer `chimes=`.

`place` re-runs the placement check with a synthetic saved position, as if it had just been read
from the config file, and `placement` reports what was decided and against which monitors — so a
position left behind by a disconnected screen is reproducible without unplugging one:

```sh
pwsh -File scripts/placement-test.ps1
```

That script seeds a scratch `app.ron` with a position outside every screen, points the app at it
with `CHRONODESK_CONFIG`, and checks the overlay against the work area it reads from Windows
itself rather than against the app's own numbers.

```sh
pwsh -File scripts/lifecycle-test.ps1
```

```sh
pwsh -File scripts/install-test.ps1
pwsh -File scripts/welcome-test.ps1
```

The first installs for real, but into a scratch folder and a scratch registry key
(`CHRONODESK_INSTALL_ROOT` and `CHRONODESK_RUN_KEY`, honoured by `instrument` builds only): a copy
named `ChronoDesk-Setup.exe` installs and starts the app, a second run and a second launch leave the
same process running and outlined, an existing `Run` entry is repointed, a different build makes the
old overlay quit and takes its place, and `--uninstall` removes everything but the settings. The
shortcut's target and the registry are read from PowerShell, not through the app, and the script ends
by checking that the real folder, Start menu and registry were never touched. The second covers the
welcome card: up on a launch without a config file and costing no frames while it waits, dismissed by
a click and written to disk at once, gone at the next launch, back on *Quick tips*, and never shown to
someone whose config file predates the field.

```sh
pwsh -File scripts/resume-test.ps1
```

kills a running stopwatch and a running countdown without notice, relaunches, and reads the counters
back through `state`: running, with the time away on them to within the script's own timing; a
paused stopwatch to the millisecond; a countdown whose end passed while nothing was running comes
back finished and silent; a reset leaves nothing in the file. About a minute, no sound.

```sh
pwsh -File scripts/chime-test.ps1
```

runs a one-minute countdown behind a paused stopwatch and reads `chimes=` from `stats`: three chimes,
about seven frames, asleep before and after, and a silent finish with the sound off. It takes three
minutes and plays the sound for real. The lifecycle script checks the single-instance guard (same file spelled differently: steps aside; another file: runs
alongside) and the startup entry, reading the registry from PowerShell rather than through the app.
In an `instrument` build `CHRONODESK_RUN_KEY` redirects the `Run` key to a scratch key, so the real
one is never written; a release build ignores the variable.

Without the feature the flag only prints a notice: there is no listener, no thread and no timers.

## Layout

- `src/main.rs` – window setup (transparent, borderless, always-on-top, hidden from taskbar)
- `src/app.rs` – state, rendering, window sizing, repaint scheduling
- `src/theme.rs` – visual tokens: colours, alphas and proportions
- `src/layout.rs` – derived state: metrics and glyph measurements, rebuilt only when the size or display scale changes
- `src/market.rs` – the exchange catalogue, sessions and the board readout as a pure function of UTC (unit-tested)
- `src/tz.rs` – time zones as a fixed offset plus a daylight-saving rule, computed locally (unit-tested)
- `src/board.rs` – the board's column geometry and painting, stacked or as a strip
- `src/ring.rs` – the studio seconds ring: sixty LED positions along a rounded outline (unit-tested)
- `src/matrix.rs` – the 5×7 dot-matrix face, round LEDs as polygons (unit-tested)
- `src/config.rs` – the config file: atomic saves, field-tolerant loading
- `src/instance.rs` – one overlay per config file: a named mutex keyed on the config path (unit-tested)
- `src/signal.rs` – a later launch's word to the running overlay (show yourself, quit): two named events (unit-tested)
- `src/install.rs` – the app as its own installer: fixed path, shortcut, *Installed apps* entry, upgrade and uninstall (unit-tested against a scratch folder and key)
- `src/registry.rs` – the few `HKEY_CURRENT_USER` calls the two above and *Start with Windows* share
- `src/welcome.rs` – the first-run card's text, checked against the names the menus use
- `src/autostart.rs` – *Start with Windows*: the per-user `Run` key as the only record (unit-tested against a scratch key)
- `src/placement.rs` – validating the saved position against the attached monitors and their work areas
- `src/tray.rs` – tray icon and the shared native menu; events reach egui via a channel + `request_repaint`
- `src/timer.rs` – pure stopwatch/countdown logic, the chime schedule and formatting (unit-tested)
- `src/chime.rs` – the system notification sound, one call
- `src/icon.rs` – procedurally drawn clock icon (no asset files)

## Performance notes

- Repaints only when the display changes: 1 frame/s for the clock (landing just after each second),
  10 frames/s for a running stopwatch, none while paused, and one a minute for the market board with
  seconds hidden. A timer finishing behind a sleeping mode adds about seven frames in total for its three chimes. The seconds ring brings the clock back to 1 frame/s. Idle CPU is below Windows'
  accounting resolution.
- Memory: ~40 MB private working set / ~65 MB private bytes, almost all of it the graphics driver.
  glow was chosen after measuring: wgpu used 130–310 MB private working set.
