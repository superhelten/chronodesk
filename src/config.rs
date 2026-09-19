//! Persisted configuration.
//!
//! Two properties matter more than the format itself:
//!
//! * **Writes are atomic.** A crash or power loss during a save must never
//!   leave a truncated file behind, so we write a sibling temp file and rename
//!   it over the target (`rename` replaces atomically on both Windows and unix).
//! * **Reads are tolerant per field.** One bad value must not discard every
//!   other setting, so the file is parsed into untyped values first and each
//!   field is then converted on its own. Whatever fails falls back to its
//!   default and is reported as a warning.
//!
//! Only a wholly unparseable file (or one written by a newer schema) is
//! rejected, and it is copied to `app.ron.bak` before anything overwrites it.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use ron::Value;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::app::{Mode, Size};
use crate::board::Layout;
use crate::clock::ClockFormat;
use crate::layout::Font;
use crate::market::{Labels, Market};
use crate::night::{NightMode, TimeOfDay};
use crate::theme::{self, Palette};
use crate::timer::Saved;

/// Bump when the meaning of a field changes; add a migration step for it.
/// Schema 0 is eframe's own persistence file, handled by [`migrate_from_eframe`].
pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_TIMER_MINUTES: u64 = 24 * 60;
/// Night mode may fade the readout to this alpha and no further: below it the
/// halo no longer buys enough contrast over a bright background.
pub const MIN_NIGHT_DIM: f32 = theme::TEXT_ALPHA_FLOOR;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowPos {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub mode: Mode,
    pub timer_minutes: u64,
    pub size: Size,
    pub font: Font,
    pub backdrop: bool,
    pub chroma: bool,
    pub show_seconds: bool,
    pub always_on_top: bool,
    /// Dark outline around the text, for legibility without a backdrop.
    pub text_outline: bool,
    pub clock_format: ClockFormat,
    /// Date line under the clock.
    pub show_date: bool,
    pub palette: Palette,
    pub night: NightMode,
    /// Nightly window for `night: "auto"`, as `"HH:MM"`; `from` later than
    /// `to` wraps past midnight.
    pub night_from: TimeOfDay,
    pub night_to: TimeOfDay,
    /// Factor the readout is faded by at night, `MIN_NIGHT_DIM..=1.0`.
    pub night_dim: f32,
    /// The market board's rows, top to bottom, by id (`"new-york"`, …).
    /// Never empty: a board with no rows is a mode that shows nothing.
    pub markets: Vec<Market>,
    /// Rows stacked, or one line; city names or exchange codes.
    pub board_layout: Layout,
    pub board_labels: Labels,
    /// Studio look: sixty LEDs around the readout, lit as the seconds pass.
    pub seconds_ring: bool,
    /// Chime when the countdown reaches zero (the system's notification sound).
    pub timer_sound: bool,
    /// A stopwatch or countdown that was under way, so it survives a restart
    /// or a logout. Written by the app, not meant to be edited; `None` is idle.
    pub stopwatch: Option<Saved>,
    pub countdown: Option<Saved>,
    /// Window position in points. `None` means "let the OS place it".
    pub window: Option<WindowPos>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            mode: Mode::Clock,
            timer_minutes: 25,
            size: Size::Medium,
            font: Font::Sans,
            backdrop: false,
            chroma: false,
            show_seconds: true,
            always_on_top: true,
            text_outline: true,
            clock_format: ClockFormat::H24,
            show_date: true,
            palette: Palette::Default,
            night: NightMode::Off,
            night_from: TimeOfDay::new(22, 0).expect("valid"),
            night_to: TimeOfDay::new(7, 0).expect("valid"),
            night_dim: 0.7,
            markets: Market::DEFAULT.to_vec(),
            board_layout: Layout::Vertical,
            board_labels: Labels::City,
            seconds_ring: false,
            timer_sound: true,
            stopwatch: None,
            countdown: None,
            window: None,
        }
    }
}

