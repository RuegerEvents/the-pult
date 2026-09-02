//! The files checked in beside this crate: read them, write them, read them again.
//!
//! `testdata/mvr/` is hand-written and holds one shape per file, each of them a shape
//! a real MVR turned out to have. They are XML rather than zips so that a reviewer
//! can see what is being asserted; the archive layer is exercised by zipping them
//! here, which is also how the resource lookup gets something to look up.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use pult_mvr::model::ChildNode;
use pult_mvr::{MvrFile, SpecMatch};

fn scene(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/mvr")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// One of the checked-in scenes, zipped with whatever resources it names.
fn archive(name: &str, resources: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("GeneralSceneDescription.xml", options).unwrap();
        zip.write_all(&scene(name)).unwrap();
        for (entry, bytes) in resources {
            zip.start_file(*entry, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

fn read(name: &str) -> MvrFile {
    MvrFile::parse(&archive(name, &[])).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Every object in a layer, depth first, as (tag, name).
fn walk(file: &MvrFile) -> Vec<(&'static str, String)> {
    fn go(list: Option<&pult_mvr::model::ChildList>, out: &mut Vec<(&'static str, String)>) {
        let Some(list) = list else { return };
        for node in &list.items {
            if let Some(object) = node.object() {
                out.push((node.tag(), object.name.clone()));
                go(object.children.as_ref(), out);
            }
        }
    }
    let mut out = Vec::new();
    for layer in &file.scene.scene.layers.items {
        go(layer.children.as_ref(), &mut out);
    }
    out
}

#[test]
fn a_mirrored_truss_keeps_its_reflection_through_a_round_trip() {
    let file = read("mirrored-truss.xml");

    let trusses: Vec<_> = walk(&file);
    assert_eq!(
        trusses,
        vec![
            ("Truss", "Upstage truss".to_string()),
            ("Truss", "Downstage truss, mirrored".to_string()),
        ]
    );

    let mirrored = match &file.scene.scene.layers.items[0]
        .children
        .as_ref()
        .unwrap()
        .items[1]
    {
        ChildNode::Truss(object) => object,
        other => panic!("expected a truss, got {}", other.tag()),
    };
    let placement = pult_mvr::transform::decompose(mirrored.matrix.as_ref().unwrap());
    assert!(
        placement.scale.iter().any(|s| *s < 0.0),
        "the reflection survives as a negative scale: {:?}",
        placement.scale,
    );

    let again = MvrFile::parse(&file.write().unwrap()).expect("written and read again");
    assert_eq!(again.scene, file.scene);
}

#[test]
fn two_objects_share_one_symbol_definition() {
    let file = read("mirrored-truss.xml");

    let symdefs: Vec<&str> = file
        .scene
        .scene
        .aux_data
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter_map(|item| match item {
            pult_mvr::model::AuxItem::Symdef(s) => Some(s.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(symdefs, vec!["Truss 2m"], "one definition");

    let referenced: Vec<String> = walk(&file)
        .iter()
        .zip(file.scene.scene.layers.items[0].children.as_ref().unwrap().items.iter())
        .filter_map(|(_, node)| node.object())
        .flat_map(|o| o.geometries.iter())
        .flat_map(|g| g.items.iter())
        .filter_map(|g| match g {
            pult_mvr::model::GeometryNode::Symbol(s) => Some(s.symdef.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(referenced.len(), 2, "instanced twice");
    assert_eq!(referenced[0], referenced[1], "and it is the same one");
}

#[test]
fn a_group_inside_a_group_keeps_its_fixtures() {
    let file = read("nested-groups.xml");

    assert_eq!(
        walk(&file),
        vec![
            ("GroupObject", "Back truss".to_string()),
            ("Fixture", "Bar 1".to_string()),
            ("GroupObject", "Inner group".to_string()),
            ("Fixture", "Spot 1".to_string()),
        ]
    );
}

#[test]
fn an_absolute_address_is_read_as_a_universe_and_a_channel() {
    let file = read("nested-groups.xml");
    let mut addresses = Vec::new();
    fn go(list: Option<&pult_mvr::model::ChildList>, out: &mut Vec<(u16, u16, u16)>) {
        let Some(list) = list else { return };
        for node in &list.items {
            let Some(object) = node.object() else { continue };
            for address in object.addresses.iter().flat_map(|a| a.items.iter()) {
                let (universe, channel) =
                    pult_mvr::address::to_universe_and_channel(address.absolute.unwrap_or(1));
                out.push((pult_mvr::address::to_break(address.break_id), universe, channel));
            }
            go(object.children.as_ref(), out);
        }
    }
    for layer in &file.scene.scene.layers.items {
        go(layer.children.as_ref(), &mut addresses);
    }

    assert_eq!(
        addresses,
        vec![(1, 1, 37), (1, 3, 1), (2, 2, 1)],
        "break 0 is break 1 here, and 1025 is universe 3 channel 1",
    );
}

/// The name a fixture gives its GDTF is not always the name of the file.
#[test]
fn a_gdtf_is_found_under_either_spelling() {
    let bytes = archive(
        "nested-groups.xml",
        &[
            ("Acme@Pixel Bar.gdtf", b"PK-not-really"),
            ("Acme@Spot.gdtf", b"PK-not-really-either"),
        ],
    );
    let file = MvrFile::parse(&bytes).expect("an archive with two fixture definitions");

    // Written without the extension, the way grandMA writes one.
    let (entry, _, rung) = file.gdtf_named("Acme@Pixel Bar").expect("found");
    assert_eq!(entry, "Acme@Pixel Bar.gdtf");
    assert_eq!(rung, SpecMatch::Extension);

    // And with it, the way Vectorworks does.
    let (entry, _, rung) = file.gdtf_named("Acme@Spot.gdtf").expect("found");
    assert_eq!(entry, "Acme@Spot.gdtf");
    assert_eq!(rung, SpecMatch::Exact);

    assert!(file.gdtf_named("Acme@Nothing").is_none());
}

/// The corpus's worst file, in miniature: `"None"` and `"N/A"` where numbers belong,
/// `i32::MIN` in an unsigned field, and a NUL byte after the closing tag.
#[test]
fn a_file_that_no_strict_reader_would_open_is_read_and_says_what_it_forgave() {
    let file = read("awkward-numbers.xml");

    assert!(
        file.warnings.iter().any(|w| w.message.contains("after its closing tag")),
        "the trailing NUL is forgiven and reported: {:?}",
        file.warnings,
    );

    let fixture = match &file.scene.scene.layers.items[0].children.as_ref().unwrap().items[1] {
        ChildNode::Fixture(object) => object,
        other => panic!("expected a fixture, got {}", other.tag()),
    };
    assert_eq!(fixture.fixture_id, None, "\"None\" is nothing, not a failure");
    assert_eq!(fixture.unit_number, None, "and so is \"N/A\"");
    assert_eq!(fixture.fixture_type_id, None, "and so is i32::MIN in a u32");
    assert_eq!(fixture.custom_id, Some(3), "a count of 3.0 is three");

    // An address with no break attribute is the first break.
    let address = &fixture.addresses.as_ref().unwrap().items[0];
    assert_eq!(pult_mvr::address::to_break(address.break_id), 1);
}

/// Nothing in the archive may name a path outside it.
#[test]
fn an_entry_that_climbs_out_of_the_archive_is_refused() {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("GeneralSceneDescription.xml", options).unwrap();
        zip.write_all(&scene("nested-groups.xml")).unwrap();
        zip.start_file("../../etc/passwd", options).unwrap();
        zip.write_all(b"nope").unwrap();
        zip.finish().unwrap();
    }

    let error = MvrFile::parse(&buffer.into_inner()).expect_err("refused");
    assert!(
        matches!(error, pult_mvr::Error::BadEntry(_)),
        "got {error:?}",
    );
}

/// Resources survive being written back, under the names they had.
#[test]
fn the_files_beside_the_scene_come_back_unchanged() {
    let bytes = archive(
        "mirrored-truss.xml",
        &[("truss-2m.glb", b"glTF-not-really"), ("tx603.jpg", b"jpeg")],
    );
    let file = MvrFile::parse(&bytes).expect("read");

    let again = MvrFile::parse(&file.write().unwrap()).expect("written and read again");

    let expected: BTreeMap<String, Vec<u8>> = [
        ("truss-2m.glb".to_string(), b"glTF-not-really".to_vec()),
        ("tx603.jpg".to_string(), b"jpeg".to_vec()),
    ]
    .into();
    assert_eq!(again.resources, expected);
}
