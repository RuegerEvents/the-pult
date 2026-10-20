//! Other people's GDTF files, read as fixture types this console can drive.
//!
//! `#[ignore]`d, because the corpus is gitignored: `scripts/fetch-interop-corpus.sh`
//! puts it in `testdata/corpus/gdtf`.
//!
//! cargo test -p pult-backend --test gdtf_corpus -- --ignored --nocapture

use pult_backend::infra::interop::gdtf::derive_fixture_type;
use pult_gdtf::GdtfFile;

fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/corpus/gdtf");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gdtf")))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap()))
        .collect()
}

/// Every cell's channel names a key the type reads back to its per-cell parameter, and
/// a type that has cells has a per-cell parameter in them. A head whose referenced
/// pixels used to collapse into one color now has one per cell. (A channel the console
/// has no parameter for — a fine channel's own attribute — is not a cell's and was
/// never read back; that is the DMX connector's "nothing drives it".)
#[test]
#[ignore = "needs testdata/corpus/gdtf"]
fn every_real_file_names_only_keys_it_has() {
    let files = corpus();
    assert!(!files.is_empty(), "run scripts/fetch-interop-corpus.sh");
    for (name, bytes) in files {
        let file = GdtfFile::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (fixture_type, _) = derive_fixture_type(&file, "sha");
        let per_cell = fixture_type.parameters.iter().filter(|p| p.per_cell).count();
        println!("{name}: {} — {} cells, {per_cell} per-cell parameters", fixture_type.name, fixture_type.cells);
        if fixture_type.cells > 0 {
            assert!(per_cell > 0, "{name}: cells with nothing per cell");
        }
        for mode in &fixture_type.dmx_modes {
            for channel in mode.channels.iter().filter(|c| c.parameter_key.contains('@')) {
                assert!(
                    fixture_type.parameter_for_key(&channel.parameter_key).is_some(),
                    "{name} mode {}: {} reads back to no parameter",
                    mode.name,
                    channel.parameter_key
                );
            }
        }
    }
}