impl Config {
    /// Clamps values that later code relies on being in range.
    fn validated(mut self, warnings: &mut Vec<String>) -> Self {
        let minutes = self.timer_minutes.clamp(1, MAX_TIMER_MINUTES);
        if minutes != self.timer_minutes {
            warnings.push(format!("'timer_minutes' {} out of range; clamped to {minutes}", self.timer_minutes));
            self.timer_minutes = minutes;
        }
        if let Some(w) = self.window
            && !(w.x.is_finite() && w.y.is_finite())
        {
            warnings.push("'window' has non-finite coordinates; ignored".to_owned());
            self.window = None;
        }
        let dim = if self.night_dim.is_finite() { self.night_dim.clamp(MIN_NIGHT_DIM, 1.0) } else { 0.7 };
        if dim != self.night_dim {
            warnings.push(format!("'night_dim' {} out of range; clamped to {dim}", self.night_dim));
            self.night_dim = dim;
        }
        let mut seen = Vec::with_capacity(self.markets.len());
        for market in std::mem::take(&mut self.markets) {
            if seen.contains(&market) {
                warnings.push(format!("'markets' lists '{}' twice; keeping the first", market.id()));
            } else {
                seen.push(market);
            }
        }
        if seen.is_empty() {
            warnings.push("'markets' is empty; using the default board".to_owned());
            seen = Market::DEFAULT.to_vec();
        }
        self.markets = seen;
        self.schema_version = SCHEMA_VERSION;
        self
    }
}

/// Result of [`load`]: always a usable config, plus what had to be repaired.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub warnings: Vec<String>,
    /// Set when the previous file was copied aside instead of being trusted.
    pub quarantined: Option<PathBuf>,
    /// True when the file was written by an older schema and should be rewritten.
    pub migrated: bool,
}

/// Names a config file outright, bypassing the platform location. Used by the
/// test harness so a run can be driven against a scratch file instead of the
/// one the user is actually living with.
pub const CONFIG_ENV: &str = "CHRONODESK_CONFIG";

/// `%APPDATA%\chronodesk\data\app.ron` and the equivalent elsewhere — the same
/// location eframe's own persistence used, so existing files are picked up.
pub fn config_path() -> Option<PathBuf> {
    config_path_from(std::env::var_os(CONFIG_ENV), platform_dir())
}

fn platform_dir() -> Option<PathBuf> {
    Some(if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
    })
}

fn config_path_from(overridden: Option<std::ffi::OsString>, base: Option<PathBuf>) -> Option<PathBuf> {
    match overridden {
        Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => Some(base?.join("chronodesk").join("data").join("app.ron")),
    }
}

pub fn load(path: &Path) -> Loaded {
    let mut warnings = Vec::new();

    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Loaded { config: Config::default(), warnings, quarantined: None, migrated: false };
        }
        Err(err) => {
            warnings.push(format!("could not read {}: {err}", path.display()));
            return Loaded { config: Config::default(), warnings, quarantined: None, migrated: false };
        }
    };

    let mut fields = match parse_fields(&text) {
        Some(fields) => fields,
        None => {
            warnings.push("file is not valid RON; starting from defaults".to_owned());
            let quarantined = quarantine(path, &mut warnings);
            return Loaded { config: Config::default(), warnings, quarantined, migrated: false };
        }
    };

    // A file from a newer build may give familiar names new meanings, so keep a
    // copy rather than reinterpreting it.
    let version = match fields.get("schema_version").cloned().map(Value::into_rust::<u32>) {
        Some(Ok(v)) => v,
        Some(Err(_)) => 0,
        // Files written by eframe's persistence have no version field.
        None => 0,
    };
    if version > SCHEMA_VERSION {
        warnings.push(format!("file uses schema {version}, this build knows {SCHEMA_VERSION}; starting from defaults"));
        let quarantined = quarantine(path, &mut warnings);
        return Loaded { config: Config::default(), warnings, quarantined, migrated: false };
    }
    if version < SCHEMA_VERSION && (fields.contains_key("app") || fields.contains_key("window")) {
        return Loaded {
            config: migrate_from_eframe(&fields, &mut warnings),
            warnings,
            quarantined: None,
            migrated: true,
        };
    }

    let mut config = Config::default();
    field(&mut fields, "mode", &mut config.mode, &mut warnings);
    field(&mut fields, "timer_minutes", &mut config.timer_minutes, &mut warnings);
    field(&mut fields, "size", &mut config.size, &mut warnings);
    field(&mut fields, "font", &mut config.font, &mut warnings);
    field(&mut fields, "backdrop", &mut config.backdrop, &mut warnings);
    field(&mut fields, "chroma", &mut config.chroma, &mut warnings);
    field(&mut fields, "show_seconds", &mut config.show_seconds, &mut warnings);
    field(&mut fields, "always_on_top", &mut config.always_on_top, &mut warnings);
    field(&mut fields, "text_outline", &mut config.text_outline, &mut warnings);
    field(&mut fields, "clock_format", &mut config.clock_format, &mut warnings);
    field(&mut fields, "show_date", &mut config.show_date, &mut warnings);
    field(&mut fields, "palette", &mut config.palette, &mut warnings);
    field(&mut fields, "night", &mut config.night, &mut warnings);
    field(&mut fields, "night_from", &mut config.night_from, &mut warnings);
    field(&mut fields, "night_to", &mut config.night_to, &mut warnings);
    field(&mut fields, "night_dim", &mut config.night_dim, &mut warnings);
    markets_field(&mut fields, &mut config.markets, &mut warnings);
    field(&mut fields, "board_layout", &mut config.board_layout, &mut warnings);
    field(&mut fields, "board_labels", &mut config.board_labels, &mut warnings);
    field(&mut fields, "seconds_ring", &mut config.seconds_ring, &mut warnings);
    field(&mut fields, "timer_sound", &mut config.timer_sound, &mut warnings);
    field(&mut fields, "stopwatch", &mut config.stopwatch, &mut warnings);
    field(&mut fields, "countdown", &mut config.countdown, &mut warnings);
    field(&mut fields, "window", &mut config.window, &mut warnings);
    fields.remove("schema_version");
    for key in fields.keys() {
        warnings.push(format!("unknown field '{key}' ignored"));
    }

    Loaded { config: config.validated(&mut warnings), warnings, quarantined: None, migrated: false }
}

