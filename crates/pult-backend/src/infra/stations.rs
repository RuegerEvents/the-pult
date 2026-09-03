//! Publishing what this station is, so the others can see it.
//!
//! One row, about this node only. Nobody arbitrates the collection: a station is
//! the only authority on its own memory usage, and every other station simply
//! receives what it says. A station that stops publishing goes stale rather than
//! disappearing, which is the more useful failure — an operator wants to see that
//! the machine in the roof has stopped answering, not to have its row vanish.

use chrono::Utc;
use pult_schema::{
    events::operation::NodeId,
    lifecycle::Lifecycle,
    path::{Path, PathSegment},
    types::{
        fixture::Fixture,
        output::OutputConfig,
        station::{FrameCost, MachineStats, Station},
    },
};
use sysinfo::{Components, Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::watch;
use tracing::debug;

use crate::engine::EngineHandle;

/// How often a station says what it is. Fast enough that a console appearing feels
/// immediate, slow enough that reading `/proc` is not a background job.
pub const REPORT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

pub struct StationReporter {
    node_id: NodeId,
    engine: EngineHandle,
    sync_addr: std::net::SocketAddr,
    /// Where this station serves HTTP, so a peer can fetch an asset from it.
    http_addr: String,
    /// Refreshed in place: CPU percentage is a difference between two samples, so
    /// the same `System` has to live across ticks or every reading is zero.
    system: System,
    /// The machine's interfaces, for the same reason and in the same way: `received`
    /// and `transmitted` are since the previous refresh, so this has to live across
    /// ticks or every window looks like the first one.
    ///
    /// The whole machine rather than this process, which is the point of it — and the
    /// one figure on the row that is not about the console. There is no portable way
    /// to ask what one process is putting on a wire, and the console already counts
    /// its own three legs for itself: its connectors, its peer links, and its
    /// browsers. This is the cable, and the gap between the two is everything else
    /// the box is doing.
    networks: Networks,
    /// When the interfaces were last read.
    net_window_from: std::time::Instant,
    /// The volumes, for the one that holds the showfile. Refreshed rather than rebuilt
    /// so a mount that comes and goes is noticed without re-enumerating every tick.
    disks: Disks,
    /// Where the showfile lives, so the disk figure is about the disk that matters:
    /// a show that cannot be saved is the failure this is meant to see coming.
    ///
    /// **Absolute**, resolved once here. A mount point is absolute, so a relative path
    /// matches none of them and the volume comes out as nothing at all — and a
    /// relative showfile is the ordinary case, not a corner one: `demo.sh` passes
    /// `.demo/demo.db` and a console started from its own directory passes a bare
    /// name. There is a test that would have let this through as a plausible zero.
    showfile: std::path::PathBuf,
    /// Thermal sensors, where the machine exposes any.
    components: Components,
    hostname: String,
    started: std::time::Instant,
    /// Peer latencies, measured by the peer loops and published alongside.
    links: watch::Receiver<pult_schema::types::station::PeerLinks>,
    /// What each output connector's frames have cost, over the window the output
    /// manager most recently closed.
    ///
    /// A watch rather than an accumulator shared with a lock, and for the same reason
    /// `links` is one: the manager is a single task that already owns these counters,
    /// so it can close a window and publish it without anything on the frame path
    /// taking a lock. Nothing here is a replicated write per frame — the figures ride
    /// on the station row this reporter already publishes.
    frames: watch::Receiver<Vec<FrameCost>>,
}

impl StationReporter {
    pub fn new(
        node_id: NodeId,
        engine: EngineHandle,
        sync_addr: std::net::SocketAddr,
        http_addr: String,
        links: watch::Receiver<pult_schema::types::station::PeerLinks>,
        frames: watch::Receiver<Vec<FrameCost>>,
        showfile: std::path::PathBuf,
    ) -> Self {
        StationReporter {
            node_id,
            engine,
            sync_addr,
            http_addr,
            system: System::new(),
            networks: Networks::new_with_refreshed_list(),
            net_window_from: std::time::Instant::now(),
            disks: Disks::new_with_refreshed_list(),
            showfile: absolute(showfile),
            components: Components::new_with_refreshed_list(),
            hostname: System::host_name().unwrap_or_else(|| "console".to_string()),
            started: std::time::Instant::now(),
            links,
            frames,
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
        // The whole machine, as against the process above. Both, because the pair is
        // what means something: a console at 4% on a machine at 96% is not a
        // comfortable console, it is one about to be starved by something else.
        self.system.refresh_cpu_usage();

        let process = sysinfo::get_current_pid().ok().and_then(|pid| self.system.process(pid));
        let cpu_percent = process.map(|p| p.cpu_usage()).unwrap_or(0.0);
        let mem_used = process.map(|p| p.memory()).unwrap_or(0);

        // Both ways, over every interface that is not loopback. On a laptop running
        // `demo.sh` the console's own traffic to itself would otherwise be counted
        // twice — once sent, once received — and swamp what the figure is for.
        self.networks.refresh(true);
        let (net_received, net_sent) = self
            .networks
            .iter()
            .filter(|(name, _)| !is_loopback(name))
            .fold((0u64, 0u64), |(rx, tx), (_, data)| {
                (rx + data.received(), tx + data.transmitted())
            });
        let net_window_ms = self.net_window_from.elapsed().as_millis() as u32;
        self.net_window_from = std::time::Instant::now();

        let machine = self.measure_machine();

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
            frame_costs: self.frames.borrow().clone(),
            machine,
            net_received,
            net_sent,
            net_window_ms,
            last_seen: Utc::now(),
        }
    }

    /// What the machine is doing, whoever is doing it.
    ///
    /// Read here rather than anywhere else because this task already refreshes a
    /// `System` every couple of seconds and these come off the same sample. Nothing
    /// in it is about the console.
    fn measure_machine(&mut self) -> MachineStats {
        self.disks.refresh(true);
        self.components.refresh(true);

        let load = System::load_average();
        // The volume the showfile is on, found by the longest mount point that is a
        // prefix of it — which is how nested mounts resolve, and why the longest wins
        // rather than the first that matches.
        let disk = self
            .disks
            .iter()
            .filter(|disk| self.showfile.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().as_os_str().len());

        MachineStats {
            cpu_percent: self.system.global_cpu_usage(),
            cores: self.system.cpus().len() as u32,
            mem_used: self.system.used_memory(),
            swap_used: self.system.used_swap(),
            swap_total: self.system.total_swap(),
            load_1: load.one as f32,
            load_5: load.five as f32,
            load_15: load.fifteen as f32,
            uptime_s: System::uptime(),
            disk_free: disk.map(|d| d.available_space()).unwrap_or(0),
            disk_total: disk.map(|d| d.total_space()).unwrap_or(0),
            // The warmest sensor rather than one named: what a machine calls its
            // packages differs per platform and per vendor, and the question being
            // asked — is this box too hot — is answered by the highest of them.
            cpu_temperature_c: self
                .components
                .iter()
                .filter_map(|c| c.temperature())
                .filter(|t| t.is_finite())
                .max_by(|a, b| a.total_cmp(b)),
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

/// The absolute form of a path that may not exist yet.
///
/// `canonicalize` is no good on its own: a showfile is often being created, and
/// canonicalizing a path to a file that is not there yet fails. So the *directory* is
/// resolved — it exists, or the station could not write there at all — and the file
/// name is put back on the end. A path that resolves to nothing falls back to itself,
/// which finds no volume and reports no disk, which is the honest answer.
fn absolute(path: std::path::PathBuf) -> std::path::PathBuf {
    if path.is_absolute() {
        return path;
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let resolved = match parent {
        Some(dir) => dir.canonicalize().ok(),
        None => std::env::current_dir().ok(),
    };
    match (resolved, path.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        (Some(dir), None) => dir,
        _ => path,
    }
}

/// Is this interface the machine talking to itself?
///
/// Named rather than matched on a flag, because `sysinfo` does not offer one and the
/// names are stable across the platforms this console is built for: `lo` on Linux,
/// `lo0` on macOS, and a name containing "loopback" on Windows.
fn is_loopback(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "lo" || name == "lo0" || name.contains("loopback")
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
    use pult_schema::path::PathSegment;

    fn a_cost(name: &str, frames: u32) -> FrameCost {
        FrameCost {
            output: name.into(),
            kind: "artnet".into(),
            mean_ms: 4.0,
            max_ms: 8.0,
            evaluating_mean_ms: 1.0,
            evaluating_max_ms: 2.0,
            frames,
            window_ms: 1000,
            bytes: 0,
            packets: 0,
        }
    }

    /// A mount point is absolute, so a relative showfile path matches no volume and
    /// the disk figure comes out as a plausible zero. That is the ordinary case rather
    /// than a corner one — `demo.sh` passes `.demo/demo.db` — so it is pinned here.
    mod resolving_the_showfile {
        use super::*;

        #[test]
        fn a_relative_showfile_is_resolved_against_the_working_directory() {
            let resolved = absolute(std::path::PathBuf::from("demo.db"));
            assert!(resolved.is_absolute(), "or it matches no mount point at all");
            assert!(resolved.ends_with("demo.db"));
        }

        #[test]
        fn a_showfile_that_does_not_exist_yet_still_resolves() {
            // Canonicalizing the file itself would fail here; the directory is what
            // is resolved, and a station is always able to write to it.
            let dir = std::env::temp_dir();
            let resolved = absolute(dir.join("no-such-show.db"));
            assert!(resolved.is_absolute());
            assert!(resolved.ends_with("no-such-show.db"));
        }

        #[test]
        fn an_absolute_one_is_left_alone() {
            let given = std::path::PathBuf::from("/opt/shows/tonight.db");
            assert_eq!(absolute(given.clone()), given);
        }
    }

    /// The reporter, publishing a real row into a real engine.
    ///
    /// What is worth pinning down here is that a station whose connectors emitted
    /// nothing publishes a row saying nothing about its frames, rather than one
    /// claiming its frames were instant.
    mod reporting {
        use super::*;
        use crate::engine::ShowEngine;
        use crate::infra::showfile;
        use pult_schema::types::station::Station;
        use uuid::Uuid;

        type Reporting =
            (crate::engine::EngineHandle, StationReporter, watch::Sender<Vec<FrameCost>>);

        async fn a_reporter() -> Reporting {
            let pool = std::sync::Arc::new(showfile::open_in_memory().await.expect("in-memory showfile"));
            let node_id = NodeId(Uuid::new_v4());
            let (engine, handle, _broadcast) = ShowEngine::new(node_id, pool, None);
            tokio::spawn(engine.run());

            let (frames_tx, frames) = watch::channel(Vec::new());
            let (_tx, links) = watch::channel(Default::default());
            let reporter = StationReporter::new(
                node_id,
                handle.clone(),
                "127.0.0.1:7701".parse().unwrap(),
                "127.0.0.1:7700".into(),
                links,
                frames,
                std::path::PathBuf::from("."),
            );
            (handle, reporter, frames_tx)
        }

        async fn published_row(engine: &crate::engine::EngineHandle) -> Station {
            let rows = engine.get(vec![PathSegment::Key("stations".into())]).await.unwrap();
            let rows: Vec<Station> = serde_json::from_value(rows).unwrap();
            rows.into_iter().next().expect("the reporter published a row")
        }

        /// The machine half, against the machine actually running the test.
        ///
        /// Deliberately loose: a CI container's temperature sensor and its load
        /// average are not this repository's to guarantee. What is asserted is that
        /// the figures a machine *must* be able to answer are answered, so a platform
        /// where `sysinfo` quietly returns nothing is a failing test rather than a
        /// panel full of zeroes nobody questions.
        #[tokio::test]
        async fn a_station_reports_what_the_machine_is_doing_and_not_only_itself() {
            let (engine, mut reporter, _frames) = a_reporter().await;

            // Twice: a CPU percentage is a difference between two samples, so the
            // first reading of one is zero on every platform.
            reporter.publish().await;
            reporter.publish().await;

            let row = published_row(&engine).await;
            assert!(row.machine.cores > 0, "a machine has at least one core");
            assert!(row.machine.mem_used > 0, "and is using some memory");
            assert!(
                row.machine.mem_used >= row.mem_used,
                "the machine's use includes this process's",
            );
            assert!(row.machine.uptime_s > 0, "and has been up for some time");
            assert!(
                row.machine.uptime_s >= row.uptime_s,
                "for at least as long as this backend has",
            );
            assert!(row.machine.disk_total > 0, "the showfile is on a real volume");
            assert!(row.machine.disk_free <= row.machine.disk_total);
        }

        #[tokio::test]
        async fn a_station_with_no_outputs_publishes_no_frame_cost() {
            let (engine, mut reporter, _frames) = a_reporter().await;

            reporter.publish().await;

            let row = published_row(&engine).await;
            assert!(row.frame_costs.is_empty(), "nothing emitted, so there is nothing to report");
            // And the row is otherwise a real row, not a half-published one.
            assert_eq!(row.sync_addr, "127.0.0.1:7701");
        }

        /// Two connectors are two measurements, not two samples of one. Their rates
        /// and their costs are their own, and a station is not entitled to a single
        /// figure that hides either.
        #[tokio::test]
        async fn two_connectors_are_reported_separately() {
            let (engine, mut reporter, frames) = a_reporter().await;
            frames.send(vec![a_cost("House", 40), a_cost("Guest console", 3)]).unwrap();

            reporter.publish().await;

            let costs = published_row(&engine).await.frame_costs;
            assert_eq!(costs.len(), 2);
            assert_eq!(costs[0].output, "House");
            assert_eq!(costs[0].frames, 40);
            assert_eq!(costs[1].output, "Guest console");
            assert_eq!(costs[1].frames, 3, "and the quiet one is not averaged into the busy one");
        }

        /// A station that ran an act and was then taken off has to go quiet about it.
        /// Publishing the same figure for the rest of the evening would read as a
        /// console still working at it.
        #[tokio::test]
        async fn a_station_that_stops_emitting_does_not_republish_its_last_figure() {
            let (engine, mut reporter, frames) = a_reporter().await;
            frames.send(vec![a_cost("House", 40)]).unwrap();

            reporter.publish().await;
            assert!(!published_row(&engine).await.frame_costs.is_empty());

            frames.send(Vec::new()).unwrap();
            reporter.publish().await;
            assert!(
                published_row(&engine).await.frame_costs.is_empty(),
                "the second window was empty and should have said so",
            );
        }
    }
}
