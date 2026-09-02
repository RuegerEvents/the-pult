//! Pure queries over a parsed fixture type.
//!
//! The object model is the file's shape; this is the answers a console wants out of
//! it. Every function here is a read — no allocation of a new model, no mutation —
//! so an importer, a validator and an exporter can all ask the same questions and
//! get the same answers.
//!
//! The one that shapes the rest is [`expand_mode`]. A GDTF mode's channel list is
//! not the whole layout: a `GeometryReference` says "everything under that geometry,
//! again, at this offset", so a nine-cell bar with one cell described once and
//! referenced nine times has nine times the channels its `<DMXChannels>` lists. A
//! reader that skipped the expansion would patch the bar as if it were one cell, and
//! nothing about the numbers would say so.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    Beam, DmxChannel, DmxMode, FixtureType, GeometryNode, GeometryReference, Model,
};
use crate::values::{DmxValue, Node};
use crate::Warning;

/// How deep a `GeometryReference` chain may go before we stop following it.
///
/// A reference may point at a geometry that itself contains references. A cycle is
/// possible to write and impossible to expand, so the walk is depth-capped and the
/// cap is reported as a warning rather than a panic.
const MAX_REFERENCE_DEPTH: usize = 8;

// ── Geometry ─────────────────────────────────────────────────────────

/// Find a geometry by its dot path from the roots: `Base.Yoke.Head`.
///
/// A single-segment path is matched against the roots *and* against every
/// descendant, because that is what files do: a `DMXChannel`'s `Geometry` attribute
/// commonly names a deep node by its bare name.
pub fn find_geometry<'a>(fixture: &'a FixtureType, path: &Node) -> Option<&'a GeometryNode> {
    let segments = &path.0;
    if segments.is_empty() {
        return None;
    }
    if let Some(found) = walk_path(&fixture.geometries.children, segments) {
        return Some(found);
    }
    if segments.len() == 1 {
        return find_by_name(&fixture.geometries.children, &segments[0]);
    }
    None
}

fn walk_path<'a>(nodes: &'a [GeometryNode], segments: &[String]) -> Option<&'a GeometryNode> {
    let (head, rest) = segments.split_first()?;
    let node = nodes.iter().find(|node| node.name() == head)?;
    if rest.is_empty() {
        Some(node)
    } else {
        walk_path(node.children(), rest)
    }
}

fn find_by_name<'a>(nodes: &'a [GeometryNode], name: &str) -> Option<&'a GeometryNode> {
    for node in nodes {
        if node.name() == name {
            return Some(node);
        }
        if let Some(found) = find_by_name(node.children(), name) {
            return Some(found);
        }
    }
    None
}

/// The `Beam` node under a geometry subtree, if it has one.
///
/// What the rig view wants: the origin of the light and its real beam angle, instead
/// of a constant that is right for no fixture.
pub fn find_beam<'a>(fixture: &'a FixtureType, mode: &DmxMode) -> Option<&'a Beam> {
    let roots: &[GeometryNode] = match mode.geometry.parse::<Node>().ok().as_ref() {
        Some(path) if !path.is_empty() => match find_geometry(fixture, path) {
            Some(node) => node.children(),
            None => &fixture.geometries.children,
        },
        _ => &fixture.geometries.children,
    };
    first_beam(roots).or_else(|| first_beam(&fixture.geometries.children))
}

fn first_beam(nodes: &[GeometryNode]) -> Option<&Beam> {
    for node in nodes {
        if let GeometryNode::Beam(beam) = node {
            return Some(beam);
        }
        if let Some(found) = first_beam(node.children()) {
            return Some(found);
        }
    }
    None
}

