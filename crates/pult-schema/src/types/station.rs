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
    /// What each of this station's output connectors' frames cost over the window
    /// just reported, one entry per connector.
    ///
    /// Empty when nothing emitted a frame at all in that window — a station with no
    /// output configured, or one whose protocols are all idle — because zero would
    /// read as "instant", which is the opposite of what happened. One entry per
    /// connector rather than one figure for the station, because their rates and
    /// their costs are their own: Art-Net drawing at 40 Hz beside an OpenHaunt node
    /// that was told about a fade once are not two samples of one number.
    ///
    /// Defaulted, so a peer running a build that cannot say still sends a row that
    /// deserialises.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub frame_costs: Vec<FrameCost>,
    /// What the *machine* is doing, as against what this console is costing.
    ///
    /// Every other figure on this row is the console's own — `cpu_percent` and
    /// `mem_used` are this process and deliberately so, because a console sharing a
    /// box with something else should report what it is costing rather than what the
    /// box is. These are the other half of that sentence, and the pair is the point:
    /// a station at 4% on a machine at 96% is not a comfortable station, it is a
    /// console about to be starved by something nobody is looking at.
    ///
    /// All defaulted, so a peer on an older build still deserialises.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub machine: MachineStats,
    /// What this machine's network interfaces carried over the window just reported,
    /// each way, and how long that window was.
    ///
    /// **The whole machine, not this console.** Every other figure in this row is what
    /// the console is responsible for; this is what the cable is actually carrying,
    /// which includes whatever else the box is doing. The two are worth having side by
    /// side and must never be confused: a station whose own output accounts for 2 MB/s
    /// on an interface carrying 90 MB/s has a network problem that is not its fault,
    /// and one where the two figures agree has found its own.
    ///
    /// Loopback is excluded. On a laptop running `demo.sh` every byte this console
    /// sends to itself would otherwise arrive twice — sent and received — and swamp
    /// the figure it is there to give.
    ///
    /// Defaulted, so a peer on an older build still deserialises.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub net_received: u64,
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub net_sent: u64,
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub net_window_ms: u32,
    /// When this station last said any of the above.
    #[pult(lifecycle = SYNCED)]
    pub last_seen: DateTime<Utc>,
}

impl PeerLink {
    /// What crossed this link in both directions, per second.
    pub fn bytes_per_second(&self) -> f32 {
        if self.window_ms == 0 {
            return 0.0;
        }
        (self.sent_bytes + self.received_bytes) as f32 * 1000.0 / self.window_ms as f32
    }
}

impl Station {
    /// Everything this station's connectors put on the wire in the window, per second.
    ///
    /// A sum over connectors, which is meaningful in a way a sum of their *frame
    /// times* would not be: two connectors' frames overlap in time and cannot be
    /// added, but their bytes go down the same cable and can.
    pub fn output_bytes_per_second(&self) -> f32 {
        self.frame_costs.iter().map(FrameCost::bytes_per_second).sum()
    }

    /// What the machine's interfaces carried, per second, both ways.
    pub fn net_bytes_per_second(&self) -> f32 {
        if self.net_window_ms == 0 {
            return 0.0;
        }
        (self.net_received + self.net_sent) as f32 * 1000.0 / self.net_window_ms as f32
    }

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

/// What one output connector's frames cost, over one reporting window.
///
/// The thing with a deadline is the output frame. There is no engine tick behind it
/// any more and nothing is applied to state on the way — a frame gathers what it
/// needs, works out what every parameter is doing at one moment, and emits it.
///
/// Two pairs rather than one number, because a frame has two halves that scale
/// differently: evaluating, and putting it on the wire. That is not a hypothetical
/// distinction — a two-figure split is what showed that evaluating was 0.2% of what
/// a tick used to cost, and finding what the other 99.8% actually was still needed a
/// counter added by hand and taken away again.
///
/// Mean *and* worst, because a frame has a budget and the question that matters is
/// whether it is being missed. An average over a couple of seconds hides an overrun
/// happening several times a second.
///
/// This is what a connector costs, not what the process costs. What the process costs
/// is `cpu_percent`, in the same row, which is why anything printing one prints the
/// other.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FrameCost {
    /// The connector's name, as the show configured it.
    pub output: String,
    /// The protocol it speaks.
    pub kind: String,
    /// Mean whole-frame time over the window, in milliseconds.
    pub mean_ms: f32,
    /// The worst single frame in the window, in milliseconds.
    pub max_ms: f32,
    /// Mean time spent working out what the patch is doing — the evaluating half.
    pub evaluating_mean_ms: f32,
    /// The worst single evaluating half in the window.
    pub evaluating_max_ms: f32,
    /// How many frames the window contained. Never zero: a connector that emitted
    /// nothing carries no entry at all rather than an entry of zeroes.
    pub frames: u32,
    /// How long the window was, in milliseconds, so a frame rate can be read off it.
    pub window_ms: u32,
    /// What this connector actually put on the wire in the window, in bytes and in
    /// packets.
    ///
    /// *Actually*, and that is the whole value of the figure: the DMX family skips a
    /// universe whose image has not changed and is not yet due a refresh, so a settled
    /// rig sends a fraction of what a moving one does over the same number of frames.
    /// Bytes per frame would hide that; bytes per window is what is on the cable, and
    /// it is the number an operator sizing a show LAN needs.
    ///
    /// Payload as this console wrote it — the protocol's own packet, without the
    /// UDP, IP and Ethernet headers under it. A wire carries perhaps 5% more than
    /// this says.
    ///
    /// Defaulted, so a peer on an older build still deserialises.
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub packets: u32,
}

