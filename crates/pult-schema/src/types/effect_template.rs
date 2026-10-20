//! The effect pool: effects as templates, kept to be applied to whatever is selected.
//!
//! A running effect is a spec per fixture with that fixture's phase already worked
//! out (see [`super::effect`]). The selection it was worked out over is not part of it,
//! so a pool entry cannot be one of those. It keeps what is left once the selection is
//! taken away: the curve, the rate, the ends, the width, the direction, *how* the
//! phases were spread, and which parameter it drives. Applying one works the phases out
//! again over the selection of the moment, in the selection's own order.
//!
//! # Layers
//!
//! Fire is intensity and color moving together, and a candle is a flicker with a warm
//! color drifting under it. So an entry is a list of [`EffectLayer`]s, one per
//! parameter, each applied to the fixtures in the selection that have that parameter.
//! A layer a fixture cannot take is skipped for that fixture rather than refused for
//! the selection: "fire" on a row of single-color pars is still the flicker.
//!
//! # A copy, not a reference
//!
//! Task 68 made presets a reference with the literal beside it, so editing a palette
//! reaches every cue standing on it. An effect does not fit that rule. A spec's phase
//! is a number per fixture, worked out from a selection that no longer exists by the
//! time the template is edited, so an edit would have to keep each fixture's phase and
//! replace everything else, and rebuild a spread nobody can now rebuild. So applying a
//! template copies it, and `EffectSpec::template` remembers where the copy came from
//! so the panel can name it. Editing the pool changes the next apply and nothing that
//! is already running or stored.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::effect::{Curve, Direction, Easing, Rate, Shape, Spread, Step};
use super::fixture::{ParameterKind, ParameterValue};
use crate::PultSchema;

/// Fixed for ever: the seeded templates' ids are v5 names under it.
const NAMESPACE: Uuid = Uuid::from_u128(0x6566_6665_6374_4f50_8b2e_4c1a_9d3f_7e51);

/// One parameter's part of a template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectLayer {
    /// What it drives. Applied to the fixtures in the selection that have this.
    pub kind: ParameterKind,
    pub curve: Curve,
    pub rate: Rate,
    pub low: ParameterValue,
    pub high: ParameterValue,
    pub width: f32,
    pub direction: Direction,
    pub spread: Spread,
    /// How many cycles the spread covers across the selection. A chase over a strip
    /// with `2` runs twice along it, which is what a pixel desk calls segments.
    #[serde(default = "one")]
    pub spread_cycles: f32,
}

fn one() -> f32 {
    1.0
}

/// A named effect in the pool.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "effect_templates")]
pub struct EffectTemplate {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub layers: Vec<EffectLayer>,
}

impl EffectTemplate {
    fn seeded(name: &str, layers: Vec<EffectLayer>) -> Self {
        EffectTemplate {
            id: Uuid::new_v5(&NAMESPACE, name.as_bytes()),
            name: name.to_string(),
            layers,
        }
    }
}

fn level(value: f32) -> ParameterValue {
    ParameterValue::Float(value)
}

fn layer(kind: ParameterKind, curve: Curve, hz: f32, low: ParameterValue, high: ParameterValue) -> EffectLayer {
    EffectLayer {
        kind,
        curve,
        rate: Rate::Hz(hz),
        low,
        high,
        width: 0.5,
        direction: Direction::Forward,
        spread: Spread::Even,
        spread_cycles: 1.0,
    }
}

fn steps(points: &[(f32, ParameterValue, Easing)]) -> Curve {
    Curve::Steps(
        points
            .iter()
            .map(|(at, value, easing)| Step { at: *at, value: value.clone(), easing: *easing })
            .collect(),
    )
}

