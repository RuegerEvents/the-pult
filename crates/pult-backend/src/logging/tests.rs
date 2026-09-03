use pult_schema::ws::{LogLevel, LogLine, LogSource};

use super::*;

/// A directory of this test's own, in the shape the rest of the crate's tests use.
fn scratch() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pult-log-test-{}", Uuid::new_v4()))
}

fn line(seq: u64, level: LogLevel) -> LogLine {
    LogLine {
        seq,
        node_id: Uuid::nil(),
        at_ms: 1_000 + seq,
        level,
        target: "test".into(),
        source: LogSource::Station,
        message: format!("line {seq}"),
    }
}

// ── The ring ──────────────────────────────────────────────────────────────────

#[test]
fn the_ring_keeps_the_newest_and_drops_the_oldest() {
    let mut ring = LogRing::new(3);
    for seq in 0..5 {
        ring.push(line(seq, LogLevel::Info));
    }
    let held: Vec<u64> = ring.tail(10, None).iter().map(|l| l.seq).collect();
    assert_eq!(held, vec![2, 3, 4], "a full ring drops from the front");
}

#[test]
fn a_tail_comes_back_oldest_first_so_a_panel_can_append_it() {
    let mut ring = LogRing::new(10);
    for seq in 0..5 {
        ring.push(line(seq, LogLevel::Info));
    }
    let held: Vec<u64> = ring.tail(3, None).iter().map(|l| l.seq).collect();
    assert_eq!(held, vec![2, 3, 4], "the newest three, in the order they happened");
}

#[test]
fn a_tail_at_a_level_takes_the_newest_that_pass_not_the_newest_overall() {
    let mut ring = LogRing::new(10);
    ring.push(line(0, LogLevel::Warn));
    ring.push(line(1, LogLevel::Warn));
    for seq in 2..8 {
        ring.push(line(seq, LogLevel::Debug));
    }
    let held: Vec<u64> = ring.tail(2, Some(LogLevel::Warn)).iter().map(|l| l.seq).collect();
    assert_eq!(held, vec![0, 1], "the limit applies after the level, not before it");
}

// ── Capture ───────────────────────────────────────────────────────────────────

#[test]
fn nothing_above_the_capture_level_is_kept() {
    let handle = LogHandle::for_test();
    handle.set_capture_level(LogLevel::Warn);

    handle.emit(LogLevel::Debug, "t", LogSource::Station, "chatter".into());
    handle.emit(LogLevel::Error, "t", LogSource::Station, "trouble".into());

    let held = handle.tail(10, None);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].message, "trouble");
}

#[test]
fn a_seq_is_only_spent_on_a_line_that_was_kept() {
    let handle = LogHandle::for_test();
    handle.set_capture_level(LogLevel::Warn);

    handle.emit(LogLevel::Warn, "t", LogSource::Station, "one".into());
    handle.emit(LogLevel::Debug, "t", LogSource::Station, "dropped".into());
    handle.emit(LogLevel::Warn, "t", LogSource::Station, "two".into());

    let seqs: Vec<u64> = handle.tail(10, None).iter().map(|l| l.seq).collect();
    assert_eq!(seqs, vec![0, 1], "a gap in seq means a line was lost, not filtered");
}

#[test]
fn a_line_carries_the_station_that_said_it() {
    let handle = LogHandle::for_test();
    let id = Uuid::new_v4();
    handle.set_node_id(id);
    handle.emit(LogLevel::Info, "t", LogSource::Station, "hello".into());
    assert_eq!(handle.tail(1, None)[0].node_id, id);
}

// ── Publishing, and the clamp ─────────────────────────────────────────────────

#[test]
fn a_peers_lines_are_kept_but_never_written_to_this_stations_file() {
    let handle = LogHandle::for_test();
    let theirs = LogLine { node_id: Uuid::new_v4(), ..line(41, LogLevel::Warn) };
    handle.accept_from_peer(theirs.clone());

    let held = handle.tail(10, None);
    assert_eq!(held, vec![theirs], "a peer's line keeps its own seq and clock");
    assert!(handle.file_path().is_none(), "this test station has no file to write to");
}

#[test]
fn a_peer_that_asks_for_nothing_gets_the_publish_level() {
    let handle = LogHandle::for_test();
    handle.set_capture_level(LogLevel::Debug);
    handle.set_publish_level(LogLevel::Warn);
    assert_eq!(handle.publish_level_for(None), LogLevel::Warn);
}

#[test]
fn a_raise_is_clamped_to_what_this_station_captures() {
    let handle = LogHandle::for_test();
    handle.set_capture_level(LogLevel::Info);
    handle.set_publish_level(LogLevel::Warn);

    // Asked for debug, but nothing at debug was ever kept to send.
    assert_eq!(
        handle.publish_level_for(Some(LogLevel::Debug)),
        LogLevel::Info,
        "a station cannot publish what it never captured"
    );
}

