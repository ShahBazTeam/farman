//! Versioned user configuration and Windows application-data paths.
//!
//! FARMAN never rewrites a configuration file unless the user explicitly
//! changes a setting. Invalid files are preserved and a timestamped backup is
//! made before defaults are used.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONFIG_VERSION: u32 = 1;
pub const APP_DIR: &str = "FARMAN";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub font: FontConfig,
    pub theme: String,
    pub default_profile: String,
    pub startup_directory: String,
    pub scrollback: usize,
    pub cursor: CursorConfig,
    pub window: WindowConfig,
    pub bidi: BidiConfig,
    pub keybindings: Keybindings,
    /// Additional user-defined shell profiles. Built-in profiles are separate.
    pub profiles: BTreeMap<String, UserProfile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            font: FontConfig::default(),
            theme: "farman-dark".to_owned(),
            default_profile: "powershell".to_owned(),
            startup_directory: String::new(),
            scrollback: 10_000,
            cursor: CursorConfig::default(),
            window: WindowConfig::default(),
            bidi: BidiConfig::default(),
            keybindings: Keybindings::default(),
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    /// Family name. The bundled `Vazir Code Hack` is used if this family is
    /// unavailable. The font must be monospaced to preserve terminal columns.
    pub family: String,
    pub size: f32,
    pub line_height: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "Vazir Code Hack".to_owned(),
            size: 16.0,
            line_height: 1.35,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorConfig {
    pub style: String,
    pub blink: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            style: "block".to_owned(),
            blink: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    pub opacity: f32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1180,
            height: 720,
            maximized: false,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BidiConfig {
    /// Enable Unicode bidi visual ordering inside each terminal row.
    pub enabled: bool,
    /// `auto` follows the first strong character in each row; `ltr` and `rtl`
    /// force a paragraph direction. Logical text is never modified.
    pub direction: String,
}

impl Default for BidiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            direction: "auto".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keybindings {
    pub new_tab: String,
    pub close_tab: String,
    pub copy: String,
    pub paste: String,
    pub settings: String,
    pub font_increase: String,
    pub font_decrease: String,
    pub next_tab: String,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            new_tab: "Ctrl+Shift+T".to_owned(),
            close_tab: "Ctrl+Shift+W".to_owned(),
            copy: "Ctrl+Shift+C".to_owned(),
            paste: "Ctrl+Shift+V".to_owned(),
            settings: "Ctrl+Comma".to_owned(),
            font_increase: "Ctrl+Equal".to_owned(),
            font_decrease: "Ctrl+Minus".to_owned(),
            next_tab: "Ctrl+Tab".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserProfile {
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: String,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
            name: "Custom shell".to_owned(),
            program: String::new(),
            args: Vec::new(),
            cwd: String::new(),
        }
    }
}

