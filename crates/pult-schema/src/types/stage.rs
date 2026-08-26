//! The stage: where the rig is, drawn on something you can recognise.
//!
//! A show starts from a drawing somebody was handed — a ground plan, a section, a
//! sketch. [`StagePlan`] is that drawing plus the two numbers that turn it into a
//! map: where its top-left corner sits in the world, and how many metres one of its
//! pixels covers. Everything after that is `Fixture::position`, which has existed
//! since the patch went in and until now could only be set to the origin.
//!
//! # Which way is which
//!
//! This is the first place in the system that commits to axes, so it says so here.
//! **Y is up. X increases to the right as seen from front of house. Z increases
//! downstage, towards the audience.**
//!
//! Z is chosen rather than inherited, and two things agree on it: a ground plan is
//! drawn with the audience at the bottom of the page, so the image's own downward
//! axis is downstage; and the 3D view's camera looks up the negative Z axis, which
//! puts front of house at positive Z looking at the stage. A plan therefore lies on
//! the floor with no flip anywhere.
//!
//! The image itself is not here. It lives in the asset store, content-addressed, and
//! `asset` is that hash — a stage plan replicates as a row of numbers while the
//! megabytes travel over HTTP and only to the stations that ask for them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{types::fixture::Vec3, PultSchema};

/// A drawing of the room, laid under the rig.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "stage_plans")]
pub struct StagePlan {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// The image's sha256 in the asset store.
    #[pult(lifecycle = PERSISTED)]
    pub asset: String,
    /// The image's own size, so the view can lay it out before the bytes arrive.
    #[pult(lifecycle = PERSISTED)]
    pub width_px: u32,
    #[pult(lifecycle = PERSISTED)]
    pub height_px: u32,
    /// The world point, in metres, that the image's top-left corner sits on.
    #[pult(lifecycle = PERSISTED)]
    pub origin: Vec3,
    /// How much of the room one image pixel covers. Set by measuring something on
    /// the plan whose real length is known.
    #[pult(lifecycle = PERSISTED)]
    pub metres_per_pixel: f32,
    #[pult(lifecycle = PERSISTED)]
    pub rotation_deg: f32,
    #[pult(lifecycle = PERSISTED)]
    pub opacity: f32,
    #[pult(lifecycle = PERSISTED)]
    pub visible: bool,
}

impl StagePlan {
    /// How wide and deep the plan is on the floor, in metres.
    pub fn extent_m(&self) -> (f32, f32) {
        (
            self.width_px as f32 * self.metres_per_pixel,
            self.height_px as f32 * self.metres_per_pixel,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_covers_as_much_room_as_its_scale_says() {
        let plan = StagePlan {
            id: Uuid::nil(),
            name: "Ground plan".into(),
            asset: "abc".into(),
            width_px: 2000,
            height_px: 1000,
            origin: Vec3 { x: -10.0, y: 0.0, z: -5.0 },
            // A 20 m wide stage drawn 2000 px across.
            metres_per_pixel: 0.01,
            rotation_deg: 0.0,
            opacity: 0.6,
            visible: true,
        };

        assert_eq!(plan.extent_m(), (20.0, 10.0));
    }
}
