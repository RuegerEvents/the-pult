//! Reading an MVR into a plan of writes.
//!
//! Pure: bytes and what the show already holds go in, an [`ImportPlan`] comes out,
//! and nothing is stored until [`super::super::apply::apply`] runs it. A file this
//! console will not accept therefore leaves neither an asset nor a row behind.
//!
//! # What matches what
//!
//! Everything is keyed by the uuid the file uses, all the way down to the fixture:
//! an imported fixture's `id` **is** its MVR uuid. Re-importing a drawing therefore
//! updates the rig rather than doubling it, with no lookup table to keep, and
//! exporting writes the ids back without having to invent them.
//!
//! A fixture *type* is the exception, and is keyed by the GDTF's own `FixtureTypeID`
//! rather than by the name the archive gave the file. One real drawing ships the same
//! 185,652-byte Robe file twice under two names and points fixtures at both; keyed by
//! name that show imports with two identical types, and an operator patching a spare
//! picks the wrong one half the time.
//!
//! # What wins
//!
//! The file. A re-import overwrites the transform, the address, the mode, the name,
//! the layer and the parent, and the report counts what it touched. Taking a new
//! drawing and then merging it with the old one is not something anybody asked for; a
//! diff they can undo with one Ctrl-Z is.
//!
//! What it does **not** do is delete. A row an earlier import put in a layer this
//! file also has, which this file no longer mentions, is listed under `missing` —
//! somebody may have taken that light out on purpose, and an importer that tidied up
//! would take the rig with it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use pult_gdtf::GdtfFile;
use pult_schema::stock::parse_stock_symdef;
use pult_mvr::model::{AuxItem, ChildList, ChildNode, GeometryNode, Object};
use pult_mvr::{MvrFile, SpecMatch};
use pult_schema::types::catalogue;
use pult_schema::types::dmx_mode::DmxBreak;
use pult_schema::types::mount::Mount;
use pult_schema::types::fixture::{Fixture, FixtureAddress, FixtureType};
use pult_schema::types::scene::{
    GeometryRef, Layer, NamedAsset, SceneClass, SceneObject, SceneObjectKind, Symbol, Transform,
};
use uuid::Uuid;

use crate::infra::assets;
use crate::infra::interop::apply::ImportPlan;
use crate::infra::interop::gdtf;

use super::placement_as_transform;

/// What the show already holds, for matching against.
#[derive(Default)]
pub struct Existing<'a> {
    pub fixture_types: &'a [FixtureType],
    pub fixtures: &'a [Fixture],
    pub scene_objects: &'a [SceneObject],
    pub layers: &'a [Layer],
    pub symbols: &'a [Symbol],
    pub classes: &'a [SceneClass],
    pub named_assets: &'a [NamedAsset],
}

/// Read an `.mvr` and work out everything that should happen to the show.
pub fn plan_import(bytes: &[u8], existing: &Existing) -> Result<ImportPlan, pult_mvr::Error> {
    let file = MvrFile::parse(bytes)?;
    let mut planner = Planner::new(&file, existing);
    planner.warn_all(file.warnings.iter().map(ToString::to_string));

    // The aux data first, and that ordering is load-bearing: a symdef this console
    // wrote for one of its own catalogue pieces carries a mesh nobody should store,
    // and `take_assets` has to be told which files those are before it walks them.
    planner.take_aux_data();
    planner.take_assets();
    planner.take_fixture_types();
    planner.take_layers();
    planner.note_what_is_missing();

    Ok(planner.finish(bytes))
}

