//! Sheets, and what goes on them.
//!
//! Paperwork is the drawing a rigger works off and the tables a supplier quotes from,
//! and this module holds two things that look unrelated until you notice they are the
//! same document: the *description* of a sheet — paper, viewports, tables, title block
//! — and the *arithmetic* behind the tables.
//!
//! # Why the arithmetic is here and the drawing is not
//!
//! Only the browser can draw. A viewport composes the scene, and composing the scene
//! means measuring meshes, which only `frontend/src/lib/geometry.ts` ever does — the
//! station holds a sha256 and never loads the file behind it. So [`SheetBlock`] is a
//! description this crate can hold and never render.
//!
//! The tables are the opposite. Weight, power and counts are arithmetic over rows the
//! station already has, and putting them here rather than in the browser is what lets
//! the `paperwork.tables` RPC answer a plugin, the command line, or curl asking what a
//! truss weighs. It was very nearly written in TypeScript beside the renderer, which
//! would have made the rig's own weight a number no plugin could reach.
//!
//! # Nothing prints a bare total
//!
//! This is the rule the whole module is arranged around, and it is not tidiness. A
//! per-truss loading is a figure somebody hangs a truss on. So [`Totals`] carries what
//! it could not account for as well as what it summed — how many items had no weight,
//! and whether any figure in it came from the catalogue's nominal data rather than
//! from something a person entered — and [`Totals::weight_label`] is the one place
//! that becomes text, so the PDF, the CSV and a plugin's answer cannot disagree about
//! whether a number is complete.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::catalogue;
use super::fixture::{Fixture, FixtureAddress, FixtureType};
use super::group::{self, SelectionQuery};
use super::scene::{Layer, SceneClass, SceneObject, SceneObjectKind};
use crate::PultSchema;

// ── The sheet ────────────────────────────────────────────────────────────────

/// A paper size, in the only series this console offers.
///
/// A-series only: a production office prints A3 and a rigger reads A3. Adding Letter
/// would be two more variants and a second set of margins for the one case where
/// somebody's printer disagrees with the drawing's own aspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Paper {
    A4,
    #[default]
    A3,
    A2,
    A1,
}

impl Paper {
    /// Width and height in millimetres, portrait.
    pub fn size_mm(self) -> (f32, f32) {
        match self {
            Paper::A4 => (210.0, 297.0),
            Paper::A3 => (297.0, 420.0),
            Paper::A2 => (420.0, 594.0),
            Paper::A1 => (594.0, 841.0),
        }
    }

    /// Width and height in millimetres, the way round the sheet is.
    pub fn oriented_mm(self, landscape: bool) -> (f32, f32) {
        let (w, h) = self.size_mm();
        if landscape {
            (h, w)
        } else {
            (w, h)
        }
    }
}

/// A rectangle on the sheet, in millimetres from its top-left corner.
///
/// Millimetres rather than fractions of the page, because a viewport at 1:50 is a
/// window of a known size onto the rig and a fraction would change what it framed when
/// somebody moved the sheet from A3 to A2.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
}

/// Where a viewport looks from.
///
/// The same five places `frontend/src/lib/camera.ts` offers, named here so a sheet can
/// hold one. What each of them *is* stays in the browser: the box it fits is over the
/// rig, which is geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ViewPreset {
    #[default]
    Front,
    Plan,
    /// From stage left, so the stage is on the left of the frame the way a section is
    /// drawn.
    Section,
    ThreeQuarter,
    /// Framed on what is selected rather than on the rig. Of limited use on a sheet,
    /// which has no selection — it frames whatever the viewport's layers hold.
    Focus,
}

/// Orthographic or perspective.
///
/// The distinction is not cosmetic: **only an orthographic viewport has a scale**. A
/// perspective one prints NTS, because a shaded axonometric captioned 1:50 is a lie
/// somebody measures off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Projection {
    #[default]
    Orthographic,
    Perspective,
}

/// What a viewport claims about its size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type")]
pub enum ViewScale {
    /// Fill the frame, then **snap down to a standard ratio** so the drawing can be
    /// measured with a rule somebody owns. The fitted ratio is almost never a round
    /// number — the sheet this feature was designed against prints 1:35 — and a
    /// drawing at 1:35 is a drawing nobody can check.
    Fit,
    /// One of [`STANDARD_SCALES`], or any other denominator a person typed.
    Ratio { denominator: u32 },
}

impl Default for ViewScale {
    fn default() -> Self {
        ViewScale::Fit
    }
}

/// The ratios a rule is cut for, smallest denominator first.
///
/// `Fit` snaps to one of these — the first that still fits what has to be shown, which
/// is the largest drawing that fits on the paper.
pub const STANDARD_SCALES: &[u32] = &[10, 20, 25, 50, 100, 200, 500, 1000];

impl ViewScale {
    /// The denominator to draw at, given what a free fit would have come to.
    ///
    /// Rounds *up* the denominator, never down: 1:35 becomes 1:50 and not 1:25,
    /// because a scale that does not fit crops the drawing and a scale that overfits
    /// only wastes paper. Beyond the largest standard scale it gives that one back and
    /// the caller crops — a rig that will not fit on A1 at 1:1000 is not a paperwork
    /// problem.
    pub fn resolve(self, fitted_denominator: f32) -> u32 {
        match self {
            ViewScale::Ratio { denominator } => denominator.max(1),
            ViewScale::Fit => STANDARD_SCALES
                .iter()
                .copied()
                .find(|&s| s as f32 >= fitted_denominator)
                .unwrap_or_else(|| *STANDARD_SCALES.last().unwrap()),
        }
    }
}

/// How a drafting viewport draws a solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum LineMode {
    /// Each object's projected outline filled and painted far to near, so a nearer
    /// piece covers a further one.
    ///
    /// The look of hidden-line removal for a fraction of the work, and exact for the
    /// convex pieces a rig is mostly made of. Where two objects interpenetrate it is
    /// wrong, and says so rather than pretending: the painter's algorithm has no
    /// answer for a truss run through a wall.
    #[default]
    Hidden,
    /// Every edge, nothing hidden. What a fixtures plan wants, where seeing the truss
    /// through the light is the point.
    Wireframe,
}

/// What colours a drafting viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Ink {
    /// Black on white, with line weight carrying what colour would have.
    #[default]
    Mono,
    ByLayer,
    ByClass,
}

/// What a picture viewport renders with: the rig panel's four modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PictureMode {
    Wireframe,
    /// Where a light is pointing, under a flat alpha-blended shader. The honest one
    /// for showing coverage on paper.
    Cones,
    #[default]
    Real,
    Photoreal,
}

