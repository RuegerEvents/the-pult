//! A grandMA3 command wing, as arithmetic: what a key types, what an encoder turns,
//! which keys belong to which fader, which LED lights which key.
//!
//! Pure, because every rule here was decided once at the desk and has to stay decided.
//! These were the page's (`frontend/src/lib/wing.ts`) until the station took the whole
//! wing over; the rules are the same ones, and the reasons beside them are the reasons
//! they were given there.
//!
//! Names come from MA's own control map, which the station reads out of a local
//! grandMA3 install — so nothing here carries MA's table, only the handful of codes
//! that mean something to this console.

use std::collections::HashMap;

use pult_schema::types::{
    fixture::{FixtureType, ParameterDirection, ParameterKind, ParameterValue},
    parameter_key,
    show::FadeGroup,
    wing::{ColorComponent, WingSlot},
};
use pult_wing::WingMap;
use uuid::Uuid;

// ── The keypad ────────────────────────────────────────────────────────────────

/// What a key types into the command line, by MA's code.
///
/// The legends already are this console's grammar wherever a word exists for them —
/// `fixture 1 thru 5 at 50`, `group 3`, `store sequence 1 cue 4` — so a key types its
/// word, and only the keys that have one are here. CHANNEL is deliberately not: on MA it
/// selects by channel number, which this console has no separate idea of, and typing
/// `fixture` for it lit a key that only duplicated FIXTURE.
pub fn keypad_word(code: &str) -> Option<&'static str> {
    Some(match code {
        "NUM0" => "0",
        "NUM1" => "1",
        "NUM2" => "2",
        "NUM3" => "3",
        "NUM4" => "4",
        "NUM5" => "5",
        "NUM6" => "6",
        "NUM7" => "7",
        "NUM8" => "8",
        "NUM9" => "9",
        "DOT" => ".",
        "PLUS" => "+",
        "MINUS" => "-",
        "THRU" => "thru",
        "AT" => "at",
        "FULL" => "full",
        "FIXTURE" => "fixture",
        "GROUP" => "group",
        "PRESET" => "preset",
        "CUE" => "cue",
        "SEQUENCE" => "sequence",
        "STORE" => "store",
        "UPDATE" => "update",
        "DELETE" => "delete",
        "HELP" => "help",
        "OFF" => "off",
        _ => return None,
    })
}

/// What the keys that are not words do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Please,
    Clear,
    Escape,
    Oops,
    Go,
    GoBack,
    /// UP and DOWN: the encoders' page of attributes.
    PageUp,
    PageDown,
    /// PAGE_UP and PAGE_DOWN: the page of bindings every control acts on.
    NextPage,
    PrevPage,
    Learn,
}

pub fn key_verb(code: &str) -> Option<Verb> {
    Some(match code {
        "PLEASE" => Verb::Please,
        "CLEAR" => Verb::Clear,
        "ESC" => Verb::Escape,
        "UNDO" => Verb::Oops,
        "GO" | "DEF_GO" => Verb::Go,
        "GOBACK" | "DEF_GOBACK" => Verb::GoBack,
        "UP" => Verb::PageUp,
        "DOWN" => Verb::PageDown,
        "PAGE_UP" => Verb::NextPage,
        "PAGE_DOWN" => Verb::PrevPage,
        "LEARN" => Verb::Learn,
        _ => return None,
    })
}

/// An X key, which MA gives no meaning of its own: a playback key once something is
/// stored onto it.
pub fn x_key(code: &str) -> bool {
    code.strip_prefix('X').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The line with one more key typed onto it.
///
/// Digits run together, so `1` `2` is twelve; everything else is a word of its own,
/// and that includes the dot — `fixture 1 thru 20 . 1 thru 4` cuts cells, where a dot
/// between two digits would make one number of them.
pub fn typed(line: &str, word: &str) -> String {
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        return word.to_string();
    }
    let digit = |c: char| c.is_ascii_digit();
    let single_digit = word.len() == 1 && word.chars().all(digit);
    if single_digit && trimmed.chars().last().is_some_and(digit) {
        return format!("{trimmed}{word}");
    }
    format!("{trimmed} {word}")
}

/// MA's Oops on a line being typed: the last word goes, a run of digits being one.
pub fn oops(line: &str) -> String {
    let mut words: Vec<&str> = line.split_whitespace().collect();
    words.pop();
    words.join(" ")
}

// ── Faders and their keys ─────────────────────────────────────────────────────

