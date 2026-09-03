//! Reading a GDTF file as a fixture type, and writing one back out.
//!
//! [`pult_gdtf`] knows the format and nothing about the console; this knows the
//! console and reads the format through that crate. The split is what lets the format
//! library be tested against other people's files with no station anywhere near it.
//!
//! # What a derived type is, and is not
//!
//! It is a *reading* of the file, and the file is the record. The archive is kept
//! whole in the asset store and the row points at it, so a later version of this
//! console reads more out of the same bytes rather than asking anybody to download
//! anything again — and exporting the type hands back exactly what arrived.
//!
//! And it is never rebuilt behind the operator's back.
//! [`FixtureTypeSource::is_derived_from_a_node`] is what decides that: a node's own
//! type is rebuilt whenever the node describes itself again, and doing that to an
//! imported one would throw the file away.

pub mod attributes;

use std::collections::BTreeMap;

use pult_gdtf::model::{FixtureType as GdtfType, PhysicalUnit as GdtfUnit};
use pult_gdtf::{resolve, GdtfFile, Warning};
use pult_schema::types::dmx_mode::{ChannelFunctionRange, DmxChannelLayout, DmxMode};
use pult_schema::types::fixture::{
    parameter_key, Connector, Emitter, FixtureGeometry, FixturePhysical, FixtureType,
    FixtureTypeSource, GeometryKind, ParameterDefinition, ParameterKind, ParameterValue,
    PhysicalRange, PhysicalUnit, Slot, Vec3,
};
use uuid::Uuid;

/// The namespace a placeholder fixture type id is minted in.
///
/// An MVR can name a GDTF the archive does not carry, and a fixture patched to
/// nothing at all is worse than one patched to a type that says what it is waiting
/// for. A v5 uuid over the spec name means the same missing file gets the same
/// placeholder on every station, so the row replicates rather than forking.
const PLACEHOLDER_NAMESPACE: Uuid = Uuid::from_bytes([
    0x9f, 0x2c, 0x1a, 0x44, 0x6d, 0x3e, 0x4b, 0x87, 0xa1, 0x05, 0x7c, 0x9e, 0x33, 0x11, 0x80, 0x52,
]);

/// A fixture type id for a GDTF the console does not have.
pub fn placeholder_id(gdtf_spec: &str) -> Uuid {
    Uuid::new_v5(&PLACEHOLDER_NAMESPACE, gdtf_spec.as_bytes())
}

/// Read a GDTF file as a fixture type.
///
/// `asset` is the sha256 the archive was stored under, which is what makes the row a
/// pointer at the file rather than a replacement for it.
pub fn derive_fixture_type(file: &GdtfFile, asset: &str) -> (FixtureType, Vec<Warning>) {
    let gdtf = &file.description.fixture_type;
    let mut warnings = pult_gdtf::validate::check(gdtf);

    let (parameters, emitters) = derive_parameters(gdtf, &mut warnings);
    let modes = derive_modes(gdtf, &emitters, &mut warnings);
    // Before the physical data, which measures the fixture across it.
    let geometry = derive_geometry(gdtf, file);

    // The file's own id, so importing a newer revision updates the row rather than
    // making a second one beside it.
    let id = Uuid::parse_str(gdtf.fixture_type_id.trim()).unwrap_or_else(|_| {
        warnings.push(Warning::new(
            "FixtureType",
            "has no usable FixtureTypeID; this console minted one from its name",
        ));
        placeholder_id(&format!("{}@{}", gdtf.manufacturer, gdtf.name))
    });

    let revision = gdtf
        .revisions
        .as_ref()
        .and_then(|revisions| revisions.items.last())
        .map(|revision| revision.text.clone())
        .unwrap_or_default();

    let fixture_type = FixtureType {
        id,
        name: if gdtf.name.is_empty() { gdtf.long_name.clone() } else { gdtf.name.clone() },
        manufacturer: gdtf.manufacturer.clone(),
        short_name: gdtf.short_name.clone(),
        long_name: gdtf.long_name.clone(),
        description: gdtf.description.clone(),
        // The first mode's first break, which is what the patch panel has always
        // meant by "how big is it".
        channel_count: modes.first().map(DmxMode::channel_count).unwrap_or(0),
        parameters,
        dmx_modes: modes,
        physical: derive_physical(gdtf, &geometry),
        geometry,
        source: FixtureTypeSource::Gdtf {
            asset: asset.to_string(),
            uuid: gdtf.fixture_type_id.clone(),
            revision,
            share_rid: None,
        },
    };
    (fixture_type, warnings)
}

