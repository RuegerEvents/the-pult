//! The control map, read from a real grandMA3 installation.
//!
//! `#[ignore]`d like `pult-gdtf`'s corpus tests, because the file is MA's and is only
//! on a machine that has grandMA3 installed. What it checks is the thing the whole
//! index scheme rests on: that every block size in the protocol matches the number of
//! entries in MA's own table.
//!
//!     cargo test -p pult-wing --test real_map -- --ignored

use pult_wing::map::{WingMap, COMMAND_WING};

fn ma3_xml() -> Option<String> {
    let base = std::path::PathBuf::from(std::env::var("HOME").ok()?).join("MALightingTechnology");
    let mut installs: Vec<_> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("gma3_") && n != "gma3_library")
        })
        .collect();
    installs.sort();
    installs.reverse();
    installs
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p.join("shared/resource/hardware_configurations.xml")).ok())
}

#[test]
#[ignore = "needs a local grandMA3 installation"]
fn the_table_matches_every_block_size_on_the_wire() {
    let Some(xml) = ma3_xml() else {
        panic!("no grandMA3 under ~/MALightingTechnology");
    };
    let m = WingMap::from_xml(&xml, COMMAND_WING).expect("the command wing");

    // 12 faders against 12 words of a 24-byte block.
    assert_eq!(m.faders.len(), pult_wing::input::FADERS, "faders");

    // Every assigned hardkey has to be addressable by the 56-byte bitmap.
    let highest = m.hardkeys.keys().copied().max().unwrap();
    assert!(
        (highest as usize) < pult_wing::input::KEYS,
        "hardkey {highest} does not fit in {} bits",
        pult_wing::input::KEYS
    );

    // Every encoder has to be addressable by the 77-byte block.
    let highest_enc = m.encoders.keys().copied().max().unwrap();
    assert!(
        (highest_enc as usize) < pult_wing::input::ENCODER_BYTES,
        "encoder {highest_enc} does not fit in {} bytes",
        pult_wing::input::ENCODER_BYTES
    );

    // And no LED channel may fall outside the 253-byte frame. This is the check that
    // says the LED table and the LED frame are the same thing.
    let highest_led = m
        .leds
        .iter()
        .flat_map(|l| [l.r, l.g, l.b])
        .flatten()
        .max()
        .expect("some LED channels");
    assert_eq!(
        highest_led,
        pult_wing::output::LED_BYTES - 1,
        "the highest LED channel should be the last byte of the frame"
    );

    eprintln!(
        "{}: {} hardkeys, {} faders, {} encoders, {} leds; highest LED channel {}",
        m.module,
        m.hardkeys.len(),
        m.faders.len(),
        m.encoders.len(),
        m.leds.len(),
        highest_led
    );
}

/// The file describes every MA module, not just this one.
#[test]
#[ignore = "needs a local grandMA3 installation"]
fn the_file_describes_the_whole_family() {
    let Some(xml) = ma3_xml() else { return };
    let all = WingMap::modules(&xml);
    assert!(all.iter().any(|m| m == COMMAND_WING), "{all:?}");
    eprintln!("{} modules: {}", all.len(), all.join(", "));
}
