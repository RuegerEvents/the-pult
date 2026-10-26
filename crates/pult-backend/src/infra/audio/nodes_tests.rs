//! The arithmetic of a node's loudspeaker: when a clip is heard, what is asked of a
//! node next, and how a voice is named. The conversation with a node over a real
//! broker is `infra/devices/e2e_tests.rs`, against the simulator.

use super::*;
use pult_schema::types::devices::{NodeAudioOutput, NodeClipBudget};
use pult_schema::types::timeline::{TimelineEnd, TimelineSource};

fn running(anchor_ms: u64, position_at_anchor_ms: u64) -> Timeline {
    Timeline {
        folder: None,
        id: Uuid::from_u128(1),
        name: "Scare".into(),
        clips: Vec::new(),
        detected: None,
        source: TimelineSource::Internal,
        grid: Vec::new(),
        markers: Vec::new(),
        events: Vec::new(),
        tracks: Vec::new(),
        flashes: None,
        pending: None,
        speed_master: None,
        node_id: None,
        end: None,
        running: true,
        anchor_ms,
        position_at_anchor_ms,
        rate: 1.0,
        recording: None,
        armed_by: None,
    }
}

/// Pressed play at 10 000 with the playhead at 0: a clip two seconds in is heard at
/// 12 000 and over at 15 000, whenever the question is asked.
#[test]
fn a_clip_is_heard_where_the_anchor_puts_it() {
    let timeline = running(10_000, 0);
    assert_eq!(occurrence(&timeline, 2_000, 3_000, false, 10_000), Some((12_000, Some(15_000))));
    assert_eq!(occurrence(&timeline, 2_000, 3_000, false, 13_500), Some((12_000, Some(15_000))));
    assert_eq!(occurrence(&timeline, 2_000, 3_000, false, 15_000), None, "it has played");
}

/// A locate into the middle of a clip moves its start into the past, and the node is
/// told that start: it joins where the clip would be by now, so no offset is sent.
#[test]
fn a_locate_into_a_clip_puts_its_start_in_the_past() {
    let timeline = running(20_000, 3_000);
    assert_eq!(occurrence(&timeline, 2_000, 3_000, false, 20_000), Some((19_000, Some(22_000))));
}

#[test]
fn a_stopped_timeline_plays_nothing() {
    let mut timeline = running(0, 0);
    timeline.running = false;
    assert_eq!(occurrence(&timeline, 0, 1_000, false, 0), None);
}

/// A looped clip lasts until the timeline stops, so it has no end of its own.
#[test]
fn a_looped_clip_goes_on() {
    let timeline = running(0, 0);
    assert_eq!(occurrence(&timeline, 500, 1_000, true, 100_000), Some((500, None)));
}

/// A timeline that stops at its end cuts a clip off there, and plays nothing that
/// starts after it.
#[test]
fn a_stopping_end_cuts_a_clip_off() {
    let mut timeline = running(0, 0);
    timeline.end = Some(TimelineEnd { at_ms: 4_000, then: AtEnd::Stop });
    assert_eq!(occurrence(&timeline, 2_000, 5_000, false, 0), Some((2_000, Some(4_000))));
    assert_eq!(occurrence(&timeline, 5_000, 1_000, false, 0), None);
}

/// On a looping timeline each lap is its own time round, and the next one starts at
/// the top of the next lap — before the leader has relocated the transport and after.
#[test]
fn a_looping_end_starts_the_clip_again_each_lap() {
    let mut timeline = running(0, 0);
    timeline.end = Some(TimelineEnd { at_ms: 10_000, then: AtEnd::Loop });
    assert_eq!(occurrence(&timeline, 1_000, 2_000, false, 0), Some((1_000, Some(3_000))));
    assert_eq!(occurrence(&timeline, 1_000, 2_000, false, 5_000), Some((11_000, Some(13_000))));
    // The leader has relocated to 500 at 10 500: the same lap, the same show times.
    let relocated = {
        let mut t = timeline.clone();
        t.anchor_ms = 10_500;
        t.position_at_anchor_ms = 500;
        t
    };
    assert_eq!(occurrence(&relocated, 1_000, 2_000, false, 10_500), Some((11_000, Some(13_000))));
}

/// Fifteen characters, stable for a clip on a timeline, and different for another.
#[test]
fn a_voice_name_fits_and_is_stable() {
    let a = voice_name(Uuid::from_u128(1), Uuid::from_u128(2));
    assert!(a.len() <= 15, "{a}");
    assert_eq!(a, voice_name(Uuid::from_u128(1), Uuid::from_u128(2)));
    assert_ne!(a, voice_name(Uuid::from_u128(1), Uuid::from_u128(3)));
}

fn a_node(formats: &[&str]) -> NodeAudio {
    NodeAudio {
        rate: 48_000,
        voices: 4,
        outputs: vec![NodeAudioOutput { id: "speaker".into(), name: "Speaker".into(), kind: "amp".into(), channels: 1 }],
        clips: NodeClipBudget {
            formats: formats.iter().map(|f| f.to_string()).collect(),
            bytes_total: 10_000_000,
            bytes_free: 10_000_000,
        },
        stream: None,
    }
}

