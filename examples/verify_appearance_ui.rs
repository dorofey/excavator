//! Isolated rendered appearance fixture. Native appearance overrides are app-scoped,
//! leaving the macOS-wide appearance preference unchanged.
#![allow(dead_code)]
#[path = "../src/appearance.rs"]
mod appearance;
#[path = "../src/connections.rs"]
mod connections;
#[path = "../src/credentials.rs"]
mod credentials;
#[path = "../src/domain.rs"]
mod domain;
#[path = "../src/forklift.rs"]
mod forklift;
#[path = "../src/persistence.rs"]
mod persistence;
#[path = "../src/platform.rs"]
mod platform;
#[path = "../src/providers/mod.rs"]
mod providers;
#[path = "../src/terminal.rs"]
mod terminal;
#[path = "../src/transfers/mod.rs"]
mod transfers;
#[path = "../src/ui/mod.rs"]
mod ui;
use gpui_kit::{prelude::*, *};
actions!(
    excavator_appearance_fixture,
    [Quit, NativeLight, NativeDark, NativeSystem]
);
fn main() {
    let repository = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_home = repository.join(".appearance-fixture-data/home");
    if std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .as_ref()
        != Some(&fixture_home)
    {
        eprintln!(
            "This fixture requires HOME={} and never changes global macOS appearance",
            fixture_home.display()
        );
        std::process::exit(2);
    }
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        // Use only while the settings mode is System: these exercise real native
        // effective-appearance notifications without modifying OS preferences.
        cx.on_action(|_: &NativeLight, cx| cx.set_window_appearance(Some(WindowAppearance::Light)));
        cx.on_action(|_: &NativeDark, cx| cx.set_window_appearance(Some(WindowAppearance::Dark)));
        cx.on_action(|_: &NativeSystem, cx| cx.set_window_appearance(None));
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-alt-shift-l", NativeLight, None),
            KeyBinding::new("cmd-alt-shift-d", NativeDark, None),
            KeyBinding::new("cmd-alt-shift-s", NativeSystem, None),
        ]);
        cx.set_menus([Menu::new("Excavator Appearance Fixture").items([
            MenuItem::action("Settings…", ui::Settings),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ])]);
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        if let Err(error) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(420.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Excavator · Appearance fixture".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| ui::Workspace::new(window, cx)),
        ) {
            eprintln!("Unable to open appearance fixture: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