/// Vector or raster, and the settings each needs.
///
/// **A viewport carries its own and never reads `stores/view.ts`.** The rig panel's
/// work light and render mode are one browser's, kept in `localStorage`; a sheet that
/// inherited them would export differently from the tablet than from the desk, and a
/// document that depends on which machine printed it is not a document. Haze is the
/// exception and stays the show's, being a fact about the room rather than about the
/// screen.
// Not `Copy` since a picture carries a list of cues. Nothing clones one per frame, and
// the alternative — a fixed-size list of cue ids — would put an arbitrary ceiling on how
// many sequences a look may be built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type")]
pub enum ViewStyle {
    /// Vector: lines and filled outlines, drawn into the sheet itself.
    Drafting { lines: LineMode, ink: Ink },
    /// Raster: the rig renderer, off an offscreen canvas, placed as an image.
    Picture {
        mode: PictureMode,
        /// How much flat light is on the scene, 0 to 1, the way the rig panel's View
        /// sheet says it.
        work_light: f32,
        /// Dots per inch to render at. Capped in the browser by what the GPU will
        /// allocate, and the effective figure is reported rather than silently
        /// reduced.
        dpi: u32,
        /// Which cues the rig is standing in, for the picture.
        ///
        /// A beauty shot of a rig with nothing on is a photograph of a dark room, so a
        /// picture viewport says what the lights are doing rather than taking whatever
        /// the console happens to be showing. Empty means *now* — what the show is
        /// actually doing, which is what somebody documenting a state they have just
        /// built wants.
        ///
        /// **Nothing is taken.** The values are worked out from the cue stack and
        /// handed to the renderer for that one frame; the playback is not touched and
        /// no fade starts. Rendering a sheet must not put the rig into the state it is
        /// drawing — that would be a document changing the show it documents, and on a
        /// console with the lamps on it would be visible in the room.
        ///
        /// Several, because a look is usually more than one sequence: a colour state
        /// and a movement running over it. They are applied in order, so a later one
        /// wins where two capture the same parameter.
        #[serde(default)]
        cues: Vec<CueShot>,
    },
}

/// A cue, and how long it has been running when the picture is taken.
///
/// The time is the whole reason this is not a bare id. **A cue sampled the instant it
/// is taken is a photograph of the state before it** — a five-second fade up from black
/// is black at zero, and a movement effect is at the start of its swing with every head
/// pointing the same way. Both are the least interesting frame of the cue and both are
/// what you get without this.
///
/// So a viewport says *four seconds in*, and the fade has landed and the effect has run
/// a quarter of a cycle. Different per cue, because a look is often a state that has
/// settled with a movement still running over it, and the two want different moments.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CueShot {
    pub cue: Uuid,
    /// Milliseconds since the cue was taken.
    ///
    /// Defaulted to [`SETTLED_MS`] rather than to zero, because zero is the one value
    /// that is reliably wrong.
    #[serde(default = "settled")]
    pub at_ms: u32,
}

/// How long into a cue a picture is taken when nobody says.
///
/// Five seconds: longer than the fade on almost any cue somebody writes, so the state
/// has arrived, and far enough into an effect to be somewhere other than its first
/// frame.
pub const SETTLED_MS: u32 = 5_000;

fn settled() -> u32 {
    SETTLED_MS
}

impl CueShot {
    /// This cue, at the moment it has settled.
    pub fn settled(cue: Uuid) -> Self {
        CueShot { cue, at_ms: SETTLED_MS }
    }

    /// This cue, at a moment of somebody's choosing.
    pub fn at(cue: Uuid, at_ms: u32) -> Self {
        CueShot { cue, at_ms }
    }
}

impl Default for ViewStyle {
    fn default() -> Self {
        ViewStyle::Drafting { lines: LineMode::Hidden, ink: Ink::Mono }
    }
}

/// One line of a fixture's label on a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum LabelField {
    Name,
    /// `Fixture::fixture_number` — MVR's FixtureID, the number on the label.
    Number,
    /// `Fixture::unit_number` — which of several identical units this is.
    Unit,
    /// `universe/address` for every break the fixture has, or the node and port for
    /// one on an OpenHaunt node, which has no universe to print.
    Address,
    TypeName,
    /// The four-or-so characters a patch sheet has room for.
    TypeShortName,
    /// Which DMX mode the unit is set to.
    Mode,
}

/// Which end of a bar a rigger measures from.
///
/// Every crew has a convention and none of them is the right one: some hang from stage
/// left because that is where the truss numbering starts, some from the centre line
/// because that is what the drawing is symmetrical about, and some from whichever end
/// the tape happens to be tied to. A drawing that dimensioned from the wrong end is a
/// drawing somebody measures wrong from, so this is asked rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Datum {
    /// The bar's own left end, seen the way the viewport draws it.
    #[default]
    Left,
    Right,
    /// The middle of the bar, which on a symmetrical rig is the centre line.
    Centre,
}

/// Dimensions along the bars, for the crew hanging the rig.
///
/// A plan says where a light is; it does not say how far along the truss to slide it,
/// and that is the only question anybody up a ladder is actually asking. So a drafting
/// viewport can carry a **chain of running dimensions per bar**: the distance from the
/// chosen datum to each head, and the gap between one head and the next.
///
/// Only on a drafting viewport, and not because of an implementation limit: a rendered
/// picture has no stated scale, so a measurement drawn on one would be a number with no
/// units anybody could check.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Dimensions {
    pub datum: Datum,
    /// Whether to print the running distance from the datum as well as the gaps.
    ///
    /// Both by default. A chain of gaps alone accumulates a crew's rounding errors
    /// along the bar; a set of distances alone makes somebody subtract to find a
    /// spacing. Drawings carry both for exactly that reason.
    #[serde(default = "yes")]
    pub running: bool,
    /// Which side of the bar the chain is drawn on.
    ///
    /// Below by default, because labels go above a head — putting both on one side is
    /// how a plan becomes unreadable, and the de-collision only knows about labels.
    #[serde(default)]
    pub above: bool,
}

fn yes() -> bool {
    true
}

impl Default for Dimensions {
    fn default() -> Self {
        Dimensions { datum: Datum::Left, running: true, above: false }
    }
}

