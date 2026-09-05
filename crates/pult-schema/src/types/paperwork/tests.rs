//! What a table says, and — more to the point — what it refuses to say.
//!
//! Most of these are about the honesty rules rather than about arithmetic. Summing a
//! column is not where this goes wrong; printing the sum as though it were complete is.

use std::collections::HashMap;

use uuid::Uuid;

use super::*;
use crate::types::fixture::{FixturePhysical, FixtureType};
use crate::types::scene::{Layer, SceneClass, SceneObject, SceneObjectKind, Transform};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn a_type(n: u128, name: &str, kg: Option<f32>, w: Option<f32>) -> FixtureType {
    FixtureType {
        id: id(n),
        name: name.into(),
        manufacturer: "Maker".into(),
        physical: FixturePhysical { weight_kg: kg, power_w: w, ..Default::default() },
        ..Default::default()
    }
}

fn a_fixture(n: u128, name: &str, type_id: u128, parent: Option<u128>) -> Fixture {
    Fixture {
        id: id(n),
        name: name.into(),
        fixture_type_id: id(type_id),
        address: FixtureAddress::dmx(1, (n as u16) * 10),
        position: Some(Transform::default()),
        parent: parent.map(id),
        mount: None,
        layer: None,
        class: None,
        focus: None,
        fixture_number: Some(n as u32),
        unit_number: None,
        sensed_values: HashMap::new(),
        live_effects: HashMap::new(),
        live_fades: HashMap::new(),
        home_values: HashMap::new(),
    }
}

fn an_object(n: u128, name: &str, kind: SceneObjectKind, parent: Option<u128>) -> SceneObject {
    SceneObject {
        id: id(n),
        name: name.into(),
        kind,
        transform: Transform::default(),
        parent: parent.map(id),
        layer: None,
        class: None,
        geometry: Vec::new(),
        symbol: None,
        catalogue: None,
        properties: serde_json::Value::Null,
        locked: false,
        weight_kg: None,
    }
}

struct Held {
    fixtures: Vec<Fixture>,
    types: Vec<FixtureType>,
    objects: Vec<SceneObject>,
    layers: Vec<Layer>,
    classes: Vec<SceneClass>,
}

impl Held {
    fn rig(&self) -> Rig<'_> {
        Rig {
            fixtures: &self.fixtures,
            fixture_types: &self.types,
            scene_objects: &self.objects,
            layers: &self.layers,
            classes: &self.classes,
        }
    }
}

/// One truss group holding a bar with two lights on it: the shape every loading
/// question here is asked of.
fn a_small_rig() -> Held {
    Held {
        fixtures: vec![
            a_fixture(100, "Spot 1", 10, Some(2)),
            a_fixture(101, "Spot 2", 10, Some(2)),
        ],
        types: vec![a_type(10, "Spikie", Some(20.0), Some(470.0))],
        objects: vec![
            an_object(1, "Front truss", SceneObjectKind::Group, None),
            // Weighed, so the baseline accounts for everything and a test that wants an
            // incomplete total has to make one.
            SceneObject {
                weight_kg: Some(10.0),
                ..an_object(2, "Front bar", SceneObjectKind::Truss, Some(1))
            },
        ],
        layers: Vec::new(),
        classes: Vec::new(),
    }
}

fn request(kind: TableKind) -> TableRequest {
    TableRequest { kind, ..Default::default() }
}

// ── Nothing prints a bare total ──────────────────────────────────────────────

#[test]
fn a_complete_weight_is_stated_flatly() {
    let held = a_small_rig();
    let table = table(&request(TableKind::Loading), &held.rig());
    assert_eq!(table.totals.weight_unknown, 0);
    assert_eq!(table.totals.weight_label(), "50.0 kg", "two lights and the bar");
    assert!(table.notes.is_empty(), "nothing was left out: {:?}", table.notes);
}

#[test]
fn a_weight_missing_one_figure_is_a_floor_and_names_what_is_missing() {
    let mut held = a_small_rig();
    held.types.push(a_type(11, "Titan Tube", None, None));
    held.fixtures.push(a_fixture(102, "Tube 1", 11, Some(2)));

    let table = table(&request(TableKind::Loading), &held.rig());
    assert_eq!(table.totals.weight_unknown, 1);
    assert_eq!(table.totals.weight_label(), "≥ 50.0 kg");
    assert!(
        table.notes.iter().any(|n| n.contains("Tube 1")),
        "the missing item is named: {:?}",
        table.notes
    );
}

