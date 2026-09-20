//! A window onto a grandMA3 command wing.
//!
//! Draws the desk, shows what is pressed and moved, lets LEDs and faders be driven, and
//! runs a few demo programs. The protocol is `pult-wing`; the USB handle and the page
//! are here.
//!
//! Two things it needs from a local grandMA3 install, and neither is shipped:
//!
//! * `hardware_configurations.xml`, which names every index. Read where it lies, at
//!   startup — no conversion step, and nothing derived from it left on disk.
//! * `command_wing_app.bin`, which the wing asks for at every connect, because what
//!   lives in its flash is only a bootloader.
//!
//! Both are MA's. This finds them; it does not carry them.

mod demos;
mod web;
mod wing;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use pult_wing::map::WingMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Parser, Debug)]
#[command(about = "Draw a grandMA3 command wing, watch it, drive it")]
struct Args {
    /// MA's `hardware_configurations.xml`. Found in a local grandMA3 by default.
    #[arg(long)]
    map: Option<PathBuf>,
    /// Which module in that file to draw.
    #[arg(long, default_value = pult_wing::COMMAND_WING)]
    module: String,
    /// The wing's application image. Found in a local grandMA3 install by default.
    #[arg(long)]
    image: Option<PathBuf>,
    /// Where the panel is served.
    #[arg(long, default_value_t = 7800)]
    port: u16,
    /// Draw the panel without opening the wing, for working on the page.
    #[arg(long)]
    no_device: bool,
}

/// Every grandMA3 that has ever been installed here, newest first.
fn ma3_installs() -> Vec<PathBuf> {
    let base = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h).join("MALightingTechnology"),
        Err(_) => return Vec::new(),
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("gma3_") && n != "gma3_library")
        })
        .collect();
    // Version-ish names sort usefully as strings; newest last, so reverse.
    found.sort();
    found.reverse();
    found
}

fn find(relative: &str, what: &str) -> Result<PathBuf> {
    for install in ma3_installs() {
        let p = install.join("shared/resource").join(relative);
        if p.exists() {
            return Ok(p);
        }
    }
    Err(anyhow!(
        "could not find {what} in any grandMA3 under ~/MALightingTechnology. \
         Pass it explicitly."
    ))
}

fn load_map(arg: Option<PathBuf>, module: &str) -> Result<WingMap> {
    let path = match arg {
        Some(p) => p,
        None => find("hardware_configurations.xml", "hardware_configurations.xml")?,
    };
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    tracing::info!(path = %path.display(), "control map");
    WingMap::from_xml(&text, module).with_context(|| path.display().to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wing_explorer=info".into()),
        )
        .init();
    let args = Args::parse();

    let map = Arc::new(load_map(args.map, &args.module)?);
    tracing::info!(
        module = %map.module,
        hardkeys = map.hardkeys.len(),
        faders = map.faders.len(),
        encoders = map.encoders.len(),
        leds = map.leds.len(),
        "control map"
    );

    let layout: serde_json::Value =
        serde_json::from_str(include_str!("../ui/layout.json")).context("layout.json")?;

    let (reports, _) = broadcast::channel(256);

    let (out, input) = if args.no_device {
        tracing::warn!("--no-device: drawing the panel, opening no wing");
        (
            Arc::new(std::sync::Mutex::new(pult_wing::OutputState::default())),
            Arc::new(std::sync::Mutex::new(pult_wing::InputState::default())),
        )
    } else {
        let image_path = match args.image {
            Some(p) => p,
            None => find("software/command_wing_app.bin", "command_wing_app.bin")?,
        };
        let image = std::fs::read(&image_path)
            .with_context(|| format!("reading {}", image_path.display()))?;
        tracing::info!(path = %image_path.display(), bytes = image.len(), "wing application");

        let w = wing::spawn(image)?;
        // The USB thread talks over a std channel; the page wants a broadcast. One
        // bridge rather than making the wing thread know about subscribers.
        let tx = reports.clone();
        std::thread::Builder::new()
            .name("wing-reports".into())
            .spawn(move || {
                while let Ok(r) = w.reports.recv() {
                    if let wing::Report::Status { state, detail } = &r {
                        tracing::info!(%state, %detail, "wing");
                    }
                    let _ = tx.send(r);
                }
            })
            .context("report bridge")?;
        (w.out, w.input)
    };

    let app = web::App {
        map: map.clone(),
        layout: Arc::new(layout),
        out: out.clone(),
        demos: Arc::new(demos::Runner::start(out, map, input)),
        reports,
    };

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", args.port))
        .await
        .with_context(|| format!("binding port {}", args.port))?;
    tracing::info!("the wing is at http://localhost:{}", args.port);
    axum::serve(listener, web::router(app)).await?;
    Ok(())
}
