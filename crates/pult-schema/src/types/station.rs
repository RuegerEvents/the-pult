//! Stations: the consoles running this show, as seen from inside the show.
//!
//! Every node publishes one row about itself, and only about itself. That makes
//! the collection converge without anyone arbitrating it — a station is the only
//! authority on its own memory usage, and a station that stops publishing stops
//! being current, which is visible rather than silently stale.
//!
//! SYNCED rather than PERSISTED: which machines are on tonight is not part of the
//! show, and a showfile that remembered last year's rig would be a nuisance.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{events::operation::NodeId, PultSchema};

/// One console taking part in the session.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "stations")]
pub struct Station {
    /// The station's `NodeId`, so the row is stable across restarts and every node
    /// writes to the same key for the same machine.
    #[pult(lifecycle = SYNCED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = SYNCED)]
    pub hostname: String,
    #[pult(lifecycle = SYNCED)]
    pub is_leader: bool,
    /// Where peers reach this station, as `ip:port`.
    #[pult(lifecycle = SYNCED)]
    pub sync_addr: String,
    /// Where this station serves its HTTP API, as `ip:port`.
    ///
    /// Assets are bytes rather than replicated fields, so a station that has never
    /// seen a stage plan fetches it from one that has — and this is the address it
    /// fetches from. Defaulted, because an older peer will not send it.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub http_addr: String,
    #[pult(lifecycle = SYNCED)]
    pub cpu_percent: f32,
    #[pult(lifecycle = SYNCED)]
    pub mem_used: u64,
    #[pult(lifecycle = SYNCED)]
    pub mem_total: u64,
    /// How long this backend has been up, not the machine.
    #[pult(lifecycle = SYNCED)]
    pub uptime_s: u64,
    /// The outputs this station is sending, by name.
    #[pult(lifecycle = SYNCED)]
    pub output_plugins: Vec<String>,
    /// How many fixtures this station computes, and how many exist.
    ///
    /// Equal on every station today: playback runs everywhere, which is what makes
    /// output deterministic without extra messages. They come apart when parameter
    /// computation is partitioned, and the pair is here so that when it happens the
    /// UI already says something true.
    #[pult(lifecycle = SYNCED)]
    pub computes_fixtures: u32,
    #[pult(lifecycle = SYNCED)]
    pub total_fixtures: u32,
    /// What this station's playback tick cost over the window just reported.
    ///
    /// `None` when the station performed no ticks at all in that window — a settled
    /// show stops ticking, and zero would read as "instant", which is the opposite
    /// of what happened. Defaulted, so a peer running a build that cannot say still
    /// sends a row that deserialises.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub tick_cost: Option<TickCost>,
    /// When this station last said any of the above.
    #[pult(lifecycle = SYNCED)]
    pub last_seen: DateTime<Utc>,
}

impl Station {
    pub fn node_id(&self) -> NodeId {
        NodeId(self.id)
    }

    /// Has this station gone quiet? It publishes every couple of seconds, so a gap
    /// of three times that is a station that is no longer talking.
    pub fn is_stale(&self, now: DateTime<Utc>, report_interval_s: i64) -> bool {
        (now - self.last_seen).num_seconds() > report_interval_s * 3
    }

    pub fn mem_percent(&self) -> f32 {
        if self.mem_total == 0 {
            return 0.0;
        }
        self.mem_used as f32 / self.mem_total as f32 * 100.0
    }
}

/// What a station's playback tick cost, over one reporting window.
///
/// Two pairs rather than one number, because the tick has two halves that scale
/// differently: computing what playback wants, and then applying it — one write, one
/// broadcast and one output push per fixture that moved. The second is the larger
/// and the more rig-sensitive, so a single figure would attribute all of it to
/// playback and point the next optimisation at the wrong half.
///
/// Mean *and* worst, because the tick has a budget (`TICK`, 25 ms) and the question
/// that matters is whether it is being missed. An average over a couple of seconds
/// of ticks answers a different question and hides an overrun happening several
/// times a second.
///
/// This is what *playback* costs, not what the process costs — flows and the output
/// config push are outside it. What the process costs is `cpu_percent`, in the same
/// row, which is why anything printing one prints the other.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TickCost {
    /// Mean whole-tick time over the window, in milliseconds.
    pub mean_ms: f32,
    /// The worst single tick in the window, in milliseconds.
    pub max_ms: f32,
    /// Mean time inside `Playback::tick` — the computing half.
    pub playback_mean_ms: f32,
    /// The worst single computing half in the window.
    pub playback_max_ms: f32,
    /// How many ticks the window contained. Never zero: no ticks is `None`, not a
    /// `TickCost` of zeroes. A window is about two seconds, so a station working
    /// flat out reports around 80 and one nearly settled reports a handful.
    pub ticks: u32,
}

