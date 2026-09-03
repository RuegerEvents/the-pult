//! The browsers this station is serving, and what they say about themselves.
//!
//! One map, LOCAL, replaced whole by this registry the way `peers` is replaced whole
//! by the station reporter. Nothing else writes it: a page reports through the
//! `client.report` RPC, the socket's own disconnect takes its row away, and a sweep
//! takes away whatever is left of a page that stopped talking without hanging up.
//!
//! Keyed by the *short* session id — the eight characters `LogSource::Browser`
//! already carries — so a warning in the System Log and a row in the System panel can
//! be recognised as the same tab without a lookup between them.
//!
//! Publishing on every report rather than on a tick, because a report is already
//! rate-limited to one every couple of seconds per browser and there are a handful of
//! browsers, not a rig's worth. What that buys is a row that appears the moment a
//! page opens rather than up to two seconds later.

use std::sync::{Arc, Mutex};

use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::client::{ClientStats, ClientStatsMap},
};
use uuid::Uuid;

use crate::engine::EngineHandle;

/// How long a browser may say nothing before its row is dropped.
///
/// Ninety seconds rather than sixty, and the reason is what makes rows go quiet in the
/// first place: a browser throttles a backgrounded tab's timers to roughly one a
/// minute. Pruning at the throttle would make the tablet at the back of the room —
/// which is the machine this whole panel exists to see — flicker out of the list and
/// back on alternate sweeps.
pub const SILENCE_BEFORE_DROPPED: chrono::Duration = chrono::Duration::seconds(90);

/// The short form of a session id: as much identity as a page has.
///
/// The same eight characters `api::rpcs::short_id` puts on a browser's log lines, and
/// that is the point of it rather than an economy.
pub fn short_id(id: Uuid) -> String {
    id.simple().to_string().chars().take(8).collect()
}

/// What the browsers are saying, and the one path that says it.
#[derive(Clone)]
pub struct ClientRegistry {
    inner: Arc<Mutex<ClientStatsMap>>,
    engine: EngineHandle,
}

impl ClientRegistry {
    pub fn new(engine: EngineHandle) -> Self {
        Self { inner: Arc::new(Mutex::new(ClientStatsMap::new())), engine }
    }

    /// Take one browser's report, publish the map it changed, and answer the key it
    /// landed under — which is how the page learns which row in the panel is itself.
    ///
    /// The station stamps `at_ms` and fills in `session` rather than believing what
    /// arrived: a page's clock is exactly the thing in doubt here, and a browser that
    /// could name its own key could write over another tab's row.
    pub async fn report(&self, session: Uuid, mut stats: ClientStats, sent_bytes: u64) -> String {
        let key = short_id(session);
        let now = chrono::Utc::now().timestamp_millis();
        stats.session = key.clone();
        stats.sent_bytes = sent_bytes;
        {
            let mut held = self.inner.lock().unwrap();
            // The window is between this report and the page's previous one, which is
            // the only honest span for a counter the station drained just now. A first
            // report has no previous one and so no window: it says how many bytes and
            // declines to turn that into a rate.
            stats.sent_window_ms = held
                .get(&key)
                .map(|previous| (now - previous.at_ms).clamp(0, u32::MAX as i64) as u32)
                .unwrap_or(0);
            stats.at_ms = now;
            held.insert(key.clone(), stats);
        }
        self.publish().await;
        key
    }

    /// A socket has gone. Nothing here outlives one.
    pub async fn forget(&self, session: Uuid) {
        let removed = self.inner.lock().unwrap().remove(&short_id(session)).is_some();
        if removed {
            self.publish().await;
        }
    }

    /// Drop whatever has been quiet for too long, and say whether anything went.
    pub async fn prune(&self, silence: chrono::Duration) -> usize {
        let cutoff = chrono::Utc::now().timestamp_millis() - silence.num_milliseconds();
        let dropped = {
            let mut held = self.inner.lock().unwrap();
            let before = held.len();
            held.retain(|_, stats| stats.at_ms >= cutoff);
            before - held.len()
        };
        if dropped > 0 {
            self.publish().await;
        }
        dropped
    }

    /// What is held, for a test or a caller that wants to look without the engine.
    pub fn snapshot(&self) -> ClientStatsMap {
        self.inner.lock().unwrap().clone()
    }

    async fn publish(&self) {
        let map = self.snapshot();
        if let Ok(value) = serde_json::to_value(&map) {
            let path = vec![PathSegment::Key("clients".into())];
            let _ = self.engine.set(path, Lifecycle::Local, value).await;
        }
    }
}