/// Everything the walk accumulates on its way through a file.
struct Planner<'a> {
    file: &'a MvrFile,
    existing: &'a Existing<'a>,
    plan: ImportPlan,
    /// The fixture type each `GDTFSpec` string resolved to.
    types_by_spec: HashMap<String, Uuid>,
    /// The modes each of those types has, for matching `GDTFMode` against.
    modes_by_type: HashMap<Uuid, Vec<String>>,
    /// Every id this file wrote, so what it did *not* write can be named.
    seen: BTreeSet<Uuid>,
    /// The layers this file has, which is what scopes `missing`.
    layers_seen: BTreeSet<Uuid>,
    /// Symdefs this console wrote for its own catalogue: the piece and what it was
    /// asked for, keyed by the symdef's uuid.
    ///
    /// An object instancing one of these comes back as a `catalogue` piece rather
    /// than as a symbol with a mesh, which is what makes a rig built here survive a
    /// round trip as the thing it was. A symdef *anybody else* wrote is an ordinary
    /// symbol: its mesh is the truth about it.
    stock: HashMap<Uuid, (String, serde_json::Value)>,
    /// And the files those symdefs name, which are not stored. The console generates
    /// that mesh from the table whenever it needs one, so keeping the copy would be
    /// an asset that goes stale the next time the geometry is improved.
    stock_files: BTreeSet<String>,
    /// Which catalogue piece each object written so far turned out to be.
    ///
    /// Read by [`Planner::mount_of`]: MVR has nowhere to say that a light is *clamped*
    /// to a bar, only where it is, so a re-import would otherwise turn every clamp in
    /// the rig into a free placement. Where the parent is a piece the console knows
    /// the shape of, the clamp can be read back off the geometry — and only then, and
    /// only when it lands on it exactly.
    catalogue_of: HashMap<Uuid, String>,
}

impl<'a> Planner<'a> {
    fn new(file: &'a MvrFile, existing: &'a Existing<'a>) -> Self {
        Planner {
            file,
            existing,
            plan: ImportPlan::default(),
            types_by_spec: HashMap::new(),
            modes_by_type: HashMap::new(),
            seen: BTreeSet::new(),
            layers_seen: BTreeSet::new(),
            stock: HashMap::new(),
            stock_files: BTreeSet::new(),
            catalogue_of: HashMap::new(),
        }
    }

    fn warn(&mut self, at: &str, message: impl std::fmt::Display) {
        self.plan.report.warnings.push(format!("{at}: {message}"));
    }

    fn warn_all(&mut self, messages: impl Iterator<Item = String>) {
        self.plan.report.warnings.extend(messages);
    }

    fn finish(mut self, bytes: &[u8]) -> ImportPlan {
        // The archive itself, kept whole for the reason a `.gdtf` is: the rows are a
        // reading of it, and a later version of this console will read more out of the
        // same bytes than this one does.
        self.plan.assets.push((assets::MVR_MIME.to_string(), bytes.to_vec()));
        self.plan
    }

    // ── Assets ────────────────────────────────────────────────────────

    /// Every mesh and texture in the archive, stored under its sha and remembered
    /// under the name the file gave it.
    ///
    /// The name matters and is not decoration: a `.3ds` asks for its texture as
    /// `tx603.jpg` and nothing else, and a content-addressed store has no names in it.
    fn take_assets(&mut self) {
        let resources: Vec<(String, Vec<u8>)> = self
            .file
            .resources
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect();

        for (name, bytes) in resources {
            // A mesh this console generated for one of its own pieces. The row says
            // which piece it is and the bytes follow from that, so storing them would
            // be keeping a copy that the next version of this console disagrees with.
            if self.stock_files.contains(&name) {
                continue;
            }
            let Some(mime) = assets::mime_for_name(&name) else {
                self.warn(&name, "this console does not store files of this kind");
                continue;
            };
            // A GDTF is stored by the fixture-type pass, which needs to read it first.
            if mime == assets::GDTF_MIME {
                continue;
            }
            if bytes.len() > assets::ceiling_for(mime).unwrap_or(0) {
                self.warn(&name, "this file is larger than the console will store");
                continue;
            }

            let sha = assets::digest(&bytes);
            self.plan.assets.push((mime.to_string(), bytes));
            self.write_named_asset(&name, &sha, mime);
        }
    }

    fn write_named_asset(&mut self, name: &str, sha: &str, mime: &str) {
        let id = NamedAsset::id_for(name);
        let row = NamedAsset {
            id,
            name: name.to_string(),
            asset: sha.to_string(),
            mime: mime.to_string(),
        };
        let replaces = self.existing.named_assets.iter().find(|each| each.id == id).map(|e| e.id);
        self.seen.insert(id);
        self.plan.write("named_assets", replaces, json(&row));
    }

    // ── Fixture types ─────────────────────────────────────────────────

