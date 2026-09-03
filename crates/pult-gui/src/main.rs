//! The console as a desktop app.
//!
//! This is a window around [`pult_backend::start`] and nothing else. The window
//! points at the server it just started rather than at a copy of the frontend
//! bundled beside it, which is worth being deliberate about: the app and the
//! tablet in the rig are then looking at the same page from the same origin, so
//! there is one frontend to build, one place the socket can be, and no way for
//! the desktop build to drift from the one everybody else uses.
//!
//! The cost is that Tauri's IPC is not available to a remote origin without a
//! capability naming it, so there is no native file dialog here yet.

// A console should not also open a terminal on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{Ipv4Addr, SocketAddr};

use clap::Parser;
use pult_backend::{logging::LogHandle, Config};
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Parser, Clone)]
#[command(about = "the-pult lighting console", version)]
struct Args {
    /// The port the console serves on — the one a tablet types in. Taken as a
    /// preference rather than a demand: see [`free_or_any`].
    #[arg(long, default_value_t = 7700)]
    port: u16,
    #[arg(long, default_value_t = 7701)]
    sync_port: u16,
    /// The show to open: a `Name.pult` bundle directory. Left out, the console
    /// comes up with no show open and the window shows the welcome screen — which
    /// is what a desktop app started from the dock should do, since it has no
    /// meaningful working directory to find a show in.
    #[arg(long, value_name = "BUNDLE")]
    show: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = 1883)]
    openhaunt_broker_port: u16,
    /// Use this station id instead of the one this machine has recorded.
    #[arg(long, value_name = "UUID")]
    node_id: Option<uuid::Uuid>,
}

fn main() {
    // The desktop app is the strongest case for the capture layer: it writes to a
    // stdout nobody is looking at, and a packaged `.app` has nowhere to write one at
    // all. Installed before tauri, because it needs no runtime and the window that
    // will show the log has to be able to reach a log that already exists.
    let log = pult_backend::logging::install(pult_backend::logging::LogOptions::default())
        .expect("the log could not be set up");

    let args = Args::parse();

    tauri::Builder::default()
        .setup(move |app| {
            // The window first, then the server. Opening the showfile and binding
            // the sync port take a moment, and a desktop app that shows nothing at
            // all for that moment reads as one that did not start.
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("the-pult")
                .inner_size(1440.0, 900.0)
                .min_inner_size(900.0, 600.0)
                .build()?;

            let args = args.clone();
            let log = log.clone();
            tauri::async_runtime::spawn(async move {
                match start(&args, log).await {
                    Ok(url) => open(&window, &url),
                    Err(e) => fail(&window, &e.to_string()),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the console window could not be opened");
}

/// Bring a station up and say where it ended up listening.
async fn start(args: &Args, log: LogHandle) -> anyhow::Result<String> {
    let console = pult_backend::Console::start(Config {
        bind: Ipv4Addr::UNSPECIFIED.into(),
        port: free_or_any(args.port).await,
        sync_port: free_or_any(args.sync_port).await,
        show: args.show.clone(),
        openhaunt_broker_port: args.openhaunt_broker_port,
        node_id: args.node_id,
        log: Some(log),
        ..Config::default()
    })
    .await?;

    // `localhost` rather than the address it is bound to: this is the loopback
    // name every platform's webview is willing to load over plain HTTP. The port is
    // read before the console is handed off to its own task, and it does not move
    // afterwards: a switch reuses the port the OS gave out, so the address in the
    // title bar stays the one an operator typed into the tablet.
    let url = format!("http://localhost:{}", console.http_addr().port());
    tauri::async_runtime::spawn(async move {
        if let Err(e) = console.serve().await {
            tracing::error!("[gui] the console stopped: {e}");
        }
    });
    Ok(url)
}

/// The port if it is free, and any port if it is not.
///
/// A second console on one machine is an ordinary thing to want — it is what
/// `scripts/demo.sh --two` does — and refusing to start is a worse answer than
/// starting somewhere else and saying so in the title bar.
async fn free_or_any(preferred: u16) -> u16 {
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, preferred));
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            drop(listener);
            preferred
        }
        Err(_) => 0,
    }
}

fn open(window: &WebviewWindow, url: &str) {
    // The address is in the title because it is the thing an operator has to type
    // into the tablet that is going to run the rest of the show.
    let _ = window.set_title(&format!("the-pult — {url}"));
    match url.parse() {
        Ok(url) => {
            if let Err(e) = window.navigate(url) {
                fail(window, &e.to_string());
            }
        }
        Err(e) => fail(window, &e.to_string()),
    }
}

fn fail(window: &WebviewWindow, message: &str) {
    tracing::error!("[gui] the console did not start: {message}");
    let _ = window.eval(format!(
        "window.showError({})",
        serde_json::to_string(message).unwrap_or_else(|_| "\"unknown error\"".into())
    ));
}
