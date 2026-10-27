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
        folder: None,
        id: Uuid::from_u128(1),
        name: "Act 1".into(),
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
        node_id: node,
        end: None,
        running: false,
        anchor_ms: 0,
        position_at_anchor_ms: 0,
        rate: 1.0,
        recording: None,
        armed_by: None,
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

const ZONE: Uuid = Uuid::from_u128(20);

/// One mono zone, mapped on the stations named.
fn a_zone(mapped: &[(NodeId, u16)]) -> SpeakerZone {
    SpeakerZone {
        folder: None,
        id: ZONE,
        name: "Area 2".into(),
        color: None,
        width: 1,
        channels: mapped.iter().map(|(node, channel)| (*node, pult_schema::types::zone::one_per_side(&[*channel]))).collect(),
        nodes: Vec::new(),
        notes: String::new(),
    }
}

fn a_clip(start_ms: u64, offset_ms: u64, length_ms: Option<u64>) -> AudioClip {
    AudioClip {
        id: Uuid::from_u128(9),
        name: "Scream".into(),
        asset: "abc".into(),
        peaks: None,
        start_ms,
        offset_ms,
        length_ms,
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
        repeats: 1,
        destination: pult_schema::types::timeline::AudioDestination::Zone { zone: ZONE },
    }
}

/// The drift is measured against where the *sound* is, and the sound wraps at a
/// looping end by itself. Measured against the unfolded transport, the moment between
/// the deck wrapping and the leader relocating would read as a whole loop's drift.
#[test]
fn a_position_past_the_end_is_folded_the_way_the_deck_folds_it() {
    let mut timeline = a_timeline(None);
    assert_eq!(folded(&timeline, 12_345), 12_345, "a timeline with no end runs on");
    timeline.end = Some(pult_schema::types::timeline::TimelineEnd { at_ms: 10_000, then: AtEnd::Loop });
    assert_eq!(folded(&timeline, 10_250), 250);
    timeline.end = Some(pult_schema::types::timeline::TimelineEnd { at_ms: 10_000, then: AtEnd::Stop });
    assert_eq!(folded(&timeline, 10_250), 10_000);
    timeline.end = Some(pult_schema::types::timeline::TimelineEnd { at_ms: 0, then: AtEnd::Loop });
    assert_eq!(folded(&timeline, 10_250), 10_250, "an end at zero is no end");
}

/// A beat found in the file lands on the timeline where the clip puts that part of the
/// file, and one in the part the trim leaves out is no beat at all.
#[test]
fn a_detected_beat_is_moved_onto_the_timeline_through_the_clip() {
    let clip = a_clip(10_000, 2_000, Some(5_000));
    assert_eq!(clip_to_timeline(&clip, 1_999), None, "before the trim");
    assert_eq!(clip_to_timeline(&clip, 2_000), Some(10_000));
    assert_eq!(clip_to_timeline(&clip, 4_500), Some(12_500));
    assert_eq!(clip_to_timeline(&clip, 7_000), None, "past the length");
}

#[test]
fn a_clips_placement_is_what_the_mixer_is_handed() {
    let mut clip = a_clip(1_000, 250, None);
    clip.gain_db = -6.0;
    clip.repeats = 3;
    let placed = placement(&clip);
    assert_eq!(placed.start_ms, 1_000.0);
    assert_eq!(placed.offset_ms, 250.0);
    assert_eq!(placed.repeats, 3);
    assert!((placed.gain - 0.501).abs() < 0.001, "-6 dB is about half");
}

/// What decides whether the device has to be open: any clip this station mixes itself.
#[tokio::test]
async fn a_timeline_with_clips_wants_the_output_and_one_without_does_not() {
    let here = NodeId(Uuid::from_u128(1));
    let mut manager = a_manager(here, true);
    manager.zones = vec![a_zone(&[(here, 3)])];
    let mut timeline = a_timeline(None);
    assert!(manager.station_clips(&timeline).is_empty());
    timeline.clips.push(a_clip(0, 0, None));
    assert_eq!(manager.station_clips(&timeline).len(), 1);
}