#[test]
fn a_short_clip_goes_as_pcm_and_a_long_one_as_adpcm() {
    let node = a_node(&["pcm16", "ima-adpcm"]);
    assert_eq!(encoding_for(3_000.0, &node), (Encoding::Pcm16, 48_000));
    assert_eq!(encoding_for(60_000.0, &node), (Encoding::ImaAdpcm, ADPCM_RATE));
    let pcm_only = a_node(&["pcm16"]);
    assert_eq!(encoding_for(60_000.0, &pcm_only), (Encoding::Pcm16, 48_000));
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn store(items: &[&str]) -> Store {
    Store { stored: set(items), bytes_total: 100, bytes_free: 50 }
}

/// A node nobody has asked yet is asked first.
#[test]
fn a_node_is_asked_what_it_holds_first() {
    let wanted = BTreeMap::from([("n1".to_string(), set(&["a"]))]);
    let ops = plan(&wanted, &BTreeMap::new(), &BTreeSet::new(), |_, _| true, true);
    assert_eq!(ops, vec![Op::List("n1".into())]);
}

/// Yesterday's scream goes before today's is uploaded, so it is not today's that does
/// not fit.
#[test]
fn what_nothing_wants_is_deleted_before_anything_is_uploaded() {
    let wanted = BTreeMap::from([("n1".to_string(), set(&["a"]))]);
    let stores = BTreeMap::from([("n1".to_string(), store(&["old"]))]);
    let ops = plan(&wanted, &stores, &BTreeSet::new(), |_, _| true, true);
    assert_eq!(ops, vec![Op::Delete { serial: "n1".into(), sha: "old".into() }]);

    let stores = BTreeMap::from([("n1".to_string(), store(&[]))]);
    let ops = plan(&wanted, &stores, &BTreeSet::new(), |_, _| true, true);
    assert_eq!(ops, vec![Op::Upload { serial: "n1".into(), sha: "a".into() }]);
}

/// Before the timelines have arrived nothing is wanted, and a node's whole store would
/// look unwanted.
#[test]
fn nothing_is_deleted_before_the_timelines_are_known() {
    let wanted = BTreeMap::from([("n1".to_string(), set(&[]))]);
    let stores = BTreeMap::from([("n1".to_string(), store(&["kept"]))]);
    assert!(plan(&wanted, &stores, &BTreeSet::new(), |_, _| true, false).is_empty());
}

/// One request per node, and none for a clip still being dragged.
#[test]
fn a_busy_node_and_an_unsettled_clip_wait() {
    let wanted = BTreeMap::from([("n1".to_string(), set(&["a"])), ("n2".to_string(), set(&["b"]))]);
    let stores =
        BTreeMap::from([("n1".to_string(), store(&[])), ("n2".to_string(), store(&[]))]);
    let ops = plan(&wanted, &stores, &set(&["n1"]), |_, sha| sha != "b", true);
    assert!(ops.is_empty(), "{ops:?}");
}

#[test]
fn a_deck_position_is_the_transport_with_its_end_applied() {
    let transport = pult_render::Transport { anchor_ms: 1_000, position_at_anchor_ms: 0, rate: 1.0 };
    assert_eq!(deck_position(&transport, None, 3_500.0), 2_500.0);
    let looped = Some(pult_audio::mix::End { at_ms: 1_000.0, looped: true });
    assert_eq!(deck_position(&transport, looped, 3_500.0), 500.0);
    let stopping = Some(pult_audio::mix::End { at_ms: 1_000.0, looped: false });
    assert_eq!(deck_position(&transport, stopping, 3_500.0), 1_000.0);
}

/// Frame `n` after the session's timestamp is heard `n / rate` seconds after its `at`.
#[test]
fn a_session_maps_frames_to_show_time() {
    let session = Session { at: 10_000, ts: 0, rate: 48_000 };
    assert_eq!(session.heard_at(0), 10_000.0);
    assert_eq!(session.heard_at(48_000), 11_000.0);
    assert_eq!(session.heard_at(480), 10_010.0);
}

/// A scare routed to "Area 2" plays out of the skeleton's own speaker as well as the
/// zone's channels: the zone's node members are clips a node plays, each reached the
/// way the zone says. A clip sent to one node is that node's alone.
#[test]
fn a_zone_with_node_speakers_hands_its_clips_to_those_nodes() {
    use pult_schema::types::{
        timeline::AudioDestination,
        zone::{SpeakerZone, ZoneNode},
    };
    let zone = SpeakerZone {
        folder: None,
        id: Uuid::from_u128(5),
        name: "Area 2".into(),
        color: None,
        width: 1,
        channels: Default::default(),
        nodes: vec![
            ZoneNode { serial: "a2".into(), mode: NodeAudioMode::Clip },
            ZoneNode { serial: "a2b".into(), mode: NodeAudioMode::Stream },
        ],
        notes: String::new(),
    };
    let clip = |id: u128, destination: AudioDestination| AudioClip {
        id: Uuid::from_u128(id),
        name: "Laugh".into(),
        asset: "00".into(),
        peaks: None,
        start_ms: 0,
        offset_ms: 0,
        length_ms: None,
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
        looped: false,
        destination,
    };
    let mut timeline = running(0, 0);
    timeline.clips = vec![
        clip(1, AudioDestination::Zone { zone: zone.id }),
        clip(2, AudioDestination::Node { serial: "gate".into(), mode: NodeAudioMode::Clip }),
    ];
    let found: Vec<(u128, String, NodeAudioMode)> = node_clips(&timeline, std::slice::from_ref(&zone))
        .into_iter()
        .map(|(c, serial, mode)| (c.id.as_u128(), serial, mode))
        .collect();
    assert_eq!(
        found,
        vec![
            (1, "a2".to_string(), NodeAudioMode::Clip),
            (1, "a2b".to_string(), NodeAudioMode::Stream),
            (2, "gate".to_string(), NodeAudioMode::Clip),
        ]
    );
    // A zone that has been deleted reaches no node at all.
    assert_eq!(node_clips(&timeline, &[]).len(), 1);
}