/// A window onto the rig, on a sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ViewportBlock {
    pub rect: Rect,
    /// Printed under the frame. Empty for none.
    #[serde(default)]
    pub title: String,
    pub view: ViewPreset,
    #[serde(default)]
    pub projection: Projection,
    #[serde(default)]
    pub scale: ViewScale,
    #[serde(default)]
    pub style: ViewStyle,
    /// Which layers this viewport shows. `None` is every layer in the show —
    /// deliberately *not* "every layer this browser has visible", because a sheet
    /// whose content depended on what somebody had toggled would print differently
    /// every time.
    #[serde(default)]
    pub layers: Option<Vec<Uuid>>,
    /// The lines of a head's label, in order. Empty for an unlabelled drawing.
    #[serde(default)]
    pub labels: Vec<LabelField>,
    #[serde(default)]
    pub scale_bar: bool,
    /// A mark saying which way is upstage. Meaningless on an elevation, and drawn only
    /// on a plan.
    #[serde(default)]
    pub orientation_mark: bool,
    /// Dimensions along each bar, where this viewport carries them. See [`Dimensions`].
    #[serde(default)]
    pub dimensions: Option<Dimensions>,
}

/// What a table is a table of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TableKind {
    /// The instrument schedule: one row per fixture, with its type, address and place.
    #[default]
    Patch,
    /// What hangs where, and what it weighs. **Includes the structure's own weight**,
    /// because the number a rigger wants is what is on the point, not what is on the
    /// truss.
    Loading,
    /// What it draws.
    Power,
    /// One row per fixture type, with how many there are. The rider's table.
    Counts,
}

/// What a table's rows are grouped and subtotalled by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Grouping {
    /// By what it hangs off: the nearest [`SceneObjectKind::Group`] up the parent
    /// chain, falling back to the parent object itself.
    ///
    /// The default, and the one a loading table means — a `Group` is the handle that
    /// moves a truss and its lights together, which makes it the closest thing the
    /// drawing has to a rigging point.
    #[default]
    Structure,
    Layer,
    Class,
    FixtureType,
    /// One list, no subtotals.
    Flat,
}

/// A table on a sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TableBlock {
    pub rect: Rect,
    #[serde(default)]
    pub title: String,
    pub kind: TableKind,
    #[serde(default)]
    pub grouping: Grouping,
    /// Which fixtures are in it. `None` follows the sheet's own layers; a query is
    /// asked of the rig and stays true after somebody patches a fifth mover.
    #[serde(default)]
    pub rows: Option<SelectionQuery>,
    /// Layers to keep, when `rows` is `None`. `None` again is all of them.
    #[serde(default)]
    pub layers: Option<Vec<Uuid>>,
}

/// Free text on a sheet: a note, a legend, a warning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextBlock {
    pub rect: Rect,
    pub text: String,
    /// Cap height in millimetres. 2.5 mm is a drawing's small text, 5 mm a heading.
    #[serde(default = "default_text_mm")]
    pub size_mm: f32,
    #[serde(default)]
    pub bold: bool,
}

fn default_text_mm() -> f32 {
    2.5
}

/// One thing on a sheet.
///
/// Tagged by `type` for the reason [`super::layout::LayoutNode`] is: the frontend
/// discriminates on a field, and every operation over a sheet is written against that
/// tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type")]
pub enum SheetBlock {
    Viewport(ViewportBlock),
    Table(TableBlock),
    Text(TextBlock),
}

impl SheetBlock {
    pub fn rect(&self) -> Rect {
        match self {
            SheetBlock::Viewport(b) => b.rect,
            SheetBlock::Table(b) => b.rect,
            SheetBlock::Text(b) => b.rect,
        }
    }
}

/// One sheet of the paperwork.
///
/// Seeded into a new show rather than being a built-in preset the way a
/// [`super::layout::Layout`] is. The consequence, recorded because it will be asked
/// about: a showfile made before this feature existed opens with no sheets and stays
/// that way, and a show made today never picks up a later version's better defaults.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PultSchema)]
#[ts(export)]
#[pult(table = "sheets")]
pub struct Sheet {
    #[pult(lifecycle = PERSISTED, primary_key)]
    pub id: Uuid,
    /// The drawing's own name, printed in the title block: "Fixtures", "Sections".
    #[pult(lifecycle = PERSISTED)]
    pub name: String,
    /// Where it comes in the set. The export writes them in this order.
    ///
    /// Not `index`, which is a SQL keyword the generated `CREATE TABLE` does not quote
    /// — a column called that fails to open the show. The same trap
    /// [`super::scene::Layer::sort_order`] carries a note about, found the same way.
    #[pult(lifecycle = PERSISTED)]
    pub sort_order: i32,
    #[pult(lifecycle = PERSISTED)]
    pub paper: Paper,
    #[pult(lifecycle = PERSISTED)]
    pub landscape: bool,
    /// The A/B–1/2/3 border round the drawing, so two people on a phone can name the
    /// same part of it.
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub frame: bool,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub title_block: bool,
    #[serde(default)]
    #[pult(lifecycle = PERSISTED)]
    pub blocks: Vec<SheetBlock>,
}

// ── The arithmetic ───────────────────────────────────────────────────────────

/// What a total could and could not account for.
///
/// The unknown counts are not diagnostics. They are the reason this type exists rather
/// than an `f32`: a weight summed over the items that had one, printed as though it
/// were summed over all of them, is the single most dangerous thing this feature can
/// produce.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Totals {
    /// How many things are in it — fixtures and, for a loading table, structure.
    pub items: usize,
    pub weight_kg: f32,
    /// How many of `items` contributed a weight.
    pub weight_known: usize,
    /// And how many had none. `weight_kg` is a floor whenever this is not zero.
    pub weight_unknown: usize,
    /// Whether any figure in `weight_kg` came from
    /// [`catalogue::StockPiece::weight_kg`] rather than from something a person
    /// entered. A nominal total is a different claim and says so.
    pub weight_nominal: bool,
    pub power_w: f32,
    pub power_known: usize,
    pub power_unknown: usize,
}

impl Totals {
    fn add_weight(&mut self, kg: Option<f32>, nominal: bool) {
        self.items += 1;
        match kg {
            Some(kg) => {
                self.weight_kg += kg;
                self.weight_known += 1;
                self.weight_nominal |= nominal;
            }
            None => self.weight_unknown += 1,
        }
    }

    fn add_power(&mut self, w: Option<f32>) {
        match w {
            Some(w) => {
                self.power_w += w;
                self.power_known += 1;
            }
            None => self.power_unknown += 1,
        }
    }

    fn merge(&mut self, other: &Totals) {
        self.items += other.items;
        self.weight_kg += other.weight_kg;
        self.weight_known += other.weight_known;
        self.weight_unknown += other.weight_unknown;
        self.weight_nominal |= other.weight_nominal;
        self.power_w += other.power_w;
        self.power_known += other.power_known;
        self.power_unknown += other.power_unknown;
    }

