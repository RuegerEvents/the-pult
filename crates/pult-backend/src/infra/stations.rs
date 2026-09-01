//! Publishing what this station is, so the others can see it.
//!
//! One row, about this node only. Nobody arbitrates the collection: a station is
//! the only authority on its own memory usage, and every other station simply
//! receives what it says. A station that stops publishing goes stale rather than
//! disappearing, which is the more useful failure — an operator wants to see that
//! the machine in the roof has stopped answering, not to have its row vanish.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::Utc;
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::{Path, PathSegment},
    types::{
        fixture::Fixture,
        output::OutputConfig,
        station::{Station, TickCost},
    },
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::watch;
use tracing::debug;

use crate::engine::EngineHandle;

/// How often a station says what it is. Fast enough that a console appearing feels
/// immediate, slow enough that reading `/proc` is not a background job.
pub const REPORT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// What the engine's ticks have cost since the last time anybody asked.
///
/// The engine writes to this on every tick it actually performs; the reporter drains
/// it once a `REPORT_INTERVAL` and publishes the result. Draining resets it, so the
/// figures always describe the window just gone rather than all of history.
///
/// Plain relaxed atomics rather than a lock or a message. This sits on the tick path,
/// which is the thing being measured, so it has to cost nothing and it has to cost
/// the same on five fixtures as on five thousand: four adds and a pair of maxes,
/// whatever the rig. A `Mutex` would put a lock on the tick path to protect five
/// integers, and asking the engine for the numbers over its own command channel
/// would queue the measurement behind the writes it is trying to measure.
#[derive(Debug, Default)]
pub struct TickStats {
    ticks: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
    playback_total_us: AtomicU64,
    playback_max_us: AtomicU64,
}

impl TickStats {
    /// Record one tick: how long the whole thing took, and how much of that was
    /// computing what playback wanted rather than applying it.
    pub fn record(&self, whole: std::time::Duration, playback: std::time::Duration) {
        let whole_us = whole.as_micros() as u64;
        let playback_us = playback.as_micros() as u64;
        self.total_us.fetch_add(whole_us, Ordering::Relaxed);
        self.playback_total_us.fetch_add(playback_us, Ordering::Relaxed);
        self.max_us.fetch_max(whole_us, Ordering::Relaxed);
        self.playback_max_us.fetch_max(playback_us, Ordering::Relaxed);
        // Counted last, so a tick landing in the middle of a drain is counted by the
        // window that also has its microseconds. See `drain`.
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }

    /// Take the window and start a new one.
    ///
    /// `None` when the window contained no ticks at all. That is the ordinary state
    /// of a settled show — the timer still fires, `playback_tick` returns early, and
    /// nothing is recorded — and it has to stay distinguishable from a tick that took
    /// no time, which is why this is an `Option` rather than a `TickCost` of zeroes.
    ///
    /// The five counters are not read atomically together. Deliberately: the cost of
    /// making them so is a lock on the tick path, and the cost of not doing is that a
    /// tick landing between the first swap and the last is attributed to the window
    /// that already took its microseconds. Because the sums are taken before the
    /// count, that skews a mean very slightly low for one tick in eighty rather than
    /// producing a window that claims ticks costing nothing.
    pub fn drain(&self) -> Option<TickCost> {
        let total_us = self.total_us.swap(0, Ordering::Relaxed);
        let max_us = self.max_us.swap(0, Ordering::Relaxed);
        let playback_total_us = self.playback_total_us.swap(0, Ordering::Relaxed);
        let playback_max_us = self.playback_max_us.swap(0, Ordering::Relaxed);
        let ticks = self.ticks.swap(0, Ordering::Relaxed);

        if ticks == 0 {
            return None;
        }

        let mean = |total: u64| (total as f64 / ticks as f64 / 1000.0) as f32;
        Some(TickCost {
            mean_ms: mean(total_us),
            max_ms: us_to_ms(max_us),
            playback_mean_ms: mean(playback_total_us),
            playback_max_ms: us_to_ms(playback_max_us),
            ticks: ticks as u32,
        })
    }
}

fn us_to_ms(us: u64) -> f32 {
    us as f32 / 1000.0
}