/// Sweep the quiet ones, for ever.
///
/// Every station runs one of these about its own browsers. Unlike `prune_stale` for
/// stations, this is not the leader's job and could not be: a browser is connected to
/// one station's socket and no other station has ever heard of it.
pub async fn sweep(clients: ClientRegistry, every: std::time::Duration) {
    let mut ticker = tokio::time::interval(every);
    loop {
        ticker.tick().await;
        clients.prune(SILENCE_BEFORE_DROPPED).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ShowEngine;
    use crate::infra::showfile;
    use pult_schema::events::operation::NodeId;

    async fn a_registry() -> (ClientRegistry, EngineHandle) {
        let pool = Arc::new(showfile::open_in_memory().await.expect("in-memory showfile"));
        let (engine, handle, _broadcast) = ShowEngine::new(NodeId(Uuid::new_v4()), pool, None);
        tokio::spawn(engine.run());
        (ClientRegistry::new(handle.clone()), handle)
    }

    async fn published(engine: &EngineHandle) -> ClientStatsMap {
        let value = engine.get(vec![PathSegment::Key("clients".into())]).await.unwrap();
        serde_json::from_value(value).unwrap()
    }

    fn a_report() -> ClientStats {
        ClientStats {
            label: "Firefox on Linux".into(),
            clock_offset_ms: Some(-3.5),
            ..Default::default()
        }
    }

    /// The path exists before anything reports, so a panel opening on a station with
    /// no browsers but itself gets an empty map rather than a path error.
    #[tokio::test]
    async fn the_path_is_there_before_anybody_has_said_anything() {
        let (_clients, engine) = a_registry().await;
        assert!(published(&engine).await.is_empty());
    }

    #[tokio::test]
    async fn a_report_lands_under_the_short_session_id() {
        let (clients, engine) = a_registry().await;
        let session = Uuid::new_v4();

        clients.report(session, a_report(), 0).await;

        let map = published(&engine).await;
        let row = map.get(&short_id(session)).expect("a row for the browser that reported");
        assert_eq!(row.label, "Firefox on Linux");
        assert_eq!(row.session, short_id(session), "the station fills the key in, not the page");
        assert!(row.at_ms > 0, "and stamps it with its own clock");
    }

    /// A page cannot name its own key. Believing one would let a tab write over
    /// another tab's row by claiming to be it.
    #[tokio::test]
    async fn a_browser_cannot_report_as_somebody_else() {
        let (clients, engine) = a_registry().await;
        let mine = Uuid::new_v4();
        let yours = Uuid::new_v4();
        clients.report(yours, a_report(), 0).await;

        let mut lie = a_report();
        lie.session = short_id(yours);
        lie.label = "not mine to write".into();
        clients.report(mine, lie, 0).await;

        let map = published(&engine).await;
        assert_eq!(map.get(&short_id(yours)).unwrap().label, "Firefox on Linux");
        assert_eq!(map.get(&short_id(mine)).unwrap().label, "not mine to write");
    }

    #[tokio::test]
    async fn a_socket_going_takes_its_row_with_it() {
        let (clients, engine) = a_registry().await;
        let session = Uuid::new_v4();
        clients.report(session, a_report(), 0).await;

        clients.forget(session).await;

        assert!(published(&engine).await.is_empty());
    }

    /// The station measures what it sent, because a page cannot see its own socket —
    /// and the span it divides by is the gap to that page's *previous* report, which
    /// a first report does not have.
    #[tokio::test]
    async fn the_station_fills_in_what_it_sent_and_the_window_it_covers() {
        let (clients, engine) = a_registry().await;
        let session = Uuid::new_v4();

        clients.report(session, a_report(), 4_096).await;
        let first = published(&engine).await;
        let row = &first[&short_id(session)];
        assert_eq!(row.sent_bytes, 4_096);
        assert_eq!(row.sent_window_ms, 0, "a first report has nothing to measure against");
        assert_eq!(row.bytes_per_second(), 0.0, "so it declines to be a rate");

        // Age the row by hand, so the second report has a window to divide by.
        {
            let mut held = clients.inner.lock().unwrap();
            held.get_mut(&short_id(session)).unwrap().at_ms -= 2_000;
        }
        clients.report(session, a_report(), 2_000).await;

        let row = published(&engine).await[&short_id(session)].clone();
        assert!(row.sent_window_ms >= 2_000, "the gap to the previous report");
        assert!((row.bytes_per_second() - 1_000.0).abs() < 50.0, "≈1 kB/s");
    }

    /// The sweep is the other end of a row, for a page that stopped talking without
    /// hanging up — and it must leave alone the one that spoke a moment ago.
    #[tokio::test]
    async fn silence_drops_a_row_and_a_recent_report_keeps_one() {
        let (clients, engine) = a_registry().await;
        let quiet = Uuid::new_v4();
        let talking = Uuid::new_v4();
        clients.report(quiet, a_report(), 0).await;
        clients.report(talking, a_report(), 0).await;

        // Age one of them by hand: waiting ninety seconds is not a test.
        {
            let mut held = clients.inner.lock().unwrap();
            let row = held.get_mut(&short_id(quiet)).unwrap();
            row.at_ms -= SILENCE_BEFORE_DROPPED.num_milliseconds() + 1;
        }

        assert_eq!(clients.prune(SILENCE_BEFORE_DROPPED).await, 1);

        let map = published(&engine).await;
        assert!(!map.contains_key(&short_id(quiet)));
        assert!(map.contains_key(&short_id(talking)));
    }

    /// A sweep that found nothing must not republish, or a station with one idle
    /// browser broadcasts an identical map to every panel for ever.
    #[tokio::test]
    async fn a_sweep_that_drops_nothing_says_nothing() {
        let (clients, _engine) = a_registry().await;
        clients.report(Uuid::new_v4(), a_report(), 0).await;
        assert_eq!(clients.prune(SILENCE_BEFORE_DROPPED).await, 0);
    }
}
