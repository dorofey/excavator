//! Isolated pane split acceptance fixture exercising production workspace commands.
//! Persisted preferences, connections, and Keychain are never loaded or modified.
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
#[path = "../src/updater.rs"]
mod updater;
use gpui_kit::{prelude::*, *};
actions!(excavator_split_fixture, [Quit]);
fn main() {
    let fixture = std::env::temp_dir().join(format!("excavator-splits-{}", std::process::id()));
    std::fs::create_dir_all(&fixture).unwrap();
    let cleanup = fixture.clone();
    gpui_kit::application()
        .with_assets(ui::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let quit_cleanup = fixture.clone();
            cx.on_action(move |_: &Quit, cx| {
                let cleanup = quit_cleanup.clone();
                let task = cx.background_executor().spawn(async move {
                    if let Err(error) = std::fs::remove_dir(&cleanup) {
                        eprintln!("Unable to remove empty split fixture: {error}");
                    }
                });
                cx.spawn(async move |cx| {
                    task.await;
                    let _ = cx.update(|cx| cx.quit());
                })
                .detach();
            });
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Excavator · Split acceptance fixture".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut workspace = ui::Workspace::fixture(window, cx, fixture.clone());
                        workspace.verify_splits(window, cx);
                        workspace
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
    let _ = std::fs::remove_dir(cleanup);
}
