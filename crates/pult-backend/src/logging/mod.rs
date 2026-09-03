//! The console's own log, kept where something can show it.
//!
//! `tracing` wrote to stdout and nothing captured it, which meant that on every
//! way of running this that is not a terminal the log did not exist: the desktop
//! app writes to a stdout nobody is looking at, a packaged `.app` has nowhere to
//! write it at all, and a browser on the network — which is a whole console by
//! design — has no access to any machine's stdout. Plugins were logging into that
//! void too: `logging.log` promises a plugin author that their message "lands in
//! the station's log", and it did, and the log was nowhere.
//!
//! So this is a `tracing` layer that keeps what it is told in a bounded ring, hands
//! each line to whoever is listening, and writes this run to a file for the crash
//! that is usually the reason somebody went looking.
//!
//! # Why it is installed from `main` and not from [`crate::start`]
//!
//! `tracing_subscriber`'s `init` is once per *process*, and a station is a library
//! that a process may start more than one of. So [`install`] builds the whole
//! subscriber — the `fmt` layer, the `EnvFilter`, and the capture layer — and hands
//! back a [`LogHandle`] that the caller puts in [`crate::Config`]. A station given
//! no handle simply has no log, which is what every test wants and why none of them
//! changed. Two stations handed the *same* handle share one ring; that is
//! unavoidable, since an event carries no station with it, and it is at least
//! written at the call site rather than discovered.
//!
//! # Levels
//!
//! Two thresholds, and they do different jobs.
//!
//! - **Capture** is what this station keeps: the ring, the panel and the file.
//! - **Publish** is what it puts on the sync link for its peers, quieter by default
//!   (`warn`), because a rig is several consoles and nobody wants everyone's
//!   `debug` crossing the network that is also carrying the show.
//!
//! A peer may ask for more with a raise, and a raise is **clamped to what this
//! station captures** — publishing what was never captured is not possible, and
//! pretending otherwise would show an operator an empty escalation that looks
//! broken. No console changes another machine's ring.

mod file;
mod layer;
mod ring;

#[cfg(test)]
mod tests;

use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc, Mutex,
};

use pult_schema::ws::{LogLevel, LogLine, LogSource};
use tokio::sync::broadcast;
use uuid::Uuid;

pub use layer::CaptureLayer;
pub use ring::LogRing;

/// How many lines the ring holds.
///
/// A megabyte or two, and enough that the interesting line has not scrolled off by
/// the time somebody opens the panel to look for it. Not a preference: `log_level`
/// and `peer_log_level` change what an operator *sees*, and this only changes what
/// it costs.
pub const RING_LINES: usize = 5_000;

/// How long appends are gathered before one push to the browsers.
///
/// A burst of two hundred lines is two messages rather than two hundred. Short
/// enough that a panel still reads as live.
pub const COALESCE_MS: u64 = 100;

/// How many previous runs' files are kept beside this one.
pub const RUN_FILES_KEPT: usize = 5;

/// What this station keeps by default. Louder than what it tells its peers.
pub const CAPTURE_LEVEL_DEFAULT: LogLevel = LogLevel::Info;

/// What this station tells its peers by default.
///
/// A peer's warnings and errors always arrive; nobody's `debug` does unless it is
/// asked for. Which is the split that makes a merged view of a session affordable.
pub const PEER_LEVEL_DEFAULT: LogLevel = LogLevel::Warn;

/// A shared, cloneable grip on the log: the ring, the levels, and the stream.
///
/// Cloneable and cheap, because the layer holds one, the station holds one, and
/// every RPC that answers a question about the log holds one.
#[derive(Clone)]
pub struct LogHandle(Arc<Inner>);

