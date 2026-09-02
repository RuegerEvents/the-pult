//! GDTF attribute names, and what the console calls them.
//!
//! The one place the two vocabularies meet. A GDTF channel says `Dimmer`, `Pan`,
//! `ColorAdd_R`, `Gobo1SpinPos`; the console has [`ParameterKind`]. Every mapping in
//! the system is in this table, so a fixture whose zoom lands under the wrong key is
//! one edit away rather than a search.
//!
//! Two rules, and they are what keep the table honest.
//!
//! **A colour is one parameter, not three.** `ColorAdd_R`, `_G`, `_B`, `_W`, `_WW`,
//! `_A`, `_L`, `_UV` and the subtractive `ColorSub_C/M/Y` are all the fixture's
//! colour, and the console holds one `Color` over them with a level per emitter. A
//! reader that made three parameters would give an operator three faders where every
//! other console gives a picker.
//!
//! **Anything unrecognised keeps its own name.** `ParameterKind::Named` carries the
//! GDTF attribute verbatim, so a fixture with a channel nobody has thought about is
//! patchable, controllable and storable in a cue on the day it arrives. Dropping it
//! would be the console deciding a light cannot do something the light says it can.

use pult_gdtf::model::PhysicalUnit as GdtfUnit;
use pult_schema::types::fixture::{ParameterKind, PhysicalUnit};

/// Which of a fixture's emitters a colour attribute drives, and which way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorChannel {
    /// The emitter's name, as the console spells it: `Red`, `White`, `Cyan`.
    pub emitter: &'static str,
    /// A flag that removes light rather than a die that makes it.
    pub subtractive: bool,
}

/// The colour channel this attribute is, if it is one.
pub fn color_channel(attribute: &str) -> Option<ColorChannel> {
    let additive = |emitter| Some(ColorChannel { emitter, subtractive: false });
    let subtractive = |emitter| Some(ColorChannel { emitter, subtractive: true });
    match attribute {
        "ColorAdd_R" | "ColorRGB_Red" => additive("Red"),
        "ColorAdd_G" | "ColorRGB_Green" => additive("Green"),
        "ColorAdd_B" | "ColorRGB_Blue" => additive("Blue"),
        "ColorAdd_W" | "ColorRGB_White" => additive("White"),
        "ColorAdd_WW" => additive("Warm White"),
        "ColorAdd_CW" => additive("Cool White"),
        "ColorAdd_A" | "ColorRGB_Amber" => additive("Amber"),
        "ColorAdd_L" => additive("Lime"),
        "ColorAdd_UV" => additive("UV"),
        "ColorAdd_C" => additive("Cyan"),
        "ColorAdd_M" => additive("Magenta"),
        "ColorAdd_Y" => additive("Yellow"),
        "ColorAdd_RY" => additive("Red-Amber"),
        "ColorAdd_GY" => additive("Green-Yellow"),
        "ColorAdd_BC" => additive("Blue-Cyan"),
        "ColorSub_C" | "ColorRGB_Cyan" => subtractive("Cyan"),
        "ColorSub_M" | "ColorRGB_Magenta" => subtractive("Magenta"),
        "ColorSub_Y" | "ColorRGB_Yellow" => subtractive("Yellow"),
        _ => None,
    }
}

/// The console's parameter kind for a GDTF attribute.
///
/// `None` where the attribute is a *fine* channel — `PanRotate` is not a parameter of
/// its own, it is more bits of `Pan` — and the caller folds it into the coarse one.
pub fn kind_for(attribute: &str) -> Option<ParameterKind> {
    if color_channel(attribute).is_some() {
        return Some(ParameterKind::ColorRgb);
    }
    Some(match attribute {
        "Dimmer" => ParameterKind::Intensity,
        "Pan" => ParameterKind::Pan,
        "Tilt" => ParameterKind::Tilt,
        "Zoom" => ParameterKind::Zoom,
        "Focus" | "Focus1" | "Focus2" => ParameterKind::Focus,
        "Iris" => ParameterKind::Iris,
        "CTC" | "CTO" | "CTB" | "ColorTemperature" => ParameterKind::ColorTemperature,
        // A shutter's *level* and its *rate* are two parameters on one channel in
        // GDTF and two parameters here, because "open" and "four hertz" are not
        // points on one scale and a console that treated them as one would make a
        // strobe rate reachable only by finding it inside a shutter fader.
        "Shutter1" | "Shutter2" | "Shutter3" => ParameterKind::Shutter,
        "Shutter1Strobe" | "StrobeFrequency" | "StrobeDuration" | "StrobeRate" => {
            ParameterKind::Strobe
        }
        _ => return indexed(attribute),
    })
}

