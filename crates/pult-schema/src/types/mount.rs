//! What a light is clamped to.
//!
//! A fixture's `position` says where it is. That is enough to draw it and not enough
//! to *edit* it: a lantern on a bar can slide along the bar and roll about the chord
//! it is hooked over, and those are the two things somebody actually does to it. Told
//! only a position, a gizmo would have to offer three axes and a free rotation, and
//! every one of the six would take the light off the truss.
//!
//! So a clamped fixture carries a [`Mount`] as well: which chord, how far along it,
//! and how far round. Two degrees, which is what a clamp has.
//!
//! # The position is still stored
//!
//! `Mount` does not replace `Fixture::position` — it stands beside it, and both are
//! written together. The station cannot resolve a mount on an imported truss, because
//! resolving one means knowing where that truss's chords are and the station never
//! loads a mesh; only the browser measures one. So the browser is the writer for every
//! parent, and this module is what keeps its arithmetic and Rust's the same thing.
//! `testdata/mounts.json` is read by `crates/pult-schema/tests/mounts.rs` and by
//! `frontend/src/lib/mount.test.ts`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::fixture::Vec3;
use super::scene::Transform;

/// How far below the chord a clamped fixture's body sits, in metres.
///
/// A hook clamp plus the top of a body: 205 mm. On an F34, whose chords are 145 mm
/// either side of the centre line, that puts a hung lantern 350 mm below the bar —
/// which is the figure every demo used before there were chords to hang off, and the
/// reason it is this number and not a round one.
///
/// One constant, because the first Club demo hung its washes 300 mm below and 600 mm
/// *beside* the bar, and what that drew was a row of lights floating next to a truss.
pub const HUNG_BELOW: f32 = 0.205;

/// A line a clamp can go round, in the piece's own frame.
///
/// Always along **+X**, which is the axis every straight piece in the catalogue runs
/// along; `at` is where the line crosses `x = 0`. A box truss has four of them, a pipe
/// has one, and an imported mesh nobody declared chords for gets one worked out from
/// its bounds by the browser.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Chord {
    pub at: Vec3,
}

impl Chord {
    pub const fn at(y: f32, z: f32) -> Self {
        Chord { at: Vec3 { x: 0.0, y, z } }
    }
}

/// Where a fixture is clamped on the piece it hangs off.
///
/// `along` is metres from the piece's own origin, which for every catalogue piece is
/// its middle — so a light in the middle of a bar reads as zero whatever length of
/// bar it turns out to be. `roll` is degrees about the chord, and it snaps to the
/// quarter turns because that is what a hook clamp does: 0 hangs, 180 stands the
/// fixture on top of the bar, 90 and 270 put it on either face.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Mount {
    /// Which of the parent piece's chords, by index. Out of range wraps rather than
    /// refuses: a piece that lost a chord between two versions of this console should
    /// leave the light on the truss.
    pub chord: u8,
    /// Metres along the chord from the piece's origin.
    pub along: f32,
    /// Degrees about the chord. Zero hangs.
    pub roll: f32,
}

impl Mount {
    /// A light hung under the first chord, this far along it.
    pub fn along(metres: f32) -> Self {
        Mount { chord: 0, along: metres, roll: 0.0 }
    }

    /// Where a fixture clamped here sits, in the parent piece's own frame.
    ///
    /// The rotation is the roll and nothing else, so a fixture nobody has aimed hangs
    /// looking at the floor — a fixture's own axis is −Y, so zero rotation *is*
    /// hanging, and this keeps that true. A caller that aims the light writes its own
    /// rotation over the top; the position is the part only the mount knows.
    pub fn transform(&self, chords: &[Chord]) -> Transform {
        Transform { position: self.point(chords), rotation: self.turn(), ..Transform::default() }
    }

    /// Just the point, for a caller that is going to aim the fixture itself.
    pub fn point(&self, chords: &[Chord]) -> Vec3 {
        let chord = self.chord_of(chords);
        // Straight down from the chord, turned about the chord — which runs along X,
        // so the roll is a rotation of the offset in the YZ plane.
        let (sin, cos) = self.roll.to_radians().sin_cos();
        Vec3 {
            x: chord.at.x + self.along,
            y: chord.at.y - HUNG_BELOW * cos,
            z: chord.at.z - HUNG_BELOW * sin,
        }
    }

    /// The rotation a roll alone gives: about the chord, which is X.
    pub fn turn(&self) -> Vec3 {
        Vec3 { x: self.roll, y: 0.0, z: 0.0 }
    }

    /// The chord this names, or the first one. A piece with no chords at all answers
    /// a line through its own origin, which is what a bare pipe is.
    pub fn chord_of(&self, chords: &[Chord]) -> Chord {
        if chords.is_empty() {
            return Chord::at(0.0, 0.0);
        }
        chords[usize::from(self.chord) % chords.len()]
    }