struct Inner {
    /// Which station stamps the lines it emits. Set once the station knows its own
    /// id, which is after the subscriber exists — so lines from before that carry
    /// the nil uuid and the panel shows them as this station's, which they are.
    node_id: Mutex<Uuid>,
    ring: Mutex<LogRing>,
    seq: AtomicU64,
    /// [`LogLevel`] as its discriminant, so the layer's hot path is one relaxed
    /// load and a compare rather than a lock.
    capture: AtomicU8,
    publish: AtomicU8,
    /// Every line that passed the capture threshold, for whoever is listening: the
    /// coalescing task that feeds browsers, and the sync layer that feeds peers.
    tx: broadcast::Sender<LogLine>,
    file: Mutex<Option<file::Writer>>,
}

impl std::fmt::Debug for LogHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogHandle")
            .field("capture", &self.capture_level())
            .field("publish", &self.publish_level())
            .field("held", &self.0.ring.lock().unwrap().len())
            .finish()
    }
}

/// What [`install`] should set up.
#[derive(Debug, Clone)]
pub struct LogOptions {
    pub capture: LogLevel,
    pub publish: LogLevel,
    /// Where the per-run file goes. `None` asks [`log_dir`] for the usual place;
    /// a station that wants no file at all says so with `file: false`.
    pub dir: Option<std::path::PathBuf>,
    pub file: bool,
}

impl Default for LogOptions {
    fn default() -> Self {
        LogOptions {
            capture: CAPTURE_LEVEL_DEFAULT,
            publish: PEER_LEVEL_DEFAULT,
            dir: None,
            file: true,
        }
    }
}

fn level_of(bits: u8) -> LogLevel {
    LogLevel::ALL.get(bits as usize).copied().unwrap_or(LogLevel::Info)
}

fn bits_of(level: LogLevel) -> u8 {
    LogLevel::ALL.iter().position(|l| *l == level).unwrap_or(2) as u8
}

impl LogHandle {
    fn new(options: &LogOptions) -> LogHandle {
        // Bounded, and a listener that falls behind is told it lagged rather than
        // slowing the layer down. A log that can block the thing it is logging is
        // worse than a log with a gap in it, and a gap is visible: the browser
        // notices a jump in `seq` and says how many lines it missed.
        let (tx, _) = broadcast::channel(1024);
        LogHandle(Arc::new(Inner {
            node_id: Mutex::new(Uuid::nil()),
            ring: Mutex::new(LogRing::new(RING_LINES)),
            seq: AtomicU64::new(0),
            capture: AtomicU8::new(bits_of(options.capture)),
            publish: AtomicU8::new(bits_of(options.publish)),
            tx,
            file: Mutex::new(None),
        }))
    }

    pub fn capture_level(&self) -> LogLevel {
        level_of(self.0.capture.load(Ordering::Relaxed))
    }

    /// What this station puts on the sync link, before any peer's raise.
    pub fn publish_level(&self) -> LogLevel {
        level_of(self.0.publish.load(Ordering::Relaxed))
    }

    pub fn set_capture_level(&self, level: LogLevel) {
        self.0.capture.store(bits_of(level), Ordering::Relaxed);
    }

    pub fn set_publish_level(&self, level: LogLevel) {
        self.0.publish.store(bits_of(level), Ordering::Relaxed);
    }

    /// What a peer asking for `asked` actually gets.
    ///
    /// The whole of the clamp rule, in one place because it is the rule that fails
    /// silently in both directions: too low and an escalation shows nothing, too
    /// high and a station publishes what it never kept, which it cannot.
    pub fn publish_level_for(&self, asked: Option<LogLevel>) -> LogLevel {
        let wanted = asked.unwrap_or(self.publish_level()).max(self.publish_level());
        wanted.min(self.capture_level())
    }

    pub fn set_node_id(&self, id: Uuid) {
        *self.0.node_id.lock().unwrap() = id;
    }

    pub fn node_id(&self) -> Uuid {
        *self.0.node_id.lock().unwrap()
    }

    /// This run's file, where there is one.
    ///
    /// The panel shows it, because the ring holds the last few thousand lines and
    /// somebody looking for the line before those needs to be told where to go.
    pub fn file_path(&self) -> Option<std::path::PathBuf> {
        self.0.file.lock().unwrap().as_ref().map(|w| w.path().to_path_buf())
    }