/// Writes via a temp file so the target is either the old or the new content,
/// never a half-written mix.
pub fn save(path: &Path, config: &Config) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = ron::ser::to_string_pretty(config, ron::ser::PrettyConfig::default())
        .map_err(io::Error::other)?;

    let tmp = path.with_extension("ron.tmp");
    let write = || -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
        // Flush to disk before the rename, so a crash can't leave the new name
        // pointing at content that never landed.
        file.sync_all()
    };
    if let Err(err) = write() {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

/// Parses into untyped values so each field can be converted separately.
/// Accepts both a RON struct body and a map, which is what eframe wrote.
fn parse_fields(text: &str) -> Option<HashMap<String, Value>> {
    let Ok(Value::Map(map)) = ron::from_str::<Value>(text) else {
        return None;
    };
    Some(
        map.into_iter()
            .filter_map(|(k, v)| match k {
                Value::String(k) => Some((k, v)),
                _ => None,
            })
            .collect(),
    )
}

/// Converts one field, leaving `out` at its default if the value is unusable.
fn field<T: DeserializeOwned>(
    fields: &mut HashMap<String, Value>,
    key: &str,
    out: &mut T,
    warnings: &mut Vec<String>,
) {
    let Some(value) = fields.remove(key) else {
        return;
    };
    match value.into_rust::<T>() {
        Ok(value) => *out = value,
        Err(err) => warnings.push(format!("'{key}' is invalid ({err}); using default")),
    }
}

/// The market list is tolerant per *element*: one misspelt id drops that row
/// with a warning and keeps the others, the way one bad field keeps the rest
/// of the file. Only a value that is not a list of strings at all falls back
/// wholesale.
fn markets_field(fields: &mut HashMap<String, Value>, out: &mut Vec<Market>, warnings: &mut Vec<String>) {
    let Some(value) = fields.remove("markets") else {
        return;
    };
    let ids = match value.into_rust::<Vec<String>>() {
        Ok(ids) => ids,
        Err(err) => {
            warnings.push(format!("'markets' is invalid ({err}); using default"));
            return;
        }
    };
    let mut markets = Vec::with_capacity(ids.len());
    for id in ids {
        match Market::from_id(&id) {
            Some(market) => markets.push(market),
            None => warnings.push(format!("'markets' names an unknown exchange '{id}'; skipped")),
        }
    }
    *out = markets;
}

fn quarantine(path: &Path, warnings: &mut Vec<String>) -> Option<PathBuf> {
    let backup = path.with_extension("ron.bak");
    match fs::copy(path, &backup) {
        Ok(_) => {
            warnings.push(format!("previous file kept as {}", backup.display()));
            Some(backup)
        }
        Err(err) => {
            warnings.push(format!("could not keep a copy of the previous file: {err}"));
            None
        }
    }
}

/// Schema 0 is eframe's own persistence file: a map holding our settings and
/// the window geometry as nested RON strings, with enums written as variant
/// names. Those names are parsed here by dedicated types, because the current
/// format stores ids instead.
fn migrate_from_eframe(fields: &HashMap<String, Value>, warnings: &mut Vec<String>) -> Config {
    #[derive(Deserialize)]
    #[serde(default)]
    struct LegacySettings {
        mode: LegacyMode,
        timer_minutes: u64,
        size: LegacySize,
        backdrop: bool,
        chroma: bool,
        show_seconds: bool,
        always_on_top: bool,
    }
    impl Default for LegacySettings {
        fn default() -> Self {
            let d = Config::default();
            Self {
                mode: LegacyMode::Clock,
                timer_minutes: d.timer_minutes,
                size: LegacySize::Medium,
                backdrop: d.backdrop,
                chroma: d.chroma,
                show_seconds: d.show_seconds,
                always_on_top: d.always_on_top,
            }
        }
    }

    #[derive(Deserialize, Clone, Copy)]
    enum LegacyMode {
        Clock,
        Stopwatch,
        Timer,
    }
    #[derive(Deserialize, Clone, Copy)]
    enum LegacySize {
        Small,
        Medium,
        Large,
    }
    #[derive(Deserialize)]
    struct LegacyWindow {
        #[serde(default)]
        outer_position_pixels: Option<WindowPos>,
    }

    let mut config = Config::default();

    if let Some(Value::String(app)) = fields.get("app") {
        match ron::from_str::<LegacySettings>(app) {
            Ok(old) => {
                config.mode = match old.mode {
                    LegacyMode::Clock => Mode::Clock,
                    LegacyMode::Stopwatch => Mode::Stopwatch,
                    LegacyMode::Timer => Mode::Timer,
                };
                config.size = match old.size {
                    LegacySize::Small => Size::Small,
                    LegacySize::Medium => Size::Medium,
                    LegacySize::Large => Size::Large,
                };
                config.timer_minutes = old.timer_minutes;
                config.backdrop = old.backdrop;
                config.chroma = old.chroma;
                config.show_seconds = old.show_seconds;
                config.always_on_top = old.always_on_top;
            }
            Err(err) => warnings.push(format!("could not migrate the previous settings ({err}); using defaults")),
        }
    }

    // eframe stored the position in pixels while we store points. They differ
    // only on a scaled display, and then by one drag of the overlay.
    if let Some(Value::String(window)) = fields.get("window") {
        match ron::from_str::<LegacyWindow>(window) {
            Ok(LegacyWindow { outer_position_pixels: pos }) => config.window = pos,
            Err(err) => warnings.push(format!("could not migrate the previous window position ({err})")),
        }
    }

    config.validated(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique scratch directory per test, removed on drop.
    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("chronodesk_test_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> PathBuf {
            self.0.join("app.ron")
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, text: &str) {
        fs::write(path, text).unwrap();
    }

    #[test]
    fn round_trip_replaces_content_and_leaves_no_temp_file() {
        let dir = Dir::new("roundtrip");
        let path = dir.file();
        let mut config = Config { timer_minutes: 45, mode: Mode::Timer, ..Default::default() };
        save(&path, &config).unwrap();
        config.window = Some(WindowPos { x: 12.5, y: 34.0 });
        save(&path, &config).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config, config);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert!(!loaded.migrated);
        assert!(!path.with_extension("ron.tmp").exists(), "temp file left behind");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let dir = Dir::new("missing");
        let path = dir.file();
        write(&path, "(schema_version:1,mode:\"stopwatch\")");

        let loaded = load(&path);
        assert_eq!(loaded.config.mode, Mode::Stopwatch);
        assert_eq!(loaded.config.timer_minutes, Config::default().timer_minutes);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn invalid_enum_keeps_every_other_field() {
        let dir = Dir::new("invalid_enum");
        let path = dir.file();
        write(
            &path,
            "(schema_version:1,mode:\"timer\",timer_minutes:15,size:\"enormous\",backdrop:true,window:Some((x:300.0,y:120.0)))",
        );

        let loaded = load(&path);
        assert_eq!(loaded.config.size, Config::default().size, "bad field should use its default");
        assert_eq!(loaded.config.window, Some(WindowPos { x: 300.0, y: 120.0 }), "window must survive");
        assert_eq!(loaded.config.mode, Mode::Timer);
        assert_eq!(loaded.config.timer_minutes, 15);
        assert!(loaded.config.backdrop);
        assert!(loaded.quarantined.is_none(), "a single bad field is not a corrupt file");
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("size"), "{:?}", loaded.warnings);
    }

    #[test]
    fn wrong_type_keeps_every_other_field() {
        let dir = Dir::new("wrong_type");
        let path = dir.file();
        write(
            &path,
            r#"(schema_version:1,timer_minutes:"twenty",show_seconds:false,window:Some((x:5.0,y:6.0)))"#,
        );

        let loaded = load(&path);
        assert_eq!(loaded.config.timer_minutes, Config::default().timer_minutes);
        assert!(!loaded.config.show_seconds);
        assert_eq!(loaded.config.window, Some(WindowPos { x: 5.0, y: 6.0 }));
        assert!(loaded.warnings[0].contains("timer_minutes"), "{:?}", loaded.warnings);
    }

    /// Files written before `text_outline` existed must keep working, and get
    /// the outline enabled.
    #[test]
    fn config_without_outline_field_defaults_to_enabled() {
        let dir = Dir::new("outline_default");
        let path = dir.file();
        write(
            &path,
            "(schema_version:1,mode:\"clock\",timer_minutes:25,size:\"medium\",backdrop:false,chroma:false,show_seconds:true,always_on_top:true,window:Some((x:10.0,y:20.0)))",
        );

        let loaded = load(&path);
        assert!(loaded.config.text_outline);
        assert_eq!(loaded.config.window, Some(WindowPos { x: 10.0, y: 20.0 }));
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn outline_field_round_trips_and_tolerates_a_bad_value() {
        let dir = Dir::new("outline_bad");
        let path = dir.file();
        save(&path, &Config { text_outline: false, ..Default::default() }).unwrap();
        assert!(!load(&path).config.text_outline, "explicit false must survive a round trip");

        write(&path, "(schema_version:1,text_outline:\"yes please\",chroma:true)");
        let loaded = load(&path);
        assert!(loaded.config.text_outline, "a bad value falls back to the default");
        assert!(loaded.config.chroma, "other fields survive");
        assert!(loaded.warnings[0].contains("text_outline"), "{:?}", loaded.warnings);
    }

    /// Files written before the appearance fields existed keep working and
    /// get the previous behaviour: 24-hour clock with the date shown.
    #[test]
    fn config_without_appearance_fields_keeps_the_old_look() {
        let dir = Dir::new("appearance_default");
        let path = dir.file();
        write(
            &path,
            "(schema_version:1,mode:\"clock\",timer_minutes:25,size:\"medium\",backdrop:false,chroma:false,show_seconds:true,always_on_top:true,text_outline:true,window:Some((x:10.0,y:20.0)))",
        );

        let loaded = load(&path);
        assert_eq!(loaded.config.clock_format, ClockFormat::H24);
        assert!(loaded.config.show_date);
        assert_eq!(loaded.config.window, Some(WindowPos { x: 10.0, y: 20.0 }));
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.config.schema_version, 1, "additive fields must not bump the schema");
    }

    #[test]
    fn clock_format_and_date_round_trip_as_ids() {
        let dir = Dir::new("clock_format");
        let path = dir.file();
        save(&path, &Config { clock_format: ClockFormat::H12, show_date: false, ..Default::default() }).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("clock_format: \"12h\""), "{text}");
        assert!(text.contains("show_date: false"), "{text}");

        let loaded = load(&path);
        assert_eq!(loaded.config.clock_format, ClockFormat::H12);
        assert!(!loaded.config.show_date);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn bad_clock_format_falls_back_and_keeps_the_rest() {
        let dir = Dir::new("clock_format_bad");
        let path = dir.file();
        write(&path, "(schema_version:1,clock_format:\"13h\",show_date:false)");

        let loaded = load(&path);
        assert_eq!(loaded.config.clock_format, ClockFormat::H24);
        assert!(!loaded.config.show_date, "other fields survive");
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("clock_format"), "{:?}", loaded.warnings);
    }

    #[test]
    fn config_without_font_field_keeps_the_typeface() {
        let dir = Dir::new("font_default");
        let path = dir.file();
        write(&path, "(schema_version:1,size:\"large\")");
        let loaded = load(&path);
        assert_eq!(loaded.config.font, Font::Sans);
        assert_eq!(loaded.config.size, Size::Large);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn font_round_trips_as_an_id_and_tolerates_a_bad_value() {
        let dir = Dir::new("font_roundtrip");
        let path = dir.file();
        save(&path, &Config { font: Font::Digital, ..Default::default() }).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("font: \"digital\""));
        assert_eq!(load(&path).config.font, Font::Digital);

        write(&path, "(schema_version:1,font:\"comic\",chroma:true)");
        let loaded = load(&path);
        assert_eq!(loaded.config.font, Font::Sans);
        assert!(loaded.config.chroma);
        assert!(loaded.warnings[0].contains("font"), "{:?}", loaded.warnings);
    }

    #[test]
    fn config_without_theme_fields_gets_default_palette_and_night_off() {
        let dir = Dir::new("theme_default");
        let path = dir.file();
        write(&path, "(schema_version:1,mode:\"clock\",show_seconds:false)");

        let loaded = load(&path);
        let c = &loaded.config;
        assert_eq!(c.palette, Palette::Default);
        assert_eq!(c.night, NightMode::Off);
        assert_eq!(c.night_from, TimeOfDay::parse("22:00").unwrap());
        assert_eq!(c.night_to, TimeOfDay::parse("07:00").unwrap());
        assert_eq!(c.night_dim, 0.7);
        assert!(!c.show_seconds, "existing fields still load");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn theme_fields_round_trip_as_readable_strings() {
        let dir = Dir::new("theme_roundtrip");
        let path = dir.file();
        let config = Config {
            palette: Palette::Warm,
            night: NightMode::Auto,
            night_from: TimeOfDay::parse("23:30").unwrap(),
            night_to: TimeOfDay::parse("6:15").unwrap(),
            night_dim: 0.8,
            ..Default::default()
        };
        save(&path, &config).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("palette: \"warm\""), "{text}");
        assert!(text.contains("night: \"auto\""), "{text}");
        assert!(text.contains("night_from: \"23:30\""), "{text}");
        assert!(text.contains("night_to: \"06:15\""), "{text}");

        let loaded = load(&path);
        assert_eq!(loaded.config, config);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn a_bad_night_time_falls_back_and_keeps_the_rest() {
        let dir = Dir::new("night_bad");
        let path = dir.file();
        write(&path, "(schema_version:1,night:\"auto\",night_from:\"25:00\",night_to:\"06:00\")");

        let loaded = load(&path);
        assert_eq!(loaded.config.night_from, Config::default().night_from);
        assert_eq!(loaded.config.night_to, TimeOfDay::parse("06:00").unwrap());
        assert_eq!(loaded.config.night, NightMode::Auto);
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("night_from"), "{:?}", loaded.warnings);
    }

    /// The readout must never fade below the legibility floor, whatever the
    /// file says.
    #[test]
    fn night_dim_is_clamped_to_the_legibility_floor() {
        let dir = Dir::new("night_dim");
        let path = dir.file();
        write(&path, "(schema_version:1,night_dim:0.2)");
        let loaded = load(&path);
        assert_eq!(loaded.config.night_dim, MIN_NIGHT_DIM);
        assert!(loaded.warnings[0].contains("night_dim"), "{:?}", loaded.warnings);

        write(&path, "(schema_version:1,night_dim:1.5)");
        assert_eq!(load(&path).config.night_dim, 1.0);

        write(&path, "(schema_version:1,night_dim:0.75)");
        let loaded = load(&path);
        assert_eq!(loaded.config.night_dim, 0.75);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn config_without_markets_gets_the_default_board() {
        let dir = Dir::new("markets_default");
        let path = dir.file();
        write(&path, "(schema_version:1,mode:\"market\")");
        let loaded = load(&path);
        assert_eq!(loaded.config.mode, Mode::Market);
        assert_eq!(loaded.config.markets, Market::DEFAULT.to_vec());
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn markets_round_trip_as_a_list_of_ids_in_the_users_order() {
        let dir = Dir::new("markets_roundtrip");
        let path = dir.file();
        let config = Config { markets: vec![Market::Tokyo, Market::Oslo, Market::NewYork], ..Default::default() };
        save(&path, &config).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"tokyo\""), "{text}");
        assert!(text.contains("\"new-york\""), "{text}");
        let loaded = load(&path);
        assert_eq!(loaded.config.markets, config.markets, "hand-set order must survive");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    /// One misspelt exchange drops that row, not the list, and a duplicate
    /// is collapsed; an empty or unusable list falls back to the default.
    #[test]
    fn markets_are_tolerant_per_element() {
        let dir = Dir::new("markets_bad");
        let path = dir.file();
        write(&path, "(schema_version:1,markets:[\"oslo\",\"atlantis\",\"tokyo\",\"oslo\"],chroma:true)");
        let loaded = load(&path);
        assert_eq!(loaded.config.markets, vec![Market::Oslo, Market::Tokyo]);
        assert!(loaded.config.chroma, "other fields survive");
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
        assert!(loaded.warnings[0].contains("atlantis"), "{:?}", loaded.warnings);
        assert!(loaded.warnings[1].contains("twice"), "{:?}", loaded.warnings);

        write(&path, "(schema_version:1,markets:[])");
        let loaded = load(&path);
        assert_eq!(loaded.config.markets, Market::DEFAULT.to_vec());
        assert!(loaded.warnings[0].contains("empty"), "{:?}", loaded.warnings);

        write(&path, "(schema_version:1,markets:\"oslo\")");
        let loaded = load(&path);
        assert_eq!(loaded.config.markets, Market::DEFAULT.to_vec());
        assert!(loaded.warnings[0].contains("invalid"), "{:?}", loaded.warnings);

        write(&path, "(schema_version:1,markets:[\"atlantis\"])");
        let loaded = load(&path);
        assert_eq!(loaded.config.markets, Market::DEFAULT.to_vec(), "nothing usable falls back to the default");
    }

    #[test]
    fn board_layout_labels_and_ring_round_trip_and_default_quietly() {
        let dir = Dir::new("board_style");
        let path = dir.file();
        write(&path, "(schema_version:1,markets:[\"oslo\"])");
        let loaded = load(&path);
        assert_eq!(loaded.config.board_layout, Layout::Vertical);
        assert_eq!(loaded.config.board_labels, Labels::City);
        assert!(!loaded.config.seconds_ring);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        let config = Config {
            board_layout: Layout::Horizontal,
            board_labels: Labels::Code,
            seconds_ring: true,
            palette: Palette::Studio,
            ..Default::default()
        };
        save(&path, &config).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("board_layout: \"horizontal\""), "{text}");
        assert!(text.contains("board_labels: \"code\""), "{text}");
        assert!(text.contains("palette: \"studio\""), "{text}");
        let loaded = load(&path);
        assert_eq!(loaded.config, config);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        write(&path, "(schema_version:1,board_layout:\"diagonal\",seconds_ring:true)");
        let loaded = load(&path);
        assert_eq!(loaded.config.board_layout, Layout::Vertical, "a bad value falls back");
        assert!(loaded.config.seconds_ring, "other fields survive");
        assert!(loaded.warnings[0].contains("board_layout"), "{:?}", loaded.warnings);
    }

    #[test]
    fn timer_sound_defaults_on_and_round_trips() {
        let dir = Dir::new("timer_sound");
        let path = dir.file();
        write(&path, "(schema_version:1)");
        let loaded = load(&path);
        assert!(loaded.config.timer_sound, "a file from before the field keeps the chime");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        let config = Config { timer_sound: false, ..Default::default() };
        save(&path, &config).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("timer_sound: false"));
        assert_eq!(load(&path).config, config);

        write(&path, "(schema_version:1,timer_sound:\"loud\",timer_minutes:5)");
        let loaded = load(&path);
        assert!(loaded.config.timer_sound, "a bad value falls back");
        assert_eq!(loaded.config.timer_minutes, 5, "other fields survive");
        assert!(loaded.warnings[0].contains("timer_sound"), "{:?}", loaded.warnings);
    }

    #[test]
    fn a_timer_under_way_round_trips_and_a_bad_one_is_dropped() {
        let dir = Dir::new("timers");
        let path = dir.file();
        write(&path, "(schema_version:1)");
        let loaded = load(&path);
        assert_eq!((loaded.config.stopwatch, loaded.config.countdown), (None, None));
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        let config = Config {
            stopwatch: Some(Saved { accumulated_ms: 61_500, started_at_ms: None }),
            countdown: Some(Saved { accumulated_ms: 0, started_at_ms: Some(1_800_000_000_000) }),
            ..Default::default()
        };
        save(&path, &config).unwrap();
        assert_eq!(load(&path).config, config);

        write(&path, "(schema_version:1,stopwatch:\"running\",countdown:Some((accumulated_ms:5,started_at_ms:None)),timer_minutes:5)");
        let loaded = load(&path);
        assert_eq!(loaded.config.stopwatch, None, "a bad value falls back to idle");
        assert_eq!(loaded.config.countdown, Some(Saved { accumulated_ms: 5, started_at_ms: None }));
        assert_eq!(loaded.config.timer_minutes, 5, "other fields survive");
        assert!(loaded.warnings[0].contains("stopwatch"), "{:?}", loaded.warnings);
    }

    #[test]
    fn unknown_fields_are_reported_and_ignored() {
        let dir = Dir::new("unknown");
        let path = dir.file();
        write(&path, "(schema_version:1,chroma:true,favourite_colour:\"blue\")");

        let loaded = load(&path);
        assert!(loaded.config.chroma);
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("favourite_colour"), "{:?}", loaded.warnings);
    }

    #[test]
    fn unparseable_file_is_quarantined() {
        let dir = Dir::new("garbage");
        let path = dir.file();
        write(&path, "(schema_version:1, mode:\"clo");

        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default());
        let backup = loaded.quarantined.expect("corrupt file should be kept");
        assert_eq!(backup, path.with_extension("ron.bak"));
        assert_eq!(fs::read_to_string(&backup).unwrap(), "(schema_version:1, mode:\"clo");
    }

    #[test]
    fn newer_schema_is_not_reinterpreted() {
        let dir = Dir::new("newer");
        let path = dir.file();
        write(&path, "(schema_version:99,mode:\"timer\")");

        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.quarantined.is_some());
        assert!(loaded.warnings.iter().any(|w| w.contains("schema 99")), "{:?}", loaded.warnings);
    }

    #[test]
    fn migrates_eframe_persistence_file() {
        let dir = Dir::new("legacy");
        let path = dir.file();
        write(
            &path,
            r#"{
    "window": "(inner_position_pixels:Some((x:3005.0,y:133.0)),outer_position_pixels:Some((x:3005.0,y:133.0)),fullscreen:false,maximized:false,inner_size_points:Some((x:196.0,y:91.0)))",
    "app": "(mode:Timer,timer_minutes:60,size:Large,backdrop:false,chroma:false,show_seconds:true,always_on_top:true)",
}"#,
        );

        let loaded = load(&path);
        assert_eq!(loaded.config.mode, Mode::Timer);
        assert_eq!(loaded.config.timer_minutes, 60);
        assert_eq!(loaded.config.size, Size::Large);
        assert_eq!(loaded.config.window, Some(WindowPos { x: 3005.0, y: 133.0 }));
        assert_eq!(loaded.config.schema_version, SCHEMA_VERSION);
        assert!(loaded.migrated, "a legacy file must be flagged for rewriting");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn out_of_range_timer_is_clamped() {
        let dir = Dir::new("clamp");
        let path = dir.file();
        write(&path, "(schema_version:1,timer_minutes:99999)");

        let loaded = load(&path);
        assert_eq!(loaded.config.timer_minutes, MAX_TIMER_MINUTES);
        assert!(loaded.warnings[0].contains("timer_minutes"), "{:?}", loaded.warnings);
    }

    #[test]
    fn an_override_names_the_config_file_outright() {
        let wanted = std::env::temp_dir().join("elsewhere").join("custom.ron");
        let path = config_path_from(Some(wanted.clone().into_os_string()), Some(PathBuf::from("/base")));
        assert_eq!(path, Some(wanted), "the override must win over the platform location");
    }

    #[test]
    fn an_empty_override_falls_back_to_the_platform_location() {
        let path = config_path_from(Some(std::ffi::OsString::new()), Some(PathBuf::from("/base")));
        assert_eq!(path, Some(PathBuf::from("/base").join("chronodesk").join("data").join("app.ron")));
    }

    #[test]
    fn without_a_base_directory_there_is_no_config_path() {
        assert_eq!(config_path_from(None, None), None);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = Dir::new("absent");
        let loaded = load(&dir.file());
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.warnings.is_empty());
        assert!(loaded.quarantined.is_none());
    }

    #[test]
    fn save_replaces_a_corrupt_file_wholesale() {
        let dir = Dir::new("replace");
        let path = dir.file();
        write(&path, "garbage that is longer than the new content will be");
        save(&path, &Config::default()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with('('), "{text}");
        assert!(!text.contains("garbage"));
    }
}

