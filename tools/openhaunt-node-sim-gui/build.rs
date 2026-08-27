//! Tauri needs the panel's build to exist before it can bundle it, and `ui/dist`
//! is not in the repository. Leave a page behind that says what to run, so
//! `cargo build` works in a fresh clone rather than failing on a missing folder.

use std::{fs, path::Path};

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/dist");
    println!("cargo:rerun-if-changed={}", dist.display());

    if !dist.join("index.html").exists() {
        let _ = fs::create_dir_all(&dist);
        let _ = fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=utf-8><title>openhaunt-node-sim</title>\n\
             <body style=\"font:14px system-ui;background:#0b0f0c;color:#cfd8d2;padding:2rem\">\n\
             <p>The panel was not built.\n\
             <p><code>npm --prefix tools/openhaunt-node-sim-gui/ui install</code>\n\
             <p><code>npm --prefix tools/openhaunt-node-sim-gui/ui run build</code>\n",
        );
    }

    tauri_build::build()
}
