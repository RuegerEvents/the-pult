//! Variables: what a flow remembers.
//!
//! A flow could read the rig and act on the show, and hold nothing in between. A maze
//! area with a PIR at each door has to know whether anybody is inside, and that is a
//! number the console keeps because nothing else can work it out — the same reason
//! `Fixture::sensed_values` is state.
//!
//! **The definition is the show's and the value is the session's.** Name, kind,
//! initial value and bounds are PERSISTED, because they are how the show is built. The
//! value is SYNCED, so a hot spare that takes over knows the maze, and not stored, so
//! opening a show starts every variable at its initial value: `None` *is* "at the
//! initial value", and a SYNCED field is never read back from a showfile.
//!
//! **A value is written unattributed.** By a flow, and by an operator through the
//! `variable.*` station RPCs rather than a field write or a command, because either of
//! those carries the operator's user id — and a phone resetting an area's count would
//! then be a row in the History panel and something Ctrl-Z could take back.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// What a variable holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VariableKind {
    /// On or off. Held as 0 and 1, so a flag reads as a level and a number alike.
    Flag,
    Number,
}

/// A change to a variable, resolved against what it holds when it lands.
///
/// A change rather than a destination where it can be, for the reason `__by` is one:
/// two PIRs reporting in the same millisecond must both count.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VariableOp {
    Set(f64),
    /// Increment and decrement are `Add(1.0)` and `Add(-1.0)`.
    Add(f64),
    /// Zero becomes one and anything else becomes zero.
    Toggle,
    /// Back to the initial value.
    Reset,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "variables")]
pub struct Variable {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub kind: VariableKind,
    #[pult(lifecycle = PERSISTED)]
    pub initial: f64,
    /// Bounds a write is clamped to. A count with a floor of zero cannot be driven
    /// negative by an exit somebody walked through twice.
    #[pult(lifecycle = PERSISTED)]
    pub min: Option<f64>,
    #[pult(lifecycle = PERSISTED)]
    pub max: Option<f64>,
    /// What it holds now. `None` is the initial value — see the module docs.
    #[pult(lifecycle = SYNCED)]
    pub value: Option<f64>,
    /// The pool folder it is in, or `None` for the top level. See [`crate::types::folder`].
    #[serde(default)]
    #[ts(optional = nullable)]
    #[pult(lifecycle = PERSISTED)]
    pub folder: Option<Uuid>,
}

impl Variable {
    /// What it holds now.
    pub fn current(&self) -> f64 {
        self.value.unwrap_or(self.initial)
    }

    /// What the variable should hold after `op`, written as the field stores it:
    /// `None` for a reset, a clamped number otherwise.
    pub fn after(&self, op: VariableOp) -> Option<f64> {
        let next = match op {
            VariableOp::Reset => return None,
            VariableOp::Set(v) => v,
            VariableOp::Add(by) => self.current() + by,
            VariableOp::Toggle => {
                if self.current() != 0.0 {
                    0.0
                } else {
                    1.0
                }
            }
        };
        Some(self.clamp(next))
    }

    fn clamp(&self, value: f64) -> f64 {
        if !value.is_finite() {
            return self.current();
        }
        let value = match self.kind {
            // Anything off zero is on, and a flag added to past one is still on.
            VariableKind::Flag => {
                if value > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            VariableKind::Number => value,
        };
        let value = self.min.map_or(value, |min| value.max(min));
        self.max.map_or(value, |max| value.min(max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count() -> Variable {
        Variable {
            folder: None,
            id: Uuid::nil(),
            name: "Area 1 inside".into(),
            kind: VariableKind::Number,
            initial: 0.0,
            min: Some(0.0),
            max: None,
            value: None,
        }
    }

    #[test]
    fn an_unwritten_variable_holds_its_initial_value() {
        let v = Variable { initial: 3.0, ..count() };
        assert_eq!(v.current(), 3.0);
    }

    #[test]
    fn a_count_cannot_be_driven_below_its_floor() {
        let v = count();
        assert_eq!(v.after(VariableOp::Add(-1.0)), Some(0.0));
    }

    #[test]
    fn a_flag_is_on_or_off_whatever_it_is_given() {
        let flag = Variable { kind: VariableKind::Flag, min: None, ..count() };
        assert_eq!(flag.after(VariableOp::Set(7.0)), Some(1.0));
        assert_eq!(flag.after(VariableOp::Add(2.0)), Some(1.0));
        assert_eq!(flag.after(VariableOp::Toggle), Some(1.0));
        let on = Variable { value: Some(1.0), ..flag };
        assert_eq!(on.after(VariableOp::Toggle), Some(0.0));
        assert_eq!(on.after(VariableOp::Add(-1.0)), Some(0.0));
    }

    #[test]
    fn a_reset_is_no_value_at_all() {
        let v = Variable { value: Some(4.0), ..count() };
        assert_eq!(v.after(VariableOp::Reset), None);
    }

    #[test]
    fn nothing_that_is_not_a_number_is_written() {
        let v = Variable { value: Some(2.0), ..count() };
        assert_eq!(v.after(VariableOp::Set(f64::NAN)), Some(2.0));
    }
}
