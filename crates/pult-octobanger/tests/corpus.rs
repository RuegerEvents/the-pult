//! Every controller file this was built against, read and held to what is in it.
//!
//! Six real files: the sample project the app ships with, and five props from the Heinz
//! Horror Garten. The figures were read out of the files by hand first, with the
//! firmware beside them, and these tests are that reading written down.

use std::io::Write;

use pult_octobanger::{Controller, Error, MediaKind, PinMapping, Project, TimingOffset, TriggerLevel};

fn read(name: &str) -> Controller {
    let path = format!("{}/../../testdata/octobanger/{name}.OBC", env!("CARGO_MANIFEST_DIR"));
    Controller::read(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn names(controller: &Controller) -> Vec<&str> {
    controller.channels.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn donkey() {
    let donkey = read("Donkey");
    assert_eq!(donkey.name, "Donkey");
    assert_eq!(donkey.comment, "Created 28.10.2021 22:45:07");
    assert_eq!(
        names(&donkey),
        ["1 - Drop", "2 - Kopfbewegung", "3 - Stoboskop", "4 - NC", "5 - NC", "6 - NC", "7 - NC", "8 - NC"]
    );
    assert_eq!(donkey.channels[0].comment, "Pneumatikzyliner für die Halterung", "UTF-8, umlaut and all");
    assert_eq!(donkey.end_ms, 5049);
    assert_eq!(donkey.frames.len(), 101, "every frame that starts at or before the end");
    assert_eq!(donkey.pin_mapping, PinMapping::DefaultTtl);
    assert_eq!(donkey.trigger, TriggerLevel::Low);
    assert_eq!(donkey.timing_offset, TimingOffset::Default);
    assert_eq!((donkey.reset_delay_s, donkey.boot_delay_s), (15, 0));
    assert_eq!(donkey.media.kind, MediaKind::Mp3AmbientScare);
    assert_eq!(donkey.media.ambient.as_deref(), Some("Heinz_Ambient.mp3"));
    assert_eq!(donkey.media.scare.as_deref(), Some("Donkey_Trigger.mp3"));
    assert_eq!((donkey.media.volume, donkey.media.delay_cs), (30, 0));

    // The drop holds for the whole scare and the head moves on top of it; the strobe
    // is never used.
    assert_eq!(donkey.spans(1), [(0, 4950)]);
    assert!(donkey.used(2));
    assert!(!donkey.used(3), "Stoboskop is named and never on");
    assert_eq!(donkey.changes(1), [(0, true), (4950, false)]);
    assert_eq!(donkey.changes(3), [(0, false)], "an unused channel is off, and says so once");
}

#[test]
fn the_sample_project_the_app_ships_with() {
    let sample = read("Sample Project using Shield");
    assert_eq!(sample.pin_mapping, PinMapping::Shield);
    assert_eq!(sample.pins().unwrap()[0], "D7", "a relay shield's first relay");
    assert_eq!(sample.trigger_pin(), Some("A0"));
    assert_eq!(names(&sample)[..5], ["Red LEDs", "Solenoid 1", "Solenoid 2", "Spotlight", "SSR 1"]);
    assert_eq!(sample.end_ms, 4519);
    assert_eq!(sample.media.ambient.as_deref(), Some("Chorus.mp3"));
    assert_eq!(sample.media.scare.as_deref(), Some("chainsaw-04.mp3"));
    assert!(sample.used(5), "the SSR comes on in the middle");
}

#[test]
fn air_whips_is_one_valve_five_times() {
    let whips = read("Air Whips");
    assert_eq!(whips.end_ms, 4999);
    assert_eq!(whips.trigger, TriggerLevel::Low);
    assert_eq!(whips.spans(1), [(0, 500), (950, 1500), (1950, 2500), (2950, 3500), (3950, 4500)]);
    assert!((2..=8).all(|channel| !whips.used(channel)));
}

#[test]
fn aussenszenerie_finishes_its_lights_long_before_its_sound() {
    let outside = read("Aussenszenerie");
    assert_eq!(outside.end_ms, 21620);
    assert_eq!(outside.trigger, TriggerLevel::Hi, "a contact to ground");
    assert_eq!(outside.spans(2), [(6850, 7350)], "the flame");
    assert_eq!(outside.spans(4), [(6450, 8550)], "the fog");
    assert_eq!((1..=8).filter(|&c| outside.used(c)).count(), 5);
}

#[test]
fn kotzender_is_active_low_where_it_says() {
    let sick = read("Kotzender");
    let low: Vec<bool> = sick.channels.iter().map(|c| c.active_low).collect();
    assert_eq!(low, [true, true, false, false, false, false, false, false]);
    assert_eq!(sick.spans(2), [(1100, 5950), (8000, 15650)], "the pump");
    assert!(!sick.used(3), "the optional light is never on");
    assert_eq!(sick.end_ms, 23109);
}

#[test]
fn schaltschrank_ignores_what_lies_past_its_end() {
    let cabinet = read("Schaltschrank");
    assert_eq!(cabinet.end_ms, 17090);
    assert_eq!(cabinet.frames.len(), 342, "frames 342 to 346 are left over from a longer edit");
    assert!((1..=8).all(|channel| cabinet.used(channel)), "all eight");
    // The beacon is on for the whole scare and goes off at the end, not at the end of a
    // frame that lies past it.
    assert_eq!(cabinet.spans(3), [(0, 17090)]);
    assert_eq!(cabinet.changes(3).last(), Some(&(17090, false)));
    assert_eq!(cabinet.spans(6).last(), Some(&(15800, 17090)), "the inside lamp too");
}

#[test]
fn every_file_reads_and_every_change_list_ascends() {
    for name in ["Donkey", "Sample Project using Shield", "Air Whips", "Aussenszenerie", "Kotzender", "Schaltschrank"] {
        let controller = read(name);
        assert_eq!(controller.channels.len(), 8, "{name}");
        for channel in 1..=8 {
            let changes = controller.changes(channel);
            assert!(changes.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 != w[1].1), "{name} channel {channel}: {changes:?}");
            assert!(changes.last().unwrap().0 <= controller.end_ms);
            assert!(!changes.last().unwrap().1, "{name} channel {channel} ends off");
        }
    }
}

#[test]
fn a_damaged_file_is_an_error_and_never_a_panic() {
    let path = format!("{}/../../testdata/octobanger/Donkey.OBC", env!("CARGO_MANIFEST_DIR"));
    let whole = std::fs::read(path).unwrap();
    for end in (0..whole.len()).step_by(7) {
        assert!(Controller::read(&whole[..end]).is_err(), "{end} bytes read as a controller");
    }
    let mut flipped = whole.clone();
    for at in (40..flipped.len()).step_by(97) {
        flipped[at] ^= 0x5a;
        let _ = Controller::read(&flipped);
    }
}

#[test]
fn the_document_format_is_named_rather_than_misread() {
    assert_eq!(Controller::read(b"<?xml version=\"1.0\"?><Controller/>"), Err(Error::DocumentFormat));
}

/// An archive the way a project travels: a folder, the controller, its two sounds — and
/// a second file of the same name somewhere else, which must not be the one found.
fn archive(with_sound: bool) -> Vec<u8> {
    let donkey = std::fs::read(format!("{}/../../testdata/octobanger/Donkey.OBC", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    out.start_file("other/Donkey_Trigger.mp3", options).unwrap();
    out.write_all(b"the wrong one").unwrap();
    out.start_file("donkey-main/Donkey.OBC", options).unwrap();
    out.write_all(&donkey).unwrap();
    out.start_file("__MACOSX/donkey-main/._Donkey.OBC", options).unwrap();
    out.write_all(b"resource fork").unwrap();
    if with_sound {
        out.start_file("donkey-main/donkey_trigger.MP3", options).unwrap();
        out.write_all(b"the right one").unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn an_archive_finds_its_sound_beside_the_controller_first() {
    let project = Project::read(&archive(true)).unwrap();
    assert_eq!(project.controllers.len(), 1, "the resource fork is not a controller");
    let found = &project.controllers[0];
    assert_eq!(found.path, "donkey-main/Donkey.OBC");
    assert_eq!(project.file(&found.path, "Donkey_Trigger.mp3").unwrap().as_deref(), Some(&b"the right one"[..]));
    assert_eq!(project.file(&found.path, "Heinz_Ambient.mp3").unwrap(), None);

    let elsewhere = Project::read(&archive(false)).unwrap();
    assert_eq!(
        elsewhere.file("donkey-main/Donkey.OBC", "Donkey_Trigger.mp3").unwrap().as_deref(),
        Some(&b"the wrong one"[..]),
        "anywhere in the archive, where it is not beside the controller"
    );
}

#[test]
fn a_bare_controller_has_no_files_beside_it() {
    let bytes = std::fs::read(format!("{}/../../testdata/octobanger/Donkey.OBC", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let project = Project::read(&bytes).unwrap();
    assert_eq!(project.controllers[0].path, "");
    assert_eq!(project.file("", "Donkey_Trigger.mp3").unwrap(), None);
}