#[test]
fn a_total_with_no_figures_at_all_is_a_dash_and_never_a_zero() {
    let mut held = a_small_rig();
    held.types = vec![a_type(10, "Spikie", None, None)];
    held.objects[1].weight_kg = None;
    let table = table(&request(TableKind::Loading), &held.rig());
    assert_eq!(
        table.totals.weight_label(),
        "—",
        "zero would read as a rig that weighs nothing"
    );
}

#[test]
fn a_catalogue_weight_says_it_is_nominal_and_an_entered_one_does_not() {
    let mut held = a_small_rig();
    // The bar is two metres of F34 out of the console's own catalogue, unweighed.
    held.objects[1].weight_kg = None;
    held.objects[1].catalogue = Some("f34-2m".into());
    let nominal = table(&request(TableKind::Loading), &held.rig());
    assert!(nominal.totals.weight_nominal);
    assert!(nominal.totals.weight_label().ends_with("nominal"), "{:?}", nominal.totals);
    assert!(
        nominal.notes.iter().any(|n| n.contains("catalogue figures")),
        "the reader is told what nominal means: {:?}",
        nominal.notes
    );

    // Somebody weighs it. The claim changes.
    held.objects[1].weight_kg = Some(11.5);
    let entered = table(&request(TableKind::Loading), &held.rig());
    assert!(!entered.totals.weight_nominal);
    assert_eq!(entered.totals.weight_label(), "51.5 kg");
}

#[test]
fn an_objects_own_weight_beats_its_catalogue_figure() {
    let mut held = a_small_rig();
    held.objects[1].catalogue = Some("f34-2m".into());
    held.objects[1].weight_kg = Some(9.0);
    let table = table(&request(TableKind::Loading), &held.rig());
    assert_eq!(table.totals.weight_kg, 49.0, "40 kg of lantern and the 9 kg somebody weighed");
}

// ── The structure is in the loading ──────────────────────────────────────────

#[test]
fn a_loading_group_carries_the_truss_as_well_as_the_lights() {
    let mut held = a_small_rig();
    held.objects[1].weight_kg = None;
    held.objects[1].catalogue = Some("f34-2m".into());
    let table = table(&request(TableKind::Loading), &held.rig());

    let group = table.groups.iter().find(|g| g.name == "Front truss").expect("the group");
    assert_eq!(group.rows.len(), 3, "two lights and the bar they are on");
    assert!(
        group.rows.iter().any(|r| matches!(&r[2], Cell::Text(t) if t.starts_with("Structure"))),
        "the bar is a row"
    );
    assert_eq!(group.totals.weight_kg, 53.0, "40 kg of lantern on a 13 kg bar");
}

#[test]
fn a_group_handle_weighs_nothing_of_its_own() {
    // A `Group` is a handle on several things, not a thing. Giving it a row would
    // double-count whatever it holds.
    let held = a_small_rig();
    let table = table(&request(TableKind::Loading), &held.rig());
    let group = table.groups.iter().find(|g| g.name == "Front truss").unwrap();
    assert!(group.rows.iter().all(|r| !matches!(&r[0], Cell::Text(t) if t == "Front truss")));
}

#[test]
fn a_fixture_hanging_off_nothing_is_still_carried() {
    let mut held = a_small_rig();
    held.fixtures.push(a_fixture(103, "Floor can", 10, None));
    let table = table(&request(TableKind::Loading), &held.rig());
    let unplaced = table.groups.iter().find(|g| g.name == UNPLACED).expect("its own group");
    assert_eq!(unplaced.totals.weight_kg, 20.0, "unrigged is not weightless");
    assert_eq!(table.totals.items, 4, "three fixtures and the bar");
}

// ── Grouping ─────────────────────────────────────────────────────────────────

#[test]
fn a_fixture_rolls_up_to_the_nearest_group_and_not_to_the_bar() {
    let held = a_small_rig();
    let table = table(&request(TableKind::Patch), &held.rig());
    assert_eq!(table.groups.len(), 1);
    assert_eq!(table.groups[0].name, "Front truss", "the Group, not the Truss under it");
}

#[test]
fn a_bar_with_no_group_above_it_is_its_own_heading() {
    let mut held = a_small_rig();
    held.objects[1].parent = None;
    held.objects.remove(0);
    let table = table(&request(TableKind::Patch), &held.rig());
    assert_eq!(table.groups[0].name, "Front bar");
}

#[test]
fn a_cycle_in_the_parent_chain_does_not_hang_the_export() {
    let mut held = a_small_rig();
    held.objects[0].parent = Some(id(2)); // the Group's parent is its own child
    held.objects[0].kind = SceneObjectKind::Truss;
    let table = table(&request(TableKind::Patch), &held.rig());
    assert_eq!(table.groups.len(), 1, "it terminates and says something");
}