/// What a new show's pool starts with: the looks a haunt reaches for first.
///
/// Seeded once, when a show is made, the way the paperwork sheets are. A show made
/// before the pool existed opens with an empty one, and a later version's better
/// templates do not arrive in a show made today.
pub fn default_effect_templates() -> Vec<EffectTemplate> {
    use Easing::{EaseOut, Linear, Step as Hard};
    use ParameterKind::{ColorRgb, Intensity};
    // Each fixture flickers on its own: the random spread salts each one's noise.
    let own = Spread::Random { seed: 0x4848_4732 };

    vec![
        EffectTemplate::seeded(
            "Flicker",
            vec![EffectLayer {
                spread: own,
                ..layer(Intensity, Curve::Shape(Shape::Flicker), 3.0, level(0.35), level(1.0))
            }],
        ),
        EffectTemplate::seeded(
            "Candle",
            vec![
                EffectLayer {
                    spread: own,
                    ..layer(Intensity, Curve::Shape(Shape::Flicker), 1.2, level(0.65), level(1.0))
                },
                EffectLayer {
                    spread: own,
                    ..layer(
                        ColorRgb,
                        Curve::Shape(Shape::Flicker),
                        0.7,
                        ParameterValue::rgb(1.0, 0.42, 0.05),
                        ParameterValue::rgb(1.0, 0.62, 0.18),
                    )
                },
            ],
        ),
        EffectTemplate::seeded(
            "Lightning",
            // Even, so the whole rig strikes at once.
            vec![EffectLayer {
                width: 0.25,
                ..layer(Intensity, Curve::Shape(Shape::Lightning), 0.15, level(0.0), level(1.0))
            }],
        ),
        EffectTemplate::seeded(
            "Heartbeat",
            vec![layer(
                Intensity,
                steps(&[
                    (0.0, level(0.1), Linear),
                    (0.06, level(1.0), EaseOut),
                    (0.16, level(0.2), Linear),
                    (0.24, level(0.75), EaseOut),
                    (0.36, level(0.1), Hard),
                ]),
                1.1,
                level(0.0),
                level(1.0),
            )],
        ),
        EffectTemplate::seeded(
            "Fire",
            vec![
                EffectLayer {
                    spread: own,
                    ..layer(Intensity, Curve::Shape(Shape::Flicker), 4.0, level(0.45), level(1.0))
                },
                EffectLayer {
                    spread: own,
                    ..layer(
                        ColorRgb,
                        Curve::Shape(Shape::Flicker),
                        2.5,
                        ParameterValue::rgb(1.0, 0.08, 0.0),
                        ParameterValue::rgb(1.0, 0.45, 0.02),
                    )
                },
            ],
        ),
        EffectTemplate::seeded(
            "Chase",
            vec![EffectLayer {
                width: 0.25,
                spread: Spread::Linear,
                ..layer(Intensity, Curve::Shape(Shape::Square), 1.0, level(0.0), level(1.0))
            }],
        ),
        EffectTemplate::seeded(
            "Breathing",
            vec![layer(Intensity, Curve::Shape(Shape::Sine), 0.2, level(0.05), level(0.9))],
        ),
        EffectTemplate::seeded(
            "Police",
            // Every other fixture opposite, which is what two beacons on one car do.
            vec![EffectLayer {
                spread: Spread::Groups(2),
                ..layer(
                    ColorRgb,
                    steps(&[
                        (0.0, ParameterValue::rgb(1.0, 0.0, 0.0), Hard),
                        (0.5, ParameterValue::rgb(0.0, 0.1, 1.0), Hard),
                    ]),
                    2.0,
                    ParameterValue::rgb(0.0, 0.0, 0.0),
                    ParameterValue::rgb(1.0, 1.0, 1.0),
                )
            }],
        ),
        EffectTemplate::seeded(
            "Strobe burst",
            // Eight flashes in the first half of a two-second cycle, then dark.
            vec![layer(
                Intensity,
                {
                    let mut points: Vec<(f32, ParameterValue, Easing)> = (0..8)
                        .flat_map(|i| {
                            let at = i as f32 / 16.0;
                            [(at, level(1.0), Hard), (at + 1.0 / 32.0, level(0.0), Hard)]
                        })
                        .collect();
                    points.push((0.5, level(0.0), Hard));
                    steps(&points)
                },
                0.5,
                level(0.0),
                level(1.0),
            )],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids are names, so two shows seeded by two builds agree on what "Fire" is.
    #[test]
    fn the_seeded_templates_are_named_and_stable() {
        let a = default_effect_templates();
        let b = default_effect_templates();
        assert_eq!(a.len(), 9);
        assert_eq!(a.iter().map(|t| t.id).collect::<Vec<_>>(), b.iter().map(|t| t.id).collect::<Vec<_>>());
        let names: Vec<&str> = a.iter().map(|t| t.name.as_str()).collect();
        for want in ["Flicker", "Candle", "Lightning", "Heartbeat", "Fire", "Chase", "Breathing", "Police", "Strobe burst"] {
            assert!(names.contains(&want), "{want} is in the pool");
        }
    }

    /// Every layer's ends are the kind of value its parameter holds, or applying one
    /// would write a color into an intensity.
    #[test]
    fn every_layer_speaks_its_parameters_type() {
        for template in default_effect_templates() {
            for layer in &template.layers {
                let color = matches!(layer.kind, ParameterKind::ColorRgb);
                let fits = |v: &ParameterValue| match v {
                    ParameterValue::Color { .. } => color,
                    ParameterValue::Float(_) => !color,
                    _ => false,
                };
                assert!(fits(&layer.low) && fits(&layer.high), "{} ends", template.name);
                if let Curve::Steps(steps) = &layer.curve {
                    assert!(steps.iter().all(|s| fits(&s.value)), "{} steps", template.name);
                    assert!(steps.iter().all(|s| (0.0..1.0).contains(&s.at)), "{} positions", template.name);
                }
            }
        }
    }

    /// A template written before `spread_cycles` existed reads as one cycle.
    #[test]
    fn a_layer_without_spread_cycles_covers_one() {
        let mut value = serde_json::to_value(&default_effect_templates()[0].layers[0]).unwrap();
        value.as_object_mut().unwrap().remove("spread_cycles");
        let back: EffectLayer = serde_json::from_value(value).unwrap();
        assert_eq!(back.spread_cycles, 1.0);
    }
}