pub struct StationReporter {
    node_id: NodeId,
    engine: EngineHandle,
    sync_addr: std::net::SocketAddr,
    /// Where this station serves HTTP, so a peer can fetch an asset from it.
    http_addr: String,
    /// Refreshed in place: CPU percentage is a difference between two samples, so
    /// the same `System` has to live across ticks or every reading is zero.
    system: System,
    hostname: String,
    started: std::time::Instant,
    /// Peer latencies, measured by the peer loops and published alongside.
    links: watch::Receiver<pult_schema::types::station::PeerLinks>,
    /// What the engine's ticks have cost since the last report. Drained here, which
    /// is what makes each published figure describe the window just gone.
    tick_stats: Arc<TickStats>,
}

impl StationReporter {
    pub fn new(
        node_id: NodeId,
        engine: EngineHandle,
        sync_addr: std::net::SocketAddr,
        http_addr: String,
        links: watch::Receiver<pult_schema::types::station::PeerLinks>,
        tick_stats: Arc<TickStats>,
    ) -> Self {
        StationReporter {
            node_id,
            engine,
            sync_addr,
            http_addr,
            system: System::new(),
            hostname: System::host_name().unwrap_or_else(|| "console".to_string()),
            started: std::time::Instant::now(),
            links,
            tick_stats,
        }
    }

    pub async fn run(mut self) {
        let mut ticker = tokio::time::interval(REPORT_INTERVAL);
        loop {
            ticker.tick().await;
            self.publish().await;
        }
    }

    async fn publish(&mut self) {
        let station = self.measure().await;
        let path: Path = vec![
            PathSegment::Key("stations".into()),
            PathSegment::Id(station.id),
        ];
        let Ok(value) = serde_json::to_value(&station) else { return };

        // A station's row is replaced whole rather than patched field by field: it
        // is one measurement taken at one moment, and half of an old reading beside
        // half of a new one is not a state the machine was ever in.
        if self.engine.set(path.clone(), Lifecycle::Synced, value.clone()).await.is_err() {
            // Nothing has created the row yet.
            let create = vec![
                PathSegment::Key("stations".into()),
                PathSegment::Key("__create".into()),
            ];
            if let Err(e) = self.engine.set(create, Lifecycle::Synced, value).await {
                debug!("[stations] could not publish: {e}");
            }
        }

        self.publish_links().await;
    }

    /// Peer latencies, as LOCAL state. Measured here, about the links from here.
    async fn publish_links(&mut self) {
        let links = self.links.borrow_and_update().clone();
        if let Ok(value) = serde_json::to_value(&links) {
            let path = vec![PathSegment::Key("peers".into())];
            let _ = self.engine.set(path, Lifecycle::Local, value).await;
        }
    }

    async fn measure(&mut self) -> Station {
        // Only this process, not every process on the machine: a console sharing a
        // box with something else should report what *it* is costing.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[sysinfo::get_current_pid().unwrap_or(sysinfo::Pid::from(0))]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        self.system.refresh_memory();

        let process = sysinfo::get_current_pid().ok().and_then(|pid| self.system.process(pid));
        let cpu_percent = process.map(|p| p.cpu_usage()).unwrap_or(0.0);
        let mem_used = process.map(|p| p.memory()).unwrap_or(0);

        let fixtures: Vec<Fixture> = self.read("fixtures").await;
        let outputs: Vec<OutputConfig> = self.read("outputs").await;
        let total = fixtures.len() as u32;

        Station {
            id: self.node_id.0,
            hostname: self.hostname.clone(),
            is_leader: !self.is_follower().await,
            sync_addr: self.sync_addr.to_string(),
            http_addr: self.http_addr.clone(),
            cpu_percent,
            mem_used,
            mem_total: self.system.total_memory(),
            uptime_s: self.started.elapsed().as_secs(),
            output_plugins: outputs
                .iter()
                .filter(|o| o.runs_on(self.node_id))
                .map(|o| o.name.clone())
                .collect(),
            // Every station computes every fixture today. Reported as a pair rather
            // than a flag so the number means something once that changes.
            computes_fixtures: total,
            total_fixtures: total,
            // Drained, not read: these figures are about the window that just ended,
            // and taking them resets the counters for the next one. A station whose
            // show is settled drained nothing and says so with `None` rather than
            // repeating whatever it last managed to measure.
            tick_cost: self.tick_stats.drain(),
            last_seen: Utc::now(),
        }
    }

