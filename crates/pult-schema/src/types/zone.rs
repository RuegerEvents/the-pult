//! Speaker zones: where a clip's sound goes, named for the room rather than the card.
//!
//! A clip used to name output channels, and a channel number names one machine's sound
//! card. A spare Mac promoted mid-scare then played "channel 4" of *its* card, which is
//! somebody else's speaker or nothing at all. So a clip goes to a **zone** — "Area 2",
//! "IBC row" — and each station says which of its own channels that zone is.
//!
//! **The zone is the show's and so is its map.** The map lives on the zone row, keyed by
//! station, the shape `OutputConfig::interfaces` has and for the same reason: the row
//! replicates, and channel 4 means a different speaker on every machine. Kept on the row
//! rather than in each station's preferences because a show that travels keeps its
//! zones and their colors, and a show opened on two new Macs at the next venue needs
//! only re-mapping — which the panel can say is missing, by station, because the map
//! is where everybody can read it. A station the map does not name has **not mapped
//! the zone**, and a clip going there is a fault on that station's status row, never a
//! fold onto channels it has.
//!
//! **A zone may hold node loudspeakers too.** "Area 2" is Dante channel 4 *and* the
//! skeleton's own speaker on the Area 2 node, and a scare routed to the zone plays out
//! of both. A node is the same node from every station, so node members are not keyed
//! by station: whichever station drives the nodes plays them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    events::operation::NodeId,
    types::timeline::{AudioDestination, NodeAudioMode},
    PultSchema,
};

/// A node's loudspeaker, as a member of a zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ZoneNode {
    /// The node's serial: the key the device list and every MQTT topic use.
    pub serial: String,
    /// Stored on the node or streamed to it — chosen here rather than per clip,
    /// because it is a fact about how that node is reached in this room.
    #[serde(default)]
    pub mode: NodeAudioMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "speaker_zones")]
pub struct SpeakerZone {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// A CSS color, or `None` for the category's own.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub color: Option<String>,
    /// How many channels the zone is: 1 mono, 2 stereo, more for a cluster. What a
    /// station maps has to be exactly this many, so a stereo bed cannot quietly become
    /// a downmix because somebody ticked one box of two.
    #[pult(lifecycle = PERSISTED)]
    pub width: u16,
    /// Where each of the zone's channels comes out on each station: per station, one
    /// list per zone channel (left, right, …) of that station's output channels,
    /// counting from one. A list may hold several outputs — a bed on all eight Dante
    /// channels — and an output may be in two lists, which sums them there.
    ///
    /// A show from before there were lists reads `[1, 2]` as left on 1 and right on 2
    /// (`one_per_side`), which is what it meant.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub channels: BTreeMap<NodeId, Vec<Outputs>>,
    /// Node loudspeakers that are in the zone as well.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub nodes: Vec<ZoneNode>,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub notes: String,
    /// The pool folder it is in, or `None` for the top level. See [`crate::types::folder`].
    #[serde(default)]
    #[ts(optional = nullable)]
    #[pult(lifecycle = PERSISTED)]
    pub folder: Option<Uuid>,
}

impl SpeakerZone {
    /// "mono", "stereo", "4 channels".
    pub fn width_name(&self) -> String {
        match self.width {
            1 => "mono".into(),
            2 => "stereo".into(),
            n => format!("{n} channels"),
        }
    }

    /// Does any station mix this zone, or is it only node loudspeakers?
    fn has_station_sound(&self) -> bool {
        !self.channels.is_empty() || self.nodes.is_empty()
    }

    /// Whether a station has mapped every one of the zone's channels somewhere.
    pub fn mapped_on(&self, station: NodeId) -> bool {
        self.channels.get(&station).is_some_and(|sides| {
            sides.len() == self.width as usize && sides.iter().all(|outputs| !outputs.is_empty())
        })
    }
}

/// The outputs one zone channel comes out of on one station, counting from one.
///
/// Its own type rather than a `Vec<u16>` because of how a show is read: the SQLite path
/// parses a column by its *type*, so a field attribute would be skipped there, and a
/// show from before there were lists — `[1, 2]`, one output per zone channel — would
/// stop the show opening. This reads both, and is always written as a list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(transparent)]
#[ts(export, type = "Array<number>")]
pub struct Outputs(pub Vec<u16>);