    /// Every `GDTFSpec` any object in the file names, resolved to a fixture type.
    fn take_fixture_types(&mut self) {
        let mut specs: Vec<String> = Vec::new();
        for layer in &self.file.scene.scene.layers.items {
            collect_specs(layer.children.as_ref(), &mut specs);
        }
        specs.sort();
        specs.dedup();

        for spec in specs {
            match self.file.gdtf_named(&spec) {
                Some((entry, bytes, rung)) => {
                    let entry = entry.to_string();
                    let bytes = bytes.to_vec();
                    // Not `Extension`: writing the spec without the suffix the
                    // archive entry has is how one whole family of exporters spells
                    // one, so warning on it would put a line in the report for every
                    // fixture in every grandMA file — which is how a report becomes
                    // something nobody reads. `Case` and `Loosely` are the ones worth
                    // saying, and `Loosely` is the one that could be wrong.
                    if !matches!(rung, SpecMatch::Exact | SpecMatch::Extension) {
                        self.warn(
                            &spec,
                            format!("found in the archive as {entry:?}, matched {rung:?}"),
                        );
                    }
                    self.take_one_gdtf(&spec, &bytes);
                }
                None => {
                    // A drawing may name a fixture whose definition it does not
                    // carry. A placeholder keeps the patch — the address, the mode,
                    // the place — so a later import of the real file fills it in
                    // rather than the rig having to be drawn again.
                    let id = gdtf::placeholder_id(&spec);
                    self.warn(&spec, "the archive does not carry this GDTF; a placeholder stands in");
                    self.write_placeholder_type(&spec, id);
                    self.types_by_spec.insert(spec.clone(), id);
                }
            }
        }
    }

    fn take_one_gdtf(&mut self, spec: &str, bytes: &[u8]) {
        let file = match GdtfFile::parse(bytes) {
            Ok(file) => file,
            Err(error) => {
                self.warn(spec, format!("this GDTF does not parse ({error}); a placeholder stands in"));
                let id = gdtf::placeholder_id(spec);
                self.write_placeholder_type(spec, id);
                self.types_by_spec.insert(spec.to_string(), id);
                return;
            }
        };

        let sha = assets::digest(bytes);
        let (fixture_type, warnings) = gdtf::derive_fixture_type(&file, &sha);
        let id = fixture_type.id;
        self.warn_all(warnings.iter().map(|w| format!("{spec}: {w}")));

        // Keyed by the file's own id. Two names for one definition — which a real
        // drawing does have — become one type both fixtures point at.
        if self.types_by_spec.values().any(|each| *each == id) {
            self.warn(spec, "the same fixture definition is in this archive under another name too");
        } else {
            self.plan.assets.push((assets::GDTF_MIME.to_string(), bytes.to_vec()));
            let replaces =
                self.existing.fixture_types.iter().find(|each| each.id == id).map(|each| each.id);
            self.seen.insert(id);
            self.plan.write("fixture_types", replaces, json(&fixture_type));
        }

        self.modes_by_type
            .insert(id, fixture_type.dmx_modes.iter().map(|mode| mode.name.clone()).collect());
        self.types_by_spec.insert(spec.to_string(), id);
    }

    /// A type that stands in for a definition the archive did not carry.
    ///
    /// Deliberately empty of parameters: inventing them would be writing a lie about
    /// a fixture nobody can see, and the name is what an operator needs in order to
    /// go and find the real file.
    fn write_placeholder_type(&mut self, spec: &str, id: Uuid) {
        let (manufacturer, name) = spec.split_once('@').unwrap_or(("", spec));
        let fixture_type = FixtureType {
            id,
            name: name.trim_end_matches(".gdtf").to_string(),
            manufacturer: manufacturer.to_string(),
            ..FixtureType::default()
        };
        let replaces =
            self.existing.fixture_types.iter().find(|each| each.id == id).map(|each| each.id);
        self.seen.insert(id);
        self.plan.write("fixture_types", replaces, json(&fixture_type));
        self.modes_by_type.insert(id, Vec::new());
    }

    // ── Classes and symbols ───────────────────────────────────────────

