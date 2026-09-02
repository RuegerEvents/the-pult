//! A real GDTF file, read as a fixture type.
//!
//! The checked-in fixtures under `testdata/gdtf/` are the material, zipped in memory
//! the way `pult-gdtf`'s own tests build them. What is asserted here is the
//! *translation* — that a 16-bit pan comes through as two offsets, that four colour
//! channels come through as one parameter with four emitters, that a mode's footprint
//! is what the patch panel will show — because `pult-gdtf` has already proved it read
//! the file correctly.

use std::io::Write;
use std::path::{Path, PathBuf};

use pult_gdtf::GdtfFile;
use pult_schema::types::fixture::{parameter_key, ParameterKind};

use super::*;

fn testdata(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/gdtf").join(name)
}

fn load(name: &str) -> GdtfFile {
    let dir = testdata(name);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        writer.start_file(name, options).unwrap();
        writer.write_all(&std::fs::read(&path).unwrap()).unwrap();
    }
    GdtfFile::parse(&writer.finish().unwrap().into_inner()).unwrap()
}

fn derive(name: &str) -> (FixtureType, Vec<Warning>) {
    derive_fixture_type(&load(name), "a-sha256-that-stands-for-the-file")
}

#[test]
fn a_type_takes_the_files_own_identity_so_a_new_revision_updates_it() {
    let (fixture_type, warnings) = derive("rgbw-two-mode");
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(
        fixture_type.id.to_string().to_uppercase(),
        "1B1E4C3A-0000-4000-8000-000000000002",
        "the id is the file's FixtureTypeID, which is what makes a re-import an update",
    );
    assert_eq!(fixture_type.name, "Test RGBW Mover");
    assert_eq!(fixture_type.manufacturer, "Pult");
    assert_eq!(fixture_type.short_name, "TRGBW");
    match &fixture_type.source {
        FixtureTypeSource::Gdtf { asset, revision, .. } => {
            assert_eq!(asset, "a-sha256-that-stands-for-the-file");
            assert_eq!(revision, "First");
        }
        other => panic!("an imported type says where it came from, not {other:?}"),
    }
}

#[test]
fn the_parameter_list_is_what_the_light_can_do_across_every_mode() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let kinds: Vec<ParameterKind> =
        fixture_type.parameters.iter().map(|p| p.kind.clone()).collect();

    assert!(kinds.contains(&ParameterKind::Pan));
    assert!(kinds.contains(&ParameterKind::Tilt));
    assert!(kinds.contains(&ParameterKind::Zoom));
    assert!(kinds.contains(&ParameterKind::Gobo(1)));
    assert!(kinds.contains(&ParameterKind::Shutter));
    assert!(
        kinds.contains(&ParameterKind::Intensity),
        "the dimmer is on break 2 and is still a parameter",
    );

    // Four colour channels, one colour parameter.
    assert_eq!(
        kinds.iter().filter(|k| **k == ParameterKind::ColorRgb).count(),
        1,
        "a console gives an operator a picker, not four faders: {kinds:?}",
    );
}

#[test]
fn the_colour_parameter_carries_the_dies_that_make_it() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let colour = fixture_type
        .parameters
        .iter()
        .find(|p| p.kind == ParameterKind::ColorRgb)
        .expect("a colour");

    let names: Vec<&str> = colour.emitters.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Red", "Green", "Blue", "White"]);
    assert!(colour.emitters.iter().all(|e| !e.subtractive), "an LED head adds light");
    let red = &colour.emitters[0];
    let rgb = red.rgb.expect("the file measured it");
    assert!(rgb.x > rgb.y && rgb.x > rgb.z, "red points at red: {rgb:?}");
}

#[test]
fn a_pan_reads_in_degrees_rather_than_as_a_fraction() {
    use pult_schema::types::fixture::PhysicalUnit;

    let (fixture_type, _) = derive("rgbw-two-mode");
    let pan = fixture_type.parameters.iter().find(|p| p.kind == ParameterKind::Pan).unwrap();
    let range = pan.physical.expect("the file said how far it turns");
    assert_eq!((range.from, range.to), (-270.0, 270.0));
    assert_eq!(range.unit, PhysicalUnit::Degrees);
    assert_eq!(range.to_physical(0.5), 0.0, "the middle is straight ahead");
}

#[test]
fn a_gobo_wheel_brings_its_slots_by_name() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let gobo = fixture_type.parameters.iter().find(|p| p.kind == ParameterKind::Gobo(1)).unwrap();
    let names: Vec<&str> = gobo.slots.iter().map(|slot| slot.name.as_str()).collect();
    assert_eq!(names, vec!["Open", "Breakup", "Dots"]);
    assert_eq!(gobo.slots[1].media.as_deref(), Some("breakup"));
}

#[test]
fn every_mode_comes_through_with_its_own_footprint() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let names: Vec<&str> = fixture_type.dmx_modes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["Standard", "Basic"]);

    assert_eq!(fixture_type.footprint("Standard"), vec![10, 2], "ten on break 1, two on break 2");
    assert_eq!(fixture_type.footprint("Basic"), vec![4]);
    assert_eq!(fixture_type.channel_count, 10, "what the patch panel shows");
}

#[test]
fn a_sixteen_bit_pan_keeps_both_its_bytes_and_its_default() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let mode = fixture_type.mode("Standard");
    let pan = mode
        .channels
        .iter()
        .find(|channel| channel.parameter_key == parameter_key(&ParameterKind::Pan))
        .unwrap();

    assert_eq!(pan.offsets, vec![1, 7], "the fine byte is not beside the coarse one");
    assert_eq!(pan.byte_count(), 2);
    assert_eq!(pan.default, 32768);
    assert_eq!(pan.break_index, 0);
}