#[test]
fn a_raise_never_makes_a_station_quieter_than_it_promised() {
    let handle = LogHandle::for_test();
    handle.set_capture_level(LogLevel::Debug);
    handle.set_publish_level(LogLevel::Warn);

    // A peer asking only for errors does not stop the warnings everyone gets.
    assert_eq!(
        handle.publish_level_for(Some(LogLevel::Error)),
        LogLevel::Warn,
        "peer_log_level is a floor, and a raise only ever raises"
    );
}

#[test]
fn a_raise_within_the_capture_level_is_honoured_exactly() {
    let handle = LogHandle::for_test();
    handle.set_capture_level(LogLevel::Trace);
    handle.set_publish_level(LogLevel::Warn);
    assert_eq!(handle.publish_level_for(Some(LogLevel::Debug)), LogLevel::Debug);
}

// ── The file ──────────────────────────────────────────────────────────────────

#[test]
fn a_run_gets_a_file_and_the_oldest_runs_are_dropped() {
    let dir = scratch();
    let keep = 3;

    let mut written = Vec::new();
    for _ in 0..5 {
        let writer = super::file::Writer::open(&dir, keep).expect("a log file");
        written.push(writer.path().to_path_buf());
        // The stamp has milliseconds in it; two runs inside one must not collide.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let left: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable")
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(left.len(), keep, "five runs, three kept");
    for path in written.iter().skip(2) {
        assert!(left.contains(path), "the newest runs are the ones kept: {}", path.display());
    }
}

#[test]
fn what_reaches_the_file_is_this_stations_lines_and_a_plugins() {
    let dir = scratch();
    let writer = super::file::Writer::open(&dir, 2).expect("a log file");
    let path = writer.path().to_path_buf();

    writer.write(&LogLine {
        source: LogSource::Plugin("command-line".into()),
        message: "parsed 3 tokens".into(),
        ..line(0, LogLevel::Info)
    });
    drop(writer);
    // The writing thread ends when its channel closes; give it that moment.
    std::thread::sleep(std::time::Duration::from_millis(50));

    let text = std::fs::read_to_string(&path).expect("the file");
    assert!(text.contains("parsed 3 tokens"), "the message: {text}");
    assert!(text.contains("plugin=command-line"), "and who said it: {text}");
}

// ── Who is watching whom ──────────────────────────────────────────────────────

#[test]
fn the_first_watcher_raises_a_peer_and_the_last_one_to_go_lowers_it() {
    let watchers = Watchers::default();
    let peer = Uuid::new_v4();
    let booth = Uuid::new_v4();

    assert_eq!(watchers.watch(peer, booth, LogLevel::Debug), Some(Some(LogLevel::Debug)));
    assert_eq!(watchers.unwatch(peer, booth), Some(None), "nobody left, so stop");
}

#[test]
fn a_second_watcher_asking_for_no_more_changes_nothing() {
    let watchers = Watchers::default();
    let peer = Uuid::new_v4();
    let (booth, tablet) = (Uuid::new_v4(), Uuid::new_v4());

    watchers.watch(peer, booth, LogLevel::Debug);
    assert_eq!(
        watchers.watch(peer, tablet, LogLevel::Info),
        None,
        "the peer is already sending everything this one wants"
    );
    assert_eq!(
        watchers.unwatch(peer, tablet),
        None,
        "and the louder watcher is still there, so nothing changes"
    );
    assert_eq!(watchers.unwatch(peer, booth), Some(None));
}

#[test]
fn two_watchers_get_the_louder_of_what_they_asked_for() {
    let watchers = Watchers::default();
    let peer = Uuid::new_v4();
    let (booth, tablet) = (Uuid::new_v4(), Uuid::new_v4());

    watchers.watch(peer, booth, LogLevel::Info);
    assert_eq!(watchers.watch(peer, tablet, LogLevel::Trace), Some(Some(LogLevel::Trace)));
    // The louder one leaves; the quieter one's ask is what is left.
    assert_eq!(watchers.unwatch(peer, tablet), Some(Some(LogLevel::Info)));
}

#[test]
fn a_browser_that_vanishes_stops_watching_every_peer_it_was_watching() {
    let watchers = Watchers::default();
    let (roof, foh) = (Uuid::new_v4(), Uuid::new_v4());
    let (booth, tablet) = (Uuid::new_v4(), Uuid::new_v4());

    watchers.watch(roof, booth, LogLevel::Debug);
    watchers.watch(foh, booth, LogLevel::Debug);
    // Somebody else is still watching the roof, so only one of the two goes quiet.
    watchers.watch(roof, tablet, LogLevel::Warn);

    let dropped: std::collections::HashMap<Uuid, Option<LogLevel>> =
        watchers.forget_session(booth).into_iter().collect();

    assert_eq!(dropped.get(&foh), Some(&None), "nobody else was watching the FOH station");
    assert_eq!(
        dropped.get(&roof),
        Some(&Some(LogLevel::Warn)),
        "the tablet is still watching the roof, so it drops to what the tablet asked for"
    );
}