#[test]
fn grouping_by_layer_uses_the_layers_own_name() {
    let mut held = a_small_rig();
    held.layers.push(Layer {
        id: id(50),
        name: "Overstage".into(),
        locked: false,
        sort_order: 0,
    });
    held.fixtures[0].layer = Some(id(50));
    let mut req = request(TableKind::Patch);
    req.grouping = Grouping::Layer;
    let table = table(&req, &held.rig());
    let names: Vec<&str> = table.groups.iter().map(|g| g.name.as_str()).collect();
    assert!(names.contains(&"Overstage"), "{names:?}");
    assert!(names.contains(&"No layer"), "{names:?}");
}

// ── What a filter took out ───────────────────────────────────────────────────

#[test]
fn a_table_filtered_by_layer_says_what_it_left_out() {
    let mut held = a_small_rig();
    held.layers.push(Layer { id: id(50), name: "FOH".into(), locked: false, sort_order: 0 });
    held.fixtures[0].layer = Some(id(50));

    let mut req = request(TableKind::Loading);
    req.layers = Some(vec![]); // a sheet showing no layers at all
    let table = table(&req, &held.rig());
    assert!(
        table.notes.iter().any(|n| n.contains("not counted")),
        "a filtered total says so: {:?}",
        table.notes
    );
}

#[test]
fn an_unfiltered_table_carries_no_note_about_filtering() {
    let held = a_small_rig();
    let table = table(&request(TableKind::Patch), &held.rig());
    assert!(table.notes.is_empty(), "{:?}", table.notes);
}

// ── Addresses ────────────────────────────────────────────────────────────────

#[test]
fn a_fixture_with_two_breaks_prints_both() {
    let address = FixtureAddress::Dmx {
        mode: "Extended".into(),
        breaks: vec![
            crate::types::dmx_mode::DmxBreak { universe: 1, address: 271 },
            crate::types::dmx_mode::DmxBreak { universe: 2, address: 1 },
        ],
    };
    assert_eq!(address_of(&address), "1/271 + 2/1");
}

#[test]
fn a_node_fixture_prints_its_node_and_never_a_universe_it_has_not_got() {
    let address = FixtureAddress::OpenHaunt { serial: "4d5e6f".into(), universe: None };
    assert_eq!(address_of(&address), "node 4d5e6f");
}

// ── Scale ────────────────────────────────────────────────────────────────────

#[test]
fn a_fitted_scale_snaps_up_to_one_a_rule_is_cut_for() {
    // The sheet this was designed against prints 1:35, which no rule measures.
    assert_eq!(ViewScale::Fit.resolve(35.0), 50);
    assert_eq!(ViewScale::Fit.resolve(50.0), 50, "an exact fit is not rounded away");
    assert_eq!(ViewScale::Fit.resolve(1.0), 10);
}

#[test]
fn a_scale_that_will_not_fit_any_standard_gives_back_the_largest() {
    assert_eq!(ViewScale::Fit.resolve(9_000.0), 1000);
}

#[test]
fn an_explicit_scale_is_taken_as_it_is() {
    assert_eq!(ViewScale::Ratio { denominator: 35 }.resolve(10.0), 35);
    assert_eq!(ViewScale::Ratio { denominator: 0 }.resolve(10.0), 1, "and never divides by zero");
}

// ── Paper ────────────────────────────────────────────────────────────────────

#[test]
fn a3_landscape_is_420_by_297() {
    assert_eq!(Paper::A3.oriented_mm(true), (420.0, 297.0));
    assert_eq!(Paper::A3.oriented_mm(false), (297.0, 420.0));
}

// ── The wire ─────────────────────────────────────────────────────────────────

#[test]
fn a_sheet_block_is_tagged_by_a_field() {
    let block = SheetBlock::Text(TextBlock {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        text: "note".into(),
        size_mm: 2.5,
        bold: false,
    });
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "Text");
    let back: SheetBlock = serde_json::from_value(json).unwrap();
    assert_eq!(back, block);
}

#[test]
fn a_counts_table_is_one_row_per_type() {
    let mut held = a_small_rig();
    held.types.push(a_type(11, "Titan Tube", Some(3.5), Some(45.0)));
    held.fixtures.push(a_fixture(102, "Tube 1", 11, Some(2)));
    held.fixtures.push(a_fixture(103, "Tube 2", 11, Some(2)));

    let table = table(&request(TableKind::Counts), &held.rig());
    assert_eq!(table.groups.len(), 1, "counts are one flat list");
    assert_eq!(table.groups[0].rows.len(), 2, "two types");
    assert_eq!(table.totals.items, 4, "four fixtures");
    assert_eq!(table.totals.weight_kg, 47.0);
    assert_eq!(table.totals.power_label(), "1.03 k W");
}