/// Every `Axis` under a geometry subtree, outermost first.
///
/// Pan is the outer axis and tilt the inner one on nearly every moving head, so the
/// order is the answer to "which one does pan turn" without the fixture having to
/// say.
pub fn axes<'a>(fixture: &'a FixtureType, mode: &DmxMode) -> Vec<&'a GeometryNode> {
    let roots: &[GeometryNode] = match mode.geometry.parse::<Node>().ok().as_ref() {
        Some(path) if !path.is_empty() => match find_geometry(fixture, path) {
            Some(node) => std::slice::from_ref(node),
            None => &fixture.geometries.children,
        },
        _ => &fixture.geometries.children,
    };
    let mut found = Vec::new();
    collect_axes(roots, &mut found);
    found
}

fn collect_axes<'a>(nodes: &'a [GeometryNode], out: &mut Vec<&'a GeometryNode>) {
    for node in nodes {
        if node.is_axis() {
            out.push(node);
        }
        collect_axes(node.children(), out);
    }
}

/// The model a geometry node names, if the type declares one.
pub fn find_model<'a>(fixture: &'a FixtureType, name: &str) -> Option<&'a Model> {
    fixture
        .models
        .as_ref()?
        .items
        .iter()
        .find(|model| model.name == name)
}

/// The archive path of a model's mesh, glTF preferred.
///
/// A `Model`'s `File` has no directory and no extension, and the same stem may exist
/// in both `models/gltf/` and `models/3ds/`. glTF first because three.js loads it
/// properly and the 3DS loader is a legacy path that fails on files nothing else
/// minds.
pub fn model_file(model: &Model, resources: &BTreeMap<String, Vec<u8>>) -> Option<String> {
    if model.file.is_empty() {
        return None;
    }
    let candidates = [
        format!("models/gltf/{}.glb", model.file),
        format!("models/gltf/{}.gltf", model.file),
        format!("models/3ds/{}.3ds", model.file),
    ];
    candidates
        .into_iter()
        .find(|path| resources.contains_key(path))
        .or_else(|| {
            // Some files put the mesh somewhere else entirely; fall back to any entry
            // whose stem matches, rather than dropping the geometry.
            resources
                .keys()
                .find(|path| {
                    std::path::Path::new(path)
                        .file_stem()
                        .is_some_and(|stem| stem.eq_ignore_ascii_case(&model.file))
                })
                .cloned()
        })
}

// ── Modes ────────────────────────────────────────────────────────────

/// One DMX channel of a mode, after `GeometryReference` expansion.
///
/// Flat on purpose: the break and the offsets are absolute, so a consumer writing a
/// frame does no tree walking at all.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedChannel<'a> {
    /// Which DMX break, 1-based.
    pub break_number: u16,
    /// Byte offsets within the break, 1-based, coarse to fine. Empty for a virtual
    /// channel that occupies no slot.
    pub offsets: Vec<u16>,
    /// The channel as the file wrote it.
    pub channel: &'a DmxChannel,
    /// The geometry path this instance belongs to, so nine copies of one cell are
    /// nine distinguishable channels rather than nine identical ones.
    pub geometry_path: Vec<String>,
    /// The attribute the first logical channel names, which is what the console maps
    /// onto a parameter kind.
    pub attribute: Option<&'a Node>,
}

impl ResolvedChannel<'_> {
    /// The last offset this channel occupies, for a footprint.
    pub fn last_offset(&self) -> u16 {
        self.offsets.iter().copied().max().unwrap_or(0)
    }

    /// How many bytes wide this channel is.
    pub fn byte_count(&self) -> u8 {
        self.offsets.len().min(4) as u8
    }

    /// This channel's default, at this channel's width.
    ///
    /// A GDTF default carries the width it was written at (`128/1`), so putting it
    /// into a 16-bit channel means rescaling, not zero-extending.
    pub fn default(&self) -> u32 {
        let width = self.byte_count().max(1);
        self.channel
            .logical_channels
            .iter()
            .flat_map(|logical| logical.channel_functions.iter())
            .find_map(|function| function.default)
            .map(|value| value.rescale(width))
            .unwrap_or(0)
    }
}