// ── Parameters ───────────────────────────────────────────────────────

/// What the fixture can do, across every mode it has.
///
/// Across *every* mode on purpose: the parameter list is what the light can do, and a
/// head that has a zoom in one mode still has a zoom. Which of them a given unit can
/// reach is its mode's business, and the programmer greys out the rest.
fn derive_parameters(
    gdtf: &GdtfType,
    warnings: &mut Vec<Warning>,
) -> (Vec<ParameterDefinition>, Vec<Emitter>) {
    let by_name: BTreeMap<&str, &pult_gdtf::model::Attribute> = gdtf
        .attribute_definitions
        .attributes
        .items
        .iter()
        .map(|attribute| (attribute.name.as_str(), attribute))
        .collect();

    let mut parameters: Vec<ParameterDefinition> = Vec::new();
    let mut emitters: Vec<Emitter> = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();

    for mode in &gdtf.dmx_modes.items {
        let (channels, mut mode_warnings) = resolve::expand_mode(gdtf, mode);
        warnings.append(&mut mode_warnings);

        for channel in &channels {
            let Some(node) = channel.attribute else { continue };
            let Some(attribute) = node.last() else { continue };

            // A fine channel is more bits of its coarse one, not a parameter of its
            // own: a console that made `PanRotate` a fader would give an operator two
            // pans, one of which does almost nothing.
            if let Some(main) = by_name.get(attribute).and_then(|a| a.main_attribute.as_ref()) {
                if !main.is_empty() {
                    continue;
                }
            }

            let Some(kind) = attributes::kind_for(attribute) else { continue };
            let key = parameter_key(&kind);

            if let Some(color) = attributes::color_channel(attribute) {
                // The colour's emitters, gathered in the order the file's channels
                // reach them, which is the order the mixer answers in.
                if !emitters.iter().any(|e| e.name == color.emitter) {
                    emitters.push(Emitter {
                        name: color.emitter.to_string(),
                        rgb: emitter_rgb(gdtf, color.emitter, attribute),
                        subtractive: color.subtractive,
                    });
                }
            }

            if let Some(at) = seen.get(&key) {
                // A parameter reached from more than one mode: keep the richer
                // description rather than the first one seen, so a head whose basic
                // mode has an 8-bit pan and whose extended mode says the travel keeps
                // the travel.
                if parameters[*at].physical.is_none() {
                    parameters[*at].physical = physical_range(gdtf, channel, attribute, &by_name);
                }
                continue;
            }

            seen.insert(key, parameters.len());
            parameters.push(ParameterDefinition {
                physical: physical_range(gdtf, channel, attribute, &by_name),
                slots: slots_of(gdtf, channel),
                feature_group: by_name
                    .get(attribute)
                    .and_then(|a| a.feature.as_ref())
                    .and_then(|feature| feature.0.first().cloned()),
                ..ParameterDefinition::new(kind, resting_value_for(&kind_of(attribute)))
            });
        }
    }

    // Every colour channel is the one colour parameter, so the emitter list belongs
    // to it and to nothing else.
    if let Some(colour) = parameters.iter_mut().find(|p| p.kind == ParameterKind::ColorRgb) {
        colour.emitters = emitters.clone();
    }

    (parameters, emitters)
}

fn kind_of(attribute: &str) -> ParameterKind {
    attributes::kind_for(attribute).unwrap_or(ParameterKind::Named(attribute.to_string()))
}

/// Where a parameter rests before anything drives it.
///
/// Zero for everything but a position, which rests in the middle: a moving head whose
/// pan homed to one end would point at the wall.
fn resting_value_for(kind: &ParameterKind) -> ParameterValue {
    match kind {
        ParameterKind::Pan | ParameterKind::Tilt => ParameterValue::Float(0.5),
        ParameterKind::ColorRgb => ParameterValue::rgb(0.0, 0.0, 0.0),
        ParameterKind::Gobo(_)
        | ParameterKind::GoboIndex
        | ParameterKind::ColorWheel(_)
        | ParameterKind::Prism(_)
        | ParameterKind::Frost(_) => ParameterValue::Int(0),
        _ => ParameterValue::Float(0.0),
    }
}

