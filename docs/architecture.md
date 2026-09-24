# Design notes

This document explains how ChronoDesk is put together and why. The README covers
what the app does and how to use it; the module list is under *Layout* there.

## State

The app keeps three kinds of state, and keeping them apart is what most of the
code structure follows from.

- **Settings** (`Config` in `src/config.rs`) are what the user chose: mode, face,
  colours, the exchanges on the board, window position, and so on. They are the
  only thing written to disk.
- **Runtime state** lives in `ChronoApp` and is never persisted, with one
  exception: a running stopwatch or countdown (see *Counters across restarts*).
  Whether the overlay is locked is deliberately runtime only, so the app always
  starts in a state the user can interact with.
- **Derived state** (`DerivedLayout` in `src/layout.rs`) is computed from the
  settings: font metrics, glyph measurements, padding and the size of the window.
  It is cached and rebuilt only when its key changes, which is the size, the face
  and the display scale. Switching mode, palette or night mode rebuilds nothing.

Every change to the settings goes through one `Command` enum, whether it comes
from the tray menu, the right-click menu, the keyboard or the mouse wheel.
`ChronoApp::apply` is the only place that handles them.

## The config file

`app.ron` lives in `%APPDATA%\chronodesk\data` and can be moved with the
`CHRONODESK_CONFIG` environment variable, which the test scripts use so they
never touch the user's own settings.

Losing settings is worse than failing to save them, so:

- Saving writes `app.ron.tmp`, syncs it and renames it over `app.ron`. A crash
  in the middle leaves either the old file or the new one, never half of one.
- Loading reads the file untyped first and converts each field on its own. An
  invalid value falls back to its default and the rest is kept; the repaired
  file is written back.
- A file that cannot be parsed, is not UTF-8 or UTF-16 text, or has a
  `schema_version` that is newer than the build knows or not a number, is
  copied to `app.ron.bak` and the app starts from defaults. A byte-order mark
  is accepted, since Windows PowerShell 5.1 writes one.
- A file that exists but cannot be read (a scanner holding it at logon, which
  is when autostart reads it) is retried for about half a second. If it still
  fails, the session runs on defaults and never writes over it.
- eframe's old file is recognised by its values being nested RON strings, not
  by its keys: every file this app writes has a `window` too.
- Every field is flat and has a default, so adding a setting does not need a
  schema change. `SCHEMA_VERSION` is still 1.
- The window position is kept twice. `window_px` is in physical pixels and is
  what counts: a point is worth whatever the monitor under the window says,
  so with screens at different scaling a position in points moves on every
  launch. `window` is the same spot in points, only as the hint the window is
  created with. Windows converts that with the scale of whichever monitor the
  window first appears on, so once the app runs, the placement check puts the
  window on the exact pixel, or somewhere visible if that screen is gone.

Settings are written 1.5 seconds after the last change and on exit. Counter
state is the exception and is written immediately (see below). A write that
fails is not retried until the next change or exit, so an unwritable file
cannot keep an idle overlay waking up.

## Drawing and repainting

The overlay is a borderless, transparent eframe window drawn with glow. wgpu
was tried and used three to eight times as much memory for the same window,
almost all of it in the graphics driver.

There are no image or font files. The seven-segment and dot-matrix faces are
polygons generated in `src/digital.rs` and `src/matrix.rs`, and the icon is
drawn in code. The regular typeface is Segoe UI Semilight from the system,
and the board labels use Segoe UI Bold.

The app only draws when something visible changes. Each frame works out when
the display will next change (the next second, the next minute on the board,
a night-mode boundary, a timer chime) and asks for a repaint just after that.
An idle clock draws one frame per second and a paused stopwatch draws none.
Timed wake-ups on Windows can fire up to a timer tick early, so the app aims
25 ms past the boundary rather than spinning until it arrives.

The window is sized to its content. Columns on the market board are sized for
the widest value they can ever hold, so the window does not resize as the
clocks tick.

## Legibility

The overlay can sit over anything, from a black terminal to a white document.
A few rules follow from that:

- Without a backdrop, text is drawn with a dark halo, so white digits stay
  readable on a white background.
- Dimming, whether for captions, closed markets or night mode, never takes text
  below 60% opacity.
- Under a chroma key background nothing translucent is drawn (no halo, no ghost
  segments, no LED sockets), because anything half transparent keys out as a
  green tint.

## Market board

`market::board` is a pure function of the current UTC time and the chosen
exchanges, which makes it easy to test. Time zones are computed in `src/tz.rs`
as a standard offset plus one of three daylight saving rules (US, EU and
Australia), instead of pulling in a time zone database that would add about a
megabyte to a 5.5 MB binary. If a country changes its rules, `tz.rs` changes
with a rebuild, just as a bundled database would.

Sessions include lunch breaks where the exchange has one. Public holidays and
half days are not modelled, so on a holiday the board shows the exchange as open.

## Counters across restarts

An `Instant` means nothing to the next process, so a running counter is saved as
the time it had accumulated plus the wall-clock time it was last started
(`timer::Saved`). On the next start the time spent away is added back. A
countdown that ran out in the meantime comes back as finished, and only chimes
if it finished within the last half minute.

The counter state is written as soon as it changes (start, pause, reset, a new
duration), because a logout or a power cut gives no warning. A counter that is
simply running does not change what is saved, so it costs no writes.

## One overlay, and talking to it

A second copy of the app against the same config file would fight the first
over it. At startup the app takes a named mutex derived from the config path;
if that is taken, it signals the running copy to show itself and exits. Because
the name depends on the path, test instances with their own `CHRONODESK_CONFIG`
can run next to the user's overlay.

The running copy listens on two named events, *show* and *quit*, on a thread of
its own. The installer uses *quit* before replacing the exe. A minimised window
draws no frames and would never see the message, so the listener restores it
before passing the message on.

## Start with Windows

The setting is stored in exactly one place: the per-user `Run` key in the
registry. It is not in `app.ron`, because the entry can be changed behind the
app's back from Settings, from Task Manager (which leaves the entry and marks it
disabled elsewhere) or by moving the exe. The menu shows what the registry says
and re-reads it at most every ten seconds.

## Installing

The app is its own installer. When the exe's name contains `setup`, or it is run
with `--install`, it copies itself to `%LOCALAPPDATA%\ChronoDesk`, creates a
Start menu shortcut and an *Installed apps* entry, and starts the installed copy.
Everything is per user, so nothing asks for administrator rights. A running exe
cannot be overwritten but can be renamed, so an upgrade copies the new file
alongside, renames the old one out of the way and the new one into place.

There is no separate installer project that could drift out of step with the
app, and no extra toolchain.

## Testing

- **Unit tests** cover everything that can be written as a pure function:
  config loading and repair, clock and timer readouts, time zones and market
  sessions, placement, ring geometry, and the installer against a scratch folder
  and registry key.
- **The instrument build** (`--features instrument`, started with
  `--instrument`) adds a loopback command channel and frame statistics, so the
  scripts in `scripts/` can drive the real app without the real mouse and check
  how many frames each state costs. None of this is compiled into a release
  build.
- **Screenshots** for the README are rendered off screen by `src/shots.rs`, with
  the clock pinned to a fixed time.
