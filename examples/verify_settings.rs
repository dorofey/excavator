//! Temporary fixtures for versioned JSON preferences. Never touches live settings.
#![allow(dead_code)]
#[path = "../src/persistence.rs"]
mod persistence;
use persistence::{AppearanceMode, AppearanceSettings, DarkTheme, LightTheme, RowDensity};
use std::{
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::PermissionsExt,
    },
    path::PathBuf,
};
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--reload-fixture") {
        let path = PathBuf::from(std::env::args_os().nth(2).expect("Fixture path"));
        let (settings, notice) = persistence::load_from_path(&path);
        assert!(notice.is_none());
        assert_eq!(settings.version, 2);
        assert_eq!(settings.appearance.mode, AppearanceMode::System);
        assert_eq!(settings.appearance.font_size, 17.5);
        assert_eq!(settings.appearance.font_family, "JetBrains Mono");
        assert_eq!(settings.appearance.row_density, RowDensity::Spacious);
        println!("PASS: fresh process reloads saved appearance preferences");
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "excavator-settings-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("preferences.json");
    let (defaults, notice) = persistence::load_from_path(&path);
    assert!(notice.is_none());
    assert_eq!(defaults.version, 2);
    assert_eq!(defaults.appearance, AppearanceSettings::default());
    assert!(!path.exists());
    assert_eq!(defaults.appearance.mode, AppearanceMode::Dark);
    assert_eq!(defaults.appearance.light_theme, LightTheme::Paper);
    assert_eq!(defaults.appearance.dark_theme, DarkTheme::Graphite);
    assert_eq!(defaults.appearance.font_family, ".SystemUIFont");
    assert_eq!(defaults.appearance.font_size, 13.0);
    assert_eq!(defaults.appearance.row_density, RowDensity::Compact);
    println!("PASS: missing settings uses agreed defaults without writing a file");

    let raw = PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'/', b'n', b'a', b't', b'i', b'v', b'e', b'-', 0xff,
    ]));
    let legacy = serde_json::json!({"version":1,"favorites":[raw.as_os_str().as_bytes(),"/Unicode-日本語"],"left":raw.as_os_str().as_bytes(),"right":"/legacy-right","show_hidden":true,"sidebar_visible":false});
    let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&path, &legacy_bytes).unwrap();
    let (migrated, notice) = persistence::load_from_path(&path);
    assert!(notice.is_none());
    assert_eq!(migrated.version, 2);
    assert_eq!(migrated.left, raw);
    assert_eq!(migrated.right, PathBuf::from("/legacy-right"));
    assert_eq!(
        migrated.favorites,
        vec![raw.clone(), PathBuf::from("/Unicode-日本語")]
    );
    assert!(migrated.show_hidden);
    assert!(!migrated.sidebar_visible);
    assert_eq!(migrated.appearance, AppearanceSettings::default());
    assert_eq!(fs::read(&path).unwrap(), legacy_bytes);
    println!(
        "PASS: v1 migration retains native bytes, legacy text paths, favorites, hidden/sidebar flags; original remains v1"
    );

    let mut changed = migrated.clone();
    changed.appearance = AppearanceSettings {
        mode: AppearanceMode::System,
        light_theme: LightTheme::Frost,
        dark_theme: DarkTheme::Midnight,
        font_family: "JetBrains Mono".into(),
        font_size: 17.5,
        row_density: RowDensity::Spacious,
    };
    persistence::save_to_path(&path, &changed).unwrap();
    let (reloaded, notice) = persistence::load_from_path(&path);
    assert!(notice.is_none());
    assert_eq!(reloaded.appearance, changed.appearance);
    assert_eq!(reloaded.left, raw);
    assert_eq!(reloaded.favorites, changed.favorites);
    assert_eq!(reloaded.version, 2);
    let valid_bytes = fs::read(&path).unwrap();
    let encoded: serde_json::Value = serde_json::from_slice(&valid_bytes).unwrap();
    assert_eq!(encoded["version"], 2);
    assert_eq!(encoded["appearance"]["mode"], "system");
    assert_eq!(encoded["appearance"]["light_theme"], "frost");
    assert_eq!(encoded["appearance"]["dark_theme"], "midnight");
    assert_eq!(encoded["appearance"]["row_density"], "spacious");
    for forbidden in [
        "password",
        "access_key",
        "secret_key",
        "session_token",
        "credential",
        "endpoint",
    ] {
        assert!(!String::from_utf8_lossy(&valid_bytes).contains(forbidden));
    }
    println!(
        "PASS: explicit save migrates atomically to v2; appearance/workspace/native paths roundtrip; schema contains no credential fields"
    );

    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--reload-fixture")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );

    for value in [10.0, 20.0] {
        let mut candidate = changed.clone();
        candidate.appearance.font_size = value;
        candidate.appearance.validate().unwrap();
    }
    for value in [9.0, 21.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut candidate = changed.clone();
        candidate.appearance.font_size = value;
        assert!(persistence::save_to_path(&path, &candidate).is_err());
        assert_eq!(fs::read(&path).unwrap(), valid_bytes);
    }
    for family in [
        "".to_string(),
        "   ".to_string(),
        "bad\nfont".to_string(),
        "x".repeat(129),
    ] {
        let mut candidate = changed.clone();
        candidate.appearance.font_family = family;
        assert!(persistence::save_to_path(&path, &candidate).is_err());
        assert_eq!(fs::read(&path).unwrap(), valid_bytes);
    }
    let mut boundary = changed.clone();
    boundary.appearance.font_family = "x".repeat(128);
    boundary.appearance.validate().unwrap();
    for density in [
        RowDensity::Compact,
        RowDensity::Comfortable,
        RowDensity::Spacious,
    ] {
        assert_eq!(
            density.row_height(13.0),
            match density {
                RowDensity::Compact => 25.0,
                RowDensity::Comfortable => 31.0,
                RowDensity::Spacious => 37.0,
            }
        );
        assert_eq!(
            density.row_height(20.0),
            match density {
                RowDensity::Compact => 32.0,
                RowDensity::Comfortable => 38.0,
                RowDensity::Spacious => 44.0,
            }
        );
    }
    println!("PASS: finite font-size bounds, font-family validation, and row-density heights");

    let invalid_cases: Vec<Vec<u8>> = vec![
        b"malformed secret-like-value".to_vec(),
        br#"{"version":999,"future":"secret-like-value"}"#.to_vec(),
        br#"{"version":"secret-like-value"}"#.to_vec(),
        br#"{"version":2,"appearance":{"mode":"secret-like-value"}}"#.to_vec(),
        br#"{"version":2,"appearance":{"light_theme":"secret-like-value"}}"#.to_vec(),
        br#"{"version":2,"appearance":{"font_size":22}}"#.to_vec(),
        br#"{"version":2,"appearance":{"font_family":""}}"#.to_vec(),
        br#"{"version":2,"appearance":{"password":"secret-like-value"}}"#.to_vec(),
        br#"{"version":2,"password":"secret-like-value"}"#.to_vec(),
    ];
    for bytes in invalid_cases {
        fs::write(&path, &bytes).unwrap();
        let (settings, notice) = persistence::load_from_path(&path);
        assert_eq!(settings.appearance, AppearanceSettings::default());
        let notice = notice.expect("Invalid settings must report a notice");
        assert!(!notice.contains("secret-like-value"));
        let failure = persistence::save_to_path(&path, &changed)
            .expect_err("Must preserve invalid existing file");
        assert!(!failure.contains("secret-like-value"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    println!(
        "PASS: corrupt/future/invalid-enum/invalid-range/unknown-field JSON is preserved on load and save; diagnostics redact arbitrary values"
    );

    fs::write(&path, &valid_bytes).unwrap();
    let permissions = fs::metadata(&root).unwrap().permissions();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
    let failure = persistence::save_to_path(&path, &changed);
    fs::set_permissions(&root, permissions).unwrap();
    assert!(
        failure.is_err(),
        "Fixture requires an unprivileged user so directory write permissions are enforced"
    );
    assert_eq!(fs::read(&path).unwrap(), valid_bytes);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    println!(
        "PASS: directory write failure returns an error, preserves original, and leaves no staging files"
    );

    let unreadable = root.join("not-a-file");
    fs::create_dir(&unreadable).unwrap();
    let (_, notice) = persistence::load_from_path(&unreadable);
    assert!(notice.is_some());
    assert!(persistence::save_to_path(&unreadable, &changed).is_err());
    assert!(unreadable.is_dir());
    fs::remove_dir_all(&root).unwrap();
    println!("PASS: preferences fixtures completed; live Application Support untouched");
}