    /// The weight as it should be printed, anywhere it is printed.
    ///
    /// One implementation, because the PDF, the CSV and a plugin's answer disagreeing
    /// about whether a figure is complete is exactly the defect the `≥` exists to
    /// prevent. `"412 kg"` is a claim that everything was counted; `"≥ 412 kg"` is not;
    /// `"≥ 412 kg nominal"` is not, and adds that some of it came off the catalogue.
    pub fn weight_label(&self) -> String {
        if self.weight_known == 0 {
            return "—".into();
        }
        let mut label = String::new();
        if self.weight_unknown > 0 {
            label.push_str("≥ ");
        }
        label.push_str(&format_kg(self.weight_kg));
        label.push_str(" kg");
        if self.weight_nominal {
            label.push_str(" nominal");
        }
        label
    }

    /// The same, for power.
    pub fn power_label(&self) -> String {
        if self.power_known == 0 {
            return "—".into();
        }
        let mut label = String::new();
        if self.power_unknown > 0 {
            label.push_str("≥ ");
        }
        label.push_str(&format_w(self.power_w));
        label.push_str(" W");
        label
    }
}

fn format_kg(kg: f32) -> String {
    if kg >= 100.0 {
        format!("{kg:.0}")
    } else {
        format!("{kg:.1}")
    }
}

fn format_w(w: f32) -> String {
    if w >= 1000.0 {
        format!("{:.2} k", w / 1000.0)
    } else {
        format!("{w:.0}")
    }
}

/// One cell of a table.
///
/// `Blank` rather than an empty string, so a CSV writes nothing and a PDF can draw an
/// em-dash: "this fixture has no unit number" and "this fixture's unit number is the
/// empty string" are different facts and only one of them is ever true.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "value")]
pub enum Cell {
    Text(String),
    Number(f64),
    Blank,
}

impl Cell {
    /// What goes in a CSV field.
    pub fn as_csv(&self) -> String {
        match self {
            Cell::Text(s) => s.clone(),
            Cell::Number(n) => format!("{n}"),
            Cell::Blank => String::new(),
        }
    }
}

/// A column heading, and how to line its cells up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Column {
    pub title: String,
    /// Right-aligned, which is what a number wants and what a name does not.
    pub numeric: bool,
}

impl Column {
    fn text(title: &str) -> Self {
        Column { title: title.into(), numeric: false }
    }
    fn number(title: &str) -> Self {
        Column { title: title.into(), numeric: true }
    }
}

/// A subtotalled section of a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TableGroup {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
    pub totals: Totals,
}

/// A computed table: what the PDF draws, what the CSV writes, and what the
/// `paperwork.tables` RPC answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Table {
    pub kind: TableKind,
    pub columns: Vec<Column>,
    pub groups: Vec<TableGroup>,
    pub totals: Totals,
    /// What this table is not saying, in words, printed under it.
    ///
    /// "8 fixtures on 2 layers this sheet does not show are not counted"; "3 items have
    /// no weight: Titan Tube ×3". A table that accounts for everything carries none,
    /// which is the point — a note appears exactly when it bites.
    pub notes: Vec<String>,
}

/// What to build a table of.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TableRequest {
    pub kind: TableKind,
    #[serde(default)]
    pub grouping: Grouping,
    /// The rows, asked of the rig. `None` takes everything the layers allow.
    #[serde(default)]
    pub rows: Option<SelectionQuery>,
    /// Layers to keep when `rows` is `None`. `None` is all of them.
    #[serde(default)]
    pub layers: Option<Vec<Uuid>>,
}

/// Everything a table is computed from.
///
/// A borrowed struct rather than four arguments, because the RPC, the browser and a
/// test all assemble the same four things and a positional list of slices is a swap
/// waiting to happen.
pub struct Rig<'a> {
    pub fixtures: &'a [Fixture],
    pub fixture_types: &'a [FixtureType],
    pub scene_objects: &'a [SceneObject],
    pub layers: &'a [Layer],
    pub classes: &'a [SceneClass],
}

/// Build one table.
///
/// Pure, total, and the whole of the reporting arithmetic. Called by the
/// `paperwork.tables` RPC, and through it by the browser building a sheet, by a plugin,
/// and by anybody with curl.
pub fn table(request: &TableRequest, rig: &Rig<'_>) -> Table {
    let types: HashMap<Uuid, &FixtureType> =
        rig.fixture_types.iter().map(|t| (t.id, t)).collect();
    let objects: HashMap<Uuid, &SceneObject> =
        rig.scene_objects.iter().map(|o| (o.id, o)).collect();
    let names = Names {
        layers: rig.layers.iter().map(|l| (l.id, l.name.as_str())).collect(),
        classes: rig.classes.iter().map(|c| (c.id, c.name.as_str())).collect(),
    };

    let (fixtures, excluded) = select(request, rig);

    match request.kind {
        TableKind::Counts => counts_table(&fixtures, &types, excluded),
        TableKind::Patch => patch_table(request, &fixtures, &types, &objects, &names, excluded),
        TableKind::Loading | TableKind::Power => {
            loading_table(request, &fixtures, &types, &objects, &names, rig, excluded)
        }
    }
}

/// The fixtures a request asks for, and how many it left out.
///
/// A query wins over the layer list — a table that named its own question should not
/// then be pruned by what the sheet happens to be showing.
fn select<'a>(request: &TableRequest, rig: &'a Rig<'a>) -> (Vec<&'a Fixture>, Excluded) {
    if let Some(query) = &request.rows {
        let picked: HashSet<Uuid> =
            group::evaluate(query, rig.fixtures, None, rig.scene_objects).into_iter().collect();
        let kept: Vec<&Fixture> = rig.fixtures.iter().filter(|f| picked.contains(&f.id)).collect();
        let left = rig.fixtures.len() - kept.len();
        return (kept, Excluded { fixtures: left, layers: 0, by_query: true });
    }

    let Some(layers) = &request.layers else {
        return (rig.fixtures.iter().collect(), Excluded::default());
    };
    let keep: HashSet<Uuid> = layers.iter().copied().collect();
    let kept: Vec<&Fixture> = rig
        .fixtures
        .iter()
        .filter(|f| f.layer.map(|l| keep.contains(&l)).unwrap_or(true))
        .collect();
    let dropped: HashSet<Uuid> = rig
        .fixtures
        .iter()
        .filter_map(|f| f.layer)
        .filter(|l| !keep.contains(l))
        .collect();
    let left = rig.fixtures.len() - kept.len();
    (kept, Excluded { fixtures: left, layers: dropped.len(), by_query: false })
}