    /// The nearest place on this piece to a point in its own frame.
    ///
    /// What snapping a dragged light onto a bar comes to: the chord whose line passes
    /// closest, how far along it the light is, and the quarter turn that puts the
    /// light on the side it was dragged to. Answers the mount **and** how far the
    /// light had to move to take it, so a caller can decide whether it is close
    /// enough to be a clamp at all.
    pub fn nearest(point: Vec3, chords: &[Chord]) -> (Mount, f32) {
        let mut best = (Mount::default(), f32::INFINITY);
        for (index, chord) in chords.iter().enumerate() {
            let dy = point.y - chord.at.y;
            let dz = point.z - chord.at.z;
            // The quarter turn whose hanging direction points at the light.
            let roll = quarter_turn(dy, dz);
            let mount = Mount { chord: index as u8, along: point.x - chord.at.x, roll };
            let landed = mount.point(chords);
            let distance = (landed.x - point.x).hypot(landed.y - point.y).hypot(landed.z - point.z);
            if distance < best.1 {
                best = (mount, distance);
            }
        }
        if chords.is_empty() {
            return (Mount::default(), f32::INFINITY);
        }
        best
    }
}

/// Which of the four quarter turns points nearest at `(dy, dz)` from the chord.
///
/// Zero hangs, so the offset a roll of zero gives is `(0, −HUNG_BELOW, 0)` — straight
/// down — and the turns go round towards +Z, which is the direction `point` rolls in.
fn quarter_turn(dy: f32, dz: f32) -> f32 {
    // The angle of (−dy, −dz) measured the way `point` measures it: y = −cos, z = −sin.
    let angle = (-dz).atan2(-dy).to_degrees();
    let snapped = (angle / 90.0).round() * 90.0;
    // Into 0..360, so a stored roll never reads as −90 where 270 was meant.
    (snapped % 360.0 + 360.0) % 360.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const F34_CHORDS: &[Chord] = &[
        Chord::at(-0.145, -0.145),
        Chord::at(-0.145, 0.145),
        Chord::at(0.145, -0.145),
        Chord::at(0.145, 0.145),
    ];

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// The figure every demo rested on before there were chords: a lantern hung under
    /// an F34 sits 350 mm below its centre line.
    #[test]
    fn a_lantern_under_an_f34_hangs_where_it_always_did() {
        let at = Mount::along(1.5).point(F34_CHORDS);
        assert!(near(at.x, 1.5), "{at:?}");
        assert!(near(at.y, -0.35), "{at:?}");
    }

    /// Half a turn stands it on top of the bar, pointing up.
    #[test]
    fn a_half_turn_stands_it_on_the_bar() {
        let mount = Mount { chord: 0, along: 0.0, roll: 180.0 };
        let placed = mount.transform(F34_CHORDS);
        assert!(near(placed.position.y, -0.145 + HUNG_BELOW), "{placed:?}");
        let up = placed.facing_direction();
        assert!(up.y > 0.9, "half a turn should look up, not {up:?}");
    }

    /// A pipe has one chord through its own middle, so a mount on one is the simple
    /// case: straight down from the centre line.
    #[test]
    fn a_pipe_hangs_from_its_own_centre() {
        let pipe = [Chord::at(0.0, 0.0)];
        let at = Mount::along(-0.75).point(&pipe);
        assert!(near(at.x, -0.75) && near(at.y, -HUNG_BELOW) && near(at.z, 0.0), "{at:?}");
    }

    /// And the round trip: a mount resolved to a point and snapped back is the mount
    /// it was. This is what makes dragging a light along a bar keep it on the bar.
    #[test]
    fn a_point_snaps_back_to_the_mount_that_made_it() {
        for chord in 0..4u8 {
            for roll in [0.0, 90.0, 180.0, 270.0] {
                let mount = Mount { chord, along: 0.6, roll };
                let (back, distance) = Mount::nearest(mount.point(F34_CHORDS), F34_CHORDS);
                assert!(distance < 1e-3, "{mount:?} landed {distance} away");
                assert_eq!(back.chord, mount.chord, "{mount:?} came back on another chord");
                assert!(near(back.roll, mount.roll), "{mount:?} came back rolled {}", back.roll);
                assert!(near(back.along, mount.along), "{mount:?} came back at {}", back.along);
            }
        }
    }

    /// A light dragged to somewhere off the bar still names the nearest clamp, and
    /// says how far off it is — which is what decides whether it snaps at all.
    #[test]
    fn something_far_away_says_how_far() {
        let (_, distance) = Mount::nearest(Vec3 { x: 0.0, y: -4.0, z: 0.0 }, F34_CHORDS);
        assert!(distance > 3.0, "a light four metres under the bar is not clamped to it");
    }
}
