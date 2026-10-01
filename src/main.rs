use gpui_kit::{prelude::*, *};

mod appearance;
mod connections;
mod credentials;
mod domain;
mod forklift;
mod persistence;
mod platform;
mod providers;
mod terminal;
mod transfers;
mod ui;

actions!(excavator, [Quit]);

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.set_menus([
                Menu::new("Excavator").items([
                    MenuItem::action("Settings…", ui::Settings),
                    MenuItem::separator(),
                    MenuItem::action("Quit Excavator", Quit),
                ]),
                Menu::new("Pane").items([
                    MenuItem::action("Split Right", ui::SplitRight),
                    MenuItem::action("Split Down", ui::SplitDown),
                    MenuItem::action("Close Split", ui::ClosePane),
                    MenuItem::separator(),
                    MenuItem::action("Focus Next Pane", ui::SwitchPane),
                    MenuItem::action("Focus Previous Pane", ui::PreviousPane),
                    MenuItem::action("Grow Nearest Split", ui::GrowLeftPane),
                    MenuItem::action("Shrink Nearest Split", ui::ShrinkLeftPane),
                ]),
                Menu::new("Terminal").items([
                    MenuItem::action("Show / Hide Terminal", ui::ToggleTerminal),
                    MenuItem::action("Focus Terminal", ui::FocusTerminal),
                    MenuItem::action("New Terminal Tab", ui::NewTerminal),
                    MenuItem::action("Split Terminal Right", ui::SplitTerminalRight),
                    MenuItem::action("Split Terminal Down", ui::SplitTerminalDown),
                    MenuItem::action("Focus File Tab", ui::FocusFiles),
                    MenuItem::action("End Terminal Session", ui::EndTerminal),
                ]),
                Menu::new("File").items([
                    MenuItem::action("Choose Folder for Active Pane…", ui::ChooseFolder),
                    MenuItem::action("Import Connections from ForkLift…", ui::ImportForkLift),
                ]),
                Menu::new("Help").items([
                    MenuItem::action("Keyboard Shortcuts", ui::ToggleShortcuts),
                ]),
            ]);
            let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
            if let Err(error) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(640.), px(420.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Excavator".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| ui::Workspace::new(window, cx)),
            ) {
                eprintln!("Unable to open Excavator window: {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
