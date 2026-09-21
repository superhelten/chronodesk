//! README screenshots, rendered off screen: no window, no desktop, no clock
//! but a pinned one, so every run draws the same pixels.
//!
//! ```sh
//! cargo test shots -- --ignored        # target/shots/*.png, transparent, 2x
//! python scripts/compose-shots.py      # docs/screenshots/, on a backdrop
//! ```
//!
//! Each scene is drawn twice: once in a roomy harness to learn the size the
//! overlay asks for, then at exactly that size, as the real window would be.
//! The board is pinned to an instant in UTC and the clock faces to a local
//! time of day, so the pictures are the same on any machine and say nothing
//! about the time zone of the one that drew them.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, TimeZone as _, Utc};
use eframe::egui::Vec2;
use egui_kittest::Harness;

use crate::app::{ChronoApp, Mode, Size};
use crate::board::Layout;
use crate::config::Config;
use crate::layout::Font;
use crate::market::Labels;
use crate::theme::Palette;
use crate::timer::Saved;

const SCALE: f32 = 2.0;

/// For the board: a Tuesday at 13:45:37 UTC, when New York has just opened,
/// London and Oslo are trading, Oslo's close is the next thing to happen, and
/// Asia is shut.
fn board_time() -> DateTime<Local> {
    Utc.with_ymd_and_hms(2026, 9, 22, 13, 45, 37).unwrap().with_timezone(&Local)
}

/// For the clock faces: 10:09 on the same Tuesday, in whatever zone renders it.
fn clock_time(second: u32) -> DateTime<Local> {
    let naive = NaiveDate::from_ymd_opt(2026, 9, 22).unwrap().and_hms_opt(10, 9, second).unwrap();
    Local.from_local_datetime(&naive).earliest().unwrap()
}

fn base() -> Config {
    Config { size: Size::Large, first_run: false, timer_sound: false, ..Config::returning() }
}

fn render(settings: &Config, at: DateTime<Local>) -> image::RgbaImage {
    let size = {
        let mut probe = Harness::builder()
            .with_size(Vec2::new(1600.0, 800.0))
            .with_pixels_per_point(SCALE)
            .build_eframe(|cc| ChronoApp::pinned(&cc.egui_ctx, settings.clone(), at));
        probe.run_steps(3);
        probe.state().requested_size().expect("the overlay asks for a size on its first frame")
    };
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(SCALE)
        .wgpu()
        .build_eframe(|cc| ChronoApp::pinned(&cc.egui_ctx, settings.clone(), at));
    harness.run_steps(3);
    harness.render().expect("rendered off screen")
}

fn out_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("shots");
    std::fs::create_dir_all(&dir).expect("target/shots");
    dir
}

fn save(name: &str, settings: &Config, at: DateTime<Local>) {
    let path = out_dir().join(format!("{name}.png"));
    render(settings, at).save(&path).expect("png written");
    println!("{}", path.display());
}

#[test]
#[ignore = "writes README screenshots; run with --ignored"]
fn shots() {
    let board = Config { mode: Mode::Market, font: Font::Matrix, palette: Palette::Green, ..base() };
    save("board", &board, board_time());
    save(
        "board-strip",
        &Config { board_layout: Layout::Horizontal, board_labels: Labels::Code, ..board.clone() },
        board_time(),
    );

    let at = clock_time(37);
    save(
        "clock-ring",
        &Config { font: Font::Matrix, palette: Palette::Studio, seconds_ring: true, show_date: true, ..base() },
        at,
    );
    save("clock-segment", &Config { font: Font::Digital, palette: Palette::Red, show_date: true, ..base() }, at);
    save("clock-sans", &Config { show_date: true, ..base() }, at);
    save(
        "timer",
        &Config {
            mode: Mode::Timer,
            font: Font::Digital,
            palette: Palette::Studio,
            seconds_ring: true,
            countdown: Some(Saved { accumulated_ms: (25 * 60 - 4 * 60 - 59) * 1000, started_at_ms: None }),
            ..base()
        },
        at,
    );
    save(
        "stopwatch",
        &Config {
            mode: Mode::Stopwatch,
            font: Font::Matrix,
            palette: Palette::Yellow,
            stopwatch: Some(Saved { accumulated_ms: 754_320, started_at_ms: None }),
            ..base()
        },
        at,
    );
}

/// Sixty frames of the seconds ring filling up, for the README's GIF.
#[test]
#[ignore = "writes README screenshots; run with --ignored"]
fn shots_ring_frames() {
    let settings = Config { font: Font::Matrix, palette: Palette::Studio, seconds_ring: true, ..base() };
    for second in 0..60 {
        save(&format!("ring-{second:02}"), &settings, clock_time(second));
    }
}
