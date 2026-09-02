//! Writing the show back out as an `.mvr`.
//!
//! The other half of [`super::plan`], and the reason this console owns an MVR
//! library rather than using a reader off the shelf: a rig that can be imported and
//! not exported is a rig somebody has to draw again.
//!
//! Pure, like the import: rows and asset bytes go in, an archive comes out. What it
//! writes back is what it read — every uuid is the row's own id, so importing an
//! export into a fresh show gives the same rig rather than a copy of it.
//!
//! What it does **not** carry is what this console does not keep: an MVR's
//! `CastShadow`, `DMXInvertPan`, `CustomCommands` and per-fixture plot colour are
//! another console's settings, and inventing values for them on the way out would be
//! writing things the operator never said.

use std::collections::{BTreeMap, BTreeSet};

use pult_mvr::model::{
    Addresses, Address, AuxData, AuxItem, ChildList, ChildNode, Class, GeneralSceneDescription,
    Geometries, Geometry3D, GeometryNode, Layer as MvrLayer, Layers, Object, Scene, Symbol as MvrSymbol,
    Symdef,
};
use pult_mvr::MvrFile;
use pult_schema::types::fixture::{Fixture, FixtureType};
use pult_schema::types::scene::{
    GeometryRef, Layer, NamedAsset, SceneClass, SceneObject, SceneObjectKind, Symbol, Transform,
};
use uuid::Uuid;

use super::transform_as_placement;

/// The show, as much of it as an export reads.
pub struct Rig<'a> {
    pub fixture_types: &'a [FixtureType],
    pub fixtures: &'a [Fixture],
    pub scene_objects: &'a [SceneObject],
    pub layers: &'a [Layer],
    pub symbols: &'a [Symbol],
    pub classes: &'a [SceneClass],
    pub named_assets: &'a [NamedAsset],
}

/// A file this export wants in the archive, by the name it must have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub name: String,
    /// The sha it is stored under, or `None` for a GDTF the caller must generate.
    pub asset: Option<String>,
    /// The fixture type a generated GDTF is for.
    pub fixture_type: Option<Uuid>,
}

/// The scene, and the files that have to go beside it.
pub struct Export {
    pub scene: GeneralSceneDescription,
    pub wanted: Vec<Wanted>,
}

/// The layer a fixture nobody drew is exported in.
///
/// A rig can be patched here without ever having been in a drawing, and dropping
/// those on the way out would hand somebody an MVR that is missing half their show.
pub const PATCHED_HERE: &str = "Patched here";

/// Build the scene, and say which files belong beside it.
///
/// `only` is the layers to write; empty means all of them.
pub fn plan_export(rig: &Rig, only: &BTreeSet<Uuid>) -> Export {
    let wants = |id: Uuid| only.is_empty() || only.contains(&id);

    let specs = specs_of(rig.fixture_types);
    let mut wanted: Vec<Wanted> = Vec::new();
    let mut used_symbols: BTreeSet<Uuid> = BTreeSet::new();
    let mut used_classes: BTreeSet<Uuid> = BTreeSet::new();
    let mut used_types: BTreeSet<Uuid> = BTreeSet::new();

    let mut layers: Vec<MvrLayer> = Vec::new();
    let mut ordered: Vec<&Layer> = rig.layers.iter().filter(|l| wants(l.id)).collect();
    ordered.sort_by_key(|layer| (layer.sort_order, layer.id));

    for layer in &ordered {
        let children = child_list(
            rig,
            Some(layer.id),
            None,
            &specs,
            &mut used_symbols,
            &mut used_classes,
            &mut used_types,
        );
        layers.push(MvrLayer {
            uuid: layer.id.to_string(),
            name: layer.name.clone(),
            matrix: None,
            children,
        });
    }

    // Anything the show has that no layer claims. Only when the whole rig is being
    // written: an export of two named layers is an export of two named layers.
    if only.is_empty() {
        let loose =
            child_list(rig, None, None, &specs, &mut used_symbols, &mut used_classes, &mut used_types);
        if loose.is_some() {
            layers.push(MvrLayer {
                uuid: Uuid::new_v5(&Uuid::NAMESPACE_OID, PATCHED_HERE.as_bytes()).to_string(),
                name: PATCHED_HERE.into(),
                matrix: None,
                children: loose,
            });
        }
    }

    // The definitions the fixtures above are patched to.
    for id in &used_types {
        let Some(fixture_type) = rig.fixture_types.iter().find(|t| t.id == *id) else { continue };
        let Some(spec) = specs.get(id) else { continue };
        wanted.push(Wanted {
            name: format!("{spec}.gdtf"),
            asset: kept_archive(fixture_type),
            fixture_type: Some(*id),
        });
    }

    // Exporting the whole show means the whole show: a symbol nothing instances and
    // a class nothing is tagged with are still the operator's, and an export that
    // quietly dropped them would not survive being imported again. A *filtered*
    // export is a filtered export, and carries what its layers use.
    if only.is_empty() {
        used_symbols.extend(rig.symbols.iter().map(|s| s.id));
        used_classes.extend(rig.classes.iter().map(|c| c.id));
    }

    // The symbols they instance, and the meshes those name.
    let mut aux: Vec<AuxItem> = Vec::new();
    for id in &used_symbols {
        let Some(symbol) = rig.symbols.iter().find(|s| s.id == *id) else { continue };
        for reference in &symbol.geometry {
            want_mesh(&mut wanted, reference);
        }
        aux.push(AuxItem::Symdef(Symdef {
            uuid: symbol.id.to_string(),
            name: symbol.name.clone(),
            children: Some(ChildList {
                items: symbol.geometry.iter().map(as_geometry_node).collect(),
            }),
        }));
    }
    for id in &used_classes {
        let Some(class) = rig.classes.iter().find(|c| c.id == *id) else { continue };
        aux.push(AuxItem::Class(Class {
            uuid: class.id.to_string(),
            name: class.name.clone(),
        }));
    }

    // And the meshes objects carry directly.
    for object in rig.scene_objects.iter().filter(|o| o.layer.is_none_or(wants)) {
        for reference in &object.geometry {
            want_mesh(&mut wanted, reference);
        }
    }

    // A name may be wanted twice — two objects sharing one mesh — and the archive has
    // one entry for it.
    wanted.sort_by(|a, b| a.name.cmp(&b.name));
    wanted.dedup_by(|a, b| a.name == b.name);
    // Anything a name maps to that this station does not hold is dropped rather than
    // written empty: a zero-byte mesh is worse than an object with no geometry.
    wanted.retain(|w| w.asset.is_some() || w.fixture_type.is_some());
    let _ = rig.named_assets;

    Export {
        scene: GeneralSceneDescription {
            ver_major: 1,
            ver_minor: 6,
            provider: Some("the-pult".into()),
            provider_version: Some(env!("CARGO_PKG_VERSION").into()),
            scene: Scene {
                aux_data: (!aux.is_empty()).then_some(AuxData { items: aux }),
                layers: Layers { items: layers },
            },
        },
        wanted,
    }
}