/// The physical range a parameter covers, from the channel and then the attribute.
fn physical_range(
    gdtf: &GdtfType,
    channel: &resolve::ResolvedChannel<'_>,
    attribute: &str,
    by_name: &BTreeMap<&str, &pult_gdtf::model::Attribute>,
) -> Option<PhysicalRange> {
    let (from, to) = resolve::physical_range(channel.channel)?;
    let unit = by_name
        .get(attribute)
        .and_then(|a| a.physical_unit)
        .unwrap_or(GdtfUnit::None);
    let _ = gdtf;
    Some(PhysicalRange { from, to, unit: attributes::unit_for(unit) })
}

/// The named positions on a wheel channel.
fn slots_of(gdtf: &GdtfType, channel: &resolve::ResolvedChannel<'_>) -> Vec<Slot> {
    let wheel = channel
        .channel
        .logical_channels
        .iter()
        .flat_map(|logical| logical.channel_functions.iter())
        .find_map(|function| function.wheel.as_ref())
        .and_then(|node| node.last())
        .and_then(|name| gdtf.wheels.as_ref()?.items.iter().find(|wheel| wheel.name == name));

    let Some(wheel) = wheel else { return Vec::new() };
    wheel
        .slots
        .iter()
        .map(|slot| Slot {
            name: slot.name.clone(),
            color: slot.color.map(|cie| {
                let [r, g, b] = cie.to_linear_rgb();
                Vec3 { x: r, y: g, z: b }
            }),
            // The gobo image is in the kept archive, under this name. Extracting it
            // into the asset store is Task B's, when the browser has something to draw
            // it on; naming it now costs nothing and loses nothing.
            media: (!slot.media_file_name.is_empty()).then(|| slot.media_file_name.clone()),
        })
        .collect()
}

/// An emitter's colour, from the file's measured `<Emitter>` where it has one.
fn emitter_rgb(gdtf: &GdtfType, emitter: &str, attribute: &str) -> Option<Vec3> {
    let physical = gdtf.physical_descriptions.as_ref()?;
    // By the attribute's own colour first, which is what the file says this channel
    // *is*; by a same-named emitter second.
    let attribute_colour = gdtf
        .attribute_definitions
        .attributes
        .items
        .iter()
        .find(|a| a.name == attribute)
        .and_then(|a| a.color);
    let measured = physical
        .emitters
        .as_ref()
        .and_then(|emitters| {
            emitters.items.iter().find(|each| each.name.eq_ignore_ascii_case(emitter))
        })
        .and_then(|each| each.color)
        .or_else(|| {
            physical.filters.as_ref().and_then(|filters| {
                filters
                    .items
                    .iter()
                    .find(|each| each.name.eq_ignore_ascii_case(emitter))
                    .and_then(|each| each.color)
            })
        });
    let cie = measured.or(attribute_colour)?;
    let [r, g, b] = cie.to_linear_rgb();
    Some(Vec3 { x: r, y: g, z: b })
}

// ── Modes ────────────────────────────────────────────────────────────

fn derive_modes(gdtf: &GdtfType, emitters: &[Emitter], warnings: &mut Vec<Warning>) -> Vec<DmxMode> {
    gdtf.dmx_modes
        .items
        .iter()
        .map(|mode| {
            let (channels, _) = resolve::expand_mode(gdtf, mode);
            let breaks = resolve::footprint(gdtf, mode);
            let layouts = channels
                .iter()
                .filter_map(|channel| layout_of(channel, emitters, warnings))
                .collect();
            DmxMode { name: mode.name.clone(), breaks, channels: layouts }
        })
        .collect()
}

fn layout_of(
    channel: &resolve::ResolvedChannel<'_>,
    emitters: &[Emitter],
    warnings: &mut Vec<Warning>,
) -> Option<DmxChannelLayout> {
    let attribute = channel.attribute?.last()?.to_string();
    let kind = attributes::kind_for(&attribute)?;
    if channel.break_number == 0 {
        warnings.push(Warning::new(&attribute, "is in break 0, which does not exist"));
        return None;
    }

    let emitter = attributes::color_channel(&attribute).and_then(|colour| {
        emitters
            .iter()
            .find(|each| each.name == colour.emitter)
            .map(|each| each.name.clone())
    });

    Some(DmxChannelLayout {
        parameter_key: parameter_key(&kind),
        break_index: (channel.break_number - 1) as u8,
        offsets: channel.offsets.clone(),
        default: channel.default(),
        functions: resolve::channel_sets(channel.channel, channel.byte_count())
            .into_iter()
            .map(|range| ChannelFunctionRange {
                name: range.name.to_string(),
                attribute: attribute.clone(),
                dmx_from: range.from,
                dmx_to: range.to,
                physical_from: range.physical_from.unwrap_or(0.0),
                physical_to: range.physical_to.unwrap_or(1.0),
            })
            .collect(),
        emitter,
    })
}

