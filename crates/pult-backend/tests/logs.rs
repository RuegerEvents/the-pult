//! Two stations, one sync link, and what each can see of the other's log.
//!
//! Everything asserted here needs a second station to exist, which is why it is
//! out here rather than beside the ring: the clamp and the unwind are the two rules
//! that fail *silently* — a clamp set wrong shows an empty escalation that reads as
//! a broken panel, and an unwind that never fires leaves a station publishing debug
//! to a console that closed hours ago. Neither would be noticed by a unit test and
//! neither announces itself in a rig.
//!
//! Each station gets its own [`logging::detached`] log rather than an installed
//! one, because `tracing_subscriber::init` is once per process and this binary
//! runs several stations. Lines are emitted directly, which is what the capture
//! layer does anyway once it has decided an event is worth keeping.

use std::time::Duration;

use pult_backend::{
    logging::{self, LogOptions},
    Config, Running,
};
use pult_schema::ws::{LogLevel, LogLine, LogSource};

/// A station with a log of its own and nothing else running.
async fn a_station(capture: LogLevel, publish: LogLevel) -> Running {
    let showfile = std::env::temp_dir().join(format!("pult-logs-{}.db", uuid::Uuid::new_v4()));
    let log = logging::detached(LogOptions {
        capture,
        publish,
        // No file: this is about what crosses the link, and five of these tests
        // running at once should not each be rotating a directory.
        file: false,
        dir: None,
    });
    pult_backend::start(Config {
        port: 0,
        sync_port: 0,
        showfile: showfile.to_string_lossy().into_owned(),
        log: Some(log),
        ..Config::default()
    })
    .await
    .expect("a station starts")
}

/// Join `joiner` to `host`, and wait until each has the other.
async fn join(host: &Running, joiner: &Running) {
    joiner
        .sync
        .connect_peer(vec![host.sync_addr], uuid::Uuid::new_v4(), uuid::Uuid::nil())
        .await
        .expect("the two stations connect");

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if host.sync.peer_count().await > 0 && joiner.sync.peer_count().await > 0 {
            return;
        }
    }
    panic!("the stations never saw each other");
}

fn say(station: &Running, level: LogLevel, message: &str) {
    station.log.as_ref().expect("a log").emit(
        level,
        "test",
        LogSource::Station,
        message.to_string(),
    );
}

/// What `watcher` currently holds from `about`.
fn heard(watcher: &Running, about: &Running) -> Vec<LogLine> {
    watcher
        .log
        .as_ref()
        .expect("a log")
        .tail(1_000, None)
        .into_iter()
        .filter(|l| l.node_id == about.node_id.0)
        .collect()
}

/// Long enough for anything that is going to arrive to have arrived.
///
/// Generous, because these tests run several stations each and several of
/// themselves at once, and every sleep in here is competing with four other
/// stations' mDNS browsing for a single-threaded runtime. Nothing waits this long
/// when it succeeds.
const PATIENCE: Duration = Duration::from_secs(5);

