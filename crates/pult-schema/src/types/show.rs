use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PultSchema;

/// Top-level show metadata.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "show", singleton)]
pub struct Show {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    #[pult(lifecycle = PERSISTED)]
    pub created_at: DateTime<Utc>,
    /// The cue currently being edited, if any.
    ///
    /// Editing is load-tweak-Update rather than live: the cue is read into the
    /// programmer, changed there, and written back on Update. This says which cue is
    /// waiting for that write, and it is SYNCED so a second console shows the same
    /// banner rather than quietly storing over the first one's work.
    #[serde(default)]
    #[pult(lifecycle = SYNCED)]
    pub editing_cue: Option<Uuid>,
    /// How many of an operator's own changes stay reachable, for undo and for the
    /// history panel.
    ///
    /// Changes, not presses. An undo is a change too and shares the window with the
    /// ones it reverses, so a run of them meets itself somewhere around half way:
    /// five hundred is on the order of two hundred and fifty pressings of Ctrl-Z in
    /// a row, and every one of them a long way past where anybody is still sure what
    /// they are undoing.
    ///
    /// Show data rather than a station setting, so that two consoles working one
    /// show agree about how far back Ctrl-Z goes. A station's own preference decides
    /// what a *new* show starts with and then stops mattering — a default that kept
    /// applying would let two stations give different answers about the same show,
    /// which for undo is not a preference but a disagreement.
    #[serde(default = "default_history_depth")]
    #[pult(lifecycle = PERSISTED)]
    pub history_depth: u32,
    /// How long a parameter takes to reach its home value when nothing is left
    /// driving it — a sequence taken off, a selection sent home.
    ///
    /// Show data for the same reason `history_depth` is, and the reason is easier to
    /// see here: two stations driving one rig and fading it home over different
    /// times is not a preference but a disagreement the audience can watch. A
    /// station's own preference decides what a *new* show starts with.
    ///
    /// Zero, so nothing an operator is used to changes until they ask for it: a
    /// programmer clear has always snapped.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub home_fade_ms: u32,
    /// How much haze the rig view draws the beams through, 0 to 1.
    ///
    /// How much haze is in the room, 0 to 1. At 0 the air is clear and no beam is
    /// drawn — the light still lands on the floor — and at 1 every beam shows with the
    /// folds haze puts in it.
    ///
    /// Show data rather than a per-browser view setting, and this one was argued
    /// both ways. How hazy the room is is a fact about the room: a designer who has
    /// lit a piece for a hazy house wants the next person opening the file to see
    /// what they saw, and two operators disagreeing about it are disagreeing about
    /// the drawing rather than about their own screens. A station preference decides
    /// what a *new* show starts with, the way `home_fade_ms` does.
    ///
    /// It reaches no lamp. Nothing here changes what leaves the console.
    #[serde(default = "default_haze_density")]
    #[pult(lifecycle = PERSISTED)]
    pub haze_density: f32,
    /// How much the haze moves, 0 to 1. Separate from density because a still,
    /// heavy haze and a thin, churning one are different rooms, and one number
    /// cannot say which.
    #[serde(default = "default_haze_turbulence")]
    #[pult(lifecycle = PERSISTED)]
    pub haze_turbulence: f32,
}

/// What a show keeps unless somebody says otherwise: five hundred changes, which is
/// far more than anybody steps back through and small enough that a Ctrl-Z stays one
/// indexed query rather than a scan of the evening.
pub const HISTORY_DEPTH_DEFAULT: u32 = 500;
/// Below this, undo stops being useful faster than an operator notices it has.
pub const HISTORY_DEPTH_MIN: u32 = 10;
/// Above this, the window stops being a window. It now bounds what is *kept* as well
/// as what is read — the log is pruned to this number of authored changes — so a
/// larger value is a larger showfile rather than only a longer query.
pub const HISTORY_DEPTH_MAX: u32 = 10_000;

fn default_history_depth() -> u32 {
    HISTORY_DEPTH_DEFAULT
}

/// A depth somebody asked for, brought inside what the console will actually do.
///
/// Applied where the value is *used* rather than where it is written, because a
/// showfile can be edited by hand and a peer can be running a build with different
/// bounds — and a nonsense number should mean the nearest sensible one rather than
/// no undo at all.
pub fn clamp_history_depth(depth: u32) -> u32 {
    depth.clamp(HISTORY_DEPTH_MIN, HISTORY_DEPTH_MAX)
}

/// Above this, going home stops being a release and becomes a cue nobody wrote.
/// Half a minute is already far longer than anybody waits for a rig to let go.
pub const HOME_FADE_MS_MAX: u32 = 30_000;

/// A home time somebody asked for, brought inside what the console will do. Applied
/// where the value is used, for the same reason as [`clamp_history_depth`].
pub fn clamp_home_fade_ms(ms: u32) -> u32 {
    ms.min(HOME_FADE_MS_MAX)
}

/// A full house of haze, which is what a visualiser is for: at 1 every beam shows,
/// at 0 the air is clear and a light lands on the floor without a ray to be seen on
/// the way, and the numbers between are that much haze. A new show starts with the
/// lot because a rig view that draws no beams is the more misleading picture of the
/// two, and a designer lighting a clear room turns it down.
pub const HAZE_DENSITY_DEFAULT: f32 = 1.0;
/// Slow drift. Fast enough that the air is visibly alive, slow enough that nobody
/// reads it as smoke being pumped.
pub const HAZE_TURBULENCE_DEFAULT: f32 = 0.25;

fn default_haze_density() -> f32 {
    HAZE_DENSITY_DEFAULT
}

fn default_haze_turbulence() -> f32 {
    HAZE_TURBULENCE_DEFAULT
}

/// A haze figure somebody asked for, brought inside what the shader will do. Applied
/// where the value is used, for the same reason as [`clamp_history_depth`] — and with
/// one extra reason here: these cross into a shader, where a NaN out of a hand-edited
/// showfile paints the whole view black rather than failing anywhere findable.
pub fn clamp_haze(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_time_is_brought_inside_what_the_console_will_do() {
        assert_eq!(clamp_home_fade_ms(0), 0, "snapping is allowed and is the default");
        assert_eq!(clamp_home_fade_ms(3_000), 3_000);
        assert_eq!(clamp_home_fade_ms(u32::MAX), HOME_FADE_MS_MAX);
    }

    #[test]
    fn haze_is_brought_inside_what_the_shader_will_do() {
        assert_eq!(clamp_haze(0.0), 0.0, "no haze at all is a legitimate room");
        assert_eq!(clamp_haze(0.35), 0.35);
        assert_eq!(clamp_haze(4.0), 1.0);
        assert_eq!(clamp_haze(-1.0), 0.0);
    }

    #[test]
    fn a_nan_haze_reads_as_none_rather_than_painting_the_view_black() {
        // A hand-edited showfile is the way this arrives. `clamp` alone returns the
        // NaN back, which reaches the shader and takes the whole picture with it.
        assert_eq!(clamp_haze(f32::NAN), 0.0);
    }
}
