//! Other people's MVR files.
//!
//! `#[ignore]`d, because the corpus is gitignored: these are other people's rigs
//! under other people's licenses, and a clone that has never run
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

/// A `Debug` rendering as text and numbers, so two can be compared with a tolerance.
#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Number(f64),
}

fn tokens(debug: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut text = String::new();
    let chars: Vec<char> = debug.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // A number with a point: a float. Integers — counts, addresses, UUID hex — are
        // compared as text, exactly, which is what they deserve.
        let starts = chars[i].is_ascii_digit() || (chars[i] == '-' && chars.get(i + 1).is_some_and(char::is_ascii_digit));
        let in_word = i > 0 && (chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '-' || chars[i - 1] == '_');
        if starts && !in_word {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == 'e' || (chars[j] == '-' && chars[j - 1] == 'e'))
            {
                j += 1;
            }
            let token: String = chars[i..j].iter().collect();
            let word_follows = chars.get(j).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_');
            if (token.contains('.') || token.contains('e')) && !word_follows {
                if let Ok(value) = token.parse::<f64>() {
                    out.push(Token::Text(std::mem::take(&mut text)));
                    out.push(Token::Number(value));
                    i = j;
                    continue;
                }
            }
            text.push_str(&token);
            i = j;
            continue;
        }
        text.push(chars[i]);
        i += 1;
    }
    out.push(Token::Text(text));
    out
}

/// Where two renderings first differ, beyond 1e-5 of a number (relative, for
/// coordinates in the thousands of millimeters), or `None`.
fn first_difference(a: &str, b: &str) -> Option<String> {
    let (a, b) = (tokens(a), tokens(b));
    if a.len() != b.len() {
        return Some(format!("{} parts against {}", a.len(), b.len()));
    }
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        let same = match (x, y) {
            (Token::Number(x), Token::Number(y)) => (x - y).abs() <= 1e-5 * x.abs().max(y.abs()).max(1.0),
            _ => x == y,
        };
        if !same {
            let context: Vec<String> = a[i.saturating_sub(4)..i].iter().map(|t| format!("{t:?}")).collect();
            return Some(format!("{x:?} against {y:?}, after {}", context.join(" ")));
        }
    }
    None
}

/// The comparison above, on its own: noise is the same scene, a moved number, a lost
/// fixture or a changed name is not. Runs without the corpus.
#[test]
fn the_comparison_forgives_noise_and_nothing_else() {
    let file = r#"Fixture { uuid: "2e149740-6a41", matrix: [[8.535905e-8, 1.0000004], [-6670.873, 5565.0166]], fixture_id: Some(11) }"#;
    let noise = r#"Fixture { uuid: "2e149740-6a41", matrix: [[0.0, 1.0], [-6670.873, 5565.017]], fixture_id: Some(11) }"#;
    assert_eq!(first_difference(noise, file), None);

    let moved = r#"Fixture { uuid: "2e149740-6a41", matrix: [[0.0, 1.0], [-6671.873, 5565.0166]], fixture_id: Some(11) }"#;
    let renumbered = r#"Fixture { uuid: "2e149740-6a41", matrix: [[0.0, 1.0], [-6670.873, 5565.0166]], fixture_id: Some(12) }"#;
    let other = r#"Fixture { uuid: "2e149740-6a42", matrix: [[0.0, 1.0], [-6670.873, 5565.0166]], fixture_id: Some(11) }"#;
    let fewer = r#"Fixture { uuid: "2e149740-6a41", matrix: [[0.0, 1.0]], fixture_id: Some(11) }"#;
    for changed in [moved, renumbered, other, fewer] {
        assert!(first_difference(changed, file).is_some(), "{changed}");
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

        // The same to within 1e-5, not to the last bit: the writer prints six decimals,
        // so a matrix a drawing program left at `1.0000004` or `8.5e-8` comes back `1`
        // and `0`, which is the point of writing them that way (gdtf.eu's own example
        // files carry that noise). Relative for large numbers, so under a tenth of a
        // millimeter at 6 m — near what an f32 holds there — or a rotation nobody could
        // see is not the scene changing; a millimeter, a fixture or an address is.
        if let Some(difference) = first_difference(&format!("{:?}", again.scene), &format!("{:?}", file.scene)) {
            panic!("{name} changed on the way out: {difference}");
        }
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
