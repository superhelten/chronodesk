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

/// Bump when the meaning of a field changes; add a migration step for it.
/// Schema 0 is eframe's own persistence file, handled by [`migrate_from_eframe`].
pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_TIMER_MINUTES: u64 = 24 * 60;

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
    pub backdrop: bool,
    pub chroma: bool,
    pub show_seconds: bool,
    pub always_on_top: bool,
    /// Dark outline around the text, for legibility without a backdrop.
    pub text_outline: bool,
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
            backdrop: false,
            chroma: false,
            show_seconds: true,
            always_on_top: true,
            text_outline: true,
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

/// `%APPDATA%\chronodesk\data\app.ron` and the equivalent elsewhere — the same
/// location eframe's own persistence used, so existing files are picked up.
pub fn config_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
    };
    Some(base.join("chronodesk").join("data").join("app.ron"))
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
    field(&mut fields, "backdrop", &mut config.backdrop, &mut warnings);
    field(&mut fields, "chroma", &mut config.chroma, &mut warnings);
    field(&mut fields, "show_seconds", &mut config.show_seconds, &mut warnings);
    field(&mut fields, "always_on_top", &mut config.always_on_top, &mut warnings);
    field(&mut fields, "text_outline", &mut config.text_outline, &mut warnings);
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

