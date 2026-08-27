//! The frontend is embedded from `frontend/build`, which does not exist in a
//! fresh clone — and `rust-embed` fails to compile against a directory that is
//! not there. Leave a placeholder so `cargo build` works before anyone has run
//! npm, and rebuild whenever the real build changes underneath us.

use std::{fs, path::Path};

fn main() {
    let build = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/build");
    println!("cargo:rerun-if-changed={}", build.display());

    if build.join("index.html").exists() {
        return;
    }
    let _ = fs::create_dir_all(&build);
    let _ = fs::write(
        build.join("index.html"),
        "<!doctype html><meta charset=utf-8><title>the-pult</title>\n\
         <body style=\"font:14px system-ui;background:#1a1a1a;color:#e0e0e0;padding:2rem\">\n\
         <p>The frontend was not built into this binary.\n\
         <p><code>npm --prefix frontend run build</code>, then build again.\n",
    );
}