    fn take_aux_data(&mut self) {
        let Some(aux) = self.file.scene.scene.aux_data.clone() else { return };
        for item in &aux.items {
            match item {
                AuxItem::Class(class) => {
                    let Some(id) = self.uuid(&class.uuid, &class.name) else { continue };
                    let row = SceneClass { id, name: class.name.clone() };
                    let replaces =
                        self.existing.classes.iter().find(|each| each.id == id).map(|e| e.id);
                    self.seen.insert(id);
                    self.plan.write("classes", replaces, json(&row));
                }
                AuxItem::Symdef(symdef) => {
                    let Some(id) = self.uuid(&symdef.uuid, &symdef.name) else { continue };
                    // One this console wrote for a catalogue piece. The name says
                    // which piece and the uuid is a v5 of the name, so a drawing that
                    // merely *called* a symbol this cannot take its mesh away.
                    if let Some((piece, properties)) = parse_stock_symdef(&symdef.name, id) {
                        for node in symdef.children.iter().flat_map(|list| list.items.iter()) {
                            if let ChildNode::Geometry3D(mesh) = node {
                                self.stock_files.insert(mesh.file_name.clone());
                            }
                        }
                        self.stock.insert(id, (piece, properties));
                        continue;
                    }
                    let geometry = self.geometry_of_child_list(symdef.children.as_ref());
                    let row = Symbol { id, name: symdef.name.clone(), geometry };
                    let replaces =
                        self.existing.symbols.iter().find(|each| each.id == id).map(|e| e.id);
                    self.seen.insert(id);
                    self.plan.write("symbols", replaces, json(&row));
                }
                // A `Position` is a name for a place a fixture hangs, and this console
                // keeps where a fixture hangs on the fixture. A `MappingDefinition` is
                // for video mapping, which it does not do.
                AuxItem::Position(_) | AuxItem::MappingDefinition(_) => {}
            }
        }
    }

    // ── Layers and what is in them ────────────────────────────────────

    fn take_layers(&mut self) {
        let layers = self.file.scene.scene.layers.items.clone();
        for (index, layer) in layers.iter().enumerate() {
            let Some(id) = self.uuid(&layer.uuid, &layer.name) else { continue };
            let row = Layer {
                id,
                name: layer.name.clone(),
                // A drawing does not say; a layer arrives unlocked and somebody locks
                // it here if they want to.
                locked: false,
                sort_order: index as u32,
            };
            let replaces = self.existing.layers.iter().find(|each| each.id == id).map(|e| e.id);
            self.seen.insert(id);
            self.layers_seen.insert(id);
            self.plan.write("layers", replaces, json(&row));

            self.walk(layer.children.as_ref(), id, None);
        }
    }

    /// One `ChildList`, and everything under it.
    ///
    /// `parent` is the object this list hangs off, which is what makes a light on a
    /// truss move when the truss does.
    fn walk(&mut self, list: Option<&ChildList>, layer: Uuid, parent: Option<Uuid>) {
        let Some(list) = list else { return };
        for node in &list.items {
            let Some(object) = node.object() else { continue };
            let Some(id) = self.uuid(&object.uuid, &object.name) else { continue };

            match node {
                ChildNode::Fixture(_) => self.write_fixture(object, id, layer, parent),
                _ => {
                    self.write_scene_object(node, object, id, layer, parent);
                    self.walk(object.children.as_ref(), layer, Some(id));
                }
            }
        }
    }

    fn write_fixture(&mut self, object: &Object, id: Uuid, layer: Uuid, parent: Option<Uuid>) {
        let spec = object.gdtf_spec.as_deref().unwrap_or("").trim().to_string();
        let Some(&fixture_type_id) = self.types_by_spec.get(&spec) else {
            self.warn(&object.name, "this fixture names no GDTF, so there is nothing to patch it as");
            return;
        };

        let position = self.transform_of(object);
        let fixture = Fixture {
            id,
            name: object.name.clone(),
            fixture_type_id,
            address: self.address_of(object, fixture_type_id),
            mount: self.mount_of(parent, &position),
            position: Some(position),
            parent,
            layer: Some(layer),
            class: object.classing.as_deref().and_then(|c| Uuid::parse_str(c.trim()).ok()),
            focus: object.focus.as_deref().and_then(|f| Uuid::parse_str(f.trim()).ok()),
            fixture_number: object.fixture_id,
            unit_number: object.unit_number,
            ..Fixture::default()
        };
        let replaces = self.existing.fixtures.iter().find(|each| each.id == id).map(|e| e.id);
        self.seen.insert(id);
        self.plan.write("fixtures", replaces, json(&fixture));
    }

