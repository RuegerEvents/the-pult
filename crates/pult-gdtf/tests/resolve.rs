//! What a console asks a GDTF file, asked of the checked-in fixtures.

mod common;

use pult_gdtf::{resolve, validate, GdtfFile};

fn load(name: &str) -> GdtfFile {
    let bytes = common::zip_dir(&common::testdata().join(name));
    GdtfFile::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn a_sixteen_bit_channel_keeps_both_its_offsets() {
    let file = load("rgbw-two-mode");
    let fixture = &file.description.fixture_type;
    let mode = resolve::mode(fixture, "Standard").unwrap();
    let (channels, warnings) = resolve::expand_mode(fixture, mode);
    assert!(warnings.is_empty(), "{warnings:?}");

    let pan = channels
        .iter()
        .find(|channel| channel.attribute.map(ToString::to_string).as_deref() == Some("Pan"))
        .expect("a Pan channel");
    assert_eq!(
        pan.offsets,
        vec![1, 7],
        "the fine byte is not beside the coarse one"
    );
    assert_eq!(pan.byte_count(), 2);
    assert_eq!(
        pan.default(),
        32768,
        "the default was written at two bytes and stays there"
    );
    assert_eq!(resolve::physical_range(pan.channel), Some((-270.0, 270.0)));
}

#[test]
fn a_default_written_at_one_byte_is_rescaled_into_a_wider_channel() {
    let file = load("rgbw-two-mode");
    let fixture = &file.description.fixture_type;
    let mode = resolve::mode(fixture, "Standard").unwrap();
    let (channels, _) = resolve::expand_mode(fixture, mode);
    let zoom = channels
        .iter()
        .find(|channel| channel.attribute.map(ToString::to_string).as_deref() == Some("Zoom"))
        .unwrap();
    assert_eq!(
        zoom.default(),
        128,
        "an 8-bit default in an 8-bit channel is itself"
    );
}

#[test]
fn a_mode_with_two_breaks_reports_a_footprint_for_each() {
    let file = load("rgbw-two-mode");
    let fixture = &file.description.fixture_type;

    let standard = resolve::mode(fixture, "Standard").unwrap();
    assert_eq!(
        resolve::footprint(fixture, standard),
        vec![10, 2],
        "ten channels on break 1 — the pan fine byte is at offset 7, not 11 — and two on break 2"
    );

    let basic = resolve::mode(fixture, "Basic").unwrap();
    assert_eq!(resolve::footprint(fixture, basic), vec![4]);
}

#[test]
fn a_referenced_cell_is_counted_once_per_reference_and_not_once_more() {
    let file = load("multicell-bar");
    let fixture = &file.description.fixture_type;
    let mode = resolve::mode(fixture, "Pixel").unwrap();
    let (channels, warnings) = resolve::expand_mode(fixture, mode);
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(
        channels.len(),
        16,
        "four channels times four cells, and no template copy"
    );
    assert_eq!(resolve::footprint(fixture, mode), vec![16]);

    let mut dimmers: Vec<u16> = channels
        .iter()
        .filter(|channel| channel.attribute.map(ToString::to_string).as_deref() == Some("Dimmer"))
        .map(|channel| channel.offsets[0])
        .collect();
    dimmers.sort();
    assert_eq!(
        dimmers,
        vec![1, 5, 9, 13],
        "each cell's dimmer at its own reference's offset"
    );

    let cells: Vec<&str> = channels
        .iter()
        .filter(|channel| channel.attribute.map(ToString::to_string).as_deref() == Some("Dimmer"))
        .map(|channel| channel.geometry_path[1].as_str())
        .collect();
    assert_eq!(cells.len(), 4);
    assert!(
        cells.contains(&"Cell 1") && cells.contains(&"Cell 4"),
        "{cells:?}"
    );
}

#[test]
fn the_axes_come_out_outermost_first_and_the_beam_carries_its_angle() {
    let file = load("rgbw-two-mode");
    let fixture = &file.description.fixture_type;
    let mode = resolve::mode(fixture, "Standard").unwrap();

    let axes: Vec<&str> = resolve::axes(fixture, mode)
        .iter()
        .map(|node| node.name())
        .collect();
    assert_eq!(axes, vec!["Yoke", "Head"], "pan turns the outer one");
    assert!(resolve::axes(fixture, mode)
        .iter()
        .all(|node| node.is_axis()));

    let beam = resolve::find_beam(fixture, mode).expect("a beam");
    assert_eq!(beam.beam_angle, Some(12.0));
    assert_eq!(beam.field_angle, Some(15.0));
}

#[test]
fn an_unknown_mode_name_falls_back_to_the_first_one() {
    let file = load("rgbw-two-mode");
    let fixture = &file.description.fixture_type;
    let mode = resolve::mode(fixture, "A mode from a later revision").unwrap();
    assert_eq!(mode.name, "Standard");
}

#[test]
fn a_gobo_channel_names_its_slots_with_both_ends_of_each_range() {
    let file = load("rgbw-two-mode");
    let fixture = &file.description.fixture_type;
    let mode = resolve::mode(fixture, "Standard").unwrap();
    let (channels, _) = resolve::expand_mode(fixture, mode);
    let gobo = channels
        .iter()
        .find(|channel| channel.attribute.map(ToString::to_string).as_deref() == Some("Gobo1"))
        .unwrap();

    let sets = resolve::channel_sets(gobo.channel, gobo.byte_count());
    let names: Vec<&str> = sets.iter().map(|set| set.name).collect();
    assert_eq!(names, vec!["Open", "Breakup", "Dots"]);
    assert_eq!(
        (sets[0].from, sets[0].to),
        (0, 9),
        "a set ends where the next begins"
    );
    assert_eq!(
        (sets[2].from, sets[2].to),
        (20, 255),
        "the last one runs to full"
    );
}

#[test]
fn the_checked_in_fixtures_have_nothing_to_warn_about() {
    for name in ["minimal", "rgbw-two-mode", "multicell-bar"] {
        let file = load(name);
        let warnings = validate::check(&file.description.fixture_type);
        assert!(warnings.is_empty(), "{name}: {warnings:?}");
    }
}

#[test]
fn a_generated_file_parses_back_as_what_it_was_generated_from() {
    use pult_gdtf::minimal::{build, MinimalChannel, MinimalSpec};

    let spec = MinimalSpec {
        name: "Handmade".into(),
        short_name: "HM".into(),
        manufacturer: "Pult".into(),
        fixture_type_id: "1B1E4C3A-0000-4000-8000-00000000000F".into(),
        mode_name: "Default".into(),
        channels: vec![
            MinimalChannel {
                attribute: "Dimmer".into(),
                offsets: vec![1],
                feature: "Dimmer".into(),
                ..MinimalChannel::default()
            },
            MinimalChannel {
                attribute: "Pan".into(),
                offsets: vec![2, 3],
                default: 32768,
                physical_from: Some(-180.0),
                physical_to: Some(180.0),
                feature: "Position".into(),
                ..MinimalChannel::default()
            },
        ],
        weight_kg: Some(4.0),
        beam_angle: Some(20.0),
        ..MinimalSpec::default()
    };

    let written = build(&spec).write().unwrap();
    let file = GdtfFile::parse(&written).unwrap();
    let fixture = &file.description.fixture_type;

    assert_eq!(fixture.name, "Handmade");
    let mode = resolve::mode(fixture, "Default").unwrap();
    assert_eq!(resolve::footprint(fixture, mode), vec![3]);
    assert!(
        validate::check(fixture).is_empty(),
        "{:?}",
        validate::check(fixture)
    );

    let (channels, _) = resolve::expand_mode(fixture, mode);
    let pan = channels
        .iter()
        .find(|c| c.attribute.map(ToString::to_string).as_deref() == Some("Pan"))
        .unwrap();
    assert_eq!(pan.default(), 32768);
    assert_eq!(
        resolve::find_beam(fixture, mode).and_then(|beam| beam.beam_angle),
        Some(20.0)
    );
}
