//! Pure appearance resolution and contrast checks; no live settings or OS changes.
#![allow(dead_code)]
#[path = "../src/appearance.rs"]
mod appearance;
#[path = "../src/persistence.rs"]
mod persistence;
use gpui_kit::WindowAppearance;
use persistence::{AppearanceMode, AppearanceSettings, DarkTheme, LightTheme, RowDensity};
fn luminance(rgb: u32) -> f64 {
    let channel = |shift: u32| {
        let n = ((rgb >> shift) & 255u32) as f64 / 255.;
        if n <= 0.04045 {
            n / 12.92
        } else {
            ((n + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}
fn contrast(a: u32, b: u32) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn main() {
    for mode in [AppearanceMode::Light, AppearanceMode::Dark] {
        for light in [LightTheme::Paper, LightTheme::Frost] {
            for dark in [DarkTheme::Graphite, DarkTheme::Midnight] {
                let settings = AppearanceSettings {
                    mode,
                    light_theme: light,
                    dark_theme: dark,
                    ..Default::default()
                };
                let t = appearance::resolve(&settings, WindowAppearance::Dark);
                for background in [
                    t.background,
                    t.surface,
                    t.sidebar,
                    t.hover,
                    t.dialog,
                    t.selection,
                    t.selection_hover,
                ] {
                    assert!(
                        contrast(t.text, background) >= 4.5,
                        "primary contrast {mode:?} {light:?} {dark:?}: {:.2}",
                        contrast(t.text, background)
                    );
                }
                for background in [t.background, t.surface, t.sidebar, t.hover, t.dialog] {
                    assert!(
                        contrast(t.muted, background) >= 4.5,
                        "secondary contrast {mode:?} {light:?} {dark:?}: {:.2}",
                        contrast(t.muted, background)
                    );
                }
                for foreground in [t.accent, t.error, t.warning] {
                    for background in [t.background, t.surface, t.dialog] {
                        assert!(
                            contrast(foreground, background) >= 4.5,
                            "status/action contrast {mode:?} {light:?} {dark:?}: {:.2}",
                            contrast(foreground, background)
                        );
                    }
                }
                assert_eq!(
                    appearance::resolve(&settings, WindowAppearance::Light).dark,
                    mode == AppearanceMode::Dark
                );
            }
        }
    }
    let settings = AppearanceSettings {
        mode: AppearanceMode::System,
        ..Default::default()
    };
    assert!(!appearance::resolve(&settings, WindowAppearance::Light).dark);
    assert!(!appearance::resolve(&settings, WindowAppearance::VibrantLight).dark);
    assert!(appearance::resolve(&settings, WindowAppearance::Dark).dark);
    assert!(appearance::resolve(&settings, WindowAppearance::VibrantDark).dark);
    for font in [10., 13., 20.] {
        assert!(RowDensity::Compact.row_height(font) < RowDensity::Comfortable.row_height(font));
        assert!(RowDensity::Comfortable.row_height(font) < RowDensity::Spacious.row_height(font));
    }
    println!("PASS: four palettes readable contrast, fixed/System resolution, density sizing");
}
