//! Turning a colour into levels for the lights that actually make it.
//!
//! A console holds one colour. A fixture has emitters — three of them on a plain RGB
//! par, four on an RGBW head, seven on a good one, and three *subtractive* flags on
//! anything with a CMY mixing system. Getting from the one to the other is arithmetic
//! that has to be the same on the station and in the browser, or the swatch and the
//! lamp disagree, so it lives here and is compiled twice like everything else in this
//! crate.
//!
//! # The rules, and why they are these
//!
//! **Red, green and blue pass through.** An RGB emitter takes the component it is
//! named for, unchanged. Anything cleverer would mean a colour command not producing
//! the colour that was asked for, which is the one thing an operator will not forgive.
//!
//! **Every other additive emitter takes as much as fits.** White, amber, lime, UV:
//! each takes the largest multiple of its own colour that stays under the target on
//! every channel. For white that is `min(r, g, b)` — the neutral part of the colour,
//! which is exactly what a white die is for — and for amber `(1, 0.5, 0)` it is
//! `min(r, 2g)`, which is zero for pure red and zero for pure green and full for
//! amber. An emitter whose colour the file never measured takes nothing, because
//! guessing which channel a die called "Cyan-ish" belongs to is how a colour comes
//! out wrong in a way nobody can trace.
//!
//! Note what this does *not* do: it does not take the extra emitter's contribution
//! back off the primaries. An RGBW head told to go white lights all four, and is
//! brighter than the arithmetic of a single luminaire would suggest. That is what
//! nearly every console does and what an operator expects from the fader — and where
//! it is not wanted, the colour carries a per-emitter override, which is what
//! overrides are for.
//!
//! **A subtractive emitter is one minus the component it removes.** Cyan takes red
//! out, so full cyan is `1 − r`. Which component it removes comes from the emitter's
//! own colour: it is the channel that colour has least of.
//!
//! **An override wins, last.** A colour may name any emitter explicitly, and that
//! number is used as it stands. This is the escape hatch for every fixture whose
//! white is warmer than the file says and every operator who wants the amber off.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One light source of a fixture, as the mixer needs it.
///
/// A parallel of the schema's `Emitter` rather than the same type: this crate is
/// compiled for the browser and cannot depend on `pult-schema`. The backend converts
/// in one place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmitterSpec {
    pub name: String,
    /// Where this emitter sits in linear RGB. `None` for one nothing measured.
    #[serde(default)]
    pub rgb: Option<[f32; 3]>,
    /// A flag that removes light rather than a die that makes it.
    #[serde(default)]
    pub subtractive: bool,
}

/// The colour half of a [`crate::ParameterValue::Color`], on its own.
///
/// So that `mix` can be called with a colour that is not inside a parameter value —
/// which is what the browser's colour control and the wasm export both want.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// Levels named for specific emitters, which win over anything derived.
    #[serde(default)]
    pub overrides: BTreeMap<String, f32>,
}

/// What each emitter should be at, in the order the fixture lists them.
///
/// The shape a caller wants when it is about to *show* the answer. A caller writing
/// one byte wants [`level_of`], which is the same arithmetic without the allocation —
/// and that matters: a DMX frame over a rig of thousands asks this question once per
/// colour channel per fixture, forty times a second.
pub fn mix(color: &Color, emitters: &[EmitterSpec]) -> Vec<(String, f32)> {
    emitters.iter().map(|emitter| (emitter.name.clone(), level_of(color, emitter))).collect()
}

/// One emitter's level, and nothing else.
///
/// Allocation-free on purpose. This is on the frame path — the DMX connector calls it
/// once per colour channel per fixture per frame — and building a vector of cloned
/// names per call was measurably the whole cost of a colour on a rig of five hundred.
pub fn level_of(color: &Color, emitter: &EmitterSpec) -> f32 {
    level_from([color.r, color.g, color.b], &color.overrides, emitter)
}

