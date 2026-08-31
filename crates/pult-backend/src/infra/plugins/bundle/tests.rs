//! Bundles, well-formed and otherwise.
//!
//! The hostile archives matter more than the good one. A zip is a description of
//! where to write files, and every test here is a description somebody could
//! write on purpose.

use std::io::Write;

use zip::write::SimpleFileOptions;

use super::*;

const MANIFEST: &str = r#"
[plugin]
id = "example"
name = "Example"
version = "0.1.0"
api = "0.1"
wasm = "example.wasm"
"#;

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pult-bundle-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A zip built from `(name, contents)` pairs, written the way a bundler would.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
        for (name, body) in entries {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap();
    }
    buf
}

/// A zip holding one entry with the unix mode bits that mean "symlink", whose
/// contents are the path it points at.
fn zip_with_symlink(name: &str, target: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
        w.start_file(MANIFEST_NAME, SimpleFileOptions::default()).unwrap();
        w.write_all(MANIFEST.as_bytes()).unwrap();
        w.start_file("example.wasm", SimpleFileOptions::default()).unwrap();
        w.write_all(b"\0asm").unwrap();
        w.add_symlink(name, target, SimpleFileOptions::default()).unwrap();
        w.finish().unwrap();
    }
    buf
}

fn a_good_bundle() -> Vec<u8> {
    zip_of(&[
        (MANIFEST_NAME, MANIFEST.as_bytes()),
        ("example.wasm", b"\0asm\x01\0\0\0"),
        ("assets/panel.js", b"customElements.define('x-panel', class extends HTMLElement {});"),
    ])
}

#[test]
fn a_well_formed_bundle_unpacks_into_something_the_runtime_accepts() {
    let root = tempdir();
    let dir = root.join("unpacked");

    let manifest = extract(&a_good_bundle(), &dir).expect("a good bundle unpacks");

    assert_eq!(manifest.plugin.id, "example");
    assert!(manifest.wasm_path().is_file(), "the component is where the manifest says");
    assert!(dir.join("assets/panel.js").is_file(), "a panel's script comes with it");
    // The whole point of the layout: what comes out is a plugin directory, so
    // everything the runtime already does with one works unchanged.
    assert!(dir.join(MANIFEST_NAME).is_file());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_bundle_that_is_not_a_zip_is_refused_rather_than_guessed_at() {
    let root = tempdir();
    let err = extract(b"this is not a zip at all", &root.join("x")).unwrap_err().to_string();
    assert!(err.contains("not a readable zip"), "{err}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_bundle_with_no_manifest_is_not_a_plugin() {
    let root = tempdir();
    let bundle = zip_of(&[("example.wasm", b"\0asm")]);

    let err = extract(&bundle, &root.join("x")).unwrap_err().to_string();
    assert!(err.contains(MANIFEST_NAME), "{err}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_manifest_naming_a_component_the_bundle_does_not_carry_is_refused() {
    let root = tempdir();
    // Refused at unpack rather than at load, so the failure names the bundle
    // rather than turning up later as a plugin that will not start.
    let bundle = zip_of(&[(MANIFEST_NAME, MANIFEST.as_bytes())]);

    let err = extract(&bundle, &root.join("x")).unwrap_err().to_string();
    assert!(err.contains("example.wasm"), "{err}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn an_entry_that_would_escape_the_directory_is_refused() {
    let root = tempdir();

    for name in ["../escaped.txt", "assets/../../escaped.txt", "/etc/passwd"] {
        let bundle = zip_of(&[
            (MANIFEST_NAME, MANIFEST.as_bytes()),
            ("example.wasm", b"\0asm"),
            (name, b"owned"),
        ]);
        let err = extract(&bundle, &root.join("x")).unwrap_err().to_string();
        assert!(
            err.contains("outside") || err.contains("relative path"),
            "{name:?} should be refused by name, got: {err}",
        );
        assert!(!root.join("escaped.txt").exists(), "{name:?} wrote outside the directory");
        assert!(!root.join("x").exists(), "a refused bundle leaves no half-unpacked directory");
    }

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_symlink_is_refused_because_writing_through_one_writes_anywhere() {
    let root = tempdir();
    // The entry's own name is innocent; its contents are the path it points at,
    // so nothing about the name catches this one.
    let bundle = zip_with_symlink("assets/innocent.js", "/etc/passwd");

    let err = extract(&bundle, &root.join("x")).unwrap_err().to_string();
    assert!(err.contains("symlink"), "{err}");
    assert!(!root.join("x").exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_bundle_with_more_files_than_a_plugin_could_want_is_refused() {
    let root = tempdir();
    let names: Vec<String> = (0..MAX_ENTRIES + 1).map(|i| format!("assets/{i}.js")).collect();
    let mut entries: Vec<(&str, &[u8])> = vec![
        (MANIFEST_NAME, MANIFEST.as_bytes()),
        ("example.wasm", b"\0asm"),
    ];
    for name in &names {
        entries.push((name.as_str(), b""));
    }
    let bundle = zip_of(&entries);

    let err = extract(&bundle, &root.join("x")).unwrap_err().to_string();
    assert!(err.contains(&MAX_ENTRIES.to_string()), "{err}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_bundle_that_unpacks_to_more_than_it_claims_is_refused() {
    let root = tempdir();
    // The compressed size is what the asset store's ceiling sees; this is the
    // number that matters, because zeroes compress to almost nothing.
    let big = vec![0u8; (MAX_UNPACKED_BYTES + 1) as usize];
    let bundle = zip_of(&[
        (MANIFEST_NAME, MANIFEST.as_bytes()),
        ("example.wasm", b"\0asm"),
        ("assets/big.bin", &big),
    ]);
    assert!(
        (bundle.len() as u64) < MAX_UNPACKED_BYTES,
        "the archive is small; only what it unpacks to is not",
    );

    let err = extract(&bundle, &root.join("x")).unwrap_err().to_string();
    assert!(err.contains("unpacks to more"), "{err}");
    assert!(!root.join("x").exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_manifest_can_be_read_without_unpacking_anything() {
    let root = tempdir();
    let dir = root.join("would-be");

    // What the install path needs: enough to refuse a bad bundle before its
    // bytes are stored, so a rejected upload leaves nothing behind.
    let info = read_manifest(&a_good_bundle(), &dir).expect("the manifest reads");

    assert_eq!(info.manifest.plugin.id, "example");
    assert_eq!(info.manifest.plugin.api, "0.1");
    assert!(!dir.exists(), "reading a manifest writes nothing");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn unpacking_over_something_that_exists_is_refused() {
    let root = tempdir();
    let dir = root.join("taken");
    std::fs::create_dir_all(&dir).unwrap();

    let err = extract(&a_good_bundle(), &dir).unwrap_err().to_string();
    assert!(err.contains("already exists"), "{err}");

    std::fs::remove_dir_all(&root).ok();
}