/// Every channel of a mode, with references expanded and breaks resolved.
///
/// A channel belongs to a geometry, and how many times it appears is how many times
/// that geometry appears. Two ways it can:
///
/// - **Directly**, when the geometry sits in the mode's own subtree. One instance, at
///   the offsets the channel names.
/// - **Through a `GeometryReference`**, when it does not — the four-cell bar
///   describes one `Pixel` outside the mode's tree and points at it four times, so its
///   four channels become sixteen, each shifted by its reference's `DMXOffset`.
///
/// Counting a template channel *and* its reference copies is the mistake this
/// function exists to not make: it would patch a sixteen-channel bar as twenty.
pub fn expand_mode<'a>(
    fixture: &'a FixtureType,
    mode: &'a DmxMode,
) -> (Vec<ResolvedChannel<'a>>, Vec<Warning>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let at = format!("DMXModes.{}", mode.name);

    // The subtree this mode drives. A mode that names nothing, or names a geometry
    // the file does not have, drives everything — the alternative is a mode with no
    // channels at all, which is never what the file meant.
    let roots: Vec<&GeometryNode> = match mode.geometry.parse::<Node>().ok().as_ref() {
        Some(path) if !path.is_empty() => match find_geometry(fixture, path) {
            Some(node) => vec![node],
            None => {
                warnings.push(Warning::new(
                    &at,
                    format!(
                        "names a geometry {:?} the file does not have",
                        mode.geometry
                    ),
                ));
                fixture.geometries.children.iter().collect()
            }
        },
        _ => fixture.geometries.children.iter().collect(),
    };

    // Which geometry names are in that subtree without crossing a reference, and
    // where each one sits.
    let mut direct: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut references: Vec<(Vec<String>, &GeometryReference)> = Vec::new();
    let mut path = Vec::new();
    for root in &roots {
        collect(root, &mut path, &mut direct, &mut references);
    }

    // Each reference, expanded to the set of geometry names it stands for.
    let expanded: Vec<Instance<'a>> = references
        .iter()
        .flat_map(|(path, reference)| instantiate(fixture, path, reference, 0, &at, &mut warnings))
        .collect();

    for channel in &mode.dmx_channels.items {
        let geometry: Node = channel.geometry.parse().unwrap_or_default();
        let name = geometry.last().unwrap_or_default();

        if name.is_empty() || direct.contains_key(name) {
            out.push(ResolvedChannel {
                break_number: channel.break_number().unwrap_or(1),
                offsets: channel.offsets(),
                channel,
                geometry_path: direct.get(name).cloned().unwrap_or_default(),
                attribute: first_attribute(channel),
            });
            continue;
        }

        let mut placed = false;
        for instance in expanded.iter().filter(|instance| instance.covers(name)) {
            placed = true;
            let (break_number, base) = instance.place(channel);
            out.push(ResolvedChannel {
                break_number,
                offsets: channel
                    .offsets()
                    .into_iter()
                    .map(|offset| offset + base.saturating_sub(1))
                    .collect(),
                channel,
                geometry_path: instance.path.clone(),
                attribute: first_attribute(channel),
            });
        }

        if !placed {
            // Neither in the tree nor behind a reference. Keeping it where it says it
            // is beats dropping it: the offsets are still the file's own, and a
            // fixture short a channel is harder to notice than a warning.
            warnings.push(Warning::new(
                format!("{at}.{name}"),
                "belongs to a geometry this mode does not reach; kept at its own offsets",
            ));
            out.push(ResolvedChannel {
                break_number: channel.break_number().unwrap_or(1),
                offsets: channel.offsets(),
                channel,
                geometry_path: geometry.0.clone(),
                attribute: first_attribute(channel),
            });
        }
    }

    (out, warnings)
}

