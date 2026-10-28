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
    assert_eq!(occurrence(&timeline, 2_000, 3_000, 1, 10_000), Some((12_000, Some(15_000))));
    assert_eq!(occurrence(&timeline, 2_000, 3_000, 1, 13_500), Some((12_000, Some(15_000))));
    assert_eq!(occurrence(&timeline, 2_000, 3_000, 1, 15_000), None, "it has played");
}

/// A locate into the middle of a clip moves its start into the past, and the node is
/// told that start: it joins where the clip would be by now, so no offset is sent.
#[test]
fn a_locate_into_a_clip_puts_its_start_in_the_past() {
    let timeline = running(20_000, 3_000);
    assert_eq!(occurrence(&timeline, 2_000, 3_000, 1, 20_000), Some((19_000, Some(22_000))));
}

#[test]
fn a_stopped_timeline_plays_nothing() {
    let mut timeline = running(0, 0);
    timeline.running = false;
    assert_eq!(occurrence(&timeline, 0, 1_000, 1, 0), None);
}

/// A repeated clip lasts as many times round as it says, and is over after the last.
#[test]
fn a_repeated_clip_lasts_its_times_round() {
    let timeline = running(0, 0);
    assert_eq!(occurrence(&timeline, 500, 1_000, 3, 1_000), Some((500, Some(3_500))));
    assert_eq!(occurrence(&timeline, 500, 1_000, 3, 4_000), None);
}

/// A timeline that stops at its end cuts a clip off there, and plays nothing that
/// starts after it.
#[test]
fn a_stopping_end_cuts_a_clip_off() {
    let mut timeline = running(0, 0);
    timeline.end = Some(TimelineEnd { at_ms: 4_000, then: AtEnd::Stop });
    assert_eq!(occurrence(&timeline, 2_000, 5_000, 1, 0), Some((2_000, Some(4_000))));
    assert_eq!(occurrence(&timeline, 5_000, 1_000, 1, 0), None);
}

/// On a looping timeline each lap is its own time round, and the next one starts at
/// the top of the next lap — before the leader has relocated the transport and after.
#[test]
fn a_looping_end_starts_the_clip_again_each_lap() {
    let mut timeline = running(0, 0);
    timeline.end = Some(TimelineEnd { at_ms: 10_000, then: AtEnd::Loop });
    assert_eq!(occurrence(&timeline, 1_000, 2_000, 1, 0), Some((1_000, Some(3_000))));
    assert_eq!(occurrence(&timeline, 1_000, 2_000, 1, 5_000), Some((11_000, Some(13_000))));
    // The leader has relocated to 500 at 10 500: the same lap, the same show times.
    let relocated = {
        let mut t = timeline.clone();
        t.anchor_ms = 10_500;
        t.position_at_anchor_ms = 500;
        t
    };
    assert_eq!(occurrence(&relocated, 1_000, 2_000, 1, 10_500), Some((11_000, Some(13_000))));
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
            ZoneNode { serial: "a2".into(), mode: NodeAudioMode::Clip, channels: Vec::new() },
            ZoneNode { serial: "a2b".into(), mode: NodeAudioMode::Stream, channels: Vec::new() },
        ],
        level_db: None,
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
        repeats: 1,
        destination,
    };
    let mut timeline = running(0, 0);
    timeline.clips = vec![
        clip(1, AudioDestination::Zone { zone: zone.id }),
        clip(2, AudioDestination::Node { serial: "gate".into(), mode: NodeAudioMode::Clip }),
    ];
    let found: Vec<(u128, String, NodeAudioMode)> = node_clips(&timeline, std::slice::from_ref(&zone))
        .into_iter()
        .map(|(c, serial, mode, _)| (c.id.as_u128(), serial, mode))
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

// ── The stream, against a directory and a device manager that are channels ──

/// A node streamed to, listening on `port` at `ip`.
fn a_streamed_node(ip: &str, port: u16, rate: u32) -> DeviceDirectory {
    use crate::infra::devices::DeviceEntry;
    use pult_schema::types::devices::NodeStreamCaps;
    let mut audio = a_node(&["pcm16"]);
    audio.rate = rate;
    audio.stream = Some(NodeStreamCaps { payload: "L16".into(), max_buffer_ms: 1_000, port: Some(port) });
    let entry = DeviceEntry {
        ip: ip.into(),
        port: 80,
        module_type: 0,
        universes: Vec::new(),
        online: true,
        effects: None,
        audio: Some(audio),
        on_broker: true,
        name: "n1".into(),
        adopted: true,
        audio_summary: None,
        starts: 0,
        addresses: Vec::new(),
        transports: BTreeMap::new(),
        redundant_stream: false,
    };
    DeviceDirectory { entries: BTreeMap::from([("n1".to_string(), entry)]) }
}

