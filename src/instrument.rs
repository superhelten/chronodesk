//! Test instrumentation: a loopback command channel and frame telemetry.
//!
//! Compiled only with `--features instrument`, and even then it stays asleep
//! until the app is started with `--instrument`. A normal release build has no
//! listener and no timers at all.
//!
//! It exists because driving the overlay with real mouse input is unreliable:
//! the pointer belongs to whoever is using the machine, and the window is
//! transparent, so a screenshot of it also captures whatever moves behind it.
//! Injecting events and reading times from inside the process removes both.

use std::time::{Duration, Instant};

/// Why a frame was drawn. Classified by the app, not guessed from egui's
/// repaint causes, so the meaning stays stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    /// The scheduled wake-up for the next display change.
    Tick,
    /// Pointer, keyboard or scroll input, real or injected.
    Input,
    /// A command changed the configuration or the timer state.
    Config,
    Other,
}

/// Rolling frame statistics, reset on request.
#[cfg_attr(not(feature = "instrument"), allow(dead_code, reason = "only the telemetry tests use it"))]
#[derive(Debug, Default, Clone)]
pub struct Telemetry {
    pub frames: u64,
    pub total: Duration,
    pub max: Duration,
    pub last: Duration,
    pub by_cause: [u64; 4],
    /// Gap between consecutive frames, to tell a clean tick from a double
    /// repaint on the same boundary.
    pub last_frame: Option<Instant>,
    pub min_gap: Option<Duration>,
    pub short_gaps: u64,
}

#[cfg_attr(not(feature = "instrument"), allow(dead_code, reason = "only the telemetry tests use it"))]
impl Telemetry {
    pub fn record_at(&mut self, at: Instant, elapsed: Duration, cause: Cause) {
        if let Some(previous) = self.last_frame {
            let gap = at.saturating_duration_since(previous);
            self.min_gap = Some(self.min_gap.map_or(gap, |m: Duration| m.min(gap)));
            if gap < Duration::from_millis(200) {
                self.short_gaps += 1;
            }
        }
        self.last_frame = Some(at);
        self.frames += 1;
        self.total += elapsed;
        self.max = self.max.max(elapsed);
        self.last = elapsed;
        self.by_cause[cause as usize] += 1;
    }

    pub fn report(&self, window: Duration) -> String {
        let mean = self.total.checked_div(self.frames.max(1) as u32).unwrap_or_default();
        let fps = self.frames as f64 / window.as_secs_f64().max(0.001);
        format!(
            "frames={} window_s={:.2} fps={:.2} mean_ms={:.3} max_ms={:.3} last_ms={:.3} \
             tick={} input={} config={} other={} min_gap_ms={:.1} short_gaps={}",
            self.frames,
            window.as_secs_f64(),
            fps,
            mean.as_secs_f64() * 1000.0,
            self.max.as_secs_f64() * 1000.0,
            self.last.as_secs_f64() * 1000.0,
            self.by_cause[Cause::Tick as usize],
            self.by_cause[Cause::Input as usize],
            self.by_cause[Cause::Config as usize],
            self.by_cause[Cause::Other as usize],
            self.min_gap.map_or(f64::NAN, |g| g.as_secs_f64() * 1000.0),
            self.short_gaps,
        )
    }
}

#[cfg(feature = "instrument")]
mod imp {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use eframe::egui::{self, Event, Pos2, RawInput, pos2};

    use super::{Cause, Telemetry};
    use crate::tray::{self, Command};

    /// Where the port number is published for test scripts.
    pub fn port_file() -> std::path::PathBuf {
        std::env::temp_dir().join("chronodesk-instrument.port")
    }

    #[derive(Default)]
    struct Shared {
        commands: Vec<Command>,
        /// Injected pointer position, in points, or `None` for "no pointer".
        hover: Option<Pos2>,
        /// Set when `hover` changed and the event still has to be delivered.
        hover_dirty: bool,
        /// Queued synthetic clicks: press one frame, release the next.
        click: Option<ClickPhase>,
        telemetry: Telemetry,
        window_start: Option<Instant>,
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum ClickPhase {
        Press(Pos2),
        Release(Pos2),
    }