/// Everything in one layer that hangs off `parent`, as MVR sees it.
fn child_list(
    rig: &Rig,
    layer: Option<Uuid>,
    parent: Option<Uuid>,
    specs: &BTreeMap<Uuid, String>,
    symbols: &mut BTreeSet<Uuid>,
    classes: &mut BTreeSet<Uuid>,
    types: &mut BTreeSet<Uuid>,
) -> Option<ChildList> {
    let mut items: Vec<ChildNode> = Vec::new();

    let mut objects: Vec<&SceneObject> =
        rig.scene_objects.iter().filter(|o| o.layer == layer && o.parent == parent).collect();
    objects.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    for object in objects {
        if let Some(id) = object.symbol {
            symbols.insert(id);
        }
        if let Some(id) = object.class {
            classes.insert(id);
        }
        let children = child_list(rig, layer, Some(object.id), specs, symbols, classes, types);
        items.push(as_child_node(object, children));
    }

    let mut fixtures: Vec<&Fixture> =
        rig.fixtures.iter().filter(|f| f.layer == layer && f.parent == parent).collect();
    fixtures.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    for fixture in fixtures {
        types.insert(fixture.fixture_type_id);
        if let Some(id) = fixture.class {
            classes.insert(id);
        }
        items.push(ChildNode::Fixture(as_fixture_object(
            fixture,
            specs.get(&fixture.fixture_type_id),
        )));
    }

    (!items.is_empty()).then_some(ChildList { items })
}

fn as_child_node(object: &SceneObject, children: Option<ChildList>) -> ChildNode {
    let inner = Object {
        uuid: object.id.to_string(),
        name: object.name.clone(),
        matrix: Some(matrix_of(&object.transform)),
        geometries: geometries_of(&object.geometry, object.symbol),
        classing: object.class.map(|id| id.to_string()),
        children,
        ..Object::default()
    };
    match object.kind {
        SceneObjectKind::Truss => ChildNode::Truss(inner),
        SceneObjectKind::Support => ChildNode::Support(inner),
        SceneObjectKind::VideoScreen => ChildNode::VideoScreen(inner),
        SceneObjectKind::Projector => ChildNode::Projector(inner),
        SceneObjectKind::FocusPoint => ChildNode::FocusPoint(inner),
        SceneObjectKind::Group => ChildNode::GroupObject(inner),
        SceneObjectKind::Object => ChildNode::SceneObject(inner),
    }
}