/// The same, without a [`Color`] to hold it.
///
/// The frame path's entry point: a connector has three floats and a borrowed map, and
/// building a `Color` around them to ask this question would clone the map once per
/// channel per fixture per frame for the overwhelmingly common case of it being empty.
pub fn level_from(
    rgb: [f32; 3],
    overrides: &BTreeMap<String, f32>,
    emitter: &EmitterSpec,
) -> f32 {
    if let Some(pinned) = overrides.get(&emitter.name) {
        return pinned.clamp(0.0, 1.0);
    }
    derive(emitter, [rgb[0].clamp(0.0, 1.0), rgb[1].clamp(0.0, 1.0), rgb[2].clamp(0.0, 1.0)])
}

/// One emitter's level, from the colour alone.
fn derive(emitter: &EmitterSpec, target: [f32; 3]) -> f32 {
    let Some(rgb) = emitter.rgb.or_else(|| rgb_from_name(&emitter.name)) else {
        return 0.0;
    };

    if emitter.subtractive {
        // The channel this flag takes out is the one its own colour has least of:
        // cyan is (0, 1, 1) and removes red.
        let removes = (0..3)
            .min_by(|a, b| rgb[*a].total_cmp(&rgb[*b]))
            .expect("three channels");
        return (1.0 - target[removes]).clamp(0.0, 1.0);
    }

    // A primary passes its own component through.
    if let Some(primary) = primary_channel(rgb) {
        return target[primary];
    }

    // Everything else takes the largest multiple of itself that fits under the
    // target on every channel it contributes to.
    let mut level = 1.0f32;
    let mut contributes = false;
    for channel in 0..3 {
        if rgb[channel] <= 1e-4 {
            continue;
        }
        contributes = true;
        level = level.min(target[channel] / rgb[channel]);
    }
    if contributes {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Which single channel this colour is, if it is one.
///
/// A tolerance rather than an equality, because a file's measured red is
/// `(1, 0.02, 0.01)` and not `(1, 0, 0)`, and a red die that failed this test would
/// go through the fitting path and come out darker than it was asked for.
fn primary_channel(rgb: [f32; 3]) -> Option<usize> {
    let brightest = (0..3).max_by(|a, b| rgb[*a].total_cmp(&rgb[*b]))?;
    if rgb[brightest] < 0.5 {
        return None;
    }
    let others_dark = (0..3).all(|channel| channel == brightest || rgb[channel] < 0.15);
    others_dark.then_some(brightest)
}

/// A colour for an emitter the file never measured, from what it is called.
///
/// Only the names that are unambiguous. Anything else gets nothing rather than a
/// guess: an emitter driven from a mistaken guess is worse than one left dark,
/// because the dark one is visible as a problem.
fn rgb_from_name(name: &str) -> Option<[f32; 3]> {
    let lower = name.trim().to_ascii_lowercase();
    Some(match lower.as_str() {
        "red" | "r" => [1.0, 0.0, 0.0],
        "green" | "g" => [0.0, 1.0, 0.0],
        "blue" | "b" => [0.0, 0.0, 1.0],
        "white" | "w" | "warmwhite" | "coolwhite" => [1.0, 1.0, 1.0],
        "amber" | "a" => [1.0, 0.55, 0.0],
        "lime" | "l" => [0.75, 1.0, 0.0],
        "cyan" | "c" => [0.0, 1.0, 1.0],
        "magenta" | "m" => [1.0, 0.0, 1.0],
        "yellow" | "y" => [1.0, 1.0, 0.0],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn additive(name: &str, rgb: [f32; 3]) -> EmitterSpec {
        EmitterSpec {
            name: name.into(),
            rgb: Some(rgb),
            subtractive: false,
        }
    }

    fn rgbw() -> Vec<EmitterSpec> {
        vec![
            additive("Red", [1.0, 0.0, 0.0]),
            additive("Green", [0.0, 1.0, 0.0]),
            additive("Blue", [0.0, 0.0, 1.0]),
            additive("White", [1.0, 1.0, 1.0]),
        ]
    }

    fn levels(color: Color, emitters: &[EmitterSpec]) -> Vec<f32> {
        mix(&color, emitters)
            .into_iter()
            .map(|(_, level)| level)
            .collect()
    }

    #[test]
    fn a_primary_passes_through_and_white_takes_the_neutral_part() {
        let red = Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            ..Color::default()
        };
        assert_eq!(
            levels(red, &rgbw()),
            vec![1.0, 0.0, 0.0, 0.0],
            "red is not white"
        );

        let white = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            ..Color::default()
        };
        assert_eq!(levels(white, &rgbw()), vec![1.0, 1.0, 1.0, 1.0]);

        let dim_pink = Color {
            r: 1.0,
            g: 0.4,
            b: 0.4,
            ..Color::default()
        };
        assert_eq!(
            levels(dim_pink, &rgbw()),
            vec![1.0, 0.4, 0.4, 0.4],
            "white takes min(r,g,b)"
        );
    }

    #[test]
    fn an_amber_die_is_zero_on_a_primary_and_full_on_its_own_colour() {
        let emitters = vec![additive("Amber", [1.0, 0.5, 0.0])];
        assert_eq!(
            levels(
                Color {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    ..Color::default()
                },
                &emitters
            ),
            vec![0.0]
        );
        assert_eq!(
            levels(
                Color {
                    r: 0.0,
                    g: 1.0,
                    b: 0.0,
                    ..Color::default()
                },
                &emitters
            ),
            vec![0.0]
        );
        assert_eq!(
            levels(
                Color {
                    r: 1.0,
                    g: 0.5,
                    b: 0.0,
                    ..Color::default()
                },
                &emitters
            ),
            vec![1.0]
        );
    }

    #[test]
    fn a_cmy_flag_is_one_minus_the_component_it_removes() {
        let emitters = vec![
            EmitterSpec {
                name: "Cyan".into(),
                rgb: Some([0.0, 1.0, 1.0]),
                subtractive: true,
            },
            EmitterSpec {
                name: "Magenta".into(),
                rgb: Some([1.0, 0.0, 1.0]),
                subtractive: true,
            },
            EmitterSpec {
                name: "Yellow".into(),
                rgb: Some([1.0, 1.0, 0.0]),
                subtractive: true,
            },
        ];
        // White: every flag out of the beam.
        assert_eq!(
            levels(
                Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    ..Color::default()
                },
                &emitters
            ),
            vec![0.0; 3]
        );
        // Red: cyan full in, the other two out.
        assert_eq!(
            levels(
                Color {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    ..Color::default()
                },
                &emitters
            ),
            vec![0.0, 1.0, 1.0]
        );
    }

    #[test]
    fn an_override_wins_over_anything_derived() {
        let mut overrides = BTreeMap::new();
        overrides.insert("White".to_string(), 0.0);
        let color = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            overrides,
        };
        assert_eq!(
            levels(color, &rgbw()),
            vec![1.0, 1.0, 1.0, 0.0],
            "the white was turned off"
        );
    }

    #[test]
    fn an_unmeasured_emitter_the_name_does_not_settle_stays_dark() {
        let emitters = vec![EmitterSpec {
            name: "Special".into(),
            rgb: None,
            subtractive: false,
        }];
        assert_eq!(
            levels(
                Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    ..Color::default()
                },
                &emitters
            ),
            vec![0.0]
        );
    }

    #[test]
    fn an_unmeasured_emitter_with_a_name_we_know_mixes_by_it() {
        let emitters = vec![EmitterSpec {
            name: "White".into(),
            rgb: None,
            subtractive: false,
        }];
        assert_eq!(
            levels(
                Color {
                    r: 0.5,
                    g: 0.5,
                    b: 0.5,
                    ..Color::default()
                },
                &emitters
            ),
            vec![0.5]
        );
    }

    #[test]
    fn a_measured_red_that_is_not_exactly_red_still_passes_through() {
        let emitters = vec![additive("Red", [1.0, 0.02, 0.01])];
        assert_eq!(
            levels(
                Color {
                    r: 0.7,
                    g: 0.0,
                    b: 0.0,
                    ..Color::default()
                },
                &emitters
            ),
            vec![0.7]
        );
    }
}
