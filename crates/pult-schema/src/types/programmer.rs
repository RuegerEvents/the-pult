//! The programmer: the scratch buffer an operator works in before anything is stored.
//!
//! A console has two sources of truth about what a light is doing. Playback says what
//! the cue asks for; the programmer says what the operator is asking for *right now*,
//! and the programmer wins. Nothing is written to the show until it is stored, and
//! clearing puts every touched parameter back where playback had it.
//!
//! # Why this is a collection and not a field on anything
//!
//! One value one operator is holding is one row. That makes two consoles working the
//! same rig converge without arbitration, the same way [`crate::types::station`] does
//! — as long as they agree on the id. They do: the frontend derives the id from the
//! fixture and the parameter key rather than minting a fresh one, so two people
//! grabbing the same fader write the same row instead of two rows that fight.
//!
//! SYNCED rather than PERSISTED. A programmer buffer is what is in the operator's
//! hands, not what is in the show; a showfile that reopened with somebody's
//! half-finished look asserted over playback would be a fault, not a feature.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    types::effect::EffectSpec,
    types::fixture::{ParameterKind, ParameterValue},
    PultSchema,
};

/// One parameter of one fixture, held by the programmer.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "programmer_values")]
pub struct ProgrammerValue {
    /// Derived from `fixture_id` and the parameter key rather than minted, so two
    /// consoles writing the same parameter converge on one entry.
    #[pult(lifecycle = SYNCED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = SYNCED)]
    pub fixture_id: Uuid,
    #[pult(lifecycle = SYNCED)]
    pub parameter_kind: ParameterKind,
    #[pult(lifecycle = SYNCED)]
    pub value: ParameterValue,
    /// A shape held instead of a value.
    ///
    /// An entry asserts either its value or its effect for the key, never both, so
    /// the id derivation is unchanged: grabbing a fader and putting a sine on it are
    /// the same act of taking hold of one parameter.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub effect: Option<EffectSpec>,
    /// Parked: survives Clear and Store, so one value can go into several cues.
    ///
    /// The spec calls this the parking function and asks for it explicitly — a value
    /// held "without saving, to be saved in multiple sequences without the need of a
    /// store menu".
    #[pult(lifecycle = SYNCED)]
    pub locked: bool,
}

// ── The derived id ────────────────────────────────────────────────────────────

const FNV_PRIME: u64 = 1099511628211;
const FNV_OFFSET: u64 = 14695981039346656037;

fn fnv1a(text: &str, seed: u64) -> u64 {
    let mut hash = seed;
    // `charCodeAt` on the JS side: UTF-16 code units. Everything hashed here is
    // ASCII (a uuid, a slash, a parameter key), where code units are bytes.
    for unit in text.encode_utf16() {
        hash ^= unit as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// The id of the programmer entry for one parameter of one fixture: two FNV-1a
/// passes over `"<fixture_id>/<key>"`, dressed as a version-8 UUID.
///
/// Here because it is the schema's rule — [`ProgrammerValue`] above says the id is
/// derived and why, and this is the derivation it means. Anything that writes the
/// programmer has to agree on it or two rows fight over one fader.
///
/// It is implemented twice more, and neither copy can be deleted:
/// `frontend/src/lib/programmer.ts` because a browser cannot run this, and
/// `plugins/command-line/core/src/ids.rs` because the plugins workspace builds
/// guests for `wasm32-wasip2` and this crate does not belong in that graph. All
/// three are pinned to the same literal examples, so a change to any one of them
/// fails two suites rather than going quiet.
pub fn programmer_entry_id(fixture_id: &str, key: &str) -> String {
    let source = format!("{fixture_id}/{key}");
    let hi = fnv1a(&source, FNV_OFFSET);
    let lo = fnv1a(&source, FNV_OFFSET ^ u64::MAX);
    let mut hex: Vec<char> = format!("{hi:016x}{lo:016x}").chars().collect();
    // Version 8 (custom) and the RFC 4122 variant, so what comes out is a UUID and
    // says truthfully how it was made.
    hex[12] = '8';
    let variant = hex[16].to_digit(16).unwrap_or(0) & 0x3 | 0x8;
    hex[16] = char::from_digit(variant, 16).unwrap_or('8');
    let s: String = hex.into_iter().collect();
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same pairs appear verbatim in `plugins/command-line/core/src/ids.rs`
    /// and `frontend/src/lib/programmer.test.ts`. Three implementations of one
    /// naming scheme can only drift loudly.
    #[test]
    fn the_three_derivations_agree_by_pinned_example() {
        assert_eq!(
            programmer_entry_id("2f6b535b-9a71-4c39-9d95-6d6ab2f0f639", "Intensity"),
            "5f13b718-4585-810f-9f90-15d7509267f4"
        );
        assert_eq!(
            programmer_entry_id("00000000-0000-0000-0000-000000000000", "ColorRgb"),
            "3ad6b4b5-4891-8a54-ae06-93999b3641bd"
        );
    }

    #[test]
    fn the_result_is_a_wellformed_v8_uuid() {
        let id = programmer_entry_id("2f6b535b-9a71-4c39-9d95-6d6ab2f0f639", "Pan");
        assert!(id.parse::<Uuid>().is_ok(), "{id} is not a uuid");
        assert_eq!(&id[14..15], "8", "version nibble");
        let variant = id.as_bytes()[19] as char;
        assert!(matches!(variant, '8' | '9' | 'a' | 'b'), "variant nibble was {variant}");
    }
}
