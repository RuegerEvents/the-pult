//! `<Geometries>`: the tree of parts the fixture is made of.
//!
//! The console reads three things out of it. `Axis` nodes are what pan and tilt
//! actually turn, so a 3D body articulates instead of swinging as a block. `Beam`
//! gives the origin of the light and its real beam angle, which is the number the rig
//! view's cone has been guessing. `GeometryReference` is how a multi-cell bar says
//! "the same head, eight times, each at its own DMX offset" without repeating itself.
//!
//! The tree interleaves child kinds — an `Axis` may sit between two `Geometry`
//! elements — so children are one `$value` list over an enum of tag names rather than
//! a field per kind, and quick-xml needs `overlapped-lists` to fill it.

use serde::{Deserialize, Serialize};

use crate::values::{ColorCie, DmxValue, Matrix, Node, de_from_str_opt, de_number_opt, de_value_opt, ser_display_opt};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Geometries {
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

/// One node of the geometry tree.
///
/// Every kind the spec names, because an unrecognised element inside `<Geometries>`
/// stops the parse dead — there is no lenient position to fall back to when the
/// children are a `$value` list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GeometryNode {
    Geometry(Geometry),
    Axis(Geometry),
    FilterBeam(Geometry),
    FilterColor(Geometry),
    FilterGobo(Geometry),
    FilterShaper(Geometry),
    Beam(Beam),
    MediaServerLayer(Geometry),
    MediaServerCamera(Geometry),
    MediaServerMaster(Geometry),
    Display(Display),
    GeometryReference(GeometryReference),
    Laser(Laser),
    WiringObject(WiringObject),
    Inventory(Inventory),
    Structure(Structure),
    Support(Support),
    Magnet(Geometry),
}

