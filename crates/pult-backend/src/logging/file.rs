//! This run's log, on the disk, for after the crash.
//!
//! The ring is what the panel shows and it dies with the process — which is a
//! problem, because the reason somebody goes looking for a log is usually that the
//! console stopped. So each run also gets a file, and the last few runs are kept.
//!
//! Two things it deliberately does not do. It **does not write a peer's lines**:
//! this file is this station's own log, and a peer's own file is the authority on
//! what the peer said. And it **does not block whoever emitted the line** — a
//! dedicated thread does the writing, fed by a channel that drops rather than waits
//! if it ever backs up, which is `OutputHandle::push`'s rule for the same reason.
//! A log that can stall the engine is worse than a log with a gap in it.

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc,
};

use pult_schema::ws::{LogLine, LogSource};

/// How many lines may be waiting to be written before new ones are dropped.
const QUEUE: usize = 4096;

pub struct Writer {
    tx: mpsc::SyncSender<LogLine>,
    path: PathBuf,
}

impl Writer {
    /// Open this run's file in `dir`, and remove all but the newest `keep` of the
    /// runs already there.
    pub fn open(dir: &Path, keep: usize) -> std::io::Result<Writer> {
        std::fs::create_dir_all(dir)?;
        prune(dir, keep.saturating_sub(1));

        let path = dir.join(format!("station-{}.log", stamp()));
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;

        let (tx, rx) = mpsc::sync_channel::<LogLine>(QUEUE);
        // A plain OS thread, not a tokio task: `install` runs before a runtime
        // exists in the desktop app, which starts tauri rather than tokio.
        std::thread::Builder::new().name("pult-log-file".into()).spawn(move || {
            while let Ok(line) = rx.recv() {
                let _ = writeln!(file, "{}", format_line(&line));
                // Flushed per line rather than per buffer, because the run this
                // file exists for is the one that ends without warning.
                let _ = file.flush();
            }
        })?;

        Ok(Writer { tx, path })
    }

    /// This run's file. Named so a panel can say where the rest of it went.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, line: &LogLine) {
        // `try_send`, so a burst that outruns the disk loses lines rather than
        // holding up the thread that logged them.
        let _ = self.tx.try_send(line.clone());
    }
}

/// One line, in the shape `.demo/*.log` has always been read in.
fn format_line(line: &LogLine) -> String {
    let source = match &line.source {
        LogSource::Station => String::new(),
        LogSource::Plugin(id) => format!(" plugin={id}"),
        LogSource::Browser(id) => format!(" browser={id}"),
    };
    format!(
        "{} {:>5} {}: {}{}",
        line.at_ms,
        line.level.as_str().to_uppercase(),
        line.target,
        line.message,
        source
    )
}

/// A filename-safe stamp, sortable, one per run.
fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H-%M-%S%.3f").to_string()
}

/// Keep the newest `keep` files and remove the rest.
///
/// By name, which sorts chronologically because the stamp does. Failures are
/// ignored on purpose: not being able to tidy up is no reason to refuse to log.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut runs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("station-") && n.ends_with(".log"))
        })
        .collect();
    runs.sort();
    let excess = runs.len().saturating_sub(keep);
    for path in runs.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}
