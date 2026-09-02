//! Building a valid GDTF from very little.
//!
//! The console's own fixture types — the ones derived from an OpenHaunt node's port
//! description, or typed into the editor — have no geometry, no wheels and no
//! measured emitters. Exporting one still has to produce a file another console will
//! open, which means a geometry tree with a model on it, an attribute definition per
//! channel, and one mode.
//!
//! Deliberately the *minimum* the spec allows and no more. A generated file that
//! invented a beam angle or a weight would be a lie another console would then act
//! on, and the honest answer to "how heavy is it" is that this file does not say.

use crate::model::*;
use crate::values::{DmxValue, Matrix};
use crate::GdtfFile;

/// What a caller has to say to get a file out.
#[derive(Debug, Clone, Default)]
pub struct MinimalSpec {
    pub name: String,
    pub short_name: String,
    pub manufacturer: String,
    pub description: String,
    /// The console's own id for the type, written as the `FixtureTypeID` so that
    /// exporting and re-importing lands on the same row rather than a second one.
    pub fixture_type_id: String,
    pub mode_name: String,
    pub channels: Vec<MinimalChannel>,
    /// Kilograms, where the console knows.
    pub weight_kg: Option<f32>,
    /// Watts, where the console knows.
    pub power_w: Option<f32>,
    /// Degrees, where the console knows.
    pub beam_angle: Option<f32>,
}

/// One channel of the generated mode.
#[derive(Debug, Clone, Default)]
pub struct MinimalChannel {
    /// A GDTF attribute name: `Dimmer`, `Pan`, `ColorAdd_R`.
    pub attribute: String,
    /// 1-based offsets from the fixture's start address, coarse first.
    pub offsets: Vec<u16>,
    pub default: u32,
    pub physical_from: Option<f32>,
    pub physical_to: Option<f32>,
    pub physical_unit: Option<PhysicalUnit>,
    /// Which feature group the attribute belongs to: `Dimmer`, `Position`, `Color`,
    /// `Beam`, `Control`.
    pub feature: String,
}

/// The model name the generated geometry hangs off, and the model this crate writes
/// when there is no mesh: a box the size of nothing in particular.
const BODY: &str = "Body";

/// Build a complete, valid, one-mode GDTF.
pub fn build(spec: &MinimalSpec) -> GdtfFile {
    let mut features: Vec<FeatureGroup> = Vec::new();
    let mut attributes: Vec<Attribute> = Vec::new();

    for channel in &spec.channels {
        let feature = if channel.feature.is_empty() {
            "Control"
        } else {
            &channel.feature
        };
        if !features.iter().any(|group| group.name == feature) {
            features.push(FeatureGroup {
                name: feature.to_string(),
                pretty: feature.to_string(),
                features: vec![Feature {
                    name: feature.to_string(),
                }],
            });
        }
        if !attributes
            .iter()
            .any(|attribute| attribute.name == channel.attribute)
        {
            attributes.push(Attribute {
                name: channel.attribute.clone(),
                pretty: channel.attribute.clone(),
                feature: format!("{feature}.{feature}").parse().ok(),
                physical_unit: channel.physical_unit,
                ..Attribute::default()
            });
        }
    }

    let dmx_channels: Vec<DmxChannel> = spec
        .channels
        .iter()
        .map(|channel| {
            let width = channel.offsets.len().clamp(1, 4) as u8;
            DmxChannel {
                dmx_break: "1".into(),
                offset: channel
                    .offsets
                    .iter()
                    .map(|offset| offset.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                geometry: BODY.into(),
                logical_channels: vec![LogicalChannel {
                    attribute: channel.attribute.parse().ok(),
                    channel_functions: vec![ChannelFunction {
                        name: channel.attribute.clone(),
                        attribute: channel.attribute.parse().ok(),
                        dmx_from: Some(DmxValue::new(0, width)),
                        default: Some(DmxValue::new(channel.default, width)),
                        physical_from: Some(channel.physical_from.unwrap_or(0.0)),
                        physical_to: Some(channel.physical_to.unwrap_or(1.0)),
                        ..ChannelFunction::default()
                    }],
                    ..LogicalChannel::default()
                }],
                ..DmxChannel::default()
            }
        })
        .collect();

    let beam = spec.beam_angle.map(|angle| {
        GeometryNode::Beam(Beam {
            name: "Beam".into(),
            model: Some(BODY.into()),
            position: Some(Matrix::default()),
            beam_angle: Some(angle),
            field_angle: Some(angle),
            beam_type: Some(BeamType::Wash),
            ..Beam::default()
        })
    });

    let properties = (spec.weight_kg.is_some() || spec.power_w.is_some()).then(|| Properties {
        weight: spec.weight_kg.map(|value| Weight { value: Some(value) }),
        power_consumption: spec
            .power_w
            .map(|value| {
                vec![PowerConsumption {
                    value: Some(value),
                    ..Default::default()
                }]
            })
            .unwrap_or_default(),
        ..Properties::default()
    });

    let fixture_type = FixtureType {
        name: spec.name.clone(),
        short_name: spec.short_name.clone(),
        long_name: spec.name.clone(),
        manufacturer: spec.manufacturer.clone(),
        description: spec.description.clone(),
        fixture_type_id: spec.fixture_type_id.clone(),
        can_have_children: Some(YesNo::No),
        attribute_definitions: AttributeDefinitions {
            activation_groups: None,
            feature_groups: FeatureGroups { items: features },
            attributes: Attributes { items: attributes },
        },
        physical_descriptions: properties.map(|properties| PhysicalDescriptions {
            properties: Some(properties),
            ..PhysicalDescriptions::default()
        }),
        models: Some(Models {
            items: vec![Model {
                name: BODY.into(),
                length: Some(0.3),
                width: Some(0.3),
                height: Some(0.3),
                primitive_type: Some(PrimitiveType::Cube),
                ..Model::default()
            }],
        }),
        geometries: Geometries {
            children: vec![GeometryNode::Geometry(Geometry {
                name: BODY.into(),
                model: Some(BODY.into()),
                position: Some(Matrix::default()),
                children: beam.into_iter().collect(),
            })],
        },
        dmx_modes: DmxModes {
            items: vec![DmxMode {
                name: if spec.mode_name.is_empty() {
                    "Default".into()
                } else {
                    spec.mode_name.clone()
                },
                geometry: BODY.into(),
                dmx_channels: DmxChannels {
                    items: dmx_channels,
                },
                ..DmxMode::default()
            }],
        },
        ..FixtureType::default()
    };

    GdtfFile {
        description: Gdtf {
            data_version: Gdtf::DATA_VERSION.into(),
            fixture_type,
        },
        resources: Default::default(),
    }
}