/// One copy of a referenced geometry: where it sits, what it covers, and where its
/// channels land.
struct Instance<'a> {
    path: Vec<String>,
    /// Every geometry name inside the referenced subtree, including its root.
    covers: BTreeSet<&'a str>,
    /// Offset base per DMX break, from the reference's `<Break>` list.
    by_break: BTreeMap<u16, u16>,
    /// The last `<Break>`, which the spec makes the one a `DMXBreak="Overwrite"`
    /// channel uses.
    overwrite: Option<(u16, u16)>,
}

impl Instance<'_> {
    fn covers(&self, name: &str) -> bool {
        self.covers.contains(name)
    }

    /// Which break this channel lands in, and at what offset base.
    fn place(&self, channel: &DmxChannel) -> (u16, u16) {
        match channel.break_number() {
            Some(number) => (number, self.by_break.get(&number).copied().unwrap_or(1)),
            None => self.overwrite.unwrap_or((1, 1)),
        }
    }
}

/// Walk a subtree, recording the geometries in it and the references out of it.
///
/// Deliberately does not descend into a `GeometryReference`: what is behind one is
/// not part of this subtree, it is a copy of somewhere else.
fn collect<'a>(
    node: &'a GeometryNode,
    path: &mut Vec<String>,
    direct: &mut BTreeMap<&'a str, Vec<String>>,
    references: &mut Vec<(Vec<String>, &'a GeometryReference)>,
) {
    path.push(node.name().to_string());
    match node {
        GeometryNode::GeometryReference(reference) => references.push((path.clone(), reference)),
        _ => {
            direct.entry(node.name()).or_insert_with(|| path.clone());
            for child in node.children() {
                collect(child, path, direct, references);
            }
        }
    }
    path.pop();
}

/// One reference, and any reference nested inside what it points at.
fn instantiate<'a>(
    fixture: &'a FixtureType,
    path: &[String],
    reference: &'a GeometryReference,
    depth: usize,
    at: &str,
    warnings: &mut Vec<Warning>,
) -> Vec<Instance<'a>> {
    if depth >= MAX_REFERENCE_DEPTH {
        warnings.push(Warning::new(
            format!("{at}.{}", path.join(".")),
            "geometry references nest deeper than this reader follows; \
             the rest of this branch was skipped",
        ));
        return Vec::new();
    }

    let target: Node = reference.geometry.parse().unwrap_or_default();
    let Some(node) = find_geometry(fixture, &target) else {
        warnings.push(Warning::new(
            format!("{at}.{}", path.join(".")),
            format!(
                "references a geometry {:?} the file does not have",
                reference.geometry
            ),
        ));
        return Vec::new();
    };

    let mut covers = BTreeSet::new();
    let mut nested: Vec<(Vec<String>, &GeometryReference)> = Vec::new();
    let mut inner = Vec::new();
    let mut names = BTreeMap::new();
    collect(node, &mut inner, &mut names, &mut nested);
    covers.extend(names.into_keys());

    let by_break: BTreeMap<u16, u16> = reference
        .breaks
        .iter()
        .filter_map(|entry| Some((entry.dmx_break?, offset_of(entry))))
        .collect();
    let overwrite = reference
        .breaks
        .last()
        .map(|entry| (entry.dmx_break.unwrap_or(1), offset_of(entry)));

    let mut out = vec![Instance {
        path: path.to_vec(),
        covers,
        by_break,
        overwrite,
    }];
    for (inner_path, nested_reference) in nested {
        let mut full = path.to_vec();
        full.extend(inner_path);
        out.extend(instantiate(
            fixture,
            &full,
            nested_reference,
            depth + 1,
            at,
            warnings,
        ));
    }
    out
}

fn offset_of(entry: &crate::model::Break) -> u16 {
    entry
        .dmx_offset
        .map(|value| value.value as u16)
        .unwrap_or(1)
}