/// The same clip on two stations is two sound cards' channels — and on a station the
/// zone does not name, a fault by name rather than a fold onto channels it has.
#[tokio::test]
async fn each_station_resolves_a_clips_zone_through_its_own_map() {
    let booth = NodeId(Uuid::from_u128(1));
    let spare = NodeId(Uuid::from_u128(2));
    let elsewhere = NodeId(Uuid::from_u128(3));
    let zones = vec![a_zone(&[(booth, 4), (spare, 1)])];
    let mut timeline = a_timeline(None);
    timeline.clips.push(a_clip(0, 0, None));

    for (node, wanted) in [(booth, vec![vec![4]]), (spare, vec![vec![1]])] {
        let mut manager = a_manager(node, true);
        manager.zones = zones.clone();
        let clips = manager.station_clips(&timeline);
        assert_eq!(clips[0].1, Ok(wanted));
    }
    let mut manager = a_manager(elsewhere, true);
    manager.station_name = "stage-rack".into();
    manager.zones = zones;
    let clips = manager.station_clips(&timeline);
    assert_eq!(clips[0].1, Err("Area 2 is not mapped on stage-rack".to_string()));
}

/// A listening page is sent the running timeline's sound a lead ahead of the clock,
/// each chunk saying the show millisecond it belongs to; a stopped timeline sends it
/// nothing; and a page that has gone is forgotten at the next chunk.
#[tokio::test]
async fn a_listening_page_hears_the_running_timeline_ahead_of_the_clock() {
    let mut manager = a_manager(NodeId(Uuid::from_u128(1)), false);
    // Ten seconds of a steady half-level tone, decoded into the cache as a file would be.
    let tone: Vec<f32> = (0..48_000 * 10).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
    let wav = pult_audio::wav::encode(&tone, 1, 48_000, pult_audio::wav::Encoding::Pcm16).unwrap();
    manager.cache.decode("abc", wav).await.expect("it decodes");

    let now = 1_000_000;
    let mut playing = a_timeline(None);
    playing.clips = vec![a_clip(0, 0, None)];
    playing.running = true;
    playing.anchor_ms = now - 2_000;
    manager.timelines = vec![playing.clone()];

    let (tx, mut rx) = tokio::sync::mpsc::channel(monitor::QUEUE);
    manager.monitors.push(monitor::Listener::new(None, tx));
    manager.feed_monitors(now).await;
    let mut heard = Vec::new();
    while let Ok(bytes) = rx.try_recv() {
        heard.push(pult_audio::monitor::decode(&bytes).expect("a chunk"));
    }
    assert!(!heard.is_empty(), "something was sent");
    assert!(heard.iter().all(|c| c.at_ms >= now as f64 && c.at_ms < (now + pult_audio::monitor::LEAD_MS) as f64));
    assert!(heard.iter().all(|c| c.samples.iter().any(|s| s.abs() > 0.1)), "and it is the tone, not silence");

    // Stopped: nothing more is owed.
    manager.timelines[0].running = false;
    manager.feed_monitors(now + 500).await;
    assert!(rx.try_recv().is_err(), "a stopped timeline sends nothing");

    // Gone: forgotten.
    drop(rx);
    manager.timelines[0].running = true;
    manager.feed_monitors(now + 1_000).await;
    assert!(manager.monitors.is_empty(), "a page that went is not rendered for");
}

/// A page starting to listen mid-show must never stall the station playing the show:
/// its chase, its anchor writes and its node streams all run on the manager's loop. So
/// a file not decoded here is loaded on a task of its own and the tick returns at once,
/// its chunks saying "loading". Here the load can never finish — the file is in no
/// store and the engine asked for peers answers nobody — and the tick still returns.
#[tokio::test]
async fn a_listener_on_an_undecoded_timeline_does_not_hold_up_the_tick() {
    let mut manager = a_manager(NodeId(Uuid::from_u128(1)), false);
    let pool = std::sync::Arc::new(crate::infra::showfile::open_in_memory().await.unwrap());
    let dir = std::env::temp_dir().join(format!("pult-monitor-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    manager.assets = Some(crate::infra::assets::AssetStore::new(Some(dir.clone()), pool));

    let now = 1_000_000;
    let mut playing = a_timeline(None);
    playing.clips = vec![a_clip(0, 0, None)];
    playing.running = true;
    playing.anchor_ms = now - 2_000;
    manager.timelines = vec![playing];

    let (tx, mut rx) = tokio::sync::mpsc::channel(monitor::QUEUE);
    manager.monitors.push(monitor::Listener::new(None, tx));
    let ticked = tokio::time::timeout(std::time::Duration::from_millis(200), manager.feed_monitors(now)).await;
    assert!(ticked.is_ok(), "the tick waited on a file");
    let chunk = pult_audio::monitor::decode(&rx.try_recv().expect("a chunk")).unwrap();
    assert!(chunk.loading, "and the page is told the file is still coming");
    assert!(chunk.samples.iter().all(|s| *s == 0.0), "without the clip in it meanwhile");
    let _ = std::fs::remove_dir_all(dir);
}
