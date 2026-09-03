//! What a browser says about itself.
//!
//! A console *is* a browser. Since the engine lost its tick, a page showing a rig
//! is evaluating that rig in wasm at animation frame rate against a clock it had to
//! estimate — so the thing that is struggling in a room where every station is
//! comfortable can be the tablet at the back of it, and until this existed there was
//! no figure anywhere that said so.
//!
//! **A browser is not a station and must not appear in `stations`.** That collection
//! is one row per node, written by the node about itself and replicated, and a tab
//! that closes has to leave nothing behind. This is the LOCAL `clients` path instead:
//! a map keyed by the WebSocket session, owned by the station serving those sockets,
//! and emptied of a browser by the same disconnect that ends its socket.
//!
//! **A row is a reading, so it is also dropped when it stops being one.** Unlike a
//! log raise — which task 48 could leave to expire with its connection, because it is
//! an *ask* — a client row is the last thing a page said about itself, and a socket
//! can stay open long after the page stopped saying anything. So the disconnect is
//! the usual end of a row and a sweep is the other one, at ninety seconds of silence:
//! comfortably past the once-a-minute a browser throttles a backgrounded tab's timers
//! to, so a tablet nobody is looking at keeps its row rather than flickering in and
//! out of the list on every sweep.
//!
//! **It stays LOCAL, and the exception crosses as a line.** Task 48 answered the
//! neighbouring question for a log line with "yes, at a quieter threshold" — a
//! browser's fault reaches its station over `log.report` and goes to peers like any
//! other line. A fault is occasional and a frame rate is every second, and that is
//! the whole difference: a row per browser per report crossing the sync link for
//! ever is a stream nobody is reading, on the same LAN as the Art-Net. So what
//! replicates is the *exception* — a browser that has been struggling for a window
//! says so at `warn`, which already reaches every console — and the continuous
//! figures stay with the station serving the page, the way `peers` does.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One browser, as it last described itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientStats {
    /// The short form of the WebSocket session id — as much identity as a page has,
    /// and the same eight characters `LogSource::Browser` carries, so a line in the
    /// log and a row in the panel can be recognised as the same tab.
    pub session: String,
    /// What the browser is, in as many words as it was willing to say.
    pub label: String,
    /// What its frames cost, or `None` when it drew none in the window.
    ///
    /// Absent rather than zero, for the reason `Station::frame_costs` is empty rather
    /// than zeroed: a page with no light on it is not drawing, and a frame rate of
    /// zero would read as a browser in trouble instead of one with nothing to do. A
    /// backgrounded tab is the ordinary case — a browser stops serving animation
    /// frames to a tab nobody is looking at, which is correct and is not a fault.
    pub frames: Option<BrowserFrames>,
    /// Heap in use and the ceiling on it, where the browser offers the figures at
    /// all. Chromium does; the others answer nothing, and nothing is what is stored.
    pub heap_used: Option<u64>,
    pub heap_limit: Option<u64>,
    /// What this page believes the station's clock is, as an offset from its own,
    /// and the round trip that estimate came from.
    ///
    /// The one number that says whether anything else the page is showing can be
    /// trusted: everything driving a parameter is anchored in console milliseconds,
    /// so a page evaluating against a clock it has placed wrongly draws every fade
    /// out by exactly that much, and does it plausibly. `None` is a page that has not
    /// placed itself yet and is showing gaps rather than numbers.
    ///
    /// Read from the estimate the page is already using rather than measured again
    /// here — a second estimate of the same quantity is a second answer to it.
    pub clock_offset_ms: Option<f32>,
    pub clock_rtt_ms: Option<f32>,
    /// When the station heard this, by the station's clock, so the panel can grey a
    /// browser that has gone quiet without trusting the browser's own clock to say.
    pub at_ms: i64,
    /// What the station sent down this socket since the page's previous report, and
    /// how long that was.
    ///
    /// **Measured by the station, not claimed by the page.** A browser cannot see its
    /// own socket — there is no API that tells a page how many bytes arrived on its
    /// WebSocket — so this is the one figure here the station fills in for itself,
    /// beside `session` and `at_ms` and for the same reason.
    ///
    /// It is worth having because it is the cost of *watching*: a console is a browser
    /// subscribed to paths, and a panel open on a busy collection is traffic the
    /// station pays for. Task 44's promise is that a three-second fade over two
    /// thousand fixtures is one push and nothing in between, and this is where that
    /// promise is visible or is not.
    ///
    /// Defaulted, so a report from an older page still deserialises.
    #[serde(default)]
    pub sent_bytes: u64,
    #[serde(default)]
    pub sent_window_ms: u32,
}

/// What one browser's frames cost over one window.
///
/// The same shape `FrameCost` takes for an output connector, and deliberately so: a
/// frame is a frame, it has a whole and an evaluating half, and the question in both
/// cases is whether the budget is being missed rather than what the average was. What
/// it does not share is a connector's name and protocol, and what it adds is how many
/// parameters were evaluated — the figure that says whether a slow browser is slow
/// because of the rig or in spite of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BrowserFrames {
    /// Mean whole-frame time over the window, in milliseconds. The gap between one
    /// animation frame and the next, which is what an operator sees.
    pub mean_ms: f32,
    /// The worst single frame in the window. A mean of 17 ms hiding a 300 ms stall
    /// is a page that visibly jerked and reported nothing.
    pub max_ms: f32,
    /// Mean time inside the evaluator, and the worst of those.
    pub evaluating_mean_ms: f32,
    pub evaluating_max_ms: f32,
    /// How many parameters the last frame of the window asked for.
    pub parameters: u32,
    /// How many frames the window contained, and how long it was, so a frame rate
    /// can be read off the pair rather than stored as a third figure that could
    /// disagree with them.
    pub frames: u32,
    pub window_ms: u32,
}

impl ClientStats {
    /// What the station is sending this page, per second.
    pub fn bytes_per_second(&self) -> f32 {
        if self.sent_window_ms == 0 {
            return 0.0;
        }
        self.sent_bytes as f32 * 1000.0 / self.sent_window_ms as f32
    }
}

impl BrowserFrames {
    /// Frames per second over the window.
    pub fn fps(&self) -> f32 {
        if self.window_ms == 0 {
            return 0.0;
        }
        self.frames as f32 * 1000.0 / self.window_ms as f32
    }
}

/// Every browser this station is serving, keyed by the short session id: the LOCAL
/// `clients` path.
pub type ClientStatsMap = std::collections::BTreeMap<String, ClientStats>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_rate_is_read_off_the_window_rather_than_stored() {
        let frames = BrowserFrames { frames: 118, window_ms: 2000, ..Default::default() };
        assert!((frames.fps() - 59.0).abs() < 0.01);
    }

    /// A window nothing was measured in cannot be divided by, and answering zero is
    /// the only honest thing left — but the *caller* is expected not to publish one
    /// at all, which is what `frames: None` is for.
    #[test]
    fn an_empty_window_has_no_frame_rate() {
        assert_eq!(BrowserFrames::default().fps(), 0.0);
    }
}