impl FrameCost {
    /// Bytes a second, read off the window rather than stored as a third figure that
    /// could disagree with the two it comes from.
    pub fn bytes_per_second(&self) -> f32 {
        if self.window_ms == 0 {
            return 0.0;
        }
        self.bytes as f32 * 1000.0 / self.window_ms as f32
    }

    /// How much of the frame budget the mean took, as a percentage.
    pub fn share_of_budget(&self, budget_ms: f32) -> f32 {
        if budget_ms <= 0.0 {
            return 0.0;
        }
        self.mean_ms / budget_ms * 100.0
    }

    /// Did any frame in this window miss the budget?
    pub fn overran(&self, budget_ms: f32) -> bool {
        self.max_ms > budget_ms
    }
}

/// What the whole machine is doing, whoever is doing it.
///
/// Distinct from the console's own figures on purpose, and never to be summed with
/// them: `Station::cpu_percent` is a share of what `cpu_percent` here is the whole of.
///
/// Everything is optional-shaped rather than `Option`, because these come from a
/// platform layer that answers zero rather than nothing where it cannot tell — and a
/// zero that means "not available" has to be read as such by whatever displays it.
/// The two that genuinely vary by platform say so: `load_average` is zero on Windows,
/// which has no such concept, and `cpu_temperature_c` is `None` wherever no sensor is
/// exposed, which includes most virtual machines.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MachineStats {
    /// Every core together, as a percentage of one machine rather than of one core.
    pub cpu_percent: f32,
    /// How many cores that is over, so a load average can be read against something.
    pub cores: u32,
    /// Memory in use across the machine. `Station::mem_total` is the total it is of.
    pub mem_used: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    /// One, five and fifteen minutes. Zero on Windows, which has no load average —
    /// and zero on a truly idle Unix box, so it is read beside `cpu_percent`.
    pub load_1: f32,
    pub load_5: f32,
    pub load_15: f32,
    /// How long the *machine* has been up, which is not `Station::uptime_s` — that is
    /// how long this backend has been running. A console whose process is younger than
    /// its machine has been restarted, and the pair is what says so.
    pub uptime_s: u64,
    /// Free and total space where the showfile lives.
    ///
    /// That disk and not the root one: a show that cannot be saved is the failure this
    /// figure exists to see coming, and on a machine with several volumes the one that
    /// matters is the one being written to.
    pub disk_free: u64,
    pub disk_total: u64,
    /// The warmest sensor the machine exposes, in Celsius, where it exposes any.
    ///
    /// `None` on most virtual machines and on plenty of real ones. Worth having
    /// because a station in a truss-mounted case in a roof void is a thermal question
    /// long before it is a processing one, and a console that throttles at the top of
    /// the show has no other way of saying why.
    pub cpu_temperature_c: Option<f32>,
}

impl MachineStats {
    pub fn mem_percent(&self, total: u64) -> f32 {
        if total == 0 {
            return 0.0;
        }
        self.mem_used as f32 / total as f32 * 100.0
    }

    pub fn disk_percent(&self) -> f32 {
        if self.disk_total == 0 {
            return 0.0;
        }
        (self.disk_total - self.disk_free) as f32 / self.disk_total as f32 * 100.0
    }

    /// Load per core, which is the form that means the same thing on every machine:
    /// 1.0 is "as much work queued as there are cores to do it".
    pub fn load_per_core(&self) -> f32 {
        if self.cores == 0 {
            return 0.0;
        }
        self.load_1 / self.cores as f32
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
    /// What crossed this link in the window just closed, each way, in bytes.
    ///
    /// Counted where every byte between two stations passes anyway — the length-
    /// prefixed frame in `infra::sync::protocol` — so this is the whole conversation
    /// rather than a sample of it: the oplog, the snapshots, the heartbeats, and any
    /// log a console asked a peer to raise.
    ///
    /// Both directions, because they are not the same conversation. A follower
    /// catching up from a leader's oplog is almost all one way, and a link that is
    /// busy inbound and silent outbound is a different situation from the reverse.
    ///
    /// Defaulted for the same reason `FrameCost::bytes` is.
    #[serde(default)]
    pub sent_bytes: u64,
    #[serde(default)]
    pub received_bytes: u64,
    /// How long that window was, so a rate can be read off it.
    #[serde(default)]
    pub window_ms: u32,
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
            frame_costs: Vec::new(),
            machine: MachineStats::default(),
            net_received: 0,
            net_sent: 0,
            net_window_ms: 0,
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
        // Absent, and not mistaken for a station whose frames took no time.
        assert!(station.frame_costs.is_empty());
        // And nothing else was lost getting there.
        assert_eq!(station.hostname, "roof");
        assert_eq!(station.cpu_percent, 8.0);
        assert_eq!(station.total_fixtures, 7);
        assert_eq!(station.output_plugins, vec!["Art-Net".to_string()]);
    }

    #[test]
    fn a_frame_cost_is_read_against_the_budget() {
        let cost = FrameCost {
            output: "House".into(),
            kind: "artnet".into(),
            mean_ms: 7.9,
            max_ms: 31.0,
            evaluating_mean_ms: 2.4,
            evaluating_max_ms: 9.0,
            frames: 80,
            window_ms: 2000,
            bytes: 0,
            packets: 0,
        };
        assert_eq!(cost.share_of_budget(25.0).round(), 32.0);
        // The mean is inside the budget and a frame still missed it, which is the
        // distinction the pair exists to make.
        assert!(cost.overran(25.0));
    }

    #[test]
    fn a_share_of_no_budget_does_not_divide_by_zero() {
        assert_eq!(FrameCost::default().share_of_budget(0.0), 0.0);
    }
}