// ── Physical data ────────────────────────────────────────────────────

fn derive_physical(gdtf: &GdtfType, geometry: &[FixtureGeometry]) -> FixturePhysical {
    let mut physical = FixturePhysical::default();

    if let Some(descriptions) = &gdtf.physical_descriptions {
        if let Some(properties) = &descriptions.properties {
            physical.weight_kg = properties.weight.as_ref().and_then(|weight| weight.value);
            physical.power_w =
                properties.power_consumption.first().and_then(|power| power.value);
            physical.leg_height_m =
                properties.leg_height.as_ref().and_then(|height| height.value);
            physical.operating_temperature = properties
                .operating_temperature
                .as_ref()
                .and_then(|range| Some((range.low?, range.high?)));
        }
        if let Some(connectors) = &descriptions.connectors {
            physical.connectors = connectors
                .items
                .iter()
                .map(|connector| Connector {
                    name: connector.name.clone(),
                    kind: connector.kind.clone(),
                    dmx_break: connector.dmx_break,
                })
                .collect();
        }
    }

    // The first mode's beam, which is the fixture's beam: a head does not change its
    // optics when it is patched differently.
    if let Some(mode) = gdtf.dmx_modes.items.first() {
        if let Some(beam) = resolve::find_beam(gdtf, mode) {
            physical.beam_angle_deg = beam.beam_angle.or(beam.field_angle);
        }
    }

    physical.dimensions_m = envelope(geometry);

    physical
}

/// How big the fixture is, across everything it is made of.
///
/// The outermost geometry's own model is *not* the answer, which is the mistake this
/// replaced: on a real moving head that model is the base plate, and reading it gave a
/// MegaPointe a height of nine and a half centimetres. What a rider wants is the
/// envelope — every part's box, at the place its own geometry puts it, unioned.
///
/// Each box is taken as centred on its node, which is what a `Model`'s dimensions mean
/// without a mesh to say otherwise. `None` where no part declared a size at all, which
/// is honest: a file that never said how big it is has not said.
fn envelope(geometry: &[FixtureGeometry]) -> Option<Vec3> {
    // Where each part sits in the fixture's own space, following the parent chain: a
    // head 500 mm up a yoke that is 300 mm up the base is 800 mm up.
    let mut at: BTreeMap<&str, Vec3> = BTreeMap::new();
    let mut low = [f32::MAX; 3];
    let mut high = [f32::MIN; 3];
    let mut any = false;

    for node in geometry {
        let parent = node
            .parent
            .as_deref()
            .and_then(|name| at.get(name).copied())
            .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let here = Vec3 {
            x: parent.x + node.offset.x,
            y: parent.y + node.offset.y,
            z: parent.z + node.offset.z,
        };
        at.insert(node.name.as_str(), here);

        let Some(size) = node.size else { continue };
        any = true;
        for (axis, (centre, extent)) in
            [(here.x, size.x), (here.y, size.y), (here.z, size.z)].into_iter().enumerate()
        {
            low[axis] = low[axis].min(centre - extent / 2.0);
            high[axis] = high[axis].max(centre + extent / 2.0);
        }
    }

    any.then(|| Vec3 { x: high[0] - low[0], y: high[1] - low[1], z: high[2] - low[2] })
}

// ── Importing ────────────────────────────────────────────────────────