    /// Listen to every line this station captures from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.0.tx.subscribe()
    }

    /// The most recent lines, oldest first, optionally only those a listener at
    /// `level` would keep.
    pub fn tail(&self, limit: usize, level: Option<LogLevel>) -> Vec<LogLine> {
        self.0.ring.lock().unwrap().tail(limit, level)
    }

    /// Record a line this station is emitting: stamps it, keeps it, files it, and
    /// tells whoever is listening.
    ///
    /// The layer's path and `log.report`'s path are the same one, so a browser's
    /// exception is a line like any other and reaches peers on the same threshold.
    pub fn emit(&self, level: LogLevel, target: &str, source: LogSource, message: String) {
        if !level.passes(self.capture_level()) {
            return;
        }
        let line = LogLine {
            seq: self.0.seq.fetch_add(1, Ordering::Relaxed),
            node_id: self.node_id(),
            at_ms: pult_schema::types::sequence::now_ms(),
            level,
            target: target.to_string(),
            source,
            message,
        };
        if let Some(writer) = self.0.file.lock().unwrap().as_ref() {
            writer.write(&line);
        }
        self.0.ring.lock().unwrap().push(line.clone());
        let _ = self.0.tx.send(line);
    }

    /// Take in a line a peer sent.
    ///
    /// It keeps the peer's `seq` and the peer's clock, because those are what make
    /// it identifiable and orderable against the rest of that peer's stream. It
    /// goes in the ring, so the merged panel and `log.tail` both see it — and
    /// **not** in the file, which is this station's own log, nor back onto the sync
    /// link, since every station is already connected to every other and a relay
    /// would only duplicate.
    pub fn accept_from_peer(&self, line: LogLine) {
        self.0.ring.lock().unwrap().push(line.clone());
        let _ = self.0.tx.send(line);
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> LogHandle {
        detached(LogOptions { file: false, ..LogOptions::default() })
    }
}

/// Which browsers are watching which peer's log, and how loudly.
///
/// The whole of the unwind lives here. A raise is not a timer and not a lease: it
/// is a function of who is currently looking, recomputed whenever that changes and
/// sent to the peer as the new answer — including the answer "nobody, stop". A
/// browser that closes its panel is one recompute; a browser that vanishes is
/// `remove_session` and the same recompute; a station that vanishes takes the
/// connection with it, and the raise with that. So there is nothing to expire.
///
/// Two consoles watching one peer get the louder of what they asked for, because
/// the peer sends one stream down one connection and the quieter watcher can filter
/// what it does not want.
#[derive(Clone, Default)]
pub struct Watchers(Arc<Mutex<std::collections::HashMap<Uuid, std::collections::HashMap<Uuid, LogLevel>>>>);

impl Watchers {
    /// Note that `session` is watching `node` at `level`, and say what the peer
    /// should now be asked for — `None` if the answer has not changed.
    pub fn watch(&self, node: Uuid, session: Uuid, level: LogLevel) -> Option<Option<LogLevel>> {
        let mut by_node = self.0.lock().unwrap();
        let before = effective(by_node.get(&node));
        by_node.entry(node).or_default().insert(session, level);
        let after = effective(by_node.get(&node));
        (before != after).then_some(after)
    }

    /// Note that `session` has stopped watching `node`.
    pub fn unwatch(&self, node: Uuid, session: Uuid) -> Option<Option<LogLevel>> {
        let mut by_node = self.0.lock().unwrap();
        let before = effective(by_node.get(&node));
        if let Some(watchers) = by_node.get_mut(&node) {
            watchers.remove(&session);
            if watchers.is_empty() {
                by_node.remove(&node);
            }
        }
        let after = effective(by_node.get(&node));
        (before != after).then_some(after)
    }

