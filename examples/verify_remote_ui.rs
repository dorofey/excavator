//! Rendered loopback fixture UI. Run scripts/remote-fixtures.py first.
//! Set HOME to the repository's .remote-fixture-data/ui-home explicitly.
//! Public fixture credentials are injected into this workspace only; no Keychain writes.
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
use connections::{ConnectionRecord, ConnectionSecrets, Protocol};
use gpui_kit::{prelude::*, *};
use providers::ProviderRegistry;
use std::path::PathBuf;
actions!(excavator_fixture, [Quit]);
fn record(protocol: Protocol, ca: &str) -> ConnectionRecord {
    let (id, name, port) = match protocol {
        Protocol::Sftp => ("fixture-sftp", "Fixture SFTP", 22220),
        Protocol::Ftps => ("fixture-ftps", "Fixture FTPS", 22221),
        Protocol::S3 => ("fixture-s3", "Fixture S3", 22222),
    };
    ConnectionRecord {
        id: id.into(),
        name: name.into(),
        protocol,
        host: if protocol == Protocol::S3 {
            String::new()
        } else {
            "localhost".into()
        },
        port,
        username: if protocol == Protocol::S3 {
            String::new()
        } else {
            "fixture".into()
        },
        root: if protocol == Protocol::S3 {
            String::new()
        } else {
            "/".into()
        },
        bucket: if protocol == Protocol::S3 {
            "excavator-fixture".into()
        } else {
            String::new()
        },
        region: if protocol == Protocol::S3 {
            "us-east-1".into()
        } else {
            String::new()
        },
        endpoint: if protocol == Protocol::S3 {
            "http://127.0.0.1:22222".into()
        } else {
            String::new()
        },
        ca_bundle: if protocol == Protocol::Ftps {
            ca.into()
        } else {
            String::new()
        },
    }
}
fn prepare() -> Result<ProviderRegistry, String> {
    let repository = std::env::current_dir().map_err(|e| e.to_string())?;
    let home = repository.join(".remote-fixture-data/ui-home");
    if std::env::var_os("HOME").map(PathBuf::from).as_ref() != Some(&home) {
        return Err(format!(
            "This example requires HOME={} and must run from the repository root",
            home.display()
        ));
    }
    let output = home.join("copied");
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let ca = repository.join(".remote-fixture-data/ca.pem");
    if !ca.is_file() {
        return Err("Start scripts/remote-fixtures.py first; its fixture CA is missing".into());
    }
    let records = [
        record(Protocol::Sftp, &ca.display().to_string()),
        record(Protocol::Ftps, &ca.display().to_string()),
        record(Protocol::S3, &ca.display().to_string()),
    ];
    for record in &records {
        connections::save(record, &None)?;
    }
    let preferences = persistence::Preferences {
        left: home,
        right: output,
        ..persistence::Preferences::default()
    };
    persistence::save(&preferences)?;
    // Probe before the GPUI event loop: network/filesystem operations never block its UI thread.
    let fingerprint = providers::probe_host(&records[0])?;
    let secret = ConnectionSecrets {
        password: "fixture-password".into(),
        access_key: "fixture".into(),
        secret_key: "fixture-secret".into(),
        session_token: String::new(),
    };
    Ok(ProviderRegistry::with_connections(
        records
            .into_iter()
            .map(|record| (record, secret.clone()))
            .collect(),
    )
    .with_trusted_host("fixture-sftp", fingerprint))
}
fn main() {
    let registry = match prepare() {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("Fixture UI setup failed: {error}");
            std::process::exit(1);
        }
    };
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
        cx.set_window_appearance(Some(WindowAppearance::Dark));
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        if let Err(error) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(420.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Excavator · Remote UI fixture".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| ui::Workspace::with_registry(window, cx, registry)),
        ) {
            eprintln!("Fixture UI launch failed: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