/// What a `.gdtf` would do to the show, worked out before any of it happens.
///
/// Pure: it reads the bytes and the fixture types the show already holds, and answers
/// a plan. Nothing is stored, so a file that turns out not to be a GDTF at all leaves
/// nothing behind — which is what makes [`super::apply::apply`]'s recovery honest.
pub fn plan_import(
    bytes: &[u8],
    existing: &[FixtureType],
) -> Result<(super::apply::ImportPlan, Uuid), pult_gdtf::Error> {
    let file = GdtfFile::parse(bytes)?;
    let asset = crate::infra::assets::digest(bytes);
    let (fixture_type, warnings) = derive_fixture_type(&file, &asset);
    let id = fixture_type.id;

    let mut plan = super::apply::ImportPlan {
        assets: vec![(crate::infra::assets::GDTF_MIME.to_string(), bytes.to_vec())],
        ..Default::default()
    };
    plan.report.warnings = warnings.iter().map(ToString::to_string).collect();

    // By the file's own id: a newer revision of a fixture updates the row rather than
    // making a second one beside it, so every fixture patched to it follows.
    let replaces = existing.iter().find(|each| each.id == id).map(|each| each.id);
    plan.write(
        "fixture_types",
        replaces,
        serde_json::to_value(&fixture_type).expect("a fixture type serialises"),
    );
    Ok((plan, id))
}

// ── Writing one back out ─────────────────────────────────────────────

/// A GDTF file for a fixture type the console made for itself.
///
/// Only for a type that has no file of its own: an imported one exports the archive
/// it arrived in, byte for byte, because that archive is the record and a generated
/// approximation of it would be worse in every way. See [`export`].
///
/// Deliberately the minimum the spec allows. Inventing a beam angle or a weight would
/// be writing a lie another console would then act on, and the honest answer for a
/// type somebody typed into the editor is that it does not say.
pub fn generate_gdtf(fixture_type: &FixtureType) -> GdtfFile {
    use pult_gdtf::minimal::{build, MinimalChannel, MinimalSpec};

    let mode = fixture_type.mode(pult_schema::types::fixture::DEFAULT_MODE);
    let by_key: BTreeMap<String, &ParameterDefinition> =
        fixture_type.parameters.iter().map(|p| (parameter_key(&p.kind), p)).collect();

    let mut channels = Vec::new();
    for layout in &mode.channels {
        if layout.offsets.is_empty() {
            continue;
        }
        let Some(parameter) = by_key.get(&layout.parameter_key) else { continue };
        // A colour is one parameter and several channels, and each channel is the
        // attribute of the die it drives — `ColorAdd_W` and not `ColorAdd_R` again.
        let attribute = match (&parameter.kind, layout.emitter.as_deref()) {
            (ParameterKind::ColorRgb, Some(emitter)) => attributes::color_attribute_for(emitter),
            (kind, _) => attributes::attribute_for(kind),
        };
        channels.push(MinimalChannel {
            attribute,
            offsets: layout.offsets.clone(),
            default: layout.default,
            physical_from: parameter.physical.map(|range| range.from),
            physical_to: parameter.physical.map(|range| range.to),
            physical_unit: parameter.physical.map(|range| gdtf_unit(range.unit)),
            feature: parameter.feature_group.clone().unwrap_or_else(|| feature_of(&parameter.kind)),
        });
    }

    build(&MinimalSpec {
        name: fixture_type.name.clone(),
        short_name: fixture_type.short_name.clone(),
        manufacturer: fixture_type.manufacturer.clone(),
        description: fixture_type.description.clone(),
        // The console's own id, so exporting and re-importing lands on this row
        // rather than a second one beside it.
        fixture_type_id: fixture_type.id.to_string().to_uppercase(),
        mode_name: mode.name.clone(),
        channels,
        weight_kg: fixture_type.physical.weight_kg,
        power_w: fixture_type.physical.power_w,
        beam_angle: fixture_type.physical.beam_angle_deg,
    })
}

/// The bytes to hand back for a type, and the filename to hand them back under.
///
/// The kept archive where there is one — an imported fixture exports as the file it
/// arrived in, which is what makes a round trip through this console lossless.
pub async fn export(
    assets: &crate::infra::assets::AssetStore,
    fixture_type: &FixtureType,
) -> anyhow::Result<(Vec<u8>, String)> {
    let name = format!(
        "{}@{}.gdtf",
        sanitise(&fixture_type.manufacturer),
        sanitise(&fixture_type.name)
    );

    if let FixtureTypeSource::Gdtf { asset, .. } = &fixture_type.source {
        if let Some(stored) = assets.get(asset).await? {
            return Ok((stored.bytes, name));
        }
        // The row says a file, and the store does not have it — this station joined
        // after the import and has not fetched it. Generating one is better than a
        // 404: it is a real fixture type and the operator asked for a file.
        tracing::warn!(
            asset = %asset,
            "this station does not hold the GDTF this type came from; generating one instead"
        );
    }

    Ok((generate_gdtf(fixture_type).write()?, name))
}