impl<'de> Deserialize<'de> for Outputs {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Side {
            Many(Vec<u16>),
            One(u16),
        }
        Ok(match Side::deserialize(d)? {
            Side::Many(outputs) => Outputs(outputs),
            Side::One(output) => Outputs(vec![output]),
        })
    }
}

impl std::ops::Deref for Outputs {
    type Target = Vec<u16>;
    fn deref(&self) -> &Vec<u16> {
        &self.0
    }
}

/// One output per zone channel, the way a map was written before there were lists.
pub fn one_per_side(outputs: &[u16]) -> Vec<Outputs> {
    outputs.iter().map(|o| Outputs(vec![*o])).collect()
}

/// What a clip's destination comes to on one station's own sound card.
///
/// `None` where the station mixes nothing for it — a clip going to one node, or to a
/// zone that is only node loudspeakers. `Some(Err)` is a fault the status row says by
/// name: a zone that has been deleted, one this station has not mapped, one with a
/// channel left going nowhere. The answer is a list of outputs per zone channel.
pub fn station_channels(
    destination: &AudioDestination,
    zones: &[SpeakerZone],
    station: NodeId,
    station_name: &str,
) -> Option<Result<Vec<Vec<u16>>, String>> {
    let AudioDestination::Zone { zone } = destination else { return None };
    let Some(zone) = zones.iter().find(|z| z.id == *zone) else {
        return Some(Err("goes to a speaker zone that has been deleted".into()));
    };
    if !zone.has_station_sound() {
        return None;
    }
    let Some(sides) = zone.channels.get(&station).filter(|c| c.iter().any(|o| !o.is_empty())) else {
        return Some(Err(format!("{} is not mapped on {station_name}", zone.name)));
    };
    if !zone.mapped_on(station) {
        let side = |i: usize| match (zone.width, i) {
            (2, 0) => "left".to_string(),
            (2, 1) => "right".to_string(),
            (_, i) => format!("channel {}", i + 1),
        };
        let missing: Vec<String> =
            (0..zone.width as usize).filter(|i| sides.get(*i).is_none_or(|o| o.is_empty())).map(side).collect();
        return Some(Err(format!(
            "{} is {} and its {} goes nowhere on {station_name}",
            zone.name,
            zone.width_name(),
            missing.join(" and "),
        )));
    }
    Some(Ok(sides[..zone.width as usize].iter().map(|o| o.0.clone()).collect()))
}

/// The node loudspeakers a clip's destination reaches, with how each is reached.
pub fn node_targets(destination: &AudioDestination, zones: &[SpeakerZone]) -> Vec<(String, NodeAudioMode)> {
    match destination {
        AudioDestination::Node { serial, mode } => vec![(serial.clone(), *mode)],
        AudioDestination::Zone { zone } => zones
            .iter()
            .find(|z| z.id == *zone)
            .map(|z| z.nodes.iter().map(|n| (n.serial.clone(), n.mode)).collect())
            .unwrap_or_default(),
    }
}

/// What one station plays through, and what it calls the card and its channels: the
/// `station_audio` collection, keyed by the station's `NodeId`.
///
/// So a zone can be mapped for a machine from another one — the booth mapping the
/// spare's channels — with the names that machine gave them. The names stay that
/// machine's preference (`[audio.names]`); this row only says what they are, and a
/// station rewrites it only when something in it moved, like `station_networks`.
///
/// **PERSISTED, so a station is remembered while it is off.** It was SYNCED, on the
/// grounds that which card is in a machine does not travel with a show — and then a
/// station that was off was nowhere in the zone matrix, and its routing could not be
/// set until it was on (HHG, 2026-10-27). The row is also the show's list of stations
/// it has seen, which is why it carries the hostname; forgetting one is deleting it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "station_audio")]
pub struct StationAudio {
    /// The station's `NodeId`.
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// What the machine is called, so a station that is off is still named.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub hostname: String,
    /// The output card, by its own name: open, or the one the station is told to use.
    #[pult(lifecycle = PERSISTED)]
    pub device: Option<String>,
    /// What the station calls it.
    #[pult(lifecycle = PERSISTED)]
    pub alias: Option<String>,
    /// How many output channels it has: the open card's count, or as many as are named
    /// when nothing is open yet.
    #[pult(lifecycle = PERSISTED)]
    pub channels: u16,
    /// The channel names, the first being channel 1; empty is unnamed.
    #[pult(lifecycle = PERSISTED)]
    pub channel_names: Vec<String>,
}

