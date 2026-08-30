//! Who is at the console.
//!
//! The system has had a `Station` — which machine — since task 10, and never a
//! notion of *who*. That was fine while nothing needed to tell two people apart.
//! Undo needs to: an operator pressing Ctrl-Z means "take back what I did", and on a
//! two-operator tech that is a different set of changes from what the desk did.
//!
//! # Why this is not a station
//!
//! One person often has two clients — the desktop console and a tablet on the same
//! show — and both are them. So identity is *chosen* rather than derived from the
//! machine: a browser says which user it is, and the same user on two clients shares
//! one undo history. Deriving it from the station would make the tablet a stranger.
//!
//! # Why there is no password
//!
//! This is not access control and should not be mistaken for it. Everyone on the
//! network can already change everything, which is the right default for a lighting
//! desk in a room where everyone is trusted and the show is what matters. A user is
//! a name to attribute a change to and a bucket to undo from.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// One person working on the show.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "users")]
pub struct User {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// How this user's changes are marked in the history, as a CSS colour.
    ///
    /// Persisted rather than derived from the id, so somebody who dislikes the one
    /// they were given can change it and it stays changed.
    #[pult(lifecycle = PERSISTED)]
    pub colour: String,
}

/// Colours to hand out, in order, so two users are visibly different by default.
///
/// Chosen to survive the console's dark ground and to stay apart for the common
/// kinds of colour blindness — a history panel that codes by colour alone would be
/// unreadable otherwise, which is why it also always shows the name.
pub const USER_COLOURS: &[&str] =
    &["#4a9eff", "#f59e0b", "#22c55e", "#e879f9", "#f87171", "#2dd4bf"];

/// The colour for the nth user, wrapping once they run out.
pub fn colour_for(index: usize) -> &'static str {
    USER_COLOURS[index % USER_COLOURS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_round_trips() {
        let user =
            User { id: Uuid::nil(), name: "Sam".into(), colour: "#4a9eff".into() };
        let back: User = serde_json::from_value(serde_json::to_value(&user).unwrap()).unwrap();
        assert_eq!(back.name, "Sam");
        assert_eq!(back.colour, "#4a9eff");
    }

    #[test]
    fn colours_are_handed_out_in_order_and_wrap() {
        assert_eq!(colour_for(0), USER_COLOURS[0]);
        assert_eq!(colour_for(1), USER_COLOURS[1]);
        assert_eq!(colour_for(USER_COLOURS.len()), USER_COLOURS[0]);
    }
}