/// What a hardkey is, in relation to a fader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaderKey {
    /// A hand on the fader: MA models touch as a hardkey.
    Touch(u16),
    /// The key of the fader's own executor, under it: Go, for a fader that is a master.
    Go(u16),
    /// The key of the executor a hundred below it in the same column, above it: Off.
    Off(u16),
}

/// Which hardkeys belong to which fader, from MA's table. The `FADER` key of a fader's
/// own executor is the hand on it, its `EXEC` key is the one under it, and the `EXEC`
/// key a hundred below is the one above, in the same column.
pub fn fader_keys(map: &WingMap) -> HashMap<u16, FaderKey> {
    let by_executor: HashMap<&str, u16> = map
        .faders
        .iter()
        .filter_map(|(index, c)| Some((c.executor.as_deref()?, *index as u16)))
        .collect();
    let mut out = HashMap::new();
    for (index, control) in &map.hardkeys {
        let Some(executor) = control.executor.as_deref() else { continue };
        match control.code.as_deref() {
            Some("FADER") => {
                if let Some(fader) = by_executor.get(executor) {
                    out.insert(*index, FaderKey::Touch(*fader));
                }
            }
            Some("EXEC") => {
                if let Some(fader) = by_executor.get(executor) {
                    out.insert(*index, FaderKey::Go(*fader));
                }
                if let Ok(n) = executor.parse::<u32>() {
                    if let Some(fader) = by_executor.get((n + 100).to_string().as_str()) {
                        out.insert(*index, FaderKey::Off(*fader));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A fader's full travel, as the wing reports it: the ADC is 12-bit, the top one past.
pub const FADER_FULL: f32 = 4096.0;

/// A fader's travel as a level, 0..1.
pub fn level_of(raw: u16) -> f32 {
    (raw as f32 / FADER_FULL).clamp(0.0, 1.0)
}

// ── The encoders ──────────────────────────────────────────────────────────────

/// How many encoders sit under the screen, each an attribute of the bar.
pub const ENCODERS: usize = 5;

/// What an encoder is: one of the five under the screen (coarse inside, fine outside,
/// as MA's own manual has it), or the dimmer wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Slot { slot: usize, fine: bool },
    DimmerWheel,
}

pub fn encoder_role(map: &WingMap, index: u8) -> Option<Encoder> {
    let kind = map.encoders.get(&index)?.kind.as_deref()?;
    if kind == "WheelMaster" {
        return Some(Encoder::DimmerWheel);
    }
    let (fine, n) = match kind.strip_prefix("Inside") {
        Some(n) => (false, n),
        None => (true, kind.strip_prefix("Outside")?),
    };
    let slot: usize = n.parse().ok()?;
    (1..=ENCODERS).contains(&slot).then_some(Encoder::Slot { slot: slot - 1, fine })
}

/// Which of the five large encoders a key is the press of. MA's table names an inner
/// and an outer press for each; on this wing only the inner knob presses, but both are
/// read so a wing whose ring presses too is not left with a key that means nothing.
pub fn encoder_press(code: &str) -> Option<usize> {
    let n = code.strip_prefix("ENCODER_INSIDE").or_else(|| code.strip_prefix("ENCODER_OUTSIDE"))?;
    let slot: usize = n.parse().ok()?;
    (1..=ENCODERS).contains(&slot).then(|| slot - 1)
}

/// One click of the inner knob is 2% of the travel; of the outer ring a tenth of that,
/// MA's ratio.
pub const COARSE: f32 = 0.02;
pub const FINE: f32 = 0.002;
/// One click of the dimmer wheel. Its table says 256 a turn; half a percent a click puts
/// the whole travel under a little more than half of one, which has not been measured
/// against a hand.
pub const WHEEL: f32 = 0.005;

/// The attributes the selection has, in the order the encoders page through them:
/// intensity, position, color, beam and the rest — the order an operator builds a look
/// in — and the selection's own order within each. A color is three slots, red, green
/// and blue, because a knob turns one number; a line of text has no number to turn.
pub fn encoder_slots<'t>(
    fixtures: impl IntoIterator<Item = Uuid>,
    type_of: impl Fn(&Uuid) -> Option<&'t FixtureType>,
) -> Vec<WingSlot> {
    let mut rows: Vec<(ParameterKind, ParameterValue)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for fixture in fixtures {
        let Some(fixture_type) = type_of(&fixture) else { continue };
        for p in fixture_type.parameters.iter().filter(|p| p.direction == ParameterDirection::Output) {
            if seen.insert(parameter_key(&p.kind)) {
                rows.push((p.kind.clone(), p.default_value.clone()));
            }
        }
    }
    let order = |kind: &ParameterKind| match FadeGroup::of_kind(kind) {
        FadeGroup::Intensity => 0,
        FadeGroup::Position => 1,
        FadeGroup::Color => 2,
        FadeGroup::Beam => 3,
        FadeGroup::Other => 4,
    };
    rows.sort_by_key(|(kind, _)| order(kind));
    let mut out = Vec::new();
    for (kind, default_value) in rows {
        match default_value {
            ParameterValue::Text(_) => {}
            ParameterValue::Color { .. } => {
                for component in [ColorComponent::R, ColorComponent::G, ColorComponent::B] {
                    out.push(WingSlot { kind: kind.clone(), component: Some(component), default_value: default_value.clone() });
                }
            }
            _ => out.push(WingSlot { kind, component: None, default_value }),
        }
    }
    out
}

pub fn page_count(slots: usize) -> usize {
    slots.div_ceil(ENCODERS).max(1)
}

/// A value after `clicks` of an encoder moving `step` a click.
///
/// A float moves by a fraction of its travel; a whole number by one step a click
/// however it is turned, because the steps of a gobo wheel are the steps; a switch goes
/// on turned up and off turned down; one channel of a color moves alone.
pub fn turned(value: &ParameterValue, clicks: i32, step: f32, component: Option<ColorComponent>) -> ParameterValue {
    let by = step * clicks as f32;
    match value {
        ParameterValue::Float(v) => ParameterValue::Float((v + by).clamp(0.0, 1.0)),
        ParameterValue::Int(v) => ParameterValue::Int((v + clicks.signum() * clicks.abs().max(1)).max(0)),
        ParameterValue::Bool(on) => {
            if clicks == 0 {
                ParameterValue::Bool(*on)
            } else {
                ParameterValue::Bool(clicks > 0)
            }
        }
        ParameterValue::Color { r, g, b, overrides } => {
            let (mut r, mut g, mut b) = (*r, *g, *b);
            match component {
                Some(ColorComponent::R) => r = (r + by).clamp(0.0, 1.0),
                Some(ColorComponent::G) => g = (g + by).clamp(0.0, 1.0),
                Some(ColorComponent::B) => b = (b + by).clamp(0.0, 1.0),
                None => {}
            }
            ParameterValue::Color { r, g, b, overrides: overrides.clone() }
        }
        other => other.clone(),
    }
}

// ── Typing a value: the press of a large encoder ──────────────────────────────

/// The calculator's text after one keypad word: digits and one dot, as many as a value
/// needs. Any other word types nothing — the calculator is a number, and `at` going into
/// it would be a value nobody can read.
pub fn calculator_typed(text: &str, word: &str) -> String {
    if word.len() == 1 && word.chars().all(|c| c.is_ascii_digit()) {
        return if text.len() < 8 { format!("{text}{word}") } else { text.to_string() };
    }
    if word == "." {
        if text.contains('.') {
            return text.to_string();
        }
        return format!("{}.", if text.is_empty() { "0" } else { text });
    }
    text.to_string()
}

/// What the typed text sets an attribute to, read in the units its bar shows: a
/// percentage for a level or one channel of a color, a step for a wheel, zero or not for
/// a switch. `None` when there is nothing to set, or nothing the text could mean.
pub fn typed_value(text: &str, like: &ParameterValue, component: Option<ColorComponent>) -> Option<ParameterValue> {
    if text.is_empty() || text == "." {
        return None;
    }
    let n: f32 = text.parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    Some(match like {
        ParameterValue::Float(_) => ParameterValue::Float((n / 100.0).clamp(0.0, 1.0)),
        ParameterValue::Int(_) => ParameterValue::Int(n.round().max(0.0) as i32),
        ParameterValue::Bool(_) => ParameterValue::Bool(n != 0.0),
        ParameterValue::Color { r, g, b, overrides } => {
            let level = (n / 100.0).clamp(0.0, 1.0);
            let (mut r, mut g, mut b) = (*r, *g, *b);
            match component? {
                ColorComponent::R => r = level,
                ColorComponent::G => g = level,
                ColorComponent::B => b = level,
            }
            ParameterValue::Color { r, g, b, overrides: overrides.clone() }
        }
        _ => return None,
    })
}

// ── Lights ────────────────────────────────────────────────────────────────────

/// How lit a key is: off, dim for "you can press this", bright for "this is live".
pub const DIM: u8 = 24;
pub const LIT: u8 = 255;

/// Which hardkey each LED lights, as `(led, key)`. A key's LED carries the key's own
/// code — `STORE`, `X3` — and an executor button's carries no code and the executor's
/// number. An LED with three channels and no code is an encoder's ring, and the one with
/// neither is the desk lamp; neither is a key.
pub fn led_keys(map: &WingMap) -> Vec<(usize, u16)> {
    let mut by_code: HashMap<&str, u16> = HashMap::new();
    let mut by_executor: HashMap<&str, u16> = HashMap::new();
    for (index, control) in &map.hardkeys {
        match (control.code.as_deref(), control.executor.as_deref()) {
            (Some("EXEC"), Some(executor)) => {
                by_executor.insert(executor, *index);
            }
            (Some(code), _) if code != "FADER" && code != "EXEC" => {
                by_code.insert(code, *index);
            }
            _ => {}
        }
    }
    map.leds
        .iter()
        .enumerate()
        .filter_map(|(at, led)| {
            let key = match (led.code.as_deref(), led.executor.as_deref()) {
                (Some(code), _) if code != "EXEC" => by_code.get(code).copied(),
                (None, Some(executor)) if led.g.is_none() => by_executor.get(executor).copied(),
                _ => None,
            };
            key.map(|key| (at, key))
        })
        .collect()
}

/// The frame channels an LED drives.
pub fn led_channels(led: &pult_wing::map::Led) -> impl Iterator<Item = usize> + '_ {
    [led.r, led.g, led.b].into_iter().flatten()
}

/// How many digital inputs the wing has, on its rear panel.
pub const DIGITAL_INPUTS: u16 = 7;

/// Which digital inputs went down or came up between two readings, lowest first. An
/// input reads as pressed while its bit is set.
pub fn digital_edges(before: u8, after: u8) -> Vec<(u16, bool)> {
    (0..DIGITAL_INPUTS)
        .filter_map(|index| {
            let (was, now) = ((before >> index) & 1, (after >> index) & 1);
            (was != now).then_some((index, now == 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_run_together_and_words_do_not() {
        assert_eq!(typed("", "fixture"), "fixture");
        assert_eq!(typed("fixture", "1"), "fixture 1");
        assert_eq!(typed("fixture 1", "2"), "fixture 12");
        assert_eq!(typed("fixture 12", "."), "fixture 12 .");
        assert_eq!(typed("fixture 12 .", "3"), "fixture 12 . 3");
        assert_eq!(oops("fixture 12 thru"), "fixture 12");
        assert_eq!(oops(""), "");
    }

    #[test]
    fn an_encoder_turns_the_kind_it_is_on() {
        assert_eq!(turned(&ParameterValue::Float(0.5), 5, COARSE, None), ParameterValue::Float(0.6));
        assert_eq!(turned(&ParameterValue::Float(0.99), 5, COARSE, None), ParameterValue::Float(1.0));
        assert_eq!(turned(&ParameterValue::Int(3), -1, FINE, None), ParameterValue::Int(2));
        assert_eq!(turned(&ParameterValue::Int(0), -4, COARSE, None), ParameterValue::Int(0));
        assert_eq!(turned(&ParameterValue::Bool(false), 1, COARSE, None), ParameterValue::Bool(true));
        let red = ParameterValue::Color { r: 0.5, g: 0.0, b: 0.0, overrides: Default::default() };
        let ParameterValue::Color { r, g, .. } = turned(&red, 10, COARSE, Some(ColorComponent::R)) else { panic!() };
        assert!((r - 0.7).abs() < 1e-6 && g == 0.0);
    }

    #[test]
    fn the_calculator_types_a_number_and_reads_it_as_a_percentage() {
        assert_eq!(calculator_typed("", "."), "0.");
        assert_eq!(calculator_typed("5", "0"), "50");
        assert_eq!(calculator_typed("50", "at"), "50");
        assert_eq!(typed_value("50", &ParameterValue::Float(0.0), None), Some(ParameterValue::Float(0.5)));
        assert_eq!(typed_value("3", &ParameterValue::Int(0), None), Some(ParameterValue::Int(3)));
        assert_eq!(typed_value("", &ParameterValue::Float(0.0), None), None);
    }

    #[test]
    fn a_contact_closing_and_opening_are_edges() {
        assert_eq!(digital_edges(0b000, 0b010), vec![(1, true)]);
        assert_eq!(digital_edges(0b011, 0b001), vec![(1, false)]);
        assert!(digital_edges(5, 5).is_empty());
    }

    #[test]
    fn an_encoder_press_is_one_of_the_five() {
        assert_eq!(encoder_press("ENCODER_INSIDE1"), Some(0));
        assert_eq!(encoder_press("ENCODER_OUTSIDE5"), Some(4));
        assert_eq!(encoder_press("ENCODER_INSIDE6"), None);
        assert_eq!(encoder_press("STORE"), None);
    }
}
