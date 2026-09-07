//! What can be asserted about the sound with no sound card in the machine.
//!
//! Which is more than it sounds: the arithmetic that decides *which* station plays,
//! how patient a chase is at each frame rate, and the two conversions between the
//! schema's spelling of a frame rate and the codec's. The whole of the chase — LTC in,
//! anchor out, cue fired — needs an engine and a running station and is
//! `tests/audio.rs`, for the reason `tests/timecode.rs` exists at all.

use super::*;

fn a_timeline(node: Option<NodeId>) -> Timeline {
    Timeline {
        id: Uuid::from_u128(1),
        name: "Act 1".into(),
        audio: Some("abc".into()),
        peaks: None,
        detected: None,
        source: TimelineSource::Internal,
        grid: Vec::new(),
        markers: Vec::new(),
        events: Vec::new(),
        tracks: Vec::new(),
        speed_master: None,
        node_id: node,
        running: false,
        anchor_ms: 0,
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
    }
}

fn a_manager(node_id: NodeId, leader: bool) -> AudioManager {
    let net = crate::infra::net::Network::unconfigured();
    net.set_standing(crate::infra::net::Standing { is_leader: leader, sacn_slot: None });
    // An engine nothing is reading. Nothing under test here writes to it — `plays` and
    // the conversions are pure — and a manager needs one to exist.
    let (into, _never_read) = tokio::sync::mpsc::channel(1);
    let admission = crate::engine::admission::start(into);
    let engine = crate::engine::EngineHandle::for_source(
        &admission,
        crate::engine::admission::Source::Station,
    );
    AudioManager::new(node_id, engine, net, AudioPrefs::default(), None).0
}

/// The rule an output already follows, and the reason it is the same rule: *something*
/// has to play, a lone console is the leader, and a second one joining stops the
/// double-play rather than starting a fight about it.
#[tokio::test]
async fn a_timeline_naming_no_station_is_played_by_the_leader() {
    let here = NodeId(Uuid::from_u128(1));
    let there = NodeId(Uuid::from_u128(2));

    let leader = a_manager(here, true);
    let follower = a_manager(here, false);

    assert!(leader.plays(&a_timeline(None)), "the leader plays what nobody claims");
    assert!(!follower.plays(&a_timeline(None)), "and a follower does not");

    assert!(leader.plays(&a_timeline(Some(here))));
    assert!(follower.plays(&a_timeline(Some(here))), "a named station plays it whoever leads");
    assert!(!leader.plays(&a_timeline(Some(there))), "even the leader leaves a named one alone");
}

/// Four frames, which is a different number of milliseconds at each rate. A console
/// with one constant would call 24 fps unlocked between every frame of it.
#[test]
fn patience_is_four_frames_of_the_declared_rate() {
    assert_eq!(patience_ms(LtcRate::F24), 167);
    assert_eq!(patience_ms(LtcRate::F25), 160);
    assert_eq!(patience_ms(LtcRate::F30), 133);
    // 29.97 is a little slower than 30, so four of its frames take a little longer.
    assert_eq!(patience_ms(LtcRate::F2997), 133);
    assert_eq!(patience_ms(LtcRate::F2997Df), 133);
}

/// Two spellings of five values. A wrong mapping here would run a show at 30 fps
/// against a 25 fps generator, which is 20% fast and would be blamed on everything
/// except this function.
#[test]
fn the_frame_rates_map_across_the_crate_boundary() {
    let pairs = [
        (LtcRate::F24, pult_audio::LtcRate::F24),
        (LtcRate::F25, pult_audio::LtcRate::F25),
        (LtcRate::F30, pult_audio::LtcRate::F30),
        (LtcRate::F2997, pult_audio::LtcRate::F2997),
        (LtcRate::F2997Df, pult_audio::LtcRate::F2997Df),
    ];
    for (schema, codec) in pairs {
        assert_eq!(ltc_rate(schema), codec);
    }
    // And the frame length that comes out of it, which is what an offset is measured in.
    assert!((frames_per_second(pult_audio::LtcRate::F25) - 25.0).abs() < 1e-9);
    assert!((frames_per_second(pult_audio::LtcRate::F2997) - 29.97).abs() < 0.001);
}

#[test]
fn a_lock_state_crosses_the_boundary_too() {
    assert_eq!(lock_state(pult_audio::Lock::Waiting), LtcLock::Waiting);
    assert_eq!(lock_state(pult_audio::Lock::Locked), LtcLock::Locked);
    assert_eq!(lock_state(pult_audio::Lock::Lost), LtcLock::Lost);
}

/// The tuple that tells this station's own anchor write from an operator locating.
/// `rate` is in it because a half-speed rehearsal is a different transport at the same
/// position.
#[test]
fn the_transport_is_the_three_numbers_that_move_a_playhead() {
    let mut timeline = a_timeline(None);
    timeline.anchor_ms = 5;
    timeline.position_at_anchor_ms = 7;
    assert_eq!(transport_of(&timeline), (5, 7, 1.0));
    timeline.rate = 0.5;
    assert_ne!(transport_of(&timeline), (5, 7, 1.0));
}