/// Wait until `watcher` has heard something from `about` that `want` accepts.
///
/// Polling rather than sleeping a fixed time, because everything here is
/// asynchronous — the raise crosses the link, the line is gathered for
/// `COALESCE_MS`, and then it crosses back. A fixed sleep asserts on how fast the
/// machine is; this asserts on what eventually happens.
async fn until_heard(
    watcher: &Running,
    about: &Running,
    want: impl Fn(&LogLine) -> bool,
) -> Option<LogLine> {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while tokio::time::Instant::now() < deadline {
        if let Some(line) = heard(watcher, about).into_iter().find(&want) {
            return Some(line);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    None
}

/// Say something repeatedly until it is heard, or give up.
///
/// For the assertions that follow a raise: the raise is in flight, and the only
/// honest way to know it has landed is that a line it would let through arrives.
/// Saying it once and sleeping tests the timing of the link instead of the rule.
async fn until_heard_saying(
    watcher: &Running,
    about: &Running,
    level: LogLevel,
    message: &str,
) -> bool {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while tokio::time::Instant::now() < deadline {
        say(about, level, message);
        tokio::time::sleep(Duration::from_millis(logging::COALESCE_MS * 2)).await;
        if heard(watcher, about).iter().any(|l| l.message == message) {
            return true;
        }
    }
    false
}

/// How many rounds of silence count as having gone quiet.
const CLEAN_ROUNDS: u32 = 3;

/// Say something over and over, and satisfy ourselves that it stops arriving.
///
/// The negative of [`until_heard_saying`], and it is deliberately "goes quiet and
/// stays quiet" rather than "is quiet at once". When this follows a withdrawal the
/// withdrawal is still in flight, and the lines said in that window *should* still
/// arrive — asserting they do not would be asserting that a message crosses a
/// network instantly. Each round says something no previous round said, so a line
/// that arrives is this round's and not an echo of the last.
async fn stays_quiet(watcher: &Running, about: &Running, level: LogLevel, message: &str) -> bool {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    let (mut round, mut clean) = (0, 0);
    while tokio::time::Instant::now() < deadline {
        round += 1;
        let this = format!("{message} {round}");
        say(about, level, &this);
        tokio::time::sleep(Duration::from_millis(logging::COALESCE_MS * 2)).await;

        clean = if heard(watcher, about).iter().any(|l| l.message == this) { 0 } else { clean + 1 };
        if clean >= CLEAN_ROUNDS {
            return true;
        }
    }
    false
}

#[tokio::test]
async fn a_peers_warnings_arrive_without_anybody_asking() {
    let booth = a_station(LogLevel::Info, LogLevel::Warn).await;
    let roof = a_station(LogLevel::Info, LogLevel::Warn).await;
    join(&booth, &roof).await;

    assert!(
        until_heard_saying(&booth, &roof, LogLevel::Warn, "artnet: bind failed").await,
        "a peer's warning is what the booth is always told, with nobody having to ask"
    );
    assert!(
        stays_quiet(&booth, &roof, LogLevel::Info, "cue 4 taken").await,
        "and its chatter is not, or a rig's own network carries every console's log"
    );
}

#[tokio::test]
async fn a_line_keeps_the_seq_and_the_station_of_whoever_wrote_it() {
    let booth = a_station(LogLevel::Info, LogLevel::Warn).await;
    let roof = a_station(LogLevel::Info, LogLevel::Warn).await;
    join(&booth, &roof).await;

    say(&roof, LogLevel::Error, "first");
    say(&roof, LogLevel::Error, "second");

    until_heard(&booth, &roof, |l| l.message == "second").await.expect("the second line arrives");

    let seen = heard(&booth, &roof);
    let first = seen.iter().find(|l| l.message == "first").expect("the first line too");
    let second = seen.iter().find(|l| l.message == "second").expect("and the second");
    assert!(first.seq < second.seq, "the peer's own numbering, in the order it wrote them");
    assert!(
        seen.iter().all(|l| l.node_id == roof.node_id.0),
        "attributed to the station that said it, not to the one that heard it"
    );
}

#[tokio::test]
async fn watching_a_peer_raises_it_and_letting_go_lowers_it_again() {
    let booth = a_station(LogLevel::Info, LogLevel::Warn).await;
    // The roof captures at debug, so there is something for a raise to reach.
    let roof = a_station(LogLevel::Debug, LogLevel::Warn).await;
    join(&booth, &roof).await;

    assert!(
        stays_quiet(&booth, &roof, LogLevel::Debug, "before the raise").await,
        "nobody is watching, so the roof's debug stays at home"
    );

    // A browser opens the panel and lights the roof's chip.
    let watcher = uuid::Uuid::new_v4();
    let level = booth
        .log_watchers
        .watch(roof.node_id.0, watcher, LogLevel::Debug)
        .expect("the first watcher changes the ask");
    assert_eq!(level, Some(LogLevel::Debug));
    booth.sync.raise_peer_log(roof.node_id, level).await;

    assert!(
        until_heard_saying(&booth, &roof, LogLevel::Debug, "during the raise").await,
        "a raised peer sends its debug"
    );

    // The panel closes. Nothing expires: the ask is recomputed from who is left.
    let level = booth
        .log_watchers
        .unwatch(roof.node_id.0, watcher)
        .expect("the last watcher leaving changes the ask");
    assert_eq!(level, None, "nobody is watching, so the ask is withdrawn");
    booth.sync.raise_peer_log(roof.node_id, level).await;

    assert!(
        stays_quiet(&booth, &roof, LogLevel::Debug, "after the raise").await,
        "the roof went quiet again, with nothing having had to expire"
    );
}

#[tokio::test]
async fn a_raise_cannot_reach_past_what_the_peer_itself_keeps() {
    let booth = a_station(LogLevel::Info, LogLevel::Warn).await;
    // The roof captures at info. Its debug events are dropped by its own layer
    // before anything could forward them, so no ask can produce them.
    let roof = a_station(LogLevel::Info, LogLevel::Warn).await;
    join(&booth, &roof).await;

    booth.sync.raise_peer_log(roof.node_id, Some(LogLevel::Debug)).await;

    assert!(
        until_heard_saying(&booth, &roof, LogLevel::Info, "captured, and now published").await,
        "the raise reached as far as the peer's own capture level"
    );
    assert!(
        stays_quiet(&booth, &roof, LogLevel::Debug, "never captured").await,
        "and no further, because that line was never kept in the first place"
    );
}

#[tokio::test]
async fn a_peers_line_is_not_relayed_on_to_a_third_station() {
    // Every station is connected to every other, so a relayed line would arrive
    // twice — once from whoever wrote it and once from whoever passed it along.
    let booth = a_station(LogLevel::Info, LogLevel::Warn).await;
    let roof = a_station(LogLevel::Info, LogLevel::Warn).await;
    let foh = a_station(LogLevel::Info, LogLevel::Warn).await;
    join(&booth, &roof).await;
    join(&booth, &foh).await;

    say(&roof, LogLevel::Error, "one line, said once");
    until_heard(&booth, &roof, |l| l.message == "one line, said once")
        .await
        .expect("the booth hears it from the roof");

    let at_booth: Vec<LogLine> =
        heard(&booth, &roof).into_iter().filter(|l| l.message == "one line, said once").collect();
    assert_eq!(at_booth.len(), 1, "and hears it exactly once: {at_booth:?}");

    let at_foh: Vec<LogLine> = foh
        .log
        .as_ref()
        .expect("a log")
        .tail(1_000, None)
        .into_iter()
        .filter(|l| l.message == "one line, said once")
        .collect();
    assert!(
        at_foh.len() <= 1,
        "however it reached the FOH station, it did so once and not twice: {at_foh:?}"
    );
}