    pub struct Instrument {
        shared: Option<Arc<Mutex<Shared>>>,
    }

    impl Instrument {
        /// Starts the listener when the app was run with `--instrument`.
        pub fn start(ctx: &egui::Context) -> Self {
            if !std::env::args().any(|arg| arg == "--instrument") {
                return Self { shared: None };
            }
            let shared = Arc::new(Mutex::new(Shared {
                window_start: Some(Instant::now()),
                ..Shared::default()
            }));

            // Loopback only: this channel can drive the app, so it must never
            // be reachable from outside the machine.
            let listener = match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))) {
                Ok(listener) => listener,
                Err(err) => {
                    eprintln!("ChronoDesk: instrument listener failed: {err}");
                    return Self { shared: None };
                }
            };
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
            if let Err(err) = std::fs::write(port_file(), port.to_string()) {
                eprintln!("ChronoDesk: could not publish instrument port: {err}");
            }
            eprintln!("ChronoDesk: instrument channel on 127.0.0.1:{port}");

            let thread_shared = Arc::clone(&shared);
            let thread_ctx = ctx.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    serve(stream, &thread_shared, &thread_ctx);
                }
            });

            Self { shared: Some(shared) }
        }

        /// Commands sent over the channel, to be applied like any other.
        pub fn commands(&self) -> Vec<Command> {
            let Some(shared) = &self.shared else { return Vec::new() };
            std::mem::take(&mut shared.lock().expect("instrument lock").commands)
        }

        /// Injects the synthetic pointer into the raw input, so hit-testing,
        /// hovering and clicking behave exactly as with a real mouse.
        pub fn inject(&self, raw: &mut RawInput) {
            let Some(shared) = &self.shared else { return };
            let mut shared = shared.lock().expect("instrument lock");
            // Only on change: egui keeps the pointer position between frames,
            // and an event every frame would make every frame look like input.
            if shared.hover_dirty {
                shared.hover_dirty = false;
                match shared.hover {
                    Some(pos) => raw.events.push(Event::PointerMoved(pos)),
                    None => raw.events.push(Event::PointerGone),
                }
            }
            match shared.click.take() {
                Some(ClickPhase::Press(pos)) => {
                    raw.events.push(Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                    shared.click = Some(ClickPhase::Release(pos));
                }
                Some(ClickPhase::Release(pos)) => raw.events.push(Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }),
                None => {}
            }
        }

        /// True while a synthetic click is still being delivered, so the app
        /// keeps drawing frames until it lands.
        pub fn click_pending(&self) -> bool {
            self.shared
                .as_ref()
                .is_some_and(|s| s.lock().expect("instrument lock").click.is_some())
        }

        pub fn record_frame(&self, elapsed: Duration, cause: Cause) {
            let Some(shared) = &self.shared else { return };
            let mut shared = shared.lock().expect("instrument lock");
            shared.telemetry.record_at(Instant::now(), elapsed, cause);
        }
    }

    fn serve(stream: TcpStream, shared: &Arc<Mutex<Shared>>, ctx: &egui::Context) {
        let Ok(write_half) = stream.try_clone() else { return };
        let mut out = write_half;
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let reply = handle(line.trim(), shared, ctx);
            if writeln!(out, "{reply}").is_err() {
                return;
            }
            let _ = out.flush();
            ctx.request_repaint();
        }
    }

    fn handle(line: &str, shared: &Arc<Mutex<Shared>>, ctx: &egui::Context) -> String {
        let mut parts = line.split_whitespace();
        let Some(verb) = parts.next() else { return "err empty".to_owned() };
        let mut shared = shared.lock().expect("instrument lock");
        match verb {
            // Same ids the menus use, so there is one command vocabulary.
            "cmd" => match parts.next().and_then(tray::parse_command) {
                Some(cmd) => {
                    shared.commands.push(cmd);
                    "ok".to_owned()
                }
                None => "err unknown command".to_owned(),
            },
            "hover" => match (parts.next(), parts.next()) {
                (Some("off"), _) => {
                    shared.hover = None;
                    shared.hover_dirty = true;
                    "ok".to_owned()
                }
                (Some(x), Some(y)) => match (x.parse::<f32>(), y.parse::<f32>()) {
                    (Ok(x), Ok(y)) => {
                        shared.hover = Some(pos2(x, y));
                        shared.hover_dirty = true;
                        "ok".to_owned()
                    }
                    _ => "err coordinates".to_owned(),
                },
                _ => "err usage: hover <x> <y> | hover off".to_owned(),
            },
            "click" => match (parts.next(), parts.next()) {
                (Some(x), Some(y)) => match (x.parse::<f32>(), y.parse::<f32>()) {
                    (Ok(x), Ok(y)) => {
                        let pos = pos2(x, y);
                        shared.hover = Some(pos);
                        shared.hover_dirty = true;
                        shared.click = Some(ClickPhase::Press(pos));
                        "ok".to_owned()
                    }
                    _ => "err coordinates".to_owned(),
                },
                _ => "err usage: click <x> <y>".to_owned(),
            },
            "stats" => {
                let window = shared.window_start.map(|t| t.elapsed()).unwrap_or_default();
                if parts.next() == Some("reset") {
                    shared.telemetry = Telemetry::default();
                    shared.window_start = Some(Instant::now());
                    "ok reset".to_owned()
                } else {
                    shared.telemetry.report(window)
                }
            }
            "quit" => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                "ok".to_owned()
            }
            other => format!("err unknown verb '{other}'"),
        }
    }
}