    async fn is_follower(&self) -> bool {
        self.engine
            .get(vec![PathSegment::Key("session".into())])
            .await
            .ok()
            .and_then(|v| v["is_follower"].as_bool())
            .unwrap_or(false)
    }

    async fn read<T: serde::de::DeserializeOwned>(&self, table: &str) -> Vec<T> {
        self.engine
            .get(vec![PathSegment::Key(table.into())])
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }
}

/// Drop the rows of stations that have stopped talking.
///
/// Only the leader prunes, for the same reason only the leader publishes membership:
/// two nodes deleting each other's rows on different schedules is a fight, not a
/// cleanup. Kept generous — a station that is merely slow should go grey in the UI
/// long before anything removes it.
pub async fn prune_stale(engine: &EngineHandle, keep_for: chrono::Duration) {
    let Ok(value) = engine.get(vec![PathSegment::Key("stations".into())]).await else { return };
    let Ok(stations): Result<Vec<Station>, _> = serde_json::from_value(value) else { return };

    let now = Utc::now();
    for station in stations.iter().filter(|s| now - s.last_seen > keep_for) {
        let path = vec![
            PathSegment::Key("stations".into()),
            PathSegment::Id(station.id),
            PathSegment::Key("__delete".into()),
        ];
        debug!("[stations] {} has been quiet; removing it", station.hostname);
        let _ = engine.set(path, Lifecycle::Synced, serde_json::Value::Null).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A settled show stops ticking. Nothing recorded is not a tick of zero.
    #[test]
    fn a_window_with_no_ticks_in_it_reports_nothing() {
        assert_eq!(TickStats::default().drain(), None);
    }

    #[test]
    fn a_window_reports_the_mean_and_the_worst_of_what_it_saw() {
        let stats = TickStats::default();
        stats.record(ms(2), ms(1));
        stats.record(ms(8), ms(2));
        stats.record(ms(2), ms(3));

        let cost = stats.drain().expect("three ticks were recorded");
        assert_eq!(cost.ticks, 3);
        assert_eq!(cost.mean_ms, 4.0);
        // The worst tick, not the last one and not the mean — which is the whole
        // reason both are published.
        assert_eq!(cost.max_ms, 8.0);
        assert_eq!(cost.playback_mean_ms, 2.0);
        assert_eq!(cost.playback_max_ms, 3.0);
    }

    /// Draining starts a new window. A station that stops ticking must not keep
    /// republishing the last figure it managed to measure.
    #[test]
    fn draining_a_window_empties_it() {
        let stats = TickStats::default();
        stats.record(ms(5), ms(2));
        assert!(stats.drain().is_some());
        assert_eq!(stats.drain(), None);
        assert_eq!(stats.drain(), None);
    }

    /// The max is a high-water mark within one window and not across windows: a
    /// station that had one bad tick a minute ago is not still reporting it.
    #[test]
    fn the_worst_tick_does_not_outlive_its_window() {
        let stats = TickStats::default();
        stats.record(ms(40), ms(30));
        assert_eq!(stats.drain().unwrap().max_ms, 40.0);

        stats.record(ms(3), ms(1));
        let cost = stats.drain().expect("the second window had a tick in it");
        assert_eq!(cost.max_ms, 3.0);
        assert_eq!(cost.ticks, 1);
    }

    /// Both halves are recorded, and the computing half is inside the whole. The
    /// point of the pair is that the difference — applying the effects — is readable.
    #[test]
    fn the_two_halves_are_reported_separately() {
        let stats = TickStats::default();
        stats.record(ms(10), ms(3));

        let cost = stats.drain().unwrap();
        assert!(cost.playback_mean_ms < cost.mean_ms);
        assert_eq!(cost.mean_ms - cost.playback_mean_ms, 7.0);
    }

    /// The reporter, publishing a real row into a real engine.
    ///
    /// The drain lives in `measure`, so the thing worth pinning down is that a
    /// station whose window was empty publishes a row saying nothing about its tick
    /// rather than one claiming its ticks were instant.
    mod reporting {
        use super::*;
        use crate::engine::ShowEngine;
        use crate::infra::showfile;
        use pult_schema::types::station::Station;
        use uuid::Uuid;

        async fn a_reporter() -> (crate::engine::EngineHandle, StationReporter, Arc<TickStats>) {
            let pool = Arc::new(showfile::open_in_memory().await.expect("in-memory showfile"));
            let node_id = NodeId(Uuid::new_v4());
            let (engine, handle, _broadcast) = ShowEngine::new(node_id, pool, None);
            tokio::spawn(engine.run());

            let stats = Arc::new(TickStats::default());
            let (_tx, links) = watch::channel(Default::default());
            let reporter = StationReporter::new(
                node_id,
                handle.clone(),
                "127.0.0.1:7701".parse().unwrap(),
                "127.0.0.1:7700".into(),
                links,
                Arc::clone(&stats),
            );
            (handle, reporter, stats)
        }

        async fn published_row(engine: &crate::engine::EngineHandle) -> Station {
            let rows = engine.get(vec![PathSegment::Key("stations".into())]).await.unwrap();
            let rows: Vec<Station> = serde_json::from_value(rows).unwrap();
            rows.into_iter().next().expect("the reporter published a row")
        }

        #[tokio::test]
        async fn a_station_that_did_not_tick_publishes_no_tick_cost() {
            let (engine, mut reporter, _stats) = a_reporter().await;

            reporter.publish().await;

            let row = published_row(&engine).await;
            assert_eq!(row.tick_cost, None, "nothing ticked, so there is nothing to report");
            // And the row is otherwise a real row, not a half-published one.
            assert_eq!(row.sync_addr, "127.0.0.1:7701");
        }

        #[tokio::test]
        async fn a_station_that_ticked_publishes_what_it_cost() {
            let (engine, mut reporter, stats) = a_reporter().await;
            stats.record(ms(4), ms(1));
            stats.record(ms(8), ms(3));

            reporter.publish().await;

            let cost = published_row(&engine).await.tick_cost.expect("it ticked twice");
            assert_eq!(cost.ticks, 2);
            assert_eq!(cost.mean_ms, 6.0);
            assert_eq!(cost.max_ms, 8.0);
            assert_eq!(cost.playback_mean_ms, 2.0);
        }

        /// A station that ran an act and was then taken off has to go quiet about it.
        /// Publishing the same figure for the rest of the evening would read as a
        /// console still working at it.
        #[tokio::test]
        async fn a_station_that_stops_ticking_does_not_republish_its_last_figure() {
            let (engine, mut reporter, stats) = a_reporter().await;
            stats.record(ms(9), ms(3));

            reporter.publish().await;
            assert!(published_row(&engine).await.tick_cost.is_some());

            reporter.publish().await;
            assert_eq!(
                published_row(&engine).await.tick_cost,
                None,
                "the second window was empty and should have said so"
            );
        }
    }

    /// The spec asks that measuring the tick not change what the tick costs. What
    /// makes that true is that `record` is four atomic operations whatever the rig —
    /// so the thing to pin down is the *per-call* cost against the 25 ms a tick has.
    ///
    /// The bound is deliberately enormous. A relaxed add is nanoseconds; a
    /// microsecond is a thousand times that and still a twenty-five-thousandth of
    /// the budget. It is set where it is to catch somebody replacing this with a
    /// lock or an allocation, not to measure the atomics.
    #[test]
    fn recording_a_tick_costs_nothing_worth_measuring() {
        let stats = TickStats::default();
        let runs = 100_000;

        let began = std::time::Instant::now();
        for _ in 0..runs {
            stats.record(ms(7), ms(2));
        }
        let each = began.elapsed() / runs;

        println!("recording one tick costs {each:?}");
        assert!(
            each < Duration::from_micros(1),
            "recording a tick took {each:?} each, which is no longer free",
        );
        // And it is the same call on any rig: nothing here reads the show.
        assert_eq!(stats.drain().unwrap().ticks, runs);
    }

    #[test]
    fn recording_from_several_threads_loses_nothing() {
        let stats = Arc::new(TickStats::default());
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let stats = Arc::clone(&stats);
                std::thread::spawn(move || {
                    for _ in 0..250 {
                        stats.record(ms(2), ms(1));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        let cost = stats.drain().unwrap();
        assert_eq!(cost.ticks, 1000);
        assert_eq!(cost.mean_ms, 2.0);
    }
}