impl GeometryNode {
    /// The three attributes every kind shares, whichever kind this is.
    pub fn common(&self) -> GeometryCommon<'_> {
        match self {
            GeometryNode::Geometry(g)
            | GeometryNode::Axis(g)
            | GeometryNode::FilterBeam(g)
            | GeometryNode::FilterColor(g)
            | GeometryNode::FilterGobo(g)
            | GeometryNode::FilterShaper(g)
            | GeometryNode::MediaServerLayer(g)
            | GeometryNode::MediaServerCamera(g)
            | GeometryNode::MediaServerMaster(g)
            | GeometryNode::Magnet(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
            GeometryNode::Beam(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
            GeometryNode::Display(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
            GeometryNode::GeometryReference(g) => {
                GeometryCommon::of(&g.name, &g.model, &g.position)
            }
            GeometryNode::Laser(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
            GeometryNode::WiringObject(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
            GeometryNode::Inventory(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
            GeometryNode::Structure(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
            GeometryNode::Support(g) => GeometryCommon::of(&g.name, &g.model, &g.position),
        }
    }

    pub fn name(&self) -> &str {
        self.common().name
    }

    /// This node's own children. A `GeometryReference` has none of its own — what it
    /// stands for is the subtree it points at, expanded in [`crate::resolve`].
    pub fn children(&self) -> &[GeometryNode] {
        match self {
            GeometryNode::Geometry(g)
            | GeometryNode::Axis(g)
            | GeometryNode::FilterBeam(g)
            | GeometryNode::FilterColor(g)
            | GeometryNode::FilterGobo(g)
            | GeometryNode::FilterShaper(g)
            | GeometryNode::MediaServerLayer(g)
            | GeometryNode::MediaServerCamera(g)
            | GeometryNode::MediaServerMaster(g)
            | GeometryNode::Magnet(g) => &g.children,
            GeometryNode::Beam(g) => &g.children,
            GeometryNode::Display(g) => &g.children,
            GeometryNode::Laser(g) => &g.children,
            GeometryNode::WiringObject(g) => &g.children,
            GeometryNode::Inventory(g) => &g.children,
            GeometryNode::Structure(g) => &g.children,
            GeometryNode::Support(g) => &g.children,
            GeometryNode::GeometryReference(_) => &[],
        }
    }

    /// Whether this node turns: what pan and tilt drive.
    pub fn is_axis(&self) -> bool {
        matches!(self, GeometryNode::Axis(_))
    }
}

/// A borrowed view of the attributes every geometry kind carries.
pub struct GeometryCommon<'a> {
    pub name: &'a str,
    pub model: Option<&'a str>,
    pub position: Option<&'a Matrix>,
}

impl<'a> GeometryCommon<'a> {
    fn of(name: &'a str, model: &'a Option<String>, position: &'a Option<Matrix>) -> Self {
        GeometryCommon {
            name,
            model: model.as_deref(),
            position: position.as_ref(),
        }
    }
}

/// The plain kinds: a name, a model, a place, and children.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Geometry {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

/// Where the light comes out, and how wide.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Beam {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(rename = "@LampType", default, skip_serializing_if = "Option::is_none")]
    pub lamp_type: Option<String>,
    #[serde(
        rename = "@PowerConsumption",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub power_consumption: Option<f32>,
    #[serde(
        rename = "@LuminousFlux",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub luminous_flux: Option<f32>,
    #[serde(
        rename = "@ColorTemperature",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub color_temperature: Option<f32>,
    /// Degrees. What the rig view's cone should be, instead of a constant.
    #[serde(
        rename = "@BeamAngle",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub beam_angle: Option<f32>,
    #[serde(
        rename = "@FieldAngle",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub field_angle: Option<f32>,
    #[serde(
        rename = "@ThrowRatio",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub throw_ratio: Option<f32>,
    #[serde(
        rename = "@RectangleRatio",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub rectangle_ratio: Option<f32>,
    /// Metres, at the aperture.
    #[serde(
        rename = "@BeamRadius",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub beam_radius: Option<f32>,
    #[serde(rename = "@BeamType", default, skip_serializing_if = "Option::is_none")]
    pub beam_type: Option<BeamType>,
    #[serde(
        rename = "@ColorRenderingIndex",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub color_rendering_index: Option<u8>,
    #[serde(
        rename = "@EmitterSpectrum",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub emitter_spectrum: Option<Node>,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BeamType {
    #[default]
    Wash,
    Spot,
    None,
    Rectangle,
    PC,
    Fresnel,
    Glow,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Display {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(rename = "@Texture", default, skip_serializing_if = "String::is_empty")]
    pub texture: String,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

/// "The same subtree again, at another DMX offset."
///
/// A `<Break>` per DMX break says where that copy's channels start; the last `Break`
/// in the list is the special one that names the *overwrite* break, per the spec.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GeometryReference {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    /// The geometry this stands in for, by name.
    #[serde(
        rename = "@Geometry",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub geometry: String,
    #[serde(rename = "Break", default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<Break>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Break {
    #[serde(
        rename = "@DMXOffset",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub dmx_offset: Option<DmxValue>,
    #[serde(rename = "@DMXBreak", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub dmx_break: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Laser {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(
        rename = "@ColorType",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub color_type: String,
    #[serde(rename = "@Color", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_value_opt",
        serialize_with = "ser_display_opt"
    )]
    pub color: Option<ColorCie>,
    #[serde(
        rename = "@OutputStrength",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub output_strength: Option<f32>,
    #[serde(rename = "@Emitter", default, skip_serializing_if = "Option::is_none")]
    #[serde(
        deserialize_with = "de_from_str_opt",
        serialize_with = "ser_display_opt"
    )]
    pub emitter: Option<Node>,
    #[serde(
        rename = "@BeamDiameter",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub beam_diameter: Option<f32>,
    #[serde(
        rename = "@BeamDivergenceMin",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub beam_divergence_min: Option<f32>,
    #[serde(
        rename = "@BeamDivergenceMax",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub beam_divergence_max: Option<f32>,
    #[serde(
        rename = "@ScanAnglePan",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub scan_angle_pan: Option<f32>,
    #[serde(
        rename = "@ScanAngleTilt",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub scan_angle_tilt: Option<f32>,
    #[serde(
        rename = "@ScanSpeed",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub scan_speed: Option<f32>,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WiringObject {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(
        rename = "@ConnectorType",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub connector_type: String,
    #[serde(
        rename = "@ComponentType",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub component_type: String,
    #[serde(
        rename = "@SignalType",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub signal_type: String,
    #[serde(rename = "@PinCount", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub pin_count: Option<u32>,
    #[serde(
        rename = "@ElectricalPayLoad",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub electrical_pay_load: Option<f32>,
    #[serde(
        rename = "@VoltageRangeMax",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub voltage_range_max: Option<f32>,
    #[serde(
        rename = "@VoltageRangeMin",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub voltage_range_min: Option<f32>,
    #[serde(
        rename = "@FrequencyRangeMax",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub frequency_range_max: Option<f32>,
    #[serde(
        rename = "@FrequencyRangeMin",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub frequency_range_min: Option<f32>,
    #[serde(rename = "@CosPhi", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub cos_phi: Option<f32>,
    #[serde(
        rename = "@FuseCurrent",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub fuse_current: Option<f32>,
    #[serde(
        rename = "@FuseRating",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub fuse_rating: String,
    #[serde(
        rename = "@Orientation",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub orientation: String,
    #[serde(
        rename = "@WireGroup",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub wire_group: String,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Inventory {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(rename = "@Count", default, skip_serializing_if = "Option::is_none", deserialize_with = "de_number_opt")]
    pub count: Option<u32>,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Structure {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(
        rename = "@LinkedGeometry",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub linked_geometry: String,
    #[serde(
        rename = "@StructureType",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub structure_type: String,
    #[serde(
        rename = "@CrossSectionType",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub cross_section_type: String,
    #[serde(
        rename = "@CrossSectionHeight",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub cross_section_height: Option<f32>,
    #[serde(
        rename = "@CrossSectionWallThickness",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub cross_section_wall_thickness: Option<f32>,
    #[serde(
        rename = "@TrussCrossSection",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub truss_cross_section: String,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Support {
    #[serde(rename = "@Name", default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "@Model", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "@Position", default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Matrix>,
    #[serde(
        rename = "@SupportType",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub support_type: String,
    #[serde(
        rename = "@RopeCrossSection",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub rope_cross_section: String,
    #[serde(
        rename = "@RopeOffset",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub rope_offset: Option<crate::values::Matrix>,
    #[serde(
        rename = "@CapacityX",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub capacity_x: Option<f32>,
    #[serde(
        rename = "@CapacityY",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub capacity_y: Option<f32>,
    #[serde(
        rename = "@CapacityZ",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_number_opt"
    )]
    pub capacity_z: Option<f32>,
    #[serde(rename = "$value", default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GeometryNode>,
}