/// The same node, having said on `net` which paths it is on.
fn on_paths(mut directory: DeviceDirectory, addresses: &[&str], redundant: bool) -> DeviceDirectory {
    for entry in directory.entries.values_mut() {
        entry.addresses = addresses.iter().map(|a| a.to_string()).collect();
        entry.redundant_stream = redundant;
    }
    directory
}

/// A leading station's audio manager streaming one running timeline to `n1`, and what
/// it asks the device manager to tell the node. The handle is the manager's life: it
/// stops when the last one goes.
struct Streaming {
    directory: watch::Sender<DeviceDirectory>,
    commands: mpsc::Receiver<crate::infra::devices::DeviceCommand>,
    _audio: super::super::AudioHandle,
}

async fn streaming(directory: DeviceDirectory) -> Streaming {
    use pult_schema::types::timeline::AudioDestination;
    let pool = Arc::new(crate::infra::showfile::open_in_memory().await.unwrap());
    let (engine, engine_handle, _broadcast) =
        crate::engine::ShowEngine::new(crate::NodeId(Uuid::new_v4()), pool, None);
    tokio::spawn(engine.run());
    let (directory_tx, directory_rx) = watch::channel(directory);
    let (commands_tx, commands) = mpsc::channel(256);
    let net = crate::infra::net::Network::unconfigured();
    net.set_standing(crate::infra::net::Standing { is_leader: true, sacn_slot: None });
    // No assets: the clip has no sound to decode, and the stream carries silence,
    // which is still a session and still packets.
    let (manager, audio) =
        AudioManager::new(crate::NodeId(Uuid::new_v4()), engine_handle, net, Default::default(), None);
    tokio::spawn(manager.with_nodes(directory_rx, DeviceHandle(commands_tx)).run());

    let mut timeline = running(pult_schema::clock::now_ms(), 0);
    timeline.clips = vec![AudioClip {
        id: Uuid::from_u128(2),
        name: "Bed".into(),
        asset: "00".into(),
        peaks: None,
        start_ms: 0,
        offset_ms: 0,
        length_ms: None,
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
        repeats: 1_000,
        destination: AudioDestination::Node { serial: "n1".into(), mode: NodeAudioMode::Stream },
    }];
    audio.timelines(vec![timeline]);
    Streaming { directory: directory_tx, commands, _audio: audio }
}

/// Every `audio/stream` asked for so far, as `(op, ssrc)`.
fn stream_messages(commands: &mut mpsc::Receiver<crate::infra::devices::DeviceCommand>) -> Vec<(String, Option<u64>)> {
    let mut seen = Vec::new();
    while let Ok(command) = commands.try_recv() {
        if let crate::infra::devices::DeviceCommand::Audio { verb: "stream", payload, .. } = command {
            seen.push((payload["op"].as_str().unwrap_or_default().to_string(), payload["ssrc"].as_u64()));
        }
    }
    seen
}

async fn first_start(commands: &mut mpsc::Receiver<crate::infra::devices::DeviceCommand>) -> u64 {
    for _ in 0..200 {
        if let Some((_, Some(ssrc))) = stream_messages(commands).into_iter().find(|(op, _)| op == "start") {
            return ssrc;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no stream start");
}

fn ssrc_of(packet: &[u8]) -> u64 {
    u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]) as u64
}

/// A node that moved from wifi to its cable listens on the same port on every
/// interface, so the stream follows it: the same session, sent somewhere else, with no
/// `stop` and no new `start`. Found on the floor: the address change restarted the
/// stream, and the old one's `stop` arrived after the new one's `start`.
#[tokio::test]
async fn a_node_that_changes_address_keeps_its_stream() {
    let node = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
    let port = node.local_addr().unwrap().port();
    // Somewhere this machine may or may not reach, then where the node is listening.
    let Streaming { directory, mut commands, _audio } = streaming(a_streamed_node("127.0.0.2", port, 48_000)).await;
    let ssrc = first_start(&mut commands).await;

    directory.send(a_streamed_node("127.0.0.1", port, 48_000)).unwrap();
    // Two of the manager's once-a-second passes, so the move has been seen.
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    let said = stream_messages(&mut commands);
    assert!(!said.iter().any(|(op, _)| op == "stop"), "no stop for a move: {said:?}");
    assert!(said.iter().all(|(_, s)| s.is_none_or(|s| s == ssrc)), "one session throughout: {said:?}");

    // Whatever arrived before the move is let go of; what arrives now is the same session.
    let mut packet = [0u8; 2_048];
    while node.try_recv(&mut packet).is_ok() {}
    let n = tokio::time::timeout(Duration::from_secs(2), node.recv(&mut packet))
        .await
        .expect("packets at the node's new address")
        .unwrap();
    assert_eq!(ssrc_of(&packet[..n]), ssrc);
}

