//! One log line, as a panel wants to read it.
//!
//! Deliberately not the oplog. That is who changed what — attributed, undoable,
//! replicated, pruned on its own retention. This is diagnostics: per station,
//! nobody's to undo, and hundreds of lines a second at `debug`. Two panels, and
//! this comment says so because "we have a history panel" is the obvious wrong
//! answer.
//!
//! Here rather than in `pult-backend` because the browser reads it, and
//! [`HistoryEntry`](super::HistoryEntry) is the precedent: a type that is on the
//! wire to a frontend but is not an entity lives beside the messages that carry
//! it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// How loud a line is, and — as a threshold — how loud a listener is willing to be.
///
/// Ordered by *verbosity* rather than by severity, so `Error < Warn < Info < Debug
/// < Trace` and a threshold keeps everything at or below itself. That ordering is
/// the whole of the level arithmetic: a capture level, a publish level and a raise
/// are all one of these compared with `<=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Every level, quietest first. The panel's menu and the preference's
    /// validation both read this rather than writing the list again.
    pub const ALL: [LogLevel; 5] =
        [LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace];

    /// The word a preferences file and a panel both use.
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }

    /// Parse one, case-insensitively, so a preferences file may say `WARN`.
    pub fn parse(s: &str) -> Option<LogLevel> {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" => Some(LogLevel::Error),
            "warn" | "warning" => Some(LogLevel::Warn),
            "info" => Some(LogLevel::Info),
            "debug" => Some(LogLevel::Debug),
            "trace" => Some(LogLevel::Trace),
            _ => None,
        }
    }

    /// Is a line at this level kept by a listener whose threshold is `threshold`?
    pub fn passes(self, threshold: LogLevel) -> bool {
        self <= threshold
    }
}

/// Who said it.
///
/// A field rather than a prefix in the message. The plugin id has always been in
/// the text — `host_impls.rs` wrote `[plugin:<id>] …` — but a filter reading a
/// string prefix is defeated by a message that happens to contain a bracket, and
/// the audience for that filter is a plugin author debugging their own plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
#[ts(export)]
pub enum LogSource {
    /// The console itself: a peer lost, a socket that would not bind, a showfile
    /// that complained.
    Station,
    /// A WASM plugin, through `logging.log`. The id is the plugin's.
    Plugin(String),
    /// A browser, through `log.report`. The id is the short form of its WebSocket
    /// session, which is as much identity as a page has.
    Browser(String),
}

/// One line.
///
/// `seq` and `node_id` together identify it exactly, which is what lets a browser
/// reconcile the backlog from `log.tail` against the stream already arriving on
/// `Update` — and lets it say "1,204 lines dropped" instead of silently showing a
/// gap. `at_ms` is the *originating* station's clock and is only as close to this
/// one's as their skew allows; a merged view interleaves by it and is honest about
/// that, which is the best available until stations agree on a clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogLine {
    /// Monotonic within one run of one station, assigned by whoever emitted the
    /// line. A peer's lines keep the peer's numbering.
    ///
    /// `number` rather than ts-rs's `bigint` for a `u64`, which is what every other
    /// millisecond and counter on this wire is: the JSON carries a number either
    /// way, and a `bigint` annotation would describe a value `JSON.parse` never
    /// produces. Exact to 2^53, which a station emitting a thousand lines a second
    /// reaches after rather a long run.
    #[ts(type = "number")]
    pub seq: u64,
    /// Which station said it.
    pub node_id: Uuid,
    /// The emitting station's `now_ms()`.
    #[ts(type = "number")]
    pub at_ms: u64,
    pub level: LogLevel,
    /// The `tracing` target, e.g. `pult_backend::infra::sync`. What a person greps
    /// for when they know which part of the console they are chasing.
    pub target: String,
    pub source: LogSource,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_threshold_keeps_everything_quieter_than_itself() {
        assert!(LogLevel::Error.passes(LogLevel::Warn));
        assert!(LogLevel::Warn.passes(LogLevel::Warn));
        assert!(!LogLevel::Info.passes(LogLevel::Warn));
        assert!(!LogLevel::Debug.passes(LogLevel::Warn));
    }

    #[test]
    fn every_level_passes_trace_and_only_error_passes_error() {
        for level in LogLevel::ALL {
            assert!(level.passes(LogLevel::Trace), "{level:?} should reach a trace listener");
            assert_eq!(level.passes(LogLevel::Error), level == LogLevel::Error);
        }
    }

    #[test]
    fn a_level_survives_its_own_spelling() {
        for level in LogLevel::ALL {
            assert_eq!(LogLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(LogLevel::parse("WARN"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse("warning"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse("chatty"), None);
    }
}
