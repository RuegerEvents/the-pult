//! The merge, which is the part of an input that is arithmetic rather than a socket.

use super::*;

fn a_source(priority: u8, at: std::time::Instant, slots: &[(usize, u8)]) -> Source {
    let mut channels = [0u8; super::super::dmx::UNIVERSE_SIZE];
    for (slot, byte) in slots {
        channels[*slot] = *byte;
    }
    Source { priority, last_seen: at, channels }
}

#[test]
fn the_highest_priority_present_wins_outright() {
    let now = std::time::Instant::now();
    let mut merged = Merged::new();
    merged.sources.insert(SourceId::Addr("10.0.0.1".parse().unwrap()), a_source(100, now, &[(0, 255)]));
    merged.sources.insert(SourceId::Addr("10.0.0.2".parse().unwrap()), a_source(150, now, &[(0, 10)]));

    assert!(merged.remerge(now));
    assert_eq!(
        merged.image[0], 10,
        "the higher priority wins even though it is asking for less — which is the \
         whole reason E1.31 has the byte"
    );
}

#[test]
fn equal_priorities_take_the_highest_of_each_slot() {
    let now = std::time::Instant::now();
    let mut merged = Merged::new();
    merged
        .sources
        .insert(SourceId::Cid([1; 16]), a_source(100, now, &[(0, 255), (1, 0)]));
    merged
        .sources
        .insert(SourceId::Cid([2; 16]), a_source(100, now, &[(0, 100), (1, 200)]));

    merged.remerge(now);
    assert_eq!(&merged.image[..2], &[255, 200]);
}

/// A console being unplugged is not a console sending zeros. The first drops out of
/// the merge and lets whatever else is there through; the second wins on HTP against
/// nothing and blacks the universe out. Only the timeout can tell them apart.
#[test]
fn a_source_that_has_gone_quiet_leaves_the_merge() {
    let began = std::time::Instant::now();
    let mut merged = Merged::new();
    merged.sources.insert(SourceId::Cid([1; 16]), a_source(150, began, &[(0, 10)]));
    merged.sources.insert(SourceId::Cid([2; 16]), a_source(100, began, &[(0, 200)]));
    merged.remerge(began);
    assert_eq!(merged.image[0], 10);

    let later = began + SOURCE_TIMEOUT + std::time::Duration::from_millis(1);
    merged.sources.get_mut(&SourceId::Cid([2; 16])).unwrap().last_seen = later;
    assert!(merged.remerge(later), "the image moved when the higher priority went away");
    assert_eq!(merged.image[0], 200, "and the one still talking is what is left");
    assert_eq!(merged.sources.len(), 1);
}

#[test]
fn a_universe_nobody_is_sending_reads_as_dark_rather_than_as_the_last_frame() {
    let began = std::time::Instant::now();
    let mut merged = Merged::new();
    merged.sources.insert(SourceId::Cid([1; 16]), a_source(100, began, &[(5, 255)]));
    merged.remerge(began);

    let later = began + SOURCE_TIMEOUT * 2;
    assert!(merged.remerge(later));
    assert!(merged.image.iter().all(|byte| *byte == 0));
}

/// The dedup that makes a viewer cost nothing on a settled wire, and the one that
/// makes a recording small: a frame identical to the last one is not a change.
#[test]
fn an_unchanged_frame_is_not_a_change() {
    let now = std::time::Instant::now();
    let mut merged = Merged::new();
    merged.sources.insert(SourceId::Cid([1; 16]), a_source(100, now, &[(0, 7)]));
    assert!(merged.remerge(now));
    assert!(!merged.remerge(now), "the same sources give the same bytes");
}
