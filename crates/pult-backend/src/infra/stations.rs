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
    types::{fixture::Fixture, output::OutputConfig, station::Station},
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
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
    hostname: String,
    started: std::time::Instant,
    /// Peer latencies, measured by the peer loops and published alongside.
    links: watch::Receiver<pult_schema::types::station::PeerLinks>,
}

impl StationReporter {
    pub fn new(
        node_id: NodeId,
        engine: EngineHandle,
        sync_addr: std::net::SocketAddr,
        http_addr: String,
        links: watch::Receiver<pult_schema::types::station::PeerLinks>,
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