/// What a filter took out, so the table can say so.
#[derive(Debug, Clone, Copy, Default)]
struct Excluded {
    fixtures: usize,
    layers: usize,
    by_query: bool,
}

impl Excluded {
    fn note(self) -> Option<String> {
        if self.fixtures == 0 {
            return None;
        }
        let fixtures = plural(self.fixtures, "fixture", "fixtures");
        Some(if self.by_query {
            format!("{fixtures} in the show are outside this table's selection and are not counted")
        } else {
            let layers = plural(self.layers, "layer", "layers");
            format!("{fixtures} on {layers} this sheet does not show are not counted")
        })
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ── The tables themselves ────────────────────────────────────────────────────

fn counts_table(
    fixtures: &[&Fixture],
    types: &HashMap<Uuid, &FixtureType>,
    excluded: Excluded,
) -> Table {
    let mut by_type: Vec<(Uuid, usize)> = Vec::new();
    for fixture in fixtures {
        match by_type.iter_mut().find(|(id, _)| *id == fixture.fixture_type_id) {
            Some((_, n)) => *n += 1,
            None => by_type.push((fixture.fixture_type_id, 1)),
        }
    }
    by_type.sort_by_key(|(id, _)| type_name(types, *id));

    let mut totals = Totals::default();
    let mut rows = Vec::new();
    let mut unknown_weight: Vec<String> = Vec::new();
    let mut unknown_power: Vec<String> = Vec::new();

    for (id, count) in by_type {
        let name = type_name(types, id);
        let physical = types.get(&id).map(|t| &t.physical);
        let unit_kg = physical.and_then(|p| p.weight_kg);
        let unit_w = physical.and_then(|p| p.power_w);
        if unit_kg.is_none() {
            unknown_weight.push(format!("{name} ×{count}"));
        }
        if unit_w.is_none() {
            unknown_power.push(format!("{name} ×{count}"));
        }
        for _ in 0..count {
            totals.add_weight(unit_kg, false);
            totals.add_power(unit_w);
        }
        rows.push(vec![
            Cell::Text(name),
            Cell::Text(manufacturer(types, id)),
            Cell::Number(count as f64),
            cell_kg(unit_kg),
            cell_kg(unit_kg.map(|kg| kg * count as f32)),
            cell_w(unit_w),
            cell_w(unit_w.map(|w| w * count as f32)),
        ]);
    }

    let mut notes: Vec<String> = excluded.note().into_iter().collect();
    notes.extend(unknown_note("weight", &unknown_weight));
    notes.extend(unknown_note("power figure", &unknown_power));

    Table {
        kind: TableKind::Counts,
        columns: vec![
            Column::text("Type"),
            Column::text("Manufacturer"),
            Column::number("Qty"),
            Column::number("Unit kg"),
            Column::number("Total kg"),
            Column::number("Unit W"),
            Column::number("Total W"),
        ],
        groups: vec![TableGroup { name: String::new(), rows, totals }],
        totals,
        notes,
    }
}

fn patch_table(
    request: &TableRequest,
    fixtures: &[&Fixture],
    types: &HashMap<Uuid, &FixtureType>,
    objects: &HashMap<Uuid, &SceneObject>,
    names: &Names<'_>,
    excluded: Excluded,
) -> Table {
    let mut grouped = group_fixtures(request.grouping, fixtures, types, objects, names);
    for section in &mut grouped {
        section.1.sort_by_key(|f| (f.fixture_number.unwrap_or(u32::MAX), f.name.clone()));
    }

    let mut groups = Vec::new();
    let mut totals = Totals::default();
    for (name, members) in grouped {
        let mut section = Totals::default();
        let rows = members
            .iter()
            .map(|fixture| {
                let physical = types.get(&fixture.fixture_type_id).map(|t| &t.physical);
                section.add_weight(physical.and_then(|p| p.weight_kg), false);
                section.add_power(physical.and_then(|p| p.power_w));
                vec![
                    cell_number(fixture.fixture_number),
                    Cell::Text(fixture.name.clone()),
                    Cell::Text(type_name(types, fixture.fixture_type_id)),
                    Cell::Text(mode_of(&fixture.address)),
                    Cell::Text(address_of(&fixture.address)),
                    cell_number(fixture.unit_number),
                    Cell::Text(place_of(fixture, objects)),
                ]
            })
            .collect();
        totals.merge(&section);
        groups.push(TableGroup { name, rows, totals: section });
    }

    Table {
        kind: TableKind::Patch,
        columns: vec![
            Column::number("No."),
            Column::text("Name"),
            Column::text("Type"),
            Column::text("Mode"),
            Column::text("Address"),
            Column::number("Unit"),
            Column::text("Position"),
        ],
        groups,
        totals,
        notes: excluded.note().into_iter().collect(),
    }
}

/// The loading and power tables, which are one table with two sets of columns.
///
/// **The structure is in it.** A group's rows are the lights hanging off it *and* the
/// truss sections that make it up, because the figure a rigger wants is what is on the
/// point. A loading table that counted only the lanterns would be short by the heaviest
/// part of the load, and would look complete while being so.
fn loading_table(
    request: &TableRequest,
    fixtures: &[&Fixture],
    types: &HashMap<Uuid, &FixtureType>,
    objects: &HashMap<Uuid, &SceneObject>,
    names: &Names<'_>,
    rig: &Rig<'_>,
    excluded: Excluded,
) -> Table {
    let power_only = request.kind == TableKind::Power;
    let grouped = group_fixtures(request.grouping, fixtures, types, objects, names);

    // Structure belongs to a group only when the grouping is structural: a truss has no
    // fixture type and no meaningful class, and putting it in a "by type" table would
    // be inventing a row.
    let mut structure: HashMap<String, Vec<&SceneObject>> = HashMap::new();
    if request.grouping == Grouping::Structure && !power_only {
        for object in rig.scene_objects {
            if object.kind == SceneObjectKind::Group {
                continue; // A handle, not a thing: it weighs nothing of its own.
            }
            let key = structure_group_name(object.id, objects);
            structure.entry(key).or_default().push(object);
        }
    }

    let mut names: Vec<String> = grouped.iter().map(|(name, _)| name.clone()).collect();
    for name in structure.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }

    let mut groups = Vec::new();
    let mut totals = Totals::default();
    let mut unknown: Vec<String> = Vec::new();

    for name in names {
        let mut section = Totals::default();
        let mut rows: Vec<Vec<Cell>> = Vec::new();

        if let Some((_, members)) = grouped.iter().find(|(n, _)| *n == name) {
            for fixture in members {
                let physical = types.get(&fixture.fixture_type_id).map(|t| &t.physical);
                let kg = physical.and_then(|p| p.weight_kg);
                let w = physical.and_then(|p| p.power_w);
                section.add_weight(kg, false);
                section.add_power(w);
                if kg.is_none() && !power_only {
                    unknown.push(fixture.name.clone());
                }
                if w.is_none() && power_only {
                    unknown.push(fixture.name.clone());
                }
                rows.push(vec![
                    Cell::Text(fixture.name.clone()),
                    Cell::Text(type_name(types, fixture.fixture_type_id)),
                    Cell::Text("Fixture".into()),
                    if power_only { cell_w(w) } else { cell_kg(kg) },
                ]);
            }
        }

        for object in structure.get(&name).into_iter().flatten() {
            let (kg, nominal) = object_weight(object);
            section.add_weight(kg, nominal);
            if kg.is_none() {
                unknown.push(object.name.clone());
            }
            rows.push(vec![
                Cell::Text(object.name.clone()),
                Cell::Text(
                    object.catalogue.clone().unwrap_or_else(|| format!("{:?}", object.kind)),
                ),
                Cell::Text(if nominal { "Structure (nominal)".into() } else { "Structure".into() }),
                cell_kg(kg),
            ]);
        }

        if rows.is_empty() {
            continue;
        }
        totals.merge(&section);
        groups.push(TableGroup { name, rows, totals: section });
    }

    groups.sort_by(|a, b| a.name.cmp(&b.name));

    let mut notes: Vec<String> = excluded.note().into_iter().collect();
    notes.extend(unknown_note(if power_only { "power figure" } else { "weight" }, &unknown));
    if totals.weight_nominal && !power_only {
        notes.push(
            "Weights marked nominal are the console's catalogue figures for the class of \
             part, not a manufacturer's data. Enter the real weight on the object to \
             replace one."
                .into(),
        );
    }

    Table {
        kind: request.kind,
        columns: vec![
            Column::text("Item"),
            Column::text("Type"),
            Column::text("Kind"),
            if power_only { Column::number("W") } else { Column::number("kg") },
        ],
        groups,
        totals,
        notes,
    }
}

// ── Grouping ─────────────────────────────────────────────────────────────────

/// Layer and class names, which live in their own collections rather than on the
/// object that points at them.
struct Names<'a> {
    layers: HashMap<Uuid, &'a str>,
    classes: HashMap<Uuid, &'a str>,
}

