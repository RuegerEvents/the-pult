//! Other people's MVR files.
//!
//! `#[ignore]`d, because the corpus is gitignored: these are other people's rigs
//! under other people's licences, and a clone that has never run
//! `scripts/fetch-interop-corpus.sh` has a passing suite. What is checked in beside
//! them is `testdata/mvr/`, small files written here.
//!
//! ```text
//! PULT_MVR_SAMPLES=~/mvr-corpus scripts/fetch-interop-corpus.sh
//! cargo test -p pult-mvr -- --ignored --nocapture
//! ```
//!
//! This exists because of what task 45 learned about GDTF: a reader written strictly
//! passed every hand-written fixture and failed on the first real file. It is the
//! same here — the first file this was pointed at is not well-formed XML.

use std::path::PathBuf;

use pult_mvr::MvrFile;

fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/corpus/mvr")
        .canonicalize();
    let Ok(dir) = dir else { return Vec::new() };
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mvr"))
        })
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            std::fs::read(entry.path()).ok().map(|bytes| (name, bytes))
        })
        .collect();
    files.sort();
    files
}

fn parsed() -> Vec<(String, MvrFile)> {
    let files = corpus();
    assert!(
        !files.is_empty(),
        "no MVR files in testdata/corpus/mvr — run scripts/fetch-interop-corpus.sh \
         with PULT_MVR_SAMPLES set to a directory of .mvr files",
    );
    files
        .into_iter()
        .map(|(name, bytes)| {
            let file = MvrFile::parse(&bytes).unwrap_or_else(|e| panic!("{name} does not parse: {e}"));
            (name, file)
        })
        .collect()
}

#[test]
#[ignore = "needs testdata/corpus/mvr"]
fn every_file_parses_and_says_what_it_holds() {
    for (name, file) in parsed() {
        let layers = &file.scene.scene.layers.items;
        let objects: usize = layers
            .iter()
            .map(|layer| count(layer.children.as_ref()))
            .sum();
        println!(
            "{name}: MVR {}.{} by {}, {} layers, {objects} objects, {} other files",
            file.scene.ver_major,
            file.scene.ver_minor,
            file.scene.provider.as_deref().unwrap_or("nobody in particular"),
            layers.len(),
            file.resources.len(),
        );
        for warning in &file.warnings {
            println!("    ! {warning}");
        }
        assert!(!layers.is_empty(), "{name} has no layers");
    }
}

fn count(list: Option<&pult_mvr::model::ChildList>) -> usize {
    let Some(list) = list else { return 0 };
    list.items
        .iter()
        .map(|node| 1 + node.object().map_or(0, |o| count(o.children.as_ref())))
        .sum()
}

/// Written back and read again, an archive means the same thing.
///
/// Not byte-for-byte against the original: this console writes MVR's own 4x3 matrix
/// form, its own indentation, and drops the `UserData` another tool wrote for itself.
/// What has to survive is the scene.
#[test]
#[ignore = "needs testdata/corpus/mvr"]
fn every_file_rewrites_to_the_same_scene() {
    for (name, file) in parsed() {
        let bytes = file.write().unwrap_or_else(|e| panic!("{name} does not write: {e}"));
        let again = MvrFile::parse(&bytes).unwrap_or_else(|e| panic!("{name} does not re-read: {e}"));

        assert_eq!(again.scene, file.scene, "{name} changed on the way out");
        assert_eq!(
            again.resources.keys().collect::<Vec<_>>(),
            file.resources.keys().collect::<Vec<_>>(),
            "{name} lost a file",
        );
    }
}

/// Every fixture in every file names a GDTF, and the archive either carries it or
/// does not. Both are allowed; what is not allowed is this console failing to find
/// one that is there.
#[test]
#[ignore = "needs testdata/corpus/mvr"]
fn every_fixture_finds_the_gdtf_its_archive_carries() {
    for (name, file) in parsed() {
        let mut wanted: Vec<String> = Vec::new();
        for layer in &file.scene.scene.layers.items {
            collect_specs(layer.children.as_ref(), &mut wanted);
        }
        wanted.sort();
        wanted.dedup();

        let carried = file
            .resources
            .keys()
            .filter(|n| n.to_ascii_lowercase().ends_with(".gdtf"))
            .count();
        println!("{name}: {} specs named, {carried} gdtf files carried", wanted.len());

        for spec in &wanted {
            match file.gdtf_named(spec) {
                Some((entry, bytes, rung)) => {
                    println!("    {spec:?} -> {entry:?} ({rung:?}, {} bytes)", bytes.len());
                    assert!(!bytes.is_empty(), "{name}: {spec} is empty");
                }
                None => println!("    {spec:?} -> not in this archive"),
            }
        }
    }
}

fn collect_specs(list: Option<&pult_mvr::model::ChildList>, out: &mut Vec<String>) {
    let Some(list) = list else { return };
    for node in &list.items {
        let Some(object) = node.object() else { continue };
        if let Some(spec) = object.gdtf_spec.as_ref().filter(|s| !s.trim().is_empty()) {
            out.push(spec.clone());
        }
        collect_specs(object.children.as_ref(), out);
    }
}