impl Config {
    /// Clamp and normalize values that could destabilize the UI or PTY.
    pub fn normalize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.version != CONFIG_VERSION {
            notes.push(format!(
                "config version {} is not supported (expected {}); using defaults for unknown fields",
                self.version, CONFIG_VERSION
            ));
            self.version = CONFIG_VERSION;
        }
        if !self.font.size.is_finite() || !(8.0..=48.0).contains(&self.font.size) {
            let clamped = if self.font.size.is_finite() {
                self.font.size.clamp(8.0, 48.0)
            } else {
                16.0
            };
            notes.push(format!("font.size clamped to {clamped}"));
            self.font.size = clamped;
        }
        if !self.font.line_height.is_finite() || !(1.0..=2.0).contains(&self.font.line_height) {
            notes.push("font.line_height clamped to 1.35".to_owned());
            self.font.line_height = 1.35;
        }
        if self.font.family.trim().is_empty() {
            self.font.family = "Vazir Code Hack".to_owned();
            notes.push("font.family was empty; using Vazir Code Hack".to_owned());
        }
        if crate::theme::Theme::by_name(&self.theme).is_none() {
            notes.push(format!("unknown theme {:?}; using farman-dark", self.theme));
            self.theme = "farman-dark".to_owned();
        }
        if !matches!(self.bidi.direction.as_str(), "auto" | "ltr" | "rtl") {
            notes.push("bidi.direction must be auto, ltr or rtl; using auto".to_owned());
            self.bidi.direction = "auto".to_owned();
        }
        if !matches!(self.cursor.style.as_str(), "block" | "beam" | "underline") {
            notes.push("cursor.style must be block, beam or underline; using block".to_owned());
            self.cursor.style = "block".to_owned();
        }
        if self.scrollback < 100 {
            notes.push("scrollback clamped to 100".to_owned());
            self.scrollback = 100;
        } else if self.scrollback > 200_000 {
            notes.push("scrollback clamped to 200000".to_owned());
            self.scrollback = 200_000;
        }
        if self.window.width < 720 {
            self.window.width = 720;
        }
        if self.window.height < 420 {
            self.window.height = 420;
        }
        if self.window.width > 10_000 {
            self.window.width = 10_000;
        }
        if self.window.height > 10_000 {
            self.window.height = 10_000;
        }
        if !(0.5..=1.0).contains(&self.window.opacity) {
            self.window.opacity = 1.0;
        }
        if !self.startup_directory.trim().is_empty() {
            let p = Path::new(self.startup_directory.trim());
            if !p.is_dir() {
                notes.push(format!(
                    "startup_directory {:?} does not exist; using the user profile",
                    self.startup_directory
                ));
                self.startup_directory.clear();
            }
        }
        // Validate keybinding strings early. They are parsed again on demand.
        for (name, chord) in [
            ("new_tab", &self.keybindings.new_tab),
            ("close_tab", &self.keybindings.close_tab),
            ("copy", &self.keybindings.copy),
            ("paste", &self.keybindings.paste),
            ("settings", &self.keybindings.settings),
            ("font_increase", &self.keybindings.font_increase),
            ("font_decrease", &self.keybindings.font_decrease),
            ("next_tab", &self.keybindings.next_tab),
        ] {
            if crate::input::KeyChord::parse(chord).is_none() {
                notes.push(format!("keybindings.{name} = {chord:?} could not be parsed"));
            }
        }
        notes
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let body = toml::to_string_pretty(self)?;
        Ok(format!(
            "# FARMAN Terminal configuration\n# Version {CONFIG_VERSION}. Unknown fields are preserved by serde defaults.\n\n{body}"
        ))
    }
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Self {
        let roaming = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| roaming.clone());
        let config_dir = roaming.join(APP_DIR);
        let log_dir = local.join(APP_DIR).join("logs");
        Self {
            config_file: config_dir.join("config.toml"),
            config_dir,
            log_dir,
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Config,
    pub path: PathBuf,
    /// Human-readable non-fatal notices to show in the status bar.
    pub notices: Vec<String>,
}

/// Load the configuration. If it does not exist, a default file is created.
/// If it is unreadable or invalid, the original is left untouched and a backup
/// copy is made; the application runs with normalized defaults.
pub fn load_or_create(paths: &AppPaths) -> LoadedConfig {
    let mut notices = Vec::new();
    let path = paths.config_file.clone();
    if !path.exists() {
        let config = Config::default();
        if let Err(err) = paths.ensure().and_then(|_| save_config(&path, &config)) {
            notices.push(format!("could not create default config: {err}"));
        } else {
            notices.push(format!("created default configuration at {}", path.display()));
        }
        return LoadedConfig {
            config,
            path,
            notices,
        };
    }

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            notices.push(format!(
                "could not read configuration ({}); using defaults",
                err
            ));
            return LoadedConfig {
                config: Config::default(),
                path,
                notices,
            };
        }
    };

    match toml::from_str::<Config>(&text) {
        Ok(mut config) => {
            notices.extend(config.normalize());
            LoadedConfig {
                config,
                path,
                notices,
            }
        }
        Err(err) => {
            notices.push(format!(
                "invalid configuration ({}); using defaults. The file was not overwritten.",
                err
            ));
            let backup = path.with_file_name(format!(
                "config.invalid-{}.toml",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            if let Err(copy_err) = fs::copy(&path, &backup) {
                notices.push(format!("could not back up invalid config: {copy_err}"));
            } else {
                notices.push(format!("invalid config backed up to {}", backup.display()));
            }
            LoadedConfig {
                config: Config::default(),
                path,
                notices,
            }
        }
    }
}

pub fn save_config(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = config
        .to_toml()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(first_err) => {
            // Windows can refuse to replace an existing file on some
            // filesystems. Retry once after removing the destination.
            if path.exists() {
                fs::remove_file(path)?;
                fs::rename(&tmp, path).map_err(|_| first_err)
            } else {
                Err(first_err)
            }
        }
    }
}

