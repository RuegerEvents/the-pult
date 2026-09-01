//! What a parameter can be.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "type", content = "value")]
pub enum ParameterValue {
    Float(f32),
    Int(i32),
    Color { r: f32, g: f32, b: f32 },
    Bool(bool),
    Text(String),
}

impl ParameterValue {
    /// This value, moved by `delta` — what "ten percent brighter" comes to.
    ///
    /// Here rather than in the backend because it is a fact about the type: a
    /// browser nudging a fader, a plugin doing its own arithmetic and the engine
    /// resolving a relative write should all agree on what a delta does, and the
    /// only way to be sure of that is for there to be one of it.
    ///
    /// Floats are normalised 0..1 everywhere in the console, so that is the range a
    /// nudge comes to rest inside: past the top is the top, not beyond it. A colour
    /// moves on every channel, which is what makes a nudge on one mean "brighter"
    /// rather than "redder". `Bool` and `Text` refuse — addition means nothing to a
    /// relay or to a line of text, and quietly doing nothing would be worse than
    /// saying so.
    pub fn nudged(&self, delta: f32) -> Result<ParameterValue, String> {
        Ok(match self {
            ParameterValue::Float(v) => ParameterValue::Float((v + delta).clamp(0.0, 1.0)),
            // Rounded rather than truncated, so a nudge of +1 on a gobo wheel is one
            // gobo along however the float arrived.
            ParameterValue::Int(v) => {
                ParameterValue::Int(v.saturating_add(delta.round() as i32))
            }
            ParameterValue::Color { r, g, b } => ParameterValue::Color {
                r: (r + delta).clamp(0.0, 1.0),
                g: (g + delta).clamp(0.0, 1.0),
                b: (b + delta).clamp(0.0, 1.0),
            },
            ParameterValue::Bool(_) => {
                return Err("that parameter is on or off; there is no halfway to move it".into())
            }
            ParameterValue::Text(_) => {
                return Err("that parameter is a line of text, which cannot be nudged".into())
            }
        })
    }
}

/// One value on the way to another.
///
/// A boolean has nothing between its two states, so it switches at the start of the
/// fade rather than at the end, where it would look like a late cue. Text does not
/// interpolate at all and arrives when the fade does.
pub fn interpolate(from: &ParameterValue, to: &ParameterValue, t: f32) -> ParameterValue {
    use ParameterValue::*;
    match (from, to) {
        (Float(a), Float(b)) => Float(a + (b - a) * t),
        (Int(a), Int(b)) => Int((*a as f32 + (*b as f32 - *a as f32) * t).round() as i32),
        (Color { r: r0, g: g0, b: b0 }, Color { r: r1, g: g1, b: b1 }) => Color {
            r: r0 + (r1 - r0) * t,
            g: g0 + (g1 - g0) * t,
            b: b0 + (b1 - b0) * t,
        },
        (Bool(a), Bool(b)) => Bool(if t > 0.0 { *b } else { *a }),
        // Mismatched kinds, or text: there is nothing in between, so the
        // destination arrives when the fade does.
        _ => {
            if t >= 1.0 {
                to.clone()
            } else {
                from.clone()
            }
        }
    }
}