impl TickCost {
    /// How much of the tick budget the mean took, as a percentage.
    pub fn share_of_budget(&self, budget_ms: f32) -> f32 {
        if budget_ms <= 0.0 {
            return 0.0;
        }
        self.mean_ms / budget_ms * 100.0
    }

    /// Did any tick in this window miss the budget?
    pub fn overran(&self, budget_ms: f32) -> bool {
        self.max_ms > budget_ms
    }
}

/// A link to another station, measured from this one.
///
/// LOCAL, because a round-trip time is a property of the link rather than of either
/// end: the leader's latency to a follower and that follower's latency to the leader
/// are two separate measurements of two separate paths, and neither node should be
/// publishing the other's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PeerLink {
    pub node_id: Option<NodeId>,
    /// Round-trip time of the last answered heartbeat, in milliseconds.
    pub rtt_ms: Option<f32>,
    /// When that heartbeat came back.
    pub measured_at: Option<DateTime<Utc>>,
    /// Heartbeats sent with no answer yet. Non-zero is a link in trouble.
    pub unanswered: u32,
}

/// Every peer this station is connected to, keyed by node id: the LOCAL `peers` path.
pub type PeerLinks = std::collections::BTreeMap<String, PeerLink>;

#[cfg(test)]
mod tests {
    use super::*;

    fn a_station(last_seen: DateTime<Utc>) -> Station {
        Station {
            id: Uuid::new_v4(),
            hostname: "booth".into(),
            is_leader: true,
            sync_addr: "10.0.0.5:7701".into(),
            http_addr: "10.0.0.5:7700".into(),
            cpu_percent: 12.5,
            mem_used: 4_000_000_000,
            mem_total: 16_000_000_000,
            uptime_s: 90,
            output_plugins: vec!["House".into()],
            computes_fixtures: 12,
            total_fixtures: 12,
            tick_cost: None,
            last_seen,
        }
    }

    #[test]
    fn a_station_that_just_reported_is_current() {
        let now = Utc::now();
        assert!(!a_station(now).is_stale(now, 2));
        assert!(!a_station(now - chrono::Duration::seconds(4)).is_stale(now, 2));
    }

    #[test]
    fn a_station_that_stopped_talking_goes_stale() {
        let now = Utc::now();
        assert!(a_station(now - chrono::Duration::seconds(10)).is_stale(now, 2));
    }

    #[test]
    fn memory_is_reported_as_a_share_of_what_there_is() {
        assert_eq!(a_station(Utc::now()).mem_percent(), 25.0);
    }

    #[test]
    fn a_station_that_has_not_measured_memory_does_not_divide_by_zero() {
        let mut station = a_station(Utc::now());
        station.mem_total = 0;
        assert_eq!(station.mem_percent(), 0.0);
    }

    #[test]
    fn a_station_is_keyed_by_the_node_it_is() {
        let station = a_station(Utc::now());
        assert_eq!(station.node_id().0, station.id);
    }

    /// A session can mix builds. A peer on one that cannot report a tick cost sends
    /// a row without the field, and the rest of what it says has to survive that.
    #[test]
    fn a_row_from_a_build_that_cannot_report_a_tick_still_reads() {
        let without = serde_json::json!({
            "id": Uuid::new_v4(),
            "hostname": "roof",
            "is_leader": false,
            "sync_addr": "10.0.0.6:7701",
            "http_addr": "10.0.0.6:7700",
            "cpu_percent": 8.0,
            "mem_used": 1_000_000_000u64,
            "mem_total": 8_000_000_000u64,
            "uptime_s": 30,
            "output_plugins": ["Art-Net"],
            "computes_fixtures": 7,
            "total_fixtures": 7,
            "last_seen": Utc::now(),
        });

        let station: Station = serde_json::from_value(without).expect("a row without it still reads");
        // Absent, and not mistaken for a station that ticked infinitely fast.
        assert_eq!(station.tick_cost, None);
        // And nothing else was lost getting there.
        assert_eq!(station.hostname, "roof");
        assert_eq!(station.cpu_percent, 8.0);
        assert_eq!(station.total_fixtures, 7);
        assert_eq!(station.output_plugins, vec!["Art-Net".to_string()]);
    }

    #[test]
    fn a_tick_cost_is_read_against_the_budget() {
        let cost = TickCost {
            mean_ms: 7.9,
            max_ms: 31.0,
            playback_mean_ms: 2.4,
            playback_max_ms: 9.0,
            ticks: 80,
        };
        assert_eq!(cost.share_of_budget(25.0).round(), 32.0);
        // The mean is inside the budget and a tick still missed it, which is the
        // distinction the pair exists to make.
        assert!(cost.overran(25.0));
    }

    #[test]
    fn a_share_of_no_budget_does_not_divide_by_zero() {
        assert_eq!(TickCost::default().share_of_budget(0.0), 0.0);
    }
}
