//! The MVR object model: `GeneralSceneDescription` and everything under it.
//!
//! MVR 1.5 and 1.6 (DIN SPEC 15801). Written against three real files rather than
//! against the spec alone, because the spec does not say which of its optional
//! elements anybody actually writes, and the answer turns out to be "a different
//! subset per exporter".
//!
//! **One struct for every kind of object in a layer.** The spec defines `Fixture`,
//! `Truss`, `Support`, `SceneObject`, `VideoScreen`, `Projector` and `FocusPoint` by
//! inheritance from one base, and in a real file they differ only in which of the
//! same elements they carry — a truss and a scene object are byte-identical in shape.
//! So the tag lives in [`ChildNode`] and the fields live in one [`Object`], with the
//! fixture-only ones optional. A reader that made eight structs would repeat the
//! eleven shared fields eight times and still have to accept a `Truss` that carried
//! an `<Addresses>`, because files do things the spec does not allow.
//!
//! A `ChildList` interleaves kinds, so children are one `$value` list over an enum of
//! tag names, and quick-xml needs `overlapped-lists` to fill it. The same idiom as
//! GDTF's geometry tree, for the same reason.

use serde::{Deserialize, Serialize};

use crate::values::{de_number_opt, ColorCie, MvrMatrix};

/// The root of `GeneralSceneDescription.xml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneralSceneDescription {
    #[serde(rename = "@verMajor")]
    pub ver_major: u8,
    #[serde(rename = "@verMinor")]
    pub ver_minor: u8,
    /// What wrote the file. Vectorworks says so; grandMA does not.
    #[serde(rename = "@provider", default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(
        rename = "@providerVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub provider_version: Option<String>,
    #[serde(rename = "Scene")]
    pub scene: Scene,
}

impl Default for GeneralSceneDescription {
    fn default() -> Self {
        GeneralSceneDescription {
            ver_major: 1,
            ver_minor: 6,
            provider: None,
            provider_version: None,
            scene: Scene::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scene {
    #[serde(rename = "AUXData", default, skip_serializing_if = "Option::is_none")]
    pub aux_data: Option<AuxData>,
    #[serde(rename = "Layers", default)]
    pub layers: Layers,
}

/// What the layers point at rather than contain: symbol definitions, classes,
/// positions, mapping definitions.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AuxData {
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<AuxItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuxItem {
    Symdef(Symdef),
    Class(Class),
    Position(NamedThing),
    MappingDefinition(NamedThing),
}

/// A reusable piece of geometry: one truss type drawn once and instanced everywhere.
///
/// The reason geometry is not stored per object. A real file has ninety-five scene
/// objects sharing ninety-five symdefs, and the meshes are the bulk of the archive.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Symdef {
    #[serde(rename = "@uuid")]
    pub uuid: String,
    #[serde(rename = "@name", default)]
    pub name: String,
    #[serde(rename = "ChildList", default, skip_serializing_if = "Option::is_none")]
    pub children: Option<ChildList>,
}

/// A cross-layer tag: "house rig", "touring", "practicals".
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Class {
    #[serde(rename = "@uuid")]
    pub uuid: String,
    #[serde(rename = "@name", default)]
    pub name: String,
}

/// The two things a `Position` and a `MappingDefinition` have in common, which is all
/// this console reads out of either.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NamedThing {
    #[serde(rename = "@uuid")]
    pub uuid: String,
    #[serde(rename = "@name", default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Layers {
    #[serde(rename = "Layer", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Layer>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Layer {
    #[serde(rename = "@uuid")]
    pub uuid: String,
    #[serde(rename = "@name", default)]
    pub name: String,
    #[serde(rename = "Matrix", default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MvrMatrix>,
    #[serde(rename = "ChildList", default, skip_serializing_if = "Option::is_none")]
    pub children: Option<ChildList>,
}

/// The objects inside a layer, a group, or a symbol definition.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChildList {
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ChildNode>,
}

/// One object, tagged by what it is.
///
/// `Geometry3D` and `Symbol` are in here as well as in [`Geometries`], because a
/// `Symdef`'s `ChildList` holds them directly — which is not what the shape of the
/// spec suggests and is what every file does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChildNode {
    Fixture(Object),
    Truss(Object),
    Support(Object),
    SceneObject(Object),
    VideoScreen(Object),
    Projector(Object),
    FocusPoint(Object),
    GroupObject(Object),
    Geometry3D(Geometry3D),
    Symbol(Symbol),
}

impl ChildNode {
    /// The object inside, whatever kind of object it is.
    pub fn object(&self) -> Option<&Object> {
        match self {
            ChildNode::Fixture(o)
            | ChildNode::Truss(o)
            | ChildNode::Support(o)
            | ChildNode::SceneObject(o)
            | ChildNode::VideoScreen(o)
            | ChildNode::Projector(o)
            | ChildNode::FocusPoint(o)
            | ChildNode::GroupObject(o) => Some(o),
            ChildNode::Geometry3D(_) | ChildNode::Symbol(_) => None,
        }
    }