fn as_fixture_object(fixture: &Fixture, spec: Option<&String>) -> Object {
    let addresses: Vec<Address> = fixture
        .address
        .breaks()
        .iter()
        .enumerate()
        .map(|(index, span)| Address {
            break_id: Some(pult_mvr::address::from_break(index as u16 + 1)),
            absolute: Some(pult_mvr::address::to_absolute(span.universe, span.address)),
        })
        .collect();

    Object {
        uuid: fixture.id.to_string(),
        name: fixture.name.clone(),
        matrix: Some(matrix_of(&fixture.position.unwrap_or_default())),
        classing: fixture.class.map(|id| id.to_string()),
        gdtf_spec: spec.map(|spec| format!("{spec}.gdtf")),
        gdtf_mode: fixture.address.mode().map(str::to_string),
        addresses: (!addresses.is_empty()).then_some(Addresses { items: addresses }),
        fixture_id: fixture.fixture_number,
        unit_number: fixture.unit_number,
        focus: fixture.focus.map(|id| id.to_string()),
        ..Object::default()
    }
}

fn geometries_of(geometry: &[GeometryRef], symbol: Option<Uuid>) -> Option<Geometries> {
    let mut items: Vec<GeometryNode> = geometry.iter().map(as_geometry_node_inner).collect();
    if let Some(id) = symbol {
        items.push(GeometryNode::Symbol(MvrSymbol {
            // MVR gives an instance a uuid of its own; derived from the symbol it
            // instances so that exporting twice writes the same file.
            uuid: Uuid::new_v5(&Uuid::NAMESPACE_OID, id.as_bytes()).to_string(),
            symdef: id.to_string(),
            matrix: None,
        }));
    }
    (!items.is_empty()).then_some(Geometries { items })
}

fn as_geometry_node(reference: &GeometryRef) -> ChildNode {
    ChildNode::Geometry3D(Geometry3D {
        file_name: reference.file_name.clone(),
        matrix: non_identity(&reference.transform),
    })
}

fn as_geometry_node_inner(reference: &GeometryRef) -> GeometryNode {
    GeometryNode::Geometry3D(Geometry3D {
        file_name: reference.file_name.clone(),
        matrix: non_identity(&reference.transform),
    })
}

fn want_mesh(wanted: &mut Vec<Wanted>, reference: &GeometryRef) {
    wanted.push(Wanted {
        name: reference.file_name.clone(),
        asset: (!reference.asset.is_empty()).then(|| reference.asset.clone()),
        fixture_type: None,
    });
}

/// A `Vendor@Product` per fixture type, each one different from all the others.
///
/// Two types can honestly want the same name: one real drawing carries the same Robe
/// head twice, exported by Vectorworks as two definitions with two `FixtureTypeID`s
/// and one product name. Written under one archive entry they would become one type
/// on the way back in, and half the rig would repatch itself. So a name that is
/// already taken gets a number, in id order, which is stable across exports.
fn specs_of(fixture_types: &[FixtureType]) -> BTreeMap<Uuid, String> {
    let mut ordered: Vec<&FixtureType> = fixture_types.iter().collect();
    ordered.sort_by_key(|fixture_type| fixture_type.id);

    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut specs = BTreeMap::new();
    for fixture_type in ordered {
        let wanted = spec_of(fixture_type);
        let mut name = wanted.clone();
        let mut nth = 2;
        while !taken.insert(name.clone()) {
            name = format!("{wanted} ({nth})");
            nth += 1;
        }
        specs.insert(fixture_type.id, name);
    }
    specs
}

/// `Vendor@Product`, which is how every MVR names a fixture definition.
fn spec_of(fixture_type: &FixtureType) -> String {
    let vendor = if fixture_type.manufacturer.trim().is_empty() {
        "Unknown"
    } else {
        fixture_type.manufacturer.trim()
    };
    format!("{vendor}@{}", fixture_type.name.trim())
}

/// The archive an imported type arrived in, where there is one.
fn kept_archive(fixture_type: &FixtureType) -> Option<String> {
    use pult_schema::types::fixture::FixtureTypeSource;
    match &fixture_type.source {
        FixtureTypeSource::Gdtf { asset, .. } => Some(asset.clone()),
        _ => None,
    }
}

fn matrix_of(transform: &Transform) -> pult_mvr::values::MvrMatrix {
    pult_mvr::transform::compose(&transform_as_placement(transform))
}

/// A matrix only where there is something to say. A mesh at its object's own origin
/// writes no `<Matrix>`, which is what the files this reads do.
fn non_identity(transform: &Transform) -> Option<pult_mvr::values::MvrMatrix> {
    (*transform != Transform::default()).then(|| matrix_of(transform))
}

/// The archive itself, once the caller has fetched every file the plan wants.
pub fn write(export: &Export, files: BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, pult_mvr::Error> {
    MvrFile {
        scene: export.scene.clone(),
        resources: files,
        warnings: Vec::new(),
    }
    .write()
}
