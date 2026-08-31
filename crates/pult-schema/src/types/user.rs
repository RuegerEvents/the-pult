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

impl User {
    /// The user a show has before anybody has said who they are.
    ///
    /// A fixed constant rather than something derived, and both halves of that matter.
    ///
    /// *Fixed*, because the browser has to be able to work as this user before the
    /// `users` collection has arrived. Anything it had to fetch or compute would leave
    /// a window in which a change is attributed to nobody — and an unattributed write
    /// can never be taken back, not even once the operator says who they are. That
    /// window is the whole bug this exists to close, so the id is a constant the
    /// frontend can hold. `frontend/src/lib/users.ts` holds the same one, the way it
    /// already holds [`USER_COLOURS`].
    ///
    /// *The same in every show*, because the alternative — a v5 over the show's id, as
    /// a `PluginDatum` derives its own — needs the `Show` row to exist at the moment of
    /// seeding, and the load path promises no such thing for an empty showfile. Ids are
    /// only ever compared within one show, so one value in every showfile collides with
    /// nothing.
    ///
    /// Not derived from the station: this module opens by arguing that identity is
    /// *chosen* rather than taken from the machine, and one default per station would
    /// make a person's tablet a stranger to their desk.
    pub const DEFAULT_ID: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001);

    /// What the default user is called until somebody renames it.
    ///
    /// A name nobody chose beats no attribution. It is an ordinary row, so anybody who
    /// dislikes it can change it and it stays changed.
    pub const DEFAULT_NAME: &'static str = "Operator";

    /// The show's default user, as it is first written.
    pub fn default_user() -> Self {
        Self {
            id: Self::DEFAULT_ID,
            name: Self::DEFAULT_NAME.to_owned(),
            colour: colour_for(0).to_owned(),
        }
    }
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
    fn the_default_user_is_an_ordinary_user() {
        let user = User::default_user();
        assert_eq!(user.id, User::DEFAULT_ID);
        assert_eq!(user.name, "Operator");
        assert_eq!(user.colour, USER_COLOURS[0]);
        // It round-trips like any other, because it is any other.
        let back: User = serde_json::from_value(serde_json::to_value(&user).unwrap()).unwrap();
        assert_eq!(back.id, User::DEFAULT_ID);
    }

    /// The id is a constant the frontend holds too, so it has to be a fixed, valid
    /// uuid rather than something that happens to parse today.
    #[test]
    fn the_default_id_is_the_value_written_down() {
        assert_eq!(User::DEFAULT_ID.to_string(), "00000000-0000-4000-8000-000000000001");
        assert_ne!(User::DEFAULT_ID, Uuid::nil());
    }

    /// The frontend holds this id too, because the browser needs to be working as
    /// somebody before its first write and a round trip would leave a window where it
    /// is not. Duplication with a guard rather than duplication with a comment: this
    /// is the sort of thing that is correct for a year and then quietly is not.
    #[test]
    fn the_frontend_agrees_about_the_default_id() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend/src/lib/users.ts");
        let source = std::fs::read_to_string(path)
            .expect("frontend/src/lib/users.ts, which holds the same constant");
        let expected = format!("export const DEFAULT_USER_ID = '{}';", User::DEFAULT_ID);
        assert!(
            source.contains(&expected),
            "users.ts should contain `{expected}` — the backend seeds {} and a browser \
             that disagrees would work as a user the show does not have",
            User::DEFAULT_ID
        );
    }

    #[test]
    fn colours_are_handed_out_in_order_and_wrap() {
        assert_eq!(colour_for(0), USER_COLOURS[0]);
        assert_eq!(colour_for(1), USER_COLOURS[1]);
        assert_eq!(colour_for(USER_COLOURS.len()), USER_COLOURS[0]);
    }
}
