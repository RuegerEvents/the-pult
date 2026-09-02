//! Structural checks, reported as warnings.
//!
//! Not a schema validator — that is `xmllint` against the spec's XSD, which the
//! corpus tests run. This catches the class of problem a schema cannot see: a mode
//! whose channels overlap, an attribute nothing defines, a geometry reference into
//! nowhere. Every one is a warning because every one describes a file somebody still
//! has to patch tonight.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::FixtureType;
use crate::resolve;
use crate::Warning;

/// Everything questionable about a fixture type.
pub fn check(fixture: &FixtureType) -> Vec<Warning> {
    let mut warnings = Vec::new();

    if fixture.name.is_empty() {
        warnings.push(Warning::new("FixtureType", "has no name"));
    }
    if fixture.dmx_modes.items.is_empty() {
        warnings.push(Warning::new(
            "DMXModes",
            "the file declares no modes at all",
        ));
    }

    let defined: BTreeSet<&str> = fixture
        .attribute_definitions
        .attributes
        .items
        .iter()
        .map(|attribute| attribute.name.as_str())
        .collect();

    for mode in &fixture.dmx_modes.items {
        let at = format!("DMXModes.{}", mode.name);
        let (channels, mut mode_warnings) = resolve::expand_mode(fixture, mode);
        warnings.append(&mut mode_warnings);

        // Two channels in one break claiming one byte is the error that patches a
        // rig wrong in a way nothing downstream can detect.
        let mut claimed: BTreeMap<(u16, u16), String> = BTreeMap::new();
        for channel in &channels {
            let name = channel
                .attribute
                .map(ToString::to_string)
                .unwrap_or_else(|| "an unnamed channel".into());
            if let Some(node) = channel.attribute {
                let attribute = node.last().unwrap_or_default();
                if !attribute.is_empty() && !defined.contains(attribute) {
                    warnings.push(Warning::new(
                        format!("{at}.{attribute}"),
                        "uses an attribute the file never defines",
                    ));
                }
            }
            for offset in &channel.offsets {
                if let Some(existing) =
                    claimed.insert((channel.break_number, *offset), name.clone())
                {
                    warnings.push(Warning::new(
                        format!("{at}.{name}"),
                        format!(
                            "claims break {} offset {offset}, which {existing} also claims",
                            channel.break_number
                        ),
                    ));
                }
            }
        }

        if channels.iter().all(|channel| channel.offsets.is_empty()) && !channels.is_empty() {
            warnings.push(Warning::new(&at, "every channel in this mode is virtual"));
        }
    }

    warnings
}
