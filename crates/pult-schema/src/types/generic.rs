//! The fixture types a console knows without being given a file.
//!
//! A dimmer, a par in every common mix of dies, a moving head: what somebody patches
//! when there is no GDTF for the unit in front of them, or no time to find one. They
//! live here rather than in the browser because a plugin, the command line and the
//! patch dialog all offer them, and because the emitter list *is* the channel layout
//! (see [`super::fixture::implicit_width`]): an RGBWA+UV par is six bytes of color
//! because it names six emitters, and nothing else has to say so.
//!
//! **Each one has a fixed id**, a v5 over its key, so patching a second RGBW par finds
//! the type the first one brought into the show rather than making another — the rule
//! a node's derived type already follows. Copied into the show as
//! [`FixtureTypeSource::Manual`], so it is an ordinary type from then on and can be
//! edited without anything coming along to put it back.

use pult_render::ParameterValue;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::fixture::{
    implicit_width, Emitter, FixtureType, FixtureTypeSource, ParameterDefinition, ParameterKind, Vec3,
};

/// The namespace every generic type's id is derived in.
const GENERIC: Uuid = Uuid::from_u128(0x7a1c_3e52_95b4_4f0e_8d61_2c0f_6a9e_b311);

/// A named set of emitters a color can mix across: what the type editor offers as
/// one click before anybody builds their own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EmitterPreset {
    pub name: String,
    pub emitters: Vec<Emitter>,
}

fn additive(name: &str, r: f32, g: f32, b: f32) -> Emitter {
    Emitter { name: name.into(), rgb: Some(Vec3 { x: r, y: g, z: b }), subtractive: false }
}

fn subtractive(name: &str, r: f32, g: f32, b: f32) -> Emitter {
    Emitter { name: name.into(), rgb: Some(Vec3 { x: r, y: g, z: b }), subtractive: true }
}

/// Every die the presets use, by the name GDTF gives it, which is also how a color
/// override names one. Amber and UV are the colors `pult_render::color`'s own tests
/// mix against; lime is the yellow-green a lime die actually is.
fn die(name: &str) -> Emitter {
    match name {
        "Red" => additive("Red", 1.0, 0.0, 0.0),
        "Green" => additive("Green", 0.0, 1.0, 0.0),
        "Blue" => additive("Blue", 0.0, 0.0, 1.0),
        "White" => additive("White", 1.0, 1.0, 1.0),
        "Amber" => additive("Amber", 1.0, 0.55, 0.0),
        "Lime" => additive("Lime", 0.75, 1.0, 0.0),
        "UV" => additive("UV", 0.25, 0.0, 1.0),
        "Cyan" => subtractive("Cyan", 0.0, 1.0, 1.0),
        "Magenta" => subtractive("Magenta", 1.0, 0.0, 1.0),
        "Yellow" => subtractive("Yellow", 1.0, 1.0, 0.0),
        other => Emitter { name: other.into(), rgb: None, subtractive: false },
    }
}

fn dies(names: &[&str]) -> Vec<Emitter> {
    names.iter().map(|n| die(n)).collect()
}

/// The mixes the type editor offers, in the order a person reaches for them.
pub fn emitter_presets() -> Vec<EmitterPreset> {
    [
        ("RGB", &["Red", "Green", "Blue"][..]),
        ("RGBW", &["Red", "Green", "Blue", "White"]),
        ("RGBA", &["Red", "Green", "Blue", "Amber"]),
        ("RGBWA", &["Red", "Green", "Blue", "White", "Amber"]),
        ("RGBWA+UV", &["Red", "Green", "Blue", "White", "Amber", "UV"]),
        ("RGBAL", &["Red", "Green", "Blue", "Amber", "Lime"]),
        ("CMY", &["Cyan", "Magenta", "Yellow"]),
    ]
    .into_iter()
    .map(|(name, names)| EmitterPreset { name: name.into(), emitters: dies(names) })
    .collect()
}

fn color(emitters: Vec<Emitter>) -> ParameterDefinition {
    ParameterDefinition {
        emitters,
        feature_group: Some("Color".into()),
        ..ParameterDefinition::new(ParameterKind::ColorRgb, ParameterValue::rgb(1.0, 1.0, 1.0))
    }
}

fn plain(kind: ParameterKind, rest: f32, group: &str) -> ParameterDefinition {
    ParameterDefinition {
        feature_group: Some(group.into()),
        ..ParameterDefinition::new(kind, ParameterValue::Float(rest))
    }
}

fn a_type(key: &str, name: &str, beam_deg: f32, parameters: Vec<ParameterDefinition>) -> FixtureType {
    let mut kind = FixtureType {
        id: generic_id(key),
        name: name.into(),
        manufacturer: "Generic".into(),
        channel_count: parameters.iter().map(implicit_width).sum(),
        parameters,
        source: FixtureTypeSource::Manual,
        ..FixtureType::default()
    };
    kind.physical.beam_angle_deg = Some(beam_deg);
    kind
}

/// The id a generic type has in every show it is patched into.
pub fn generic_id(key: &str) -> Uuid {
    Uuid::new_v5(&GENERIC, key.as_bytes())
}

/// Every generic type, dimmer first.
pub fn library() -> Vec<FixtureType> {
    let mut types = vec![a_type("dimmer", "Generic dimmer", 30.0, vec![plain(ParameterKind::Intensity, 0.0, "Dimmer")])];
    for preset in emitter_presets() {
        types.push(a_type(
            &format!("par-{}", preset.name.to_lowercase()),
            &format!("Generic {} par", preset.name),
            25.0,
            vec![plain(ParameterKind::Intensity, 0.0, "Dimmer"), color(preset.emitters)],
        ));
    }
    types.push(a_type(
        "moving-head",
        "Generic moving head",
        15.0,
        vec![
            plain(ParameterKind::Intensity, 0.0, "Dimmer"),
            plain(ParameterKind::Pan, 0.5, "Position"),
            plain(ParameterKind::Tilt, 0.5, "Position"),
            color(dies(&["Red", "Green", "Blue", "White"])),
            plain(ParameterKind::Zoom, 0.5, "Beam"),
            plain(ParameterKind::Strobe, 0.0, "Beam"),
        ],
    ));
    types
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_rgbwa_uv_par_is_seven_channels_with_a_byte_per_die() {
        let par = library().into_iter().find(|t| t.name == "Generic RGBWA+UV par").unwrap();
        assert_eq!(par.channel_count, 7, "a dimmer and six dies");
        let mode = par.mode("Default");
        assert_eq!(mode.breaks, vec![7]);
        let dies: Vec<_> = mode.channels.iter().filter_map(|c| c.emitter.clone()).collect();
        assert_eq!(dies, ["Red", "Green", "Blue", "White", "Amber", "UV"]);
        let offsets: Vec<_> = mode.channels.iter().flat_map(|c| c.offsets.clone()).collect();
        assert_eq!(offsets, [1, 2, 3, 4, 5, 6, 7], "one byte each, in order");
    }

    #[test]
    fn a_generic_type_has_the_same_id_in_every_show() {
        let a = library();
        let b = library();
        assert_eq!(a.iter().map(|t| t.id).collect::<Vec<_>>(), b.iter().map(|t| t.id).collect::<Vec<_>>());
        let mut ids: Vec<_> = a.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), a.len(), "no two share one");
    }

    #[test]
    fn a_plain_color_is_still_three_bytes() {
        let rgb = library().into_iter().find(|t| t.name == "Generic RGB par").unwrap();
        assert_eq!(rgb.channel_count, 4);
    }
}