/// The attributes GDTF numbers: `Gobo1`, `Gobo2Pos`, `Color2`, `Prism1`, `Frost2`.
fn indexed(attribute: &str) -> Option<ParameterKind> {
    // A rotation before the wheel it rotates, so `Gobo1Pos` does not match `Gobo1`
    // and come out as the wheel itself.
    for (prefix, suffixes) in [
        ("Gobo", &["Pos", "PosRotate", "PosShake", "WheelSpin"][..]),
        ("Color", &["Spin", "WheelSpin", "WheelIndex"][..]),
    ] {
        if let Some(rest) = attribute.strip_prefix(prefix) {
            let (digits, suffix) = split_digits(rest);
            let Some(index) = digits else { continue };
            if suffixes.contains(&suffix) {
                return Some(match prefix {
                    "Gobo" => ParameterKind::GoboRotation(index),
                    _ => ParameterKind::ColorWheel(index),
                });
            }
            if suffix.is_empty() {
                return Some(match prefix {
                    "Gobo" => ParameterKind::Gobo(index),
                    _ => ParameterKind::ColorWheel(index),
                });
            }
        }
    }
    for (prefix, make) in [
        ("Prism", ParameterKind::Prism as fn(u8) -> ParameterKind),
        ("Frost", ParameterKind::Frost as fn(u8) -> ParameterKind),
    ] {
        if let Some(rest) = attribute.strip_prefix(prefix) {
            let (digits, suffix) = split_digits(rest);
            if let Some(index) = digits {
                if suffix.is_empty() || suffix == "Pos" || suffix == "PosRotate" {
                    return Some(make(index));
                }
            }
        }
    }
    // Everything else keeps its own name, which is what makes a fixture with a
    // channel nobody has thought about patchable on the day it arrives.
    Some(ParameterKind::Named(attribute.to_string()))
}

/// Leading digits of a string, and what follows them.
fn split_digits(text: &str) -> (Option<u8>, &str) {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    match text[..end].parse::<u8>() {
        Ok(index) => (Some(index), &text[end..]),
        Err(_) => (None, text),
    }
}

/// The console's unit for a GDTF one.
///
/// The subset the console has a use for. Anything else is `None`, which reads as "a
/// number between 0 and 1" — honest for a channel whose unit nothing downstream would
/// do anything with.
pub fn unit_for(unit: GdtfUnit) -> PhysicalUnit {
    match unit {
        GdtfUnit::Percent => PhysicalUnit::Percent,
        GdtfUnit::Angle => PhysicalUnit::Degrees,
        GdtfUnit::Time => PhysicalUnit::Seconds,
        GdtfUnit::Frequency => PhysicalUnit::Hertz,
        GdtfUnit::Temperature => PhysicalUnit::Kelvin,
        GdtfUnit::Length => PhysicalUnit::Metres,
        GdtfUnit::Power => PhysicalUnit::Watts,
        _ => PhysicalUnit::None,
    }
}

/// The GDTF attribute the console would write for a kind, for export.
///
/// The inverse of [`kind_for`] where there is one. A `Named` kind is its own name
/// again, which is what makes an imported type export as the file it came from.
pub fn attribute_for(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Intensity => "Dimmer".into(),
        ParameterKind::Pan => "Pan".into(),
        ParameterKind::Tilt => "Tilt".into(),
        ParameterKind::Zoom => "Zoom".into(),
        ParameterKind::Focus => "Focus".into(),
        ParameterKind::Iris => "Iris".into(),
        ParameterKind::Shutter => "Shutter1".into(),
        ParameterKind::Strobe => "Shutter1Strobe".into(),
        ParameterKind::ColorTemperature => "CTC".into(),
        ParameterKind::ColorRgb => "ColorAdd_R".into(),
        ParameterKind::Gobo(n) => format!("Gobo{n}"),
        ParameterKind::GoboIndex => "Gobo1".into(),
        ParameterKind::GoboRotation(n) => format!("Gobo{n}Pos"),
        ParameterKind::ColorWheel(n) => format!("Color{n}"),
        ParameterKind::Prism(n) => format!("Prism{n}"),
        ParameterKind::Frost(n) => format!("Frost{n}"),
        ParameterKind::Named(name) => name.clone(),
        // Nothing in GDTF means "the console has no word for this"; a raw channel and
        // a node's port both become a generic control attribute rather than a lie
        // about what they do.
        ParameterKind::Raw(n) => format!("Control{n}"),
        ParameterKind::Switch(n) => format!("Control{n}"),
        ParameterKind::Contact(n) => format!("Control{n}"),
        ParameterKind::Temperature => "Control1".into(),
        ParameterKind::Humidity => "Control1".into(),
        ParameterKind::AirQuality => "Control1".into(),
        ParameterKind::Text => "Control1".into(),
    }
}

