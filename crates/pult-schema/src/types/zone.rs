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
    /// Which of each station's output channels the zone is, counting from one.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub channels: BTreeMap<NodeId, Vec<u16>>,
    /// Node loudspeakers that are in the zone as well.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub nodes: Vec<ZoneNode>,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub notes: String,
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
}

/// What a clip's destination comes to on one station's own sound card.
///
/// `None` where the station mixes nothing for it — a clip going to one node, or to a
/// zone that is only node loudspeakers. `Some(Err)` is a fault the status row says by
/// name: a zone that has been deleted, one this station has not mapped, one mapped to
/// the wrong number of channels.
pub fn station_channels(
    destination: &AudioDestination,
    zones: &[SpeakerZone],
    station: NodeId,
    station_name: &str,
) -> Option<Result<Vec<u16>, String>> {
    let AudioDestination::Zone { zone } = destination else { return None };
    let Some(zone) = zones.iter().find(|z| z.id == *zone) else {
        return Some(Err("goes to a speaker zone that has been deleted".into()));
    };
    if !zone.has_station_sound() {
        return None;
    }
    let Some(channels) = zone.channels.get(&station).filter(|c| !c.is_empty()) else {
        return Some(Err(format!("{} is not mapped on {station_name}", zone.name)));
    };
    if channels.len() != zone.width as usize {
        return Some(Err(format!(
            "{} is {} and is mapped to {} channel{} on {station_name}",
            zone.name,
            zone.width_name(),
            channels.len(),
            if channels.len() == 1 { "" } else { "s" },
        )));
    }
    Some(Ok(channels.clone()))
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
/// SYNCED `station_audio` collection, keyed by the station's `NodeId`.
///
/// So a zone can be mapped for a machine from another one — the booth mapping the
/// spare's channels — with the names that machine gave them. The names stay that
/// machine's preference (`[audio.names]`); this row only says what they are, and a
/// station rewrites it only when something in it moved, like `station_networks`.
/// SYNCED and not PERSISTED for the reason that one is: which card is in a machine does
/// not travel with a show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "station_audio")]
pub struct StationAudio {
    /// The station's `NodeId`.
    #[pult(lifecycle = SYNCED, primary_key)]
    pub id: Uuid,
    /// The output card, by its own name: open, or the one the station is told to use.
    #[pult(lifecycle = SYNCED)]
    pub device: Option<String>,
    /// What the station calls it.
    #[pult(lifecycle = SYNCED)]
    pub alias: Option<String>,
    /// How many output channels it has: the open card's count, or as many as are named
    /// when nothing is open yet.
    #[pult(lifecycle = SYNCED)]
    pub channels: u16,
    /// The channel names, the first being channel 1; empty is unnamed.
    #[pult(lifecycle = SYNCED)]
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
        zone.channels.insert(BOOTH, vec![4]);
        zone.channels.insert(SPARE, vec![1]);
        let zones = [zone.clone()];
        assert_eq!(station_channels(&to(&zone), &zones, BOOTH, "booth"), Some(Ok(vec![4])));
        assert_eq!(station_channels(&to(&zone), &zones, SPARE, "spare"), Some(Ok(vec![1])));
    }

    /// Unmapped is a fault by name, never a fold onto a channel this station has.
    #[test]
    fn a_station_that_has_not_mapped_the_zone_says_so() {
        let mut zone = a_zone(2);
        zone.channels.insert(BOOTH, vec![1, 2]);
        let zones = [zone.clone()];
        assert_eq!(
            station_channels(&to(&zone), &zones, SPARE, "spare"),
            Some(Err("Area 2 is not mapped on spare".into()))
        );
    }

    #[test]
    fn a_stereo_zone_mapped_to_one_channel_is_refused() {
        let mut zone = a_zone(2);
        zone.channels.insert(BOOTH, vec![3]);
        let zones = [zone.clone()];
        assert_eq!(
            station_channels(&to(&zone), &zones, BOOTH, "booth"),
            Some(Err("Area 2 is stereo and is mapped to 1 channel on booth".into()))
        );
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
        zone.channels.insert(BOOTH, vec![4]);
        zone.nodes.push(ZoneNode { serial: "a2".into(), mode: NodeAudioMode::Stream });
        let zones = [zone.clone()];
        assert_eq!(station_channels(&to(&zone), &zones, BOOTH, "booth"), Some(Ok(vec![4])));
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
        zone.channels.insert(BOOTH, vec![4]);
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
