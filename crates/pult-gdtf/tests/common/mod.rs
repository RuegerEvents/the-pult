//! Turning a checked-in directory into a `.gdtf` in memory.
//!
//! The test material lives as XML and glTF under `testdata/gdtf/<name>/` rather than
//! as a zip, so a diff of a change to it is readable and a reviewer can see what the
//! test is asserting about. Zipping happens here, the way the plugin bundle tests
//! build their bundles.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the checked-in fixtures live.
pub fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/gdtf")
}

/// Where the downloaded corpus lands, if `scripts/fetch-interop-corpus.sh` has run.
pub fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/corpus/gdtf")
}

/// Every checked-in fixture directory, by name.
pub fn hand_authored() -> Vec<(String, Vec<u8>)> {
    let root = testdata();
    let mut entries: Vec<_> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("reading {}: {e}", root.display()))
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let out: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                zip_dir(&entry.path()),
            )
        })
        .collect();
    assert!(!out.is_empty(), "no fixtures in {}", root.display());
    out
}

/// The `description.xml` of a checked-in fixture, as it is on the disk.
pub fn description(name: &str) -> String {
    std::fs::read_to_string(testdata().join(name).join("description.xml")).unwrap()
}

/// Every `.gdtf` in the downloaded corpus, or an empty list when it has not been
/// fetched.
pub fn corpus_files() -> Vec<(String, Vec<u8>)> {
    let root = corpus();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("gdtf"))
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).unwrap())
        })
        .collect()
}

/// Zip a directory, deterministically, paths relative to it.
pub fn zip_dir(dir: &Path) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    for (relative, path) in files {
        writer.start_file(relative.as_str(), options).unwrap();
        writer.write_all(&std::fs::read(path).unwrap()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((relative, path));
        }
    }
}