    fn write_scene_object(
        &mut self,
        node: &ChildNode,
        object: &Object,
        id: Uuid,
        layer: Uuid,
        parent: Option<Uuid>,
    ) {
        let (mut geometry, mut symbol) = self.geometry_of(object);
        // A symbol this console wrote for one of its own pieces comes back as the
        // piece. Never *guessed* from anything else: a drawing's object says what it
        // is with its mesh, and when the mesh did not come with the file the honest
        // answer is that this console does not know how long that truss was — picking
        // an `f34-2m` because the name said "truss" would be putting a measurement
        // into somebody's rig that nobody measured.
        let stock = symbol.and_then(|id| self.stock.get(&id).cloned());
        if stock.is_some() {
            geometry = Vec::new();
            symbol = None;
        }
        let row = SceneObject {
            id,
            name: object.name.clone(),
            kind: kind_of(node),
            transform: self.transform_of(object),
            parent,
            layer: Some(layer),
            class: object.classing.as_deref().and_then(|c| Uuid::parse_str(c.trim()).ok()),
            geometry,
            symbol,
            catalogue: stock.as_ref().map(|(piece, _)| piece.clone()),
            properties: stock
                .map(|(_, properties)| properties)
                .unwrap_or(serde_json::Value::Null),
            // A drawing has nothing to say about this; an operator locks a piece here.
            locked: false,
        };
        if let Some(piece) = &row.catalogue {
            self.catalogue_of.insert(id, piece.clone());
        }
        let replaces = self.existing.scene_objects.iter().find(|each| each.id == id).map(|e| e.id);
        self.seen.insert(id);
        self.plan.write("scene_objects", replaces, json(&row));
    }

    /// The clamp a light's placement says it is on, where anything says so.
    ///
    /// A file has nowhere to write a mount — it says where a fixture is, and that is
    /// all — so this reads one back off the geometry: if the parent is a catalogue
    /// piece and the light is sitting *exactly* where one of that piece's clamps would
    /// put it, then that is what it is on. A millimetre, because the number came out
    /// of this console's own arithmetic on the way out and anything looser would be
    /// inventing a clamp for a light somebody placed by hand.
    ///
    /// A truss out of a drawing gets none. Its chords come off its mesh's bounds and
    /// only the browser measures a mesh; a station guessing here would be guessing.
    fn mount_of(&self, parent: Option<Uuid>, position: &Transform) -> Option<Mount> {
        let piece = catalogue::piece(self.catalogue_of.get(&parent?)?)?;
        let (mount, distance) = Mount::nearest(position.position, piece.chords);
        (distance < 0.001).then_some(mount)
    }

    // ── The details ───────────────────────────────────────────────────

    fn transform_of(&self, object: &Object) -> Transform {
        object
            .matrix
            .as_ref()
            .map(|matrix| placement_as_transform(&pult_mvr::transform::decompose(matrix)))
            .unwrap_or_default()
    }

    /// The meshes an object carries, and the symbol it instances.
    fn geometry_of(&mut self, object: &Object) -> (Vec<GeometryRef>, Option<Uuid>) {
        let mut geometry = Vec::new();
        let mut symbol = None;
        for item in object.geometries.iter().flat_map(|g| g.items.iter()) {
            match item {
                GeometryNode::Geometry3D(mesh) => {
                    if let Some(reference) = self.geometry_ref(mesh) {
                        geometry.push(reference);
                    }
                }
                GeometryNode::Symbol(instance) => {
                    match Uuid::parse_str(instance.symdef.trim()) {
                        Ok(id) => symbol = Some(id),
                        Err(_) => self.warn(&object.name, "its symbol names no readable uuid"),
                    }
                }
            }
        }
        (geometry, symbol)
    }

    /// The meshes directly inside a `ChildList`, which is where a symdef keeps them.
    fn geometry_of_child_list(&mut self, list: Option<&ChildList>) -> Vec<GeometryRef> {
        let Some(list) = list else { return Vec::new() };
        list.items
            .iter()
            .filter_map(|node| match node {
                ChildNode::Geometry3D(mesh) => self.geometry_ref(mesh),
                _ => None,
            })
            .collect()
    }

    fn geometry_ref(&mut self, mesh: &pult_mvr::model::Geometry3D) -> Option<GeometryRef> {
        let bytes = self.file.resources.get(&mesh.file_name)?;
        Some(GeometryRef {
            asset: assets::digest(bytes),
            file_name: mesh.file_name.clone(),
            transform: mesh
                .matrix
                .as_ref()
                .map(|matrix| placement_as_transform(&pult_mvr::transform::decompose(matrix)))
                .unwrap_or_default(),
        })
    }