pub fn user_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("farman-test-{}-{stamp}", std::process::id()))
            .join(name)
    }

    #[test]
    fn defaults_roundtrip() {
        let mut config = Config::default();
        assert!(config.normalize().is_empty());
        let text = config.to_toml().unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.theme, config.theme);
        assert_eq!(parsed.font.size, config.font.size);
        assert_eq!(parsed.version, CONFIG_VERSION);
        config.font.size = 18.0;
        assert_eq!(config.to_toml().unwrap().contains("size = 18.0"), true);
    }

    #[test]
    fn invalid_values_are_normalized() {
        let mut config = Config::default();
        config.font.size = 300.0;
        config.font.line_height = 0.1;
        config.theme = "nope".to_owned();
        config.bidi.direction = "sideways".to_owned();
        config.cursor.style = "spaceship".to_owned();
        config.scrollback = 0;
        config.window.width = 100;
        let notes = config.normalize();
        assert!(!notes.is_empty());
        assert_eq!(config.font.size, 48.0);
        assert_eq!(config.font.line_height, 1.35);
        assert_eq!(config.theme, "farman-dark");
        assert_eq!(config.bidi.direction, "auto");
        assert_eq!(config.cursor.style, "block");
        assert_eq!(config.scrollback, 100);
        assert_eq!(config.window.width, 720);
    }

    #[test]
    fn creates_and_reloads_file() {
        let path = temp_path("config.toml");
        let paths = AppPaths {
            config_dir: path.parent().unwrap().to_path_buf(),
            config_file: path.clone(),
            log_dir: path.parent().unwrap().join("logs"),
        };
        let loaded = load_or_create(&paths);
        assert!(path.exists());
        assert!(loaded.notices.iter().any(|n| n.contains("created")));
        let loaded2 = load_or_create(&paths);
        assert!(loaded2.notices.is_empty(), "{:?}", loaded2.notices);
    }

    #[test]
    fn invalid_file_is_preserved_and_backed_up() {
        let path = temp_path("config.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "this is not toml = [").unwrap();
        let paths = AppPaths {
            config_dir: path.parent().unwrap().to_path_buf(),
            config_file: path.clone(),
            log_dir: path.parent().unwrap().join("logs"),
        };
        let loaded = load_or_create(&paths);
        assert!(loaded.notices.iter().any(|n| n.contains("invalid config")));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "this is not toml = [",
            "the user's file must not be overwritten"
        );
        let backups: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("config.invalid-"))
            .collect();
        assert!(!backups.is_empty());
    }

    #[test]
    fn custom_profiles_deserialize() {
        let text = r#"
version = 1
default_profile = "my-shell"
[profiles.my-shell]
name = "My shell"
program = "C:\\Windows\\System32\\cmd.exe"
args = ["/K", "echo hello"]
cwd = "C:\\"
"#;
        let config: Config = toml::from_str(text).unwrap();
        let profile = config.profiles.get("my-shell").unwrap();
        assert_eq!(profile.name, "My shell");
        assert_eq!(profile.args.len(), 2);
    }
}