/// Whether a clip is sounding everywhere it goes, somewhere, or nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipHealth {
    /// Every member of its destination plays it.
    Fine,
    /// Some members play it and these do not, each said with what failed: an offline
    /// node beside a Dante zone that is sounding is a clip half heard, not a silent one.
    Partial(Vec<String>),
    /// Nothing plays it, and why.
    Silent(String),
}

/// Put a clip's faults against what its destination reaches: this station's channels
/// (a zone mapped somewhere) and each node loudspeaker in it. Silent only when every
/// member failed; a destination with no members at all is silent with what was said.
///
/// `station` is the fault the playing station has for the clip (a zone it has not
/// mapped, a file it cannot decode), `nodes` the faults by node serial.
pub fn clip_health(
    destination: &AudioDestination,
    zones: &[SpeakerZone],
    station: Option<&str>,
    nodes: &BTreeMap<String, String>,
) -> ClipHealth {
    if station.is_none() && nodes.is_empty() {
        return ClipHealth::Fine;
    }
    let speakers: Vec<String> = node_targets(destination, zones).into_iter().map(|(s, _)| s).collect();
    let station_member = match destination {
        AudioDestination::Zone { zone } => zones.iter().any(|z| z.id == *zone && !z.channels.is_empty()),
        AudioDestination::Node { .. } => false,
    };
    let said: Vec<String> = station.map(str::to_string).into_iter().chain(nodes.values().cloned()).collect();
    let station_plays = station_member && station.is_none();
    let a_node_plays = speakers.iter().any(|s| !nodes.contains_key(s));
    if station_plays || a_node_plays {
        ClipHealth::Partial(said)
    } else {
        ClipHealth::Silent(said.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOTH: NodeId = NodeId(Uuid::from_u128(1));
    const SPARE: NodeId = NodeId(Uuid::from_u128(2));

    fn a_zone(width: u16) -> SpeakerZone {
        SpeakerZone {
            folder: None,
            id: Uuid::from_u128(10),
            name: "Area 2".into(),
            color: None,
            width,
            channels: BTreeMap::new(),
            nodes: Vec::new(),
            notes: String::new(),
        }
    }

    fn to(zone: &SpeakerZone) -> AudioDestination {
        AudioDestination::Zone { zone: zone.id }
    }

    /// The whole reason a zone exists: the same clip, two stations, two sound cards.
    #[test]
    fn each_station_plays_a_zone_on_its_own_channels() {
        let mut zone = a_zone(1);
        zone.channels.insert(BOOTH, one_per_side(&[4]));
        zone.channels.insert(SPARE, one_per_side(&[1]));
        let zones = [zone.clone()];
        assert_eq!(station_channels(&to(&zone), &zones, BOOTH, "booth"), Some(Ok(vec![vec![4]])));
        assert_eq!(station_channels(&to(&zone), &zones, SPARE, "spare"), Some(Ok(vec![vec![1]])));
    }

    /// Unmapped is a fault by name, never a fold onto a channel this station has.
    #[test]
    fn a_station_that_has_not_mapped_the_zone_says_so() {
        let mut zone = a_zone(2);
        zone.channels.insert(BOOTH, one_per_side(&[1, 2]));
        let zones = [zone.clone()];
        assert_eq!(
            station_channels(&to(&zone), &zones, SPARE, "spare"),
            Some(Err("Area 2 is not mapped on spare".into()))
        );
    }

    #[test]
    fn a_stereo_zone_mapped_to_one_channel_is_refused() {
        let mut zone = a_zone(2);
        zone.channels.insert(BOOTH, one_per_side(&[3]));
        let zones = [zone.clone()];
        assert_eq!(
            station_channels(&to(&zone), &zones, BOOTH, "booth"),
            Some(Err("Area 2 is stereo and its right goes nowhere on booth".into()))
        );
    }

    /// One side on several outputs, and an old show's one output per side read as that.
    #[test]
    fn a_side_can_go_to_several_outputs_and_an_old_map_still_reads() {
        let mut zone = a_zone(2);
        zone.channels.insert(BOOTH, vec![Outputs(vec![1, 3, 5, 7]), Outputs(vec![2, 4, 6, 8])]);
        let zones = [zone.clone()];
        assert_eq!(
            station_channels(&to(&zone), &zones, BOOTH, "booth"),
            Some(Ok(vec![vec![1, 3, 5, 7], vec![2, 4, 6, 8]]))
        );
        let old: SpeakerZone = serde_json::from_value(serde_json::json!({
            "id": Uuid::from_u128(10), "name": "Area 2", "width": 2,
            "channels": { BOOTH.0.to_string(): [1, 2] },
        }))
        .unwrap();
        assert_eq!(old.channels.get(&BOOTH), Some(&one_per_side(&[1, 2])));
        // And by the SQLite path's own reading, which goes by the type.
        let column: BTreeMap<NodeId, Vec<Outputs>> =
            serde_json::from_str(&format!("{{\"{}\": [1, 2]}}", BOOTH.0)).unwrap();
        assert_eq!(column.get(&BOOTH), Some(&one_per_side(&[1, 2])));
    }

    #[test]
    fn a_deleted_zone_is_a_fault_and_reaches_no_node() {
        let gone = AudioDestination::Zone { zone: Uuid::from_u128(99) };
        assert!(matches!(station_channels(&gone, &[], BOOTH, "booth"), Some(Err(_))));
        assert!(node_targets(&gone, &[]).is_empty());
    }

    /// A zone that is only node loudspeakers is nothing for a station to mix, and not a
    /// fault on every station in the building.
    #[test]
    fn a_zone_of_nodes_alone_is_not_the_stations_to_mix() {
        let mut zone = a_zone(1);
        zone.nodes.push(ZoneNode { serial: "a2".into(), mode: NodeAudioMode::Clip });
        let zones = [zone.clone()];
        assert_eq!(station_channels(&to(&zone), &zones, BOOTH, "booth"), None);
        assert_eq!(node_targets(&to(&zone), &zones), vec![("a2".to_string(), NodeAudioMode::Clip)]);
    }

    /// Dante channel 4 and the skeleton's own speaker: one zone, both played.
    #[test]
    fn a_zone_can_be_channels_and_a_node_at_once() {
        let mut zone = a_zone(1);
        zone.channels.insert(BOOTH, one_per_side(&[4]));
        zone.nodes.push(ZoneNode { serial: "a2".into(), mode: NodeAudioMode::Stream });
        let zones = [zone.clone()];
        assert_eq!(station_channels(&to(&zone), &zones, BOOTH, "booth"), Some(Ok(vec![vec![4]])));
        assert_eq!(node_targets(&to(&zone), &zones), vec![("a2".to_string(), NodeAudioMode::Stream)]);
    }

    #[test]
    fn a_clip_sent_to_one_node_is_not_the_stations() {
        let node = AudioDestination::Node { serial: "a2".into(), mode: NodeAudioMode::Clip };
        assert_eq!(station_channels(&node, &[], BOOTH, "booth"), None);
        assert_eq!(node_targets(&node, &[]).len(), 1);
    }

    #[test]
    fn a_clip_is_silent_only_when_every_member_failed() {
        let mut zone = a_zone(1);
        zone.channels.insert(BOOTH, one_per_side(&[4]));
        zone.nodes = vec![ZoneNode { serial: "a2".into(), mode: NodeAudioMode::Clip }];
        let zones = vec![zone.clone()];
        let none = BTreeMap::new();
        let mut offline = BTreeMap::new();
        offline.insert("a2".to_string(), "node a2 is not on the network".to_string());

        assert_eq!(clip_health(&to(&zone), &zones, None, &none), ClipHealth::Fine);
        // The node is down and the Dante channel sounds: half heard, not silent.
        assert_eq!(
            clip_health(&to(&zone), &zones, None, &offline),
            ClipHealth::Partial(vec!["node a2 is not on the network".into()])
        );
        // Both down: nothing plays it.
        assert_eq!(
            clip_health(&to(&zone), &zones, Some("Area 2 is not mapped on booth"), &offline),
            ClipHealth::Silent("Area 2 is not mapped on booth; node a2 is not on the network".into())
        );
        // A clip going to the node alone has nothing else to sound through.
        let node = AudioDestination::Node { serial: "a2".into(), mode: NodeAudioMode::Clip };
        assert_eq!(
            clip_health(&node, &zones, None, &offline),
            ClipHealth::Silent("node a2 is not on the network".into())
        );
    }
}