    /// Where a fixture is patched, in the console's numbering.
    fn address_of(&mut self, object: &Object, fixture_type_id: Uuid) -> FixtureAddress {
        let mut breaks: BTreeMap<u16, DmxBreak> = BTreeMap::new();
        for address in object.addresses.iter().flat_map(|a| a.items.iter()) {
            let (universe, channel) =
                pult_mvr::address::to_universe_and_channel(address.absolute.unwrap_or(1));
            breaks.insert(
                pult_mvr::address::to_break(address.break_id),
                DmxBreak { universe, address: channel },
            );
        }
        if breaks.is_empty() {
            self.warn(&object.name, "this fixture has no address; it is patched at 1/1");
            breaks.insert(1, DmxBreak { universe: 1, address: 1 });
        }

        FixtureAddress::Dmx {
            mode: self.mode_of(object, fixture_type_id),
            breaks: breaks.into_values().collect(),
        }
    }

    /// Which mode a fixture is in, matched against the type's own names.
    ///
    /// Exact first, and only then trimmed: real mode names carry a numeric prefix and
    /// a trailing space — `"2: RGBW "` is a name an Astera file really has — so
    /// trimming before comparing stops finding modes that were there all along.
    fn mode_of(&mut self, object: &Object, fixture_type_id: Uuid) -> String {
        let wanted = object.gdtf_mode.as_deref().unwrap_or("").to_string();
        let Some(modes) = self.modes_by_type.get(&fixture_type_id).cloned() else {
            return wanted;
        };
        if modes.is_empty() || modes.iter().any(|name| *name == wanted) {
            return wanted;
        }
        if let Some(trimmed) = modes.iter().find(|name| name.trim() == wanted.trim()) {
            return trimmed.clone();
        }
        let first = modes[0].clone();
        self.warn(
            &object.name,
            format!("its type has no mode {wanted:?}; patched in {first:?} instead"),
        );
        first
    }

    /// A uuid as the file wrote it, or one derived from the name it gave the thing.
    fn uuid(&mut self, raw: &str, name: &str) -> Option<Uuid> {
        match Uuid::parse_str(raw.trim()) {
            Ok(id) => Some(id),
            Err(_) if raw.trim().is_empty() => {
                self.warn(name, "this object has no uuid and was skipped");
                None
            }
            Err(_) => {
                // Derived rather than random, so a second import of the same file
                // matches what the first one wrote.
                let id = gdtf::placeholder_id(raw.trim());
                self.warn(name, format!("its uuid {raw:?} is not a uuid; using {id} instead"));
                Some(id)
            }
        }
    }

    // ── What went ─────────────────────────────────────────────────────

    /// Rows in a layer this file has, that this file no longer mentions.
    ///
    /// Scoped to the layers the file is about: a drawing of the overhead rig should
    /// not report every floor light as missing. Listed and never deleted.
    fn note_what_is_missing(&mut self) {
        let mut missing = Vec::new();
        for object in self.existing.scene_objects {
            if let Some(layer) = object.layer {
                if self.layers_seen.contains(&layer) && !self.seen.contains(&object.id) {
                    missing.push(format!("{} ({})", object.name, object.id));
                }
            }
        }
        for fixture in self.existing.fixtures {
            if let Some(layer) = fixture.layer {
                if self.layers_seen.contains(&layer) && !self.seen.contains(&fixture.id) {
                    missing.push(format!("{} ({})", fixture.name, fixture.id));
                }
            }
        }
        missing.sort();
        self.plan.report.missing = missing;
    }
}

/// What kind of object a tag names.
fn kind_of(node: &ChildNode) -> SceneObjectKind {
    match node {
        ChildNode::Truss(_) => SceneObjectKind::Truss,
        ChildNode::Support(_) => SceneObjectKind::Support,
        ChildNode::VideoScreen(_) => SceneObjectKind::VideoScreen,
        ChildNode::Projector(_) => SceneObjectKind::Projector,
        ChildNode::FocusPoint(_) => SceneObjectKind::FocusPoint,
        ChildNode::GroupObject(_) => SceneObjectKind::Group,
        _ => SceneObjectKind::Object,
    }
}

fn collect_specs(list: Option<&ChildList>, out: &mut Vec<String>) {
    let Some(list) = list else { return };
    for node in &list.items {
        let Some(object) = node.object() else { continue };
        if matches!(node, ChildNode::Fixture(_)) {
            if let Some(spec) = object.gdtf_spec.as_ref().filter(|s| !s.trim().is_empty()) {
                out.push(spec.trim().to_string());
            }
        }
        collect_specs(object.children.as_ref(), out);
    }
}

fn json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("a schema type serialises")
}