/// A filename somebody can save without an operating system objecting.
fn sanitise(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "fixture".into()
    } else {
        trimmed.to_string()
    }
}

/// Which encoder page a kind belongs on, for a type that never said.
fn feature_of(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Intensity => "Dimmer",
        ParameterKind::Pan | ParameterKind::Tilt => "Position",
        ParameterKind::ColorRgb | ParameterKind::ColorWheel(_) | ParameterKind::ColorTemperature => {
            "Color"
        }
        ParameterKind::Zoom
        | ParameterKind::Focus
        | ParameterKind::Iris
        | ParameterKind::Shutter
        | ParameterKind::Strobe
        | ParameterKind::Gobo(_)
        | ParameterKind::GoboIndex
        | ParameterKind::GoboRotation(_)
        | ParameterKind::Prism(_)
        | ParameterKind::Frost(_) => "Beam",
        _ => "Control",
    }
    .to_string()
}

fn gdtf_unit(unit: PhysicalUnit) -> GdtfUnit {
    match unit {
        PhysicalUnit::Percent => GdtfUnit::Percent,
        PhysicalUnit::Degrees => GdtfUnit::Angle,
        PhysicalUnit::Seconds => GdtfUnit::Time,
        PhysicalUnit::Hertz => GdtfUnit::Frequency,
        PhysicalUnit::Kelvin => GdtfUnit::Temperature,
        PhysicalUnit::Metres => GdtfUnit::Length,
        PhysicalUnit::Watts => GdtfUnit::Power,
        PhysicalUnit::None => GdtfUnit::None,
    }
}

// ── Geometry ─────────────────────────────────────────────────────────

/// The fixture's parts, flattened, in console axes and metres.
///
/// What makes a 3D body articulate. Only the tree of the *first* mode's geometry is
/// walked: a mode is a way of addressing a fixture, not a different fixture.
fn derive_geometry(gdtf: &GdtfType, _file: &GdtfFile) -> Vec<FixtureGeometry> {
    let mut out = Vec::new();
    walk_geometry(gdtf, &gdtf.geometries.children, None, 0, &mut out);
    out
}

/// How deep the walk goes. A fixture is a base, a yoke, a head and a beam; anything
/// past this is a mesh detail nothing on screen would tell apart.
const MAX_GEOMETRY_DEPTH: usize = 8;

fn walk_geometry(
    gdtf: &GdtfType,
    nodes: &[pult_gdtf::model::GeometryNode],
    parent: Option<&str>,
    depth: usize,
    out: &mut Vec<FixtureGeometry>,
) {
    if depth >= MAX_GEOMETRY_DEPTH {
        return;
    }
    for node in nodes {
        let common = node.common();
        let mm = common.position.map(|matrix| matrix.translation_mm()).unwrap_or([0.0; 3]);
        let [x, y, z] = pult_gdtf::values::to_console(mm);
        let model = common.model.and_then(|name| resolve::find_model(gdtf, name));

        out.push(FixtureGeometry {
            name: common.name.to_string(),
            parent: parent.map(str::to_string),
            kind: match node {
                pult_gdtf::model::GeometryNode::Axis(_) => GeometryKind::Axis,
                pult_gdtf::model::GeometryNode::Beam(_) => GeometryKind::Beam,
                _ => GeometryKind::Body,
            },
            offset: Vec3 { x, y, z },
            size: model.and_then(|model| {
                // GDTF `Model` dimensions are metres while geometry translations are
                // millimetres. That inconsistency is the spec's; it is written down
                // here because it is exactly what gets silently unified.
                Some(Vec3 { x: model.length?, y: model.height?, z: model.width? })
            }),
            // The meshes stay in the kept archive until there is something to draw
            // them with. Extracting them into the asset store is Task B's.
            model_asset: None,
            beam_angle_deg: match node {
                pult_gdtf::model::GeometryNode::Beam(beam) => beam.beam_angle.or(beam.field_angle),
                _ => None,
            },
        });

        walk_geometry(gdtf, node.children(), Some(common.name), depth + 1, out);
    }
}

#[cfg(test)]
mod tests;