/// The emitter name a colour attribute drives, for export.
pub fn color_attribute_for(emitter: &str) -> String {
    match emitter {
        "Red" => "ColorAdd_R",
        "Green" => "ColorAdd_G",
        "Blue" => "ColorAdd_B",
        "White" => "ColorAdd_W",
        "Warm White" => "ColorAdd_WW",
        "Cool White" => "ColorAdd_CW",
        "Amber" => "ColorAdd_A",
        "Lime" => "ColorAdd_L",
        "UV" => "ColorAdd_UV",
        "Cyan" => "ColorSub_C",
        "Magenta" => "ColorSub_M",
        "Yellow" => "ColorSub_Y",
        other => return other.to_string(),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attributes_a_console_has_a_word_for() {
        assert_eq!(kind_for("Dimmer"), Some(ParameterKind::Intensity));
        assert_eq!(kind_for("Pan"), Some(ParameterKind::Pan));
        assert_eq!(kind_for("Zoom"), Some(ParameterKind::Zoom));
        assert_eq!(kind_for("Shutter1"), Some(ParameterKind::Shutter));
        assert_eq!(kind_for("Shutter1Strobe"), Some(ParameterKind::Strobe));
    }

    #[test]
    fn every_colour_channel_is_the_one_colour_parameter() {
        for attribute in ["ColorAdd_R", "ColorAdd_W", "ColorSub_C", "ColorAdd_UV"] {
            assert_eq!(kind_for(attribute), Some(ParameterKind::ColorRgb), "{attribute}");
        }
        assert_eq!(color_channel("ColorAdd_W").unwrap().emitter, "White");
        assert!(color_channel("ColorSub_C").unwrap().subtractive);
        assert!(!color_channel("ColorAdd_R").unwrap().subtractive);
        assert_eq!(color_channel("Zoom"), None);
    }

    #[test]
    fn a_numbered_wheel_keeps_its_number_and_a_rotation_is_not_the_wheel() {
        assert_eq!(kind_for("Gobo1"), Some(ParameterKind::Gobo(1)));
        assert_eq!(kind_for("Gobo2"), Some(ParameterKind::Gobo(2)));
        assert_eq!(kind_for("Gobo1Pos"), Some(ParameterKind::GoboRotation(1)));
        assert_eq!(kind_for("Gobo1PosRotate"), Some(ParameterKind::GoboRotation(1)));
        assert_eq!(kind_for("Color2"), Some(ParameterKind::ColorWheel(2)));
        assert_eq!(kind_for("Prism1"), Some(ParameterKind::Prism(1)));
        assert_eq!(kind_for("Frost1"), Some(ParameterKind::Frost(1)));
    }

    #[test]
    fn an_attribute_nobody_has_a_word_for_keeps_its_own() {
        assert_eq!(
            kind_for("BlowerFanSpeed"),
            Some(ParameterKind::Named("BlowerFanSpeed".into())),
            "a channel the console cannot name is still a channel it can drive",
        );
    }

    #[test]
    fn a_kind_that_came_from_an_attribute_goes_back_to_it() {
        for attribute in ["Dimmer", "Pan", "Tilt", "Zoom", "Iris", "Gobo2", "Prism1", "Frost1"] {
            let kind = kind_for(attribute).unwrap();
            assert_eq!(attribute_for(&kind), attribute, "{attribute}");
        }
        assert_eq!(attribute_for(&ParameterKind::Named("Fog".into())), "Fog");
    }
}