#[test]
fn a_second_break_becomes_a_second_index_rather_than_a_second_universe() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let mode = fixture_type.mode("Standard");
    let dimmer = mode
        .channels
        .iter()
        .find(|channel| channel.parameter_key == parameter_key(&ParameterKind::Intensity))
        .unwrap();
    assert_eq!(dimmer.break_index, 1, "which universe that is is the fixture's to say");
    assert_eq!(dimmer.offsets, vec![1]);
}

#[test]
fn each_colour_channel_says_which_die_it_drives() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let mode = fixture_type.mode("Standard");
    let colour: Vec<(&str, Vec<u16>)> = mode
        .channels
        .iter()
        .filter(|channel| channel.parameter_key == parameter_key(&ParameterKind::ColorRgb))
        .map(|channel| (channel.emitter.as_deref().unwrap_or(""), channel.offsets.clone()))
        .collect();
    assert_eq!(
        colour,
        vec![
            ("Red", vec![3]),
            ("Green", vec![4]),
            ("Blue", vec![5]),
            ("White", vec![6]),
        ],
    );
}

#[test]
fn a_gobo_channels_slots_arrive_as_ranges_with_both_ends() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let mode = fixture_type.mode("Standard");
    let gobo = mode
        .channels
        .iter()
        .find(|channel| channel.parameter_key == parameter_key(&ParameterKind::Gobo(1)))
        .unwrap();
    let ranges: Vec<(&str, u32, u32)> = gobo
        .functions
        .iter()
        .map(|range| (range.name.as_str(), range.dmx_from, range.dmx_to))
        .collect();
    assert_eq!(ranges, vec![("Open", 0, 9), ("Breakup", 10, 19), ("Dots", 20, 255)]);
}

#[test]
fn the_paperwork_comes_across() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let physical = &fixture_type.physical;
    assert_eq!(physical.weight_kg, Some(18.5));
    assert_eq!(physical.power_w, Some(450.0));
    assert_eq!(physical.operating_temperature, Some((-10.0, 45.0)));
    assert_eq!(physical.beam_angle_deg, Some(12.0), "the rig view's cone, from the file");
    assert_eq!(
        physical.connectors.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["DMX In", "Power"],
    );
    assert!(physical.dimensions_m.is_some(), "measured across the whole fixture");
}

/// How big the fixture is, rather than how big its base plate is.
///
/// The first version of this read the outermost geometry's own model, which on a real
/// moving head is the base: importing a Robe MegaPointe gave it a height of nine and a
/// half centimetres. The envelope is every part's box at the place its geometry puts
/// it, which is what a rider wants and what a case needs to fit.
#[test]
fn the_dimensions_are_the_whole_fixtures_rather_than_its_first_parts() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let size = fixture_type.physical.dimensions_m.expect("the file said how big it is");

    // Base 0.4 × 0.4 × 0.3 centred at the origin, so −0.15 to 0.15. Yoke 300 mm up and
    // 0.4 tall, head 200 mm above that, beam 150 mm above that again — whose box tops
    // out at 0.85. One metre, end to end, and taller than any one part.
    assert!(
        size.y > 0.3,
        "the base alone is 0.3 tall and the head is half a metre above it: {size:?}",
    );
    assert!((size.y - 1.0).abs() < 0.01, "{size:?}");
    assert_eq!(size.x, 0.4, "and no wider than its widest part");
}

#[test]
fn the_geometry_tree_comes_across_flat_in_the_consoles_own_axes() {
    let (fixture_type, _) = derive("rgbw-two-mode");
    let names: Vec<(&str, Option<&str>, GeometryKind)> = fixture_type
        .geometry
        .iter()
        .map(|node| (node.name.as_str(), node.parent.as_deref(), node.kind))
        .collect();
    assert_eq!(
        names,
        vec![
            ("Base", None, GeometryKind::Body),
            ("Yoke", Some("Base"), GeometryKind::Axis),
            ("Head", Some("Yoke"), GeometryKind::Axis),
            ("Beam", Some("Head"), GeometryKind::Beam),
        ],
        "pan turns the outer axis and tilt the inner one",
    );

    // The yoke sits 300 mm up the base. Millimetres Z-up become metres Y-up.
    let yoke = &fixture_type.geometry[1];
    assert_eq!(yoke.offset.y, 0.3);
    assert_eq!(yoke.offset.x, 0.0);
    assert_eq!(yoke.offset.z, 0.0);

    assert_eq!(fixture_type.geometry[3].beam_angle_deg, Some(12.0));
}

#[test]
fn a_multi_cell_bar_is_patched_as_the_whole_bar_rather_than_one_cell() {
    let (fixture_type, warnings) = derive("multicell-bar");
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(
        fixture_type.footprint("Pixel"),
        vec![16],
        "four channels times four cells, and the console patches all of it",
    );
    let mode = fixture_type.mode("Pixel");
    assert_eq!(mode.channels.len(), 16);
}

#[test]
fn a_minimal_file_is_still_a_fixture_type() {
    let (fixture_type, warnings) = derive("minimal");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(fixture_type.name, "Test Dimmer");
    assert_eq!(fixture_type.channel_count, 1);
    assert_eq!(fixture_type.parameters.len(), 1);
    assert_eq!(fixture_type.parameters[0].kind, ParameterKind::Intensity);
    assert_eq!(fixture_type.physical.beam_angle_deg, Some(25.0));
}