    /// A browser is gone. Says what every peer it was watching should now be asked
    /// for, which is the ask that would otherwise outlive the person making it.
    pub fn forget_session(&self, session: Uuid) -> Vec<(Uuid, Option<LogLevel>)> {
        let watched: Vec<Uuid> = {
            let by_node = self.0.lock().unwrap();
            by_node
                .iter()
                .filter(|(_, watchers)| watchers.contains_key(&session))
                .map(|(node, _)| *node)
                .collect()
        };
        watched
            .into_iter()
            .filter_map(|node| self.unwatch(node, session).map(|level| (node, level)))
            .collect()
    }

    /// Who is being watched, for the panel that wants to show its own chips lit.
    pub fn raised(&self) -> std::collections::HashMap<Uuid, LogLevel> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(node, watchers)| {
                watchers.values().copied().max().map(|level| (*node, level))
            })
            .collect()
    }
}

/// The loudest thing anybody is asking of one peer, or nothing if nobody is.
fn effective(watchers: Option<&std::collections::HashMap<Uuid, LogLevel>>) -> Option<LogLevel> {
    watchers.and_then(|w| w.values().copied().max())
}

/// Where the per-run files go.
///
/// `PULT_LOG_DIR` names it outright, exactly as `PULT_PREFERENCES` names the
/// preferences file and for the same reason: two stations on one machine — which
/// `demo.sh --two` starts, and which the tests start constantly — need separate
/// ones. Otherwise beside `preferences.toml`.
pub fn log_dir() -> Option<std::path::PathBuf> {
    if let Some(named) = std::env::var_os("PULT_LOG_DIR") {
        return Some(std::path::PathBuf::from(named));
    }
    Some(crate::infra::preferences::path()?.parent()?.join("logs"))
}

/// A log with nothing feeding it from `tracing`.
///
/// Unlike [`install`], the levels are exactly the ones asked for: preferences are a
/// process-wide setting and this is not the process-wide call.
///
/// Everything works — [`LogHandle::emit`], the ring, the file, publishing to peers
/// — except that no `tracing` event arrives on its own, because no subscriber was
/// installed. Which is exactly what a process that already has a subscriber needs:
/// `init` is once per process, and a test binary running two stations cannot call
/// [`install`] twice. Give each station one of these and both have a real log.
pub fn detached(options: LogOptions) -> LogHandle {
    let handle = LogHandle::new(&options);
    open_file(&handle, &options);
    handle
}

/// Build the subscriber and hand back the grip on what it keeps.
///
/// This replaces what each binary used to do by hand, so the `fmt` layer and the
/// `EnvFilter` are still exactly what they were — stdout is unchanged, and
/// `RUST_LOG` still works — with the capture layer added beside them.
pub fn install(options: LogOptions) -> anyhow::Result<LogHandle> {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let handle = LogHandle::new(&options);
    // The operator's settings, applied here and not in `start`.
    //
    // Preferences are one file per machine and this is the one call per process, so
    // this is where the two line up. Doing it per station instead would overwrite
    // whatever the caller asked for — which is exactly what a station given a
    // deliberately quiet or deliberately loud log does not want.
    let prefs = crate::infra::preferences::load();
    handle.set_capture_level(prefs.capture_level());
    handle.set_publish_level(prefs.peer_level());
    open_file(&handle, &options);

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(CaptureLayer::new(handle.clone()))
        .with(EnvFilter::from_default_env().add_directive("pult_backend=debug".parse()?))
        .init();

    Ok(handle)
}

fn open_file(handle: &LogHandle, options: &LogOptions) {
    if !options.file {
        return;
    }
    let Some(dir) = options.dir.clone().or_else(log_dir) else { return };
    match file::Writer::open(&dir, RUN_FILES_KEPT) {
        Ok(writer) => *handle.0.file.lock().unwrap() = Some(writer),
        // Deliberately not fatal, and said on stdout rather than through `tracing`,
        // which may not be built yet. A console that will not start because it could
        // not open a log file is a worse console than one with no log file.
        Err(e) => eprintln!("[log] no file this run ({}): {e}", dir.display()),
    }
}
