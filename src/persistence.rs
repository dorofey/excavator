//! Non-secret preferences. Call load/save only on a background executor.
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    Light,
    Dark,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightTheme {
    Paper,
    Frost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DarkTheme {
    Graphite,
    Midnight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDensity {
    Compact,
    Comfortable,
    Spacious,
}

impl RowDensity {
    pub fn row_height(self, font_size: f32) -> f32 {
        match self {
            Self::Compact => (font_size + 12.0).max(25.0),
            Self::Comfortable => (font_size + 18.0).max(31.0),
            Self::Spacious => (font_size + 24.0).max(37.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppearanceSettings {
    pub mode: AppearanceMode,
    pub light_theme: LightTheme,
    pub dark_theme: DarkTheme,
    pub font_family: String,
    pub font_size: f32,
    pub row_density: RowDensity,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            mode: AppearanceMode::Dark,
            light_theme: LightTheme::Paper,
            dark_theme: DarkTheme::Graphite,
            font_family: ".SystemUIFont".into(),
            font_size: 13.0,
            row_density: RowDensity::Compact,
        }
    }
}

impl AppearanceSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.font_size.is_finite() || !(10.0..=20.0).contains(&self.font_size) {
            return Err("UI font size must be a finite number between 10 and 20 pixels".into());
        }
        if self.font_family.trim().is_empty()
            || self.font_family.chars().count() > 128
            || self.font_family.chars().any(char::is_control)
        {
            return Err(
                "UI font family must contain 1 to 128 characters without control characters".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    #[serde(with = "path_list")]
    pub favorites: Vec<PathBuf>,
    #[serde(with = "native_path")]
    pub left: PathBuf,
    #[serde(with = "native_path")]
    pub right: PathBuf,
    pub show_hidden: bool,
    pub sidebar_visible: bool,
    pub vim_mode: bool,
    pub appearance: AppearanceSettings,
}

impl Default for Preferences {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Self {
            version: 2,
            favorites: vec![home.clone()],
            left: home,
            right: PathBuf::from("/"),
            show_hidden: false,
            sidebar_visible: true,
            vim_mode: false,
            appearance: AppearanceSettings::default(),
        }
    }
}

static SAVE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn settings_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    let directory = PathBuf::from(std::env::var_os("HOME").ok_or("Home directory is unavailable")?)
        .join("Library/Application Support/Excavator");
    #[cfg(not(target_os = "macos"))]
    let directory = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".config")
        })
        .join("excavator");
    Ok(directory.join("preferences.json"))
}

pub fn load() -> (Preferences, Option<String>) {
    match settings_path() {
        Ok(path) => load_from_path(&path),
        Err(error) => (Preferences::default(), Some(error)),
    }
}

/// Explicit-path counterpart for safe fixture checks. Call outside the UI thread.
pub fn load_from_path(path: &Path) -> (Preferences, Option<String>) {
    let result = (|| {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Preferences::default());
            }
            Err(error) => return Err(format!("Cannot read preferences: {error}")),
        };
        decode(&bytes)
    })();
    match result {
        Ok(settings) => (settings, None),
        Err(error) => (Preferences::default(), Some(error)),
    }
}

pub fn save(settings: &Preferences) -> Result<(), String> {
    let path = settings_path()?;
    save_to_path(&path, settings)
}

fn decode(bytes: &[u8]) -> Result<Preferences, String> {
    // Decoder diagnostics can repeat arbitrary JSON values. Keep invalid data
    // and secret-like accidental content out of errors while preserving the file.
    let envelope: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "Preferences JSON is malformed; original file preserved".to_string())?;
    let version = envelope
        .get("version")
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                "Preferences version must be an integer; original file preserved".to_string()
            })
        })
        .transpose()?
        .unwrap_or(1);
    if version != 1 && version != 2 {
        return Err(format!(
            "Unsupported preferences version {version}; original file preserved"
        ));
    }
    let mut settings: Preferences = serde_json::from_value(envelope).map_err(|_| {
        "Preferences contain invalid or unsupported settings; original file preserved".to_string()
    })?;
    if version == 1 {
        settings.appearance = AppearanceSettings::default();
    }
    settings.version = 2;
    settings.appearance.validate()?;
    Ok(settings)
}

/// Atomic explicit-path save for fixtures and background callers. This validates
/// both the candidate and any existing file before creating a staging file.
pub fn save_to_path(path: &Path, settings: &Preferences) -> Result<(), String> {
    if settings.version != 2 {
        return Err(
            "Only preferences version 2 may be saved; load older settings to migrate first".into(),
        );
    }
    settings.appearance.validate()?;
    match fs::read(&path) {
        Ok(bytes) => {
            decode(&bytes)
                .map_err(|error| format!("Cannot replace existing preferences: {error}"))?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot inspect preferences before saving: {error}")),
    }
    let parent = path.parent().ok_or("Invalid preferences directory")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create preferences directory: {error}"))?;
    let bytes =
        serde_json::to_vec_pretty(settings).map_err(|_| "Cannot encode preferences".to_string())?;
    // Unique staging names avoid racing writes. The UI serializes save requests.
    let staged = parent.join(format!(
        "preferences-{}-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SAVE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| -> io::Result<()> {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&staged, &path)?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            if let Err(cleanup) = fs::remove_file(&staged) {
                if cleanup.kind() != io::ErrorKind::NotFound {
                    return Err(format!(
                        "Cannot save preferences: {error}; staging cleanup failed at {}: {cleanup}",
                        staged.display()
                    ));
                }
            }
            Err(format!("Cannot save preferences: {error}"))
        }
    }
}

// JSON stores native Unix bytes, retaining filenames that cannot be Unicode.
// String decoding also accepts the first foundation preferences format.
mod native_path {
    use super::*;
    use serde::{Deserializer, Serializer};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Encoded {
        Text(String),
        Bytes(Vec<u8>),
    }
    pub fn serialize<S: Serializer>(
        path: &std::path::Path,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        path.as_os_str().as_bytes().serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        Ok(match Encoded::deserialize(deserializer)? {
            Encoded::Text(text) => PathBuf::from(text),
            Encoded::Bytes(bytes) => PathBuf::from(std::ffi::OsString::from_vec(bytes)),
        })
    }
}

mod path_list {
    use super::*;
    use serde::{Deserializer, Serializer};
    #[derive(Serialize, Deserialize)]
    struct Native(#[serde(with = "native_path")] PathBuf);
    pub fn serialize<S: Serializer>(paths: &[PathBuf], serializer: S) -> Result<S::Ok, S::Error> {
        paths
            .iter()
            .cloned()
            .map(Native)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<PathBuf>, D::Error> {
        Ok(Vec::<Native>::deserialize(deserializer)?
            .into_iter()
            .map(|native| native.0)
            .collect())
    }
}