/// The attribute a channel names, which is the console's whole vocabulary for it.
///
/// The logical channel's, falling back to its first function's: the spec puts the
/// attribute on both and files disagree about which they fill in.
fn first_attribute(channel: &DmxChannel) -> Option<&Node> {
    let logical = channel.logical_channels.first()?;
    logical
        .attribute
        .as_ref()
        .or_else(|| logical.channel_functions.first()?.attribute.as_ref())
}

/// How many channels a mode occupies, per break, break 1 first.
///
/// The number the patch panel needs, and it is a list rather than a number because a
/// fixture with a separate break for its dimmer occupies two spans that need not be
/// adjacent or even in the same universe.
pub fn footprint(fixture: &FixtureType, mode: &DmxMode) -> Vec<u16> {
    let (channels, _) = expand_mode(fixture, mode);
    let mut by_break: BTreeMap<u16, u16> = BTreeMap::new();
    for channel in &channels {
        let end = channel.last_offset();
        let entry = by_break.entry(channel.break_number).or_insert(0);
        *entry = (*entry).max(end);
    }
    if by_break.is_empty() {
        return Vec::new();
    }
    // Dense from break 1 to the highest, so the index into the list is the break
    // number minus one and a caller never has to carry the map.
    let highest = *by_break.keys().max().expect("checked non-empty");
    (1..=highest)
        .map(|number| by_break.get(&number).copied().unwrap_or(0))
        .collect()
}

/// The mode of this name, or the first one when the name is unknown.
///
/// Unknown rather than missing: a show patched against a file that has since been
/// revised names a mode the new file does not have, and going dark is worse than
/// going to the first mode and saying so.
pub fn mode<'a>(fixture: &'a FixtureType, name: &str) -> Option<&'a DmxMode> {
    fixture
        .dmx_modes
        .items
        .iter()
        .find(|mode| mode.name == name)
        .or_else(|| fixture.dmx_modes.items.first())
}

/// The channel sets of a channel, flattened into named DMX ranges.
///
/// What a gobo wheel's slot list becomes: an operator picks "Breakup" and the console
/// knows which byte that is.
pub fn channel_sets<'a>(channel: &'a DmxChannel, byte_count: u8) -> Vec<NamedRange<'a>> {
    let width = byte_count.max(1);
    let mut out: Vec<NamedRange<'a>> = Vec::new();
    for logical in &channel.logical_channels {
        for function in &logical.channel_functions {
            for set in &function.channel_sets {
                let Some(from) = set.dmx_from else { continue };
                out.push(NamedRange {
                    name: if set.name.is_empty() {
                        &function.name
                    } else {
                        &set.name
                    },
                    from: from.rescale(width),
                    to: DmxValue::max_for(width),
                    physical_from: set.physical_from,
                    physical_to: set.physical_to,
                    wheel_slot_index: set.wheel_slot_index,
                });
            }
        }
    }
    out.sort_by_key(|range| range.from);
    // A channel set's end is where the next one begins: the spec gives only `DMXFrom`.
    for index in 0..out.len().saturating_sub(1) {
        out[index].to = out[index + 1].from.saturating_sub(1);
    }
    out
}

/// A named slice of a channel's range, with both ends filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedRange<'a> {
    pub name: &'a str,
    pub from: u32,
    pub to: u32,
    pub physical_from: Option<f32>,
    pub physical_to: Option<f32>,
    pub wheel_slot_index: Option<u32>,
}

/// The physical range a channel covers, from its first channel function.
///
/// What turns a pan into degrees. `None` when the file says nothing, which is
/// honest: a console that invented ±270° for a fixture that never said so would
/// point it somewhere it cannot reach.
pub fn physical_range(channel: &DmxChannel) -> Option<(f32, f32)> {
    let function = channel
        .logical_channels
        .first()?
        .channel_functions
        .first()?;
    match (function.physical_from, function.physical_to) {
        (Some(from), Some(to)) if from != to => Some((from, to)),
        _ => None,
    }
}