/// A node now at another rate needs a session of its own, and the old one's `stop`
/// is said before the new one's `start`: a `stop` names no session, so one arriving
/// second stops the new stream.
#[tokio::test]
async fn a_new_session_is_started_only_after_the_old_one_has_stopped() {
    let Streaming { directory, mut commands, _audio } = streaming(a_streamed_node("127.0.0.1", 9, 48_000)).await;
    let old = first_start(&mut commands).await;

    directory.send(a_streamed_node("127.0.0.1", 9, 44_100)).unwrap();
    let mut said = Vec::new();
    for _ in 0..200 {
        said.extend(stream_messages(&mut commands));
        if said.iter().any(|(op, s)| op == "start" && *s != Some(old)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let stop = said.iter().position(|(op, _)| op == "stop").expect("the old session stopped");
    let new = said.iter().position(|(op, s)| op == "start" && *s != Some(old)).expect("a new session");
    assert!(stop < new, "stop before the new start: {said:?}");
    assert!(!said[stop..].iter().any(|(_, s)| *s == Some(old)), "nothing of the old session after its stop: {said:?}");
}

// ── Two paths ──────────────────────────────────────────────────────────────────

fn targets(addresses: &[&str]) -> Vec<SocketAddr> {
    addresses.iter().map(|a| format!("{a}:5004").parse().unwrap()).collect()
}

/// The preferred path, or every path for a node whose fixture asks for a redundant
/// stream, or where mDNS found it for firmware that does not say on `net`.
#[test]
fn a_stream_goes_where_the_node_says_it_is() {
    let directory = a_streamed_node("10.0.0.9", 5004, 48_000);
    let entry = |d: &DeviceDirectory| d.entries["n1"].clone();

    assert_eq!(stream_targets(&entry(&directory), 5004), targets(&["10.0.0.9"]), "no `net`: mDNS's address");

    let both = on_paths(directory.clone(), &["192.168.43.138", "192.168.43.134"], false);
    assert_eq!(stream_targets(&entry(&both), 5004), targets(&["192.168.43.138"]), "the preferred path");

    let redundant = on_paths(directory.clone(), &["192.168.43.138", "192.168.43.134"], true);
    assert_eq!(stream_targets(&entry(&redundant), 5004), targets(&["192.168.43.138", "192.168.43.134"]));

    let odd = on_paths(directory, &["nonsense", "192.168.43.134", "192.168.43.134"], true);
    assert_eq!(stream_targets(&entry(&odd), 5004), targets(&["192.168.43.134"]), "once each, and only addresses");
}

/// A redundant stream is every packet to every path: the same packet, so the node can
/// keep whichever copy comes first.
#[tokio::test]
async fn a_redundant_stream_sends_every_packet_down_every_path() {
    let cable = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let wifi = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (commands_tx, _commands) = mpsc::channel(64);
    let to = vec![cable.local_addr().unwrap(), wifi.local_addr().unwrap()];
    let streamer = Streamer::start(
        "n1".into(), to, 5004, 48_000, 2, 20, DeviceHandle(commands_tx), Vec::new(), String::new(),
    );

    let sequence = |socket: tokio::net::UdpSocket| async move {
        let mut seen = std::collections::BTreeSet::new();
        let mut packet = [0u8; 2_048];
        while seen.len() < 20 {
            tokio::time::timeout(Duration::from_secs(2), socket.recv(&mut packet))
                .await
                .expect("packets on this path")
                .unwrap();
            // The RTP sequence number.
            seen.insert(u16::from_be_bytes([packet[2], packet[3]]));
        }
        seen
    };
    let (on_cable, on_wifi) = tokio::join!(sequence(cable), sequence(wifi));
    streamer.stop().await;
    assert!(on_cable.intersection(&on_wifi).count() >= 10, "the same packets on both: {on_cable:?} {on_wifi:?}");
}

/// The node says on `net` that its preferred path has changed, and the very next
/// packets go there: not at the next once-a-second pass, and not as a new session.
#[tokio::test]
async fn a_stream_moves_the_moment_net_says_the_node_has() {
    let node = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
    let port = node.local_addr().unwrap().port();
    // On wifi at an address nothing here answers (TEST-NET-1), then on its cable here.
    let was = on_paths(a_streamed_node("192.0.2.1", port, 48_000), &["192.0.2.1"], false);
    let Streaming { directory, mut commands, _audio } = streaming(was.clone()).await;
    let ssrc = first_start(&mut commands).await;

    directory.send(on_paths(was, &["127.0.0.1", "192.0.2.1"], false)).unwrap();
    let mut packet = [0u8; 2_048];
    let n = tokio::time::timeout(Duration::from_millis(250), node.recv(&mut packet))
        .await
        .expect("packets at the new address well inside a second")
        .unwrap();
    assert_eq!(ssrc_of(&packet[..n]), ssrc, "the same session");

    let said = stream_messages(&mut commands);
    assert!(said.iter().all(|(op, s)| op == "start" && *s == Some(ssrc)), "no stop and no new start: {said:?}");
}