impl Names<'_> {
    /// A name, or the id when nothing holds one — a table never loses a row over a
    /// dangling reference, it just prints an unhelpful heading.
    fn look_up(map: &HashMap<Uuid, &str>, id: Option<Uuid>, fallback: &str) -> String {
        match id {
            Some(id) => {
                map.get(&id).map(|n| (*n).to_string()).unwrap_or_else(|| id.to_string())
            }
            None => fallback.into(),
        }
    }
}

fn group_fixtures<'a>(
    grouping: Grouping,
    fixtures: &[&'a Fixture],
    types: &HashMap<Uuid, &FixtureType>,
    objects: &HashMap<Uuid, &SceneObject>,
    names: &Names<'_>,
) -> Vec<(String, Vec<&'a Fixture>)> {
    let mut out: Vec<(String, Vec<&Fixture>)> = Vec::new();
    for fixture in fixtures {
        let key = match grouping {
            Grouping::Flat => String::new(),
            Grouping::FixtureType => type_name(types, fixture.fixture_type_id),
            Grouping::Layer => Names::look_up(&names.layers, fixture.layer, UNASSIGNED_LAYER),
            Grouping::Class => Names::look_up(&names.classes, fixture.class, UNASSIGNED_CLASS),
            Grouping::Structure => match fixture.parent {
                Some(parent) => structure_group_name(parent, objects),
                None => UNPLACED.into(),
            },
        };
        match out.iter_mut().find(|(name, _)| *name == key) {
            Some((_, members)) => members.push(fixture),
            None => out.push((key, vec![*fixture])),
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A fixture with no parent is in the table under its own heading, and is not dropped.
///
/// 40 kg of unrigged lantern is still 40 kg somebody is carrying, and an MVR import
/// brings plenty: MVR says where a fixture is and never what it is hooked over.
pub const UNPLACED: &str = "Unassigned";
const UNASSIGNED_LAYER: &str = "No layer";
const UNASSIGNED_CLASS: &str = "No class";

/// The nearest `Group` up the chain from an object, or the object itself.
fn structure_group_name(from: Uuid, objects: &HashMap<Uuid, &SceneObject>) -> String {
    let Some(start) = objects.get(&from) else {
        return UNPLACED.into();
    };
    let mut best: &SceneObject = start;
    let mut next: Option<&SceneObject> = Some(start);
    let mut hops = 0;
    while let Some(object) = next {
        if object.kind == SceneObjectKind::Group {
            best = object;
            break;
        }
        hops += 1;
        // The same guard `world_transform` has, for the same reason: a cycle in a
        // parent chain is a corrupt file, not a reason to hang the export.
        if hops > 64 {
            break;
        }
        next = object.parent.and_then(|p| objects.get(&p)).map(|o| *o);
    }
    best.name.clone()
}

// ── Cells ────────────────────────────────────────────────────────────────────

fn object_weight(object: &SceneObject) -> (Option<f32>, bool) {
    if let Some(kg) = object.weight_kg {
        return (Some(kg), false);
    }
    let nominal = object
        .catalogue
        .as_deref()
        .and_then(catalogue::piece)
        .and_then(|piece| piece.weight_kg);
    (nominal, nominal.is_some())
}

fn type_name(types: &HashMap<Uuid, &FixtureType>, id: Uuid) -> String {
    types.get(&id).map(|t| t.name.clone()).unwrap_or_else(|| "Unknown type".into())
}

fn manufacturer(types: &HashMap<Uuid, &FixtureType>, id: Uuid) -> String {
    types.get(&id).map(|t| t.manufacturer.clone()).unwrap_or_default()
}

/// `1/271`, or `1/271 + 2/1` for a fixture whose dimmer is on its own break.
///
/// An OpenHaunt fixture has no universe and prints its node instead of one, rather than
/// a plausible `1/1` it does not have.
pub fn address_of(address: &FixtureAddress) -> String {
    match address {
        FixtureAddress::Dmx { breaks, .. } => breaks
            .iter()
            .map(|b| format!("{}/{}", b.universe, b.address))
            .collect::<Vec<_>>()
            .join(" + "),
        FixtureAddress::OpenHaunt { serial, .. } => format!("node {serial}"),
    }
}

fn mode_of(address: &FixtureAddress) -> String {
    match address {
        FixtureAddress::Dmx { mode, .. } => mode.clone(),
        FixtureAddress::OpenHaunt { .. } => String::new(),
    }
}

fn place_of(fixture: &Fixture, objects: &HashMap<Uuid, &SceneObject>) -> String {
    match fixture.parent.and_then(|p| objects.get(&p)) {
        Some(object) => object.name.clone(),
        None => UNPLACED.into(),
    }
}

fn cell_kg(kg: Option<f32>) -> Cell {
    kg.map(|kg| Cell::Number(kg as f64)).unwrap_or(Cell::Blank)
}

fn cell_w(w: Option<f32>) -> Cell {
    w.map(|w| Cell::Number(w as f64)).unwrap_or(Cell::Blank)
}

fn cell_number(n: Option<u32>) -> Cell {
    n.map(|n| Cell::Number(n as f64)).unwrap_or(Cell::Blank)
}

/// "3 items have no weight: Titan Tube ×8, F34 truss 2 m".
///
/// Names them rather than counting them, up to a point: a reader who is told six things
/// are missing and not which six has been told nothing they can act on.
fn unknown_note(what: &str, missing: &[String]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    let mut names: Vec<String> = missing.to_vec();
    names.sort();
    names.dedup();
    let listed = if names.len() > 8 {
        format!("{}, and {} more", names[..8].join(", "), names.len() - 8)
    } else {
        names.join(", ")
    };
    Some(format!(
        "{} no {what}: {listed}",
        plural(missing.len(), "item has", "items have")
    ))
}

// ── The set a show starts with ───────────────────────────────────────────────

/// The namespace the default sheets' ids are minted in.
///
/// A v5 rather than a fresh v4 each time, so the six sheets have the same ids in every
/// show. Nothing depends on that today; what it buys is a seed that can be run twice
/// without making twelve sheets.
const SHEET_NAMESPACE: Uuid = Uuid::from_bytes([
    0x3b, 0x7d, 0x41, 0x9c, 0x5a, 0x62, 0x4e, 0x18, 0x9d, 0x0b, 0x2f, 0x84, 0x66, 0xc1, 0x07, 0x35,
]);

/// A3 landscape, and the one margin every default sheet is laid out inside.
const MARGIN: f32 = 18.0;
/// Where the title block starts, so nothing is placed under it.
const CONTENT_BOTTOM: f32 = 238.0;
const CONTENT_RIGHT: f32 = 402.0;

fn sheet_id(name: &str) -> Uuid {
    Uuid::new_v5(&SHEET_NAMESPACE, name.as_bytes())
}

fn drafting(lines: LineMode) -> ViewStyle {
    ViewStyle::Drafting { lines, ink: Ink::Mono }
}

/// A picture viewport, lit for paper rather than for a screen.
///
/// The work light is **1.0** and not the rig panel's 0.35. A dark studio is right on a
/// monitor, where the beams are the picture; on A3 it is a near-black rectangle a
/// reader cannot see the truss in and a printer would rather not be asked for. The
/// first export of this sheet came out exactly that way.
fn picture(mode: PictureMode) -> ViewStyle {
    // No cues: a fresh show has none, and a seeded set cannot name ids that do not exist
    // yet. A demo fills these in after it has built its stack — see `demo::kit::posed`.
    ViewStyle::Picture { mode, work_light: 1.0, dpi: 300, cues: Vec::new() }
}

fn viewport(rect: Rect, title: &str, view: ViewPreset, style: ViewStyle) -> SheetBlock {
    labelled(rect, title, view, style, Vec::new())
}

/// The same, with labels on the heads.
///
/// **What to label depends on how big the drawing is**, and the default set says so by
/// giving different sheets different lines. A fixtures plan at the scale it is drawn at
/// has room for a name, a number and an address; the same rig on an overview at 1:200
/// has room for a number and nothing else, and putting three lines on it would produce
/// the very pile of overlapping text the de-collision exists to prevent. So the number
/// alone goes on the general arrangement drawings — it is the smallest thing that lets
/// somebody standing under a bar say which light they are looking at — and the full set
/// goes on the sheet whose whole purpose is to say what each fixture is.
fn labelled(
    rect: Rect,
    title: &str,
    view: ViewPreset,
    style: ViewStyle,
    labels: Vec<LabelField>,
) -> SheetBlock {
    SheetBlock::Viewport(ViewportBlock {
        rect,
        title: title.into(),
        view,
        projection: Projection::Orthographic,
        scale: ViewScale::Fit,
        style,
        layers: None,
        labels,
        scale_bar: true,
        // **Not on by default.** Dimensions are for the sheet somebody takes up a
        // ladder, and that is the Fixtures plan, which turns them on for itself. On a
        // general arrangement drawing at 1:200 a chain per bar is a band of unreadable
        // figures — a 200-fixture festival plan proved it — and the drawing it buries
        // is the one being used to see the shape of the rig.
        dimensions: None,
        orientation_mark: view == ViewPreset::Plan,
    })
}

fn table_block(rect: Rect, title: &str, kind: TableKind, grouping: Grouping) -> SheetBlock {
    SheetBlock::Table(TableBlock {
        rect,
        title: title.into(),
        kind,
        grouping,
        rows: None,
        layers: None,
    })
}

fn sheet(sort_order: i32, name: &str, blocks: Vec<SheetBlock>) -> Sheet {
    Sheet {
        id: sheet_id(name),
        name: name.into(),
        sort_order,
        paper: Paper::A3,
        landscape: true,
        frame: true,
        title_block: true,
        blocks,
    }
}

/// The six sheets a new show is seeded with.
///
/// They are what "a default export creates nice paperwork" means, and they mirror the
/// Vectorworks set this feature was designed against — an overview, the rig, sections,
/// a labelled fixtures plan — with the tables that drawing set did not carry.
///
/// **Rows in the show rather than built-in presets.** A [`super::layout::Layout`] does
/// it the other way, and the consequence of doing it this way is worth knowing: a show
/// made today keeps these sheets exactly as they are, and never picks up a later
/// version's better ones.
///
/// A sheet with nothing to draw is not dropped — a show with no sections placed prints
/// its Sections sheet with an empty frame that says so, because a missing sheet reads
/// as a feature that failed and an empty one reads as a rig that has not been drawn.
pub fn default_sheets() -> Vec<Sheet> {
    let full = Rect::new(MARGIN, MARGIN, CONTENT_RIGHT - MARGIN, CONTENT_BOTTOM - MARGIN);
    let left = Rect::new(MARGIN, MARGIN, 190.0, CONTENT_BOTTOM - MARGIN);
    let right = Rect::new(212.0, MARGIN, 190.0, CONTENT_BOTTOM - MARGIN);

    vec![
        sheet(
            0,
            "Overview",
            vec![
                labelled(
                    left,
                    "Plan",
                    ViewPreset::Plan,
                    drafting(LineMode::Hidden),
                    vec![LabelField::Number],
                ),
                // The one picture in the default set. An axonometric is what somebody
                // who has not been in the room reads first, and it is the sheet a
                // production office puts in front of a client.
                //
                // `Cones` rather than `Real`, and the reason is the paper: a captured
                // line mode clears to **white**, so this draws as grey solids with the
                // beams' coverage over them, which is what the drawing set this was
                // designed against has on its own third sheet. `Real` and `Photoreal`
                // keep their dark ground — they are pictures of light and there is no
                // version of one on white — and are a deliberate choice somebody makes
                // for a mood page rather than the default for a rigging document.
                SheetBlock::Viewport(ViewportBlock {
                    rect: right,
                    title: "Axonometric".into(),
                    view: ViewPreset::ThreeQuarter,
                    projection: Projection::Orthographic,
                    scale: ViewScale::Fit,
                    style: picture(PictureMode::Cones),
                    layers: None,
                    labels: Vec::new(),
                    scale_bar: false,
                    orientation_mark: false,
                    dimensions: None,
                }),
            ],
        ),
        sheet(
            1,
            "Rig",
            vec![
                labelled(
                    Rect::new(MARGIN, MARGIN, CONTENT_RIGHT - MARGIN, 130.0),
                    "Plan",
                    ViewPreset::Plan,
                    drafting(LineMode::Hidden),
                    // The number alone. An address on every head is right on the
                    // Fixtures sheet and is a thicket here: this plan is drawn at
                    // whatever the whole rig fits at, which on a festival is 1:200.
                    vec![LabelField::Number],
                ),
                // The elevation carries the number alone: a head on an elevation is
                // already identified by the bar it is on, and the address is on the
                // plan a hand's width above it.
                labelled(
                    Rect::new(MARGIN, 152.0, 230.0, 86.0),
                    "Front elevation",
                    ViewPreset::Front,
                    drafting(LineMode::Hidden),
                    vec![LabelField::Number],
                ),
            ],
        ),
        sheet(
            2,
            "Sections",
            vec![
                labelled(
                    Rect::new(MARGIN, MARGIN, CONTENT_RIGHT - MARGIN, 105.0),
                    "Section, looking from stage left",
                    ViewPreset::Section,
                    drafting(LineMode::Hidden),
                    vec![LabelField::Number],
                ),
                labelled(
                    Rect::new(MARGIN, 130.0, 230.0, 108.0),
                    "Front elevation",
                    ViewPreset::Front,
                    drafting(LineMode::Hidden),
                    vec![LabelField::Number],
                ),
            ],
        ),
        // The sheet a rigger works off, and the reason wireframe is a mode rather than
        // an oversight: seeing the truss *through* the light is how you check a clamp
        // is where the drawing says it is.
        sheet(
            3,
            "Fixtures",
            vec![SheetBlock::Viewport(ViewportBlock {
                rect: full,
                title: String::new(),
                view: ViewPreset::Plan,
                projection: Projection::Orthographic,
                scale: ViewScale::Fit,
                style: drafting(LineMode::Wireframe),
                layers: None,
                // The sheet whose whole job is to say what each fixture is, so it
                // carries the lot: what it is called, its number, what kind of light it
                // is, and where it is patched.
                labels: vec![
                    LabelField::Name,
                    LabelField::Number,
                    LabelField::TypeShortName,
                    LabelField::Address,
                ],
                scale_bar: true,
                orientation_mark: true,
                dimensions: Some(Dimensions::default()),
            })],
        ),
        // The one sheet that is not a drawing. A production office, a client and a
        // venue all ask "what will it look like", and a plan does not answer that —
        // two rendered views in a lit state do. It is last of the drawings and before
        // the tables, which is where a set puts the thing people flick to.
        sheet(
            4,
            "Beauty",
            vec![
                SheetBlock::Viewport(ViewportBlock {
                    rect: Rect::new(MARGIN, MARGIN, 190.0, 205.0),
                    title: "From the house".into(),
                    view: ViewPreset::Front,
                    // The one perspective view in the default set, and the reason is
                    // the subject: this is a picture of a room somebody will stand in,
                    // and an orthographic photograph of a room is not what it looks
                    // like. It prints NTS, which every picture does anyway.
                    projection: Projection::Perspective,
                    scale: ViewScale::Fit,
                    style: picture(PictureMode::Photoreal),
                    layers: None,
                    labels: Vec::new(),
                    scale_bar: false,
                    orientation_mark: false,
                    dimensions: None,
                }),
                SheetBlock::Viewport(ViewportBlock {
                    rect: Rect::new(212.0, MARGIN, 190.0, 205.0),
                    title: "Three-quarter".into(),
                    view: ViewPreset::ThreeQuarter,
                    projection: Projection::Perspective,
                    scale: ViewScale::Fit,
                    style: picture(PictureMode::Real),
                    layers: None,
                    labels: Vec::new(),
                    scale_bar: false,
                    orientation_mark: false,
                    dimensions: None,
                }),
                SheetBlock::Text(TextBlock {
                    rect: Rect::new(MARGIN, 231.0, 384.0, 12.0),
                    text: "Rendered from the rig as drawn. Beam angles and positions are \
                           the fixtures' own; colour and intensity are the cues named on \
                           each view."
                        .into(),
                    size_mm: 2.4,
                    bold: false,
                }),
            ],
        ),
        sheet(
            5,
            "Patch",
            vec![table_block(full, "Instrument schedule", TableKind::Patch, Grouping::Structure)],
        ),
        sheet(
            6,
            "Loading and power",
            vec![
                table_block(left, "Loading", TableKind::Loading, Grouping::Structure),
                table_block(
                    Rect::new(212.0, MARGIN, 190.0, 105.0),
                    "Power",
                    TableKind::Power,
                    Grouping::Structure,
                ),
                table_block(
                    Rect::new(212.0, 130.0, 190.0, 108.0),
                    "Counts",
                    TableKind::Counts,
                    Grouping::Flat,
                ),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests;
