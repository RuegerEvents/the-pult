//! MVR, read into the show and written back out of it.
//!
//! `pult-mvr` knows the format and nothing about this console; this is the
//! translation. It goes through [`super::apply`] like the GDTF path does, so an
//! import is one gesture, leaves nothing behind if it is refused, and takes itself
//! back if a write fails halfway.

pub mod plan;

pub use plan::{plan_import, Existing};

use pult_mvr::transform::Placement;
use pult_schema::types::fixture::Vec3;
use pult_schema::types::scene::Transform;

/// A placement in the console's space, as the schema holds one.
///
/// Two representations of the same three vectors, and the conversion is here rather
/// than in either crate: `pult-mvr` may not know what a `Transform` is, and
/// `pult-schema` may not know what an MVR file is.
pub fn placement_as_transform(placement: &Placement) -> Transform {
    Transform {
        position: vec3(placement.position),
        rotation: vec3(placement.rotation),
        scale: vec3(placement.scale),
    }
}

/// And back, for export.
pub fn transform_as_placement(transform: &Transform) -> Placement {
    Placement {
        position: array(transform.position),
        rotation: array(transform.rotation),
        scale: array(transform.scale),
    }
}

fn vec3([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3 { x, y, z }
}

fn array(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}