    /// The tag this node was written under, for a warning that has to name it.
    pub fn tag(&self) -> &'static str {
        match self {
            ChildNode::Fixture(_) => "Fixture",
            ChildNode::Truss(_) => "Truss",
            ChildNode::Support(_) => "Support",
            ChildNode::SceneObject(_) => "SceneObject",
            ChildNode::VideoScreen(_) => "VideoScreen",
            ChildNode::Projector(_) => "Projector",
            ChildNode::FocusPoint(_) => "FocusPoint",
            ChildNode::GroupObject(_) => "GroupObject",
            ChildNode::Geometry3D(_) => "Geometry3D",
            ChildNode::Symbol(_) => "Symbol",
        }
    }
}

/// Everything any object in a layer can carry.
///
/// The fixture-only half is `Option`, so a truss simply has none of it and writes
/// none of it back. Field order is the order a file writes them, because quick-xml
/// serialises in declaration order and a diff against the original should be about
/// values rather than about ordering.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Object {
    #[serde(rename = "@uuid")]
    pub uuid: String,
    #[serde(rename = "@name", default)]
    pub name: String,

    /// Where it is, **relative to its parent**, in millimetres with Z up.
    #[serde(rename = "Matrix", default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MvrMatrix>,
    #[serde(rename = "Geometries", default, skip_serializing_if = "Option::is_none")]
    pub geometries: Option<Geometries>,
    /// The uuid of the `Class` this belongs to.
    #[serde(rename = "Classing", default, skip_serializing_if = "Option::is_none")]
    pub classing: Option<String>,
    /// The GDTF file this fixture is, named as it is named in the archive — with or
    /// without its `.gdtf` extension, depending on who wrote the file.
    #[serde(rename = "GDTFSpec", default, skip_serializing_if = "Option::is_none")]
    pub gdtf_spec: Option<String>,
    #[serde(rename = "GDTFMode", default, skip_serializing_if = "Option::is_none")]
    pub gdtf_mode: Option<String>,

    #[serde(rename = "Addresses", default, skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Addresses>,
    /// The number on the fixture's own label: what an operator calls it.
    #[serde(
        rename = "FixtureID",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub fixture_id: Option<u32>,
    #[serde(
        rename = "UnitNumber",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub unit_number: Option<u32>,
    #[serde(
        rename = "FixtureTypeId",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub fixture_type_id: Option<u32>,
    #[serde(
        rename = "CustomId",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub custom_id: Option<u32>,
    /// The colour the plot draws it in — CIE xyY, not what it is outputting.
    #[serde(rename = "Color", default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorCie>,
    #[serde(rename = "CastShadow", default, skip_serializing_if = "Option::is_none")]
    pub cast_shadow: Option<bool>,
    /// The uuid of the `FocusPoint` this fixture is aimed at.
    #[serde(rename = "Focus", default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// The uuid of the `Position` it hangs at.
    #[serde(rename = "Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(rename = "Function", default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(rename = "DMXInvertPan", default, skip_serializing_if = "Option::is_none")]
    pub dmx_invert_pan: Option<bool>,
    #[serde(rename = "DMXInvertTilt", default, skip_serializing_if = "Option::is_none")]
    pub dmx_invert_tilt: Option<bool>,
    #[serde(rename = "Mappings", default, skip_serializing_if = "Option::is_none")]
    pub mappings: Option<Mappings>,
    #[serde(rename = "Protocols", default, skip_serializing_if = "Option::is_none")]
    pub protocols: Option<Protocols>,
    #[serde(
        rename = "CustomCommands",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_commands: Option<CustomCommands>,

    #[serde(rename = "ChildList", default, skip_serializing_if = "Option::is_none")]
    pub children: Option<ChildList>,
}

/// Where an object's meshes are: a file per part, or one reference to a symbol.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Geometries {
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<GeometryNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GeometryNode {
    Geometry3D(Geometry3D),
    Symbol(Symbol),
}

/// A mesh, named by its file inside the archive.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Geometry3D {
    #[serde(rename = "@fileName")]
    pub file_name: String,
    #[serde(rename = "Matrix", default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MvrMatrix>,
}

/// An instance of a [`Symdef`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Symbol {
    #[serde(rename = "@uuid", default)]
    pub uuid: String,
    #[serde(rename = "@symdef")]
    pub symdef: String,
    #[serde(rename = "Matrix", default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MvrMatrix>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Addresses {
    #[serde(rename = "Address", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Address>,
}

/// One break's start address, written **absolute**: universe folded in.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Address {
    /// Which DMX break, numbered from zero.
    #[serde(rename = "@break", default, deserialize_with = "de_number_opt")]
    pub break_id: Option<u16>,
    /// Universe times 512 plus the channel, and the two numbering conventions that
    /// could mean are why [`crate::address`] exists rather than a `/` and a `%` at
    /// each call site.
    #[serde(rename = "$text", default, deserialize_with = "de_number_opt")]
    pub absolute: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Mappings {
    #[serde(rename = "Mapping", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Mapping>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Mapping {
    #[serde(rename = "@linkedDef", default)]
    pub linked_def: String,
}

/// Left opaque on purpose. A fixture's network protocols are somebody else's
/// addressing, and this console patches from `Addresses`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Protocols {}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CustomCommands {
    #[serde(rename = "CustomCommand", default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
}