#[cfg(not(feature = "instrument"))]
mod imp {
    use std::time::Duration;

    use eframe::egui::{self, RawInput};

    use super::Cause;
    use crate::tray::Command;

    /// Stub used by ordinary builds: no listener, no timers, no cost.
    pub struct Instrument;

    impl Instrument {
        pub fn start(_ctx: &egui::Context) -> Self {
            if std::env::args().any(|arg| arg == "--instrument") {
                eprintln!("ChronoDesk: built without the 'instrument' feature; flag ignored");
            }
            Self
        }

        pub fn commands(&self) -> Vec<Command> {
            Vec::new()
        }

        pub fn inject(&self, _raw: &mut RawInput) {}

        pub fn click_pending(&self) -> bool {
            false
        }

        pub fn record_frame(&self, _elapsed: Duration, _cause: Cause) {}
    }
}

pub use imp::Instrument;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_counts_frames_and_causes() {
        let mut t = Telemetry::default();
        let t0 = std::time::Instant::now();
        t.record_at(t0, Duration::from_micros(500), Cause::Tick);
        t.record_at(t0 + Duration::from_secs(1), Duration::from_micros(1500), Cause::Input);
        t.record_at(t0 + Duration::from_secs(2), Duration::from_micros(1000), Cause::Tick);

        let report = t.report(Duration::from_secs(3));
        assert!(report.contains("frames=3"), "{report}");
        assert!(report.contains("tick=2"), "{report}");
        assert!(report.contains("input=1"), "{report}");
        assert!(report.contains("fps=1.00"), "{report}");
        assert!(report.contains("mean_ms=1.000"), "{report}");
        assert!(report.contains("max_ms=1.500"), "{report}");
    }

    #[test]
    fn empty_telemetry_reports_zero_without_dividing_by_zero() {
        let report = Telemetry::default().report(Duration::from_secs(1));
        assert!(report.contains("frames=0"), "{report}");
        assert!(report.contains("mean_ms=0.000"), "{report}");
    }
}
