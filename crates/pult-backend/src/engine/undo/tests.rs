//! What undo does, as arithmetic over a log.
//!
//! These build operation logs by hand rather than driving the engine, because the
//! interesting questions — which change is next, what is the inverse of a delete,
//! whose history is whose — are questions about a list, and a list is a great deal
//! easier to be exact about than a running console.

use chrono::Utc;
use pult_schema::{
    events::operation::{NodeId, Operation, VectorClock},
    lifecycle::Lifecycle,
    path::{Path, PathSegment},
};
use uuid::Uuid;

use super::*;

fn path(segments: &[&str]) -> Path {
    segments.iter().map(|s| PathSegment::Key((*s).into())).collect()
}

fn entity_path(table: &str, id: Uuid, tail: &str) -> Path {
    vec![
        PathSegment::Key(table.into()),
        PathSegment::Id(id),
        PathSegment::Key(tail.into()),
    ]
}

/// One write, by `user`, over `previous`.
fn op(user: Option<Uuid>, path: Path, value: serde_json::Value, previous: Option<serde_json::Value>) -> Operation {
    Operation {
        id: Uuid::new_v4(),
        node_id: NodeId(Uuid::nil()),
        seq: 0,
        clock: VectorClock::default(),
        lifecycle: Lifecycle::Persisted,
        path,
        value,
        timestamp: Utc::now(),
        user_id: user,
        previous,
        undoes: None,
        gesture: None,
    }
}

/// The one operation of an ordinary run, for the tests that are about a single
/// write. A run of any other length is a failure those tests want to hear about.
fn only<'a>(run: Vec<&'a Operation>) -> Option<&'a Operation> {
    assert!(run.len() <= 1, "expected a single write, got {}", run.len());
    run.into_iter().next()
}

/// The log is read newest-first, as `oplog::recent_by_people` returns it.
fn newest_first(ops: Vec<Operation>) -> Vec<Operation> {
    ops.into_iter().rev().collect()
}

// ── What can be taken back ────────────────────────────────────────────────────

#[test]
fn an_ordinary_write_goes_back_to_what_was_there() {
    let sam = Uuid::new_v4();
    let renamed = op(
        Some(sam),
        path(&["show", "name"]),
        serde_json::json!("Macbeth"),
        Some(serde_json::json!("My Show")),
    );

    let inverse = inverse_of(&renamed).expect("undoable");
    assert_eq!(inverse.path, path(&["show", "name"]));
    assert_eq!(inverse.value, serde_json::json!("My Show"));
}

/// The engine's own writes have no author, so there is nobody to take them back for
/// and nothing that would mean. A fade advancing is not a change somebody made.
#[test]
fn a_write_nobody_asked_for_cannot_be_undone() {
    let engine_write = op(None, path(&["show", "name"]), serde_json::json!("x"), Some(serde_json::json!("y")));
    assert!(!engine_write.is_undoable());
    assert!(inverse_of(&engine_write).is_none());
}

/// An operation logged before undo existed captured nothing to go back to. Refusing
/// is the honest answer; guessing at a previous value would be worse than saying no.
#[test]
fn a_write_with_nothing_captured_cannot_be_undone() {
    let sam = Uuid::new_v4();
    let old = op(Some(sam), path(&["show", "name"]), serde_json::json!("x"), None);
    assert!(!old.is_undoable());
}

/// `Some(Null)` and `None` are different, and the difference matters: a field that
/// was empty should be emptied again, which is not the same as a field nobody
/// recorded.
#[test]
fn a_field_that_was_empty_is_emptied_again() {
    let sam = Uuid::new_v4();
    let filled = op(
        Some(sam),
        path(&["show", "editing_cue"]),
        serde_json::json!("abc"),
        Some(serde_json::Value::Null),
    );
    assert!(filled.is_undoable());
    assert_eq!(inverse_of(&filled).unwrap().value, serde_json::Value::Null);
}

// ── Creates and deletes ───────────────────────────────────────────────────────

/// Adding something is undone by taking it away — not by writing a previous value,
/// because there was not one. The id comes out of what was created.
#[test]
fn undoing_a_create_deletes_what_was_created() {
    let sam = Uuid::new_v4();
    let id = Uuid::new_v4();
    let created = op(
        Some(sam),
        path(&["fixtures", "__create"]),
        serde_json::json!({ "id": id, "name": "Spot 1" }),
        Some(serde_json::Value::Null),
    );

    let inverse = inverse_of(&created).expect("undoable");
    assert_eq!(inverse.path, entity_path("fixtures", id, "__delete"));
}

/// And removing something is undone by putting it back whole, which is why a delete
/// captures the entity rather than a flag: nothing else knows what was in it.
#[test]
fn undoing_a_delete_puts_the_whole_entity_back() {
    let sam = Uuid::new_v4();
    let id = Uuid::new_v4();
    let entity = serde_json::json!({ "id": id, "name": "Spot 1", "fixture_type_id": "t" });
    let deleted = op(
        Some(sam),
        entity_path("fixtures", id, "__delete"),
        serde_json::Value::Null,
        Some(entity.clone()),
    );

    let inverse = inverse_of(&deleted).expect("undoable");
    assert_eq!(inverse.path, path(&["fixtures", "__create"]));
    assert_eq!(inverse.value, entity);
}

#[test]
fn a_create_with_no_id_leaves_nothing_to_delete() {
    let sam = Uuid::new_v4();
    let created = op(
        Some(sam),
        path(&["fixtures", "__create"]),
        serde_json::json!({ "name": "nameless" }),
        Some(serde_json::Value::Null),
    );
    assert!(inverse_of(&created).is_none());
}

#[test]
fn a_delete_that_captured_nothing_leaves_nothing_to_restore() {
    let sam = Uuid::new_v4();
    let id = Uuid::new_v4();
    let deleted = op(
        Some(sam),
        entity_path("fixtures", id, "__delete"),
        serde_json::Value::Null,
        Some(serde_json::Value::Null),
    );
    assert!(inverse_of(&deleted).is_none());
}

// ── Which change is next ──────────────────────────────────────────────────────

#[test]
fn the_most_recent_change_is_the_one_taken_back() {
    let sam = Uuid::new_v4();
    let first = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("")));
    let second = op(Some(sam), path(&["show", "name"]), serde_json::json!("B"), Some(serde_json::json!("A")));
    let log = newest_first(vec![first.clone(), second.clone()]);

    assert_eq!(only(next_to_undo(&log, sam)).unwrap().id, second.id);
}

/// A run of undos steps back through history rather than fighting over the same
/// change — which it would if an undo did not count against what it named.
#[test]
fn undoing_twice_steps_back_twice() {
    let sam = Uuid::new_v4();
    let first = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("")));
    let second = op(Some(sam), path(&["show", "name"]), serde_json::json!("B"), Some(serde_json::json!("A")));

    let mut undo_of_second = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("B")));
    undo_of_second.undoes = Some(second.id);

    let log = newest_first(vec![first.clone(), second.clone(), undo_of_second]);
    assert_eq!(only(next_to_undo(&log, sam)).unwrap().id, first.id);
}

/// Two operators each have their own history, so one pressing Ctrl-Z can never take
/// back work the other is in the middle of. This is the whole reason undo is
/// per-user rather than per-show.
#[test]
fn one_operator_cannot_take_back_anothers_work() {
    let sam = Uuid::new_v4();
    let alex = Uuid::new_v4();
    let sams = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("")));
    let alexs = op(Some(alex), path(&["show", "name"]), serde_json::json!("B"), Some(serde_json::json!("A")));
    let log = newest_first(vec![sams.clone(), alexs.clone()]);

    assert_eq!(only(next_to_undo(&log, sam)).unwrap().id, sams.id, "Sam takes back Sam's");
    assert_eq!(only(next_to_undo(&log, alex)).unwrap().id, alexs.id, "and Alex takes back Alex's");
}

/// Nor does one operator's undo cancel the other's change out of the running.
#[test]
fn an_undo_by_one_operator_does_not_count_against_anothers() {
    let sam = Uuid::new_v4();
    let alex = Uuid::new_v4();
    let sams = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("")));

    let mut alexs_undo = op(Some(alex), path(&["show", "name"]), serde_json::json!(""), Some(serde_json::json!("A")));
    alexs_undo.undoes = Some(sams.id);

    let log = newest_first(vec![sams.clone(), alexs_undo]);
    assert_eq!(only(next_to_undo(&log, sam)).unwrap().id, sams.id, "still Sam's to take back");
}

#[test]
fn with_nothing_of_your_own_there_is_nothing_to_undo() {
    let sam = Uuid::new_v4();
    let engine = op(None, path(&["fixtures"]), serde_json::json!([]), Some(serde_json::json!([])));
    assert!(next_to_undo(&[engine], sam).is_empty());
    assert!(next_to_undo(&[], sam).is_empty());
}

// ── Redo ──────────────────────────────────────────────────────────────────────

/// Redo is undoing an undo, which is why one mechanism covers both.
#[test]
fn redo_takes_back_the_undo() {
    let sam = Uuid::new_v4();
    let change = op(Some(sam), path(&["show", "name"]), serde_json::json!("B"), Some(serde_json::json!("A")));
    let mut undo = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("B")));
    undo.undoes = Some(change.id);

    let log = newest_first(vec![change, undo.clone()]);
    let next = only(next_to_redo(&log, sam)).expect("something to redo");
    assert_eq!(next.id, undo.id);
    assert_eq!(inverse_of(next).unwrap().value, serde_json::json!("B"), "back to what the undo took away");
}

#[test]
fn there_is_nothing_to_redo_until_something_is_undone() {
    let sam = Uuid::new_v4();
    let change = op(Some(sam), path(&["show", "name"]), serde_json::json!("B"), Some(serde_json::json!("A")));
    assert!(next_to_redo(&[change], sam).is_empty());
}

/// A run of undos and redos, driven through the same functions the engine uses.
///
/// Written as a simulation rather than a hand-built log because the interesting bugs
/// are in the *sequence* — a redo that is itself redoable, an undo that finds nothing
/// after a redo — and a log written out by hand is exactly where those hide.
struct History {
    log: Vec<Operation>,
    user: Uuid,
}

impl History {
    fn new(user: Uuid) -> Self {
        History { log: Vec::new(), user }
    }

    /// Newest first, as `oplog::recent_by_people` returns it.
    fn push(&mut self, op: Operation) {
        self.log.insert(0, op);
    }

    /// Somebody edits the show. `was` is what the field held before.
    fn change(&mut self, was: &str, now: &str) {
        let user = self.user;
        self.push(op(
            Some(user),
            path(&["show", "name"]),
            serde_json::json!(now),
            Some(serde_json::json!(was)),
        ));
    }

    /// Press undo (or redo), the way the engine does it: reverse a whole gesture,
    /// one write per path, all pointing at the gesture they reverse and sharing a
    /// gesture of their own. Returns what the field reads afterwards, or None when
    /// there was nothing to do.
    fn press(&mut self, redo: bool) -> Option<String> {
        let run = if redo {
            next_to_redo(&self.log, self.user)
        } else {
            next_to_undo(&self.log, self.user)
        };
        let reverses = gesture_key(run.first()?);
        let inverses = inverses_of_run(&run);

        let user = self.user;
        let mine = Uuid::new_v4();
        let mut last = None;
        for inverse in inverses {
            let showing = self.showing();
            let mut written = op(
                Some(user),
                inverse.path,
                inverse.value.clone(),
                Some(serde_json::json!(showing)),
            );
            written.undoes = Some(reverses);
            written.gesture = Some(mine);
            self.push(written);
            last = Some(inverse.value.as_str().unwrap_or_default().to_string());
        }
        last
    }

    /// A drag: several writes to one path, all one gesture.
    fn drag(&mut self, was: &str, through: &[&str]) {
        let user = self.user;
        let mine = Uuid::new_v4();
        let mut before = was.to_string();
        for step in through {
            let mut written = op(
                Some(user),
                path(&["show", "name"]),
                serde_json::json!(step),
                Some(serde_json::json!(before)),
            );
            written.gesture = Some(mine);
            self.push(written);
            before = (*step).to_string();
        }
    }

    /// What the field currently reads: the value of the newest operation.
    fn showing(&self) -> String {
        self.log
            .first()
            .and_then(|op| op.value.as_str())
            .unwrap_or_default()
            .to_string()
    }
}

/// The sequence an operator actually performs, and what they should see.
#[test]
fn undoing_and_redoing_walks_the_history_both_ways() {
    let mut h = History::new(Uuid::new_v4());
    h.change("", "A");
    h.change("A", "B");
    h.change("B", "C");
    assert_eq!(h.showing(), "C");

    assert_eq!(h.press(false).as_deref(), Some("B"), "undo once");
    assert_eq!(h.press(false).as_deref(), Some("A"), "undo twice");
    assert_eq!(h.press(false).as_deref(), Some(""), "undo three times");
    assert_eq!(h.press(false), None, "and there is nothing left to take back");

    assert_eq!(h.press(true).as_deref(), Some("A"), "redo once");
    assert_eq!(h.press(true).as_deref(), Some("B"), "redo twice");
    assert_eq!(h.press(true).as_deref(), Some("C"), "redo three times");
    assert_eq!(h.press(true), None, "and there is nothing left to put back");
}

/// Undo, redo, undo — the case that broke the first version of this, where a redo
/// was itself a candidate for redoing and pressing redo twice quietly undid.
#[test]
fn a_redo_is_not_itself_redoable() {
    let mut h = History::new(Uuid::new_v4());
    h.change("", "A");
    h.change("A", "B");

    assert_eq!(h.press(false).as_deref(), Some("A"), "undo");
    assert_eq!(h.press(true).as_deref(), Some("B"), "redo");
    assert_eq!(h.press(true), None, "nothing more to put back");
    assert_eq!(h.press(false).as_deref(), Some("A"), "and undo works again");
}

/// Doing something new after undoing ends the redo chain: the new work is now the
/// thing to take back, and the branch that was undone is not coming back.
#[test]
fn a_fresh_change_after_undoing_leaves_nothing_to_redo() {
    let mut h = History::new(Uuid::new_v4());
    h.change("", "A");
    h.change("A", "B");
    h.press(false);

    h.change("A", "Z");
    assert_eq!(h.press(true), None, "the undone branch is gone");
    assert_eq!(h.press(false).as_deref(), Some("A"), "and undo takes back the new work");
}

// ── Gestures ──────────────────────────────────────────────────────────────────

/// The point of the whole thing: one drag costs one Ctrl-Z, and lands on the value
/// from before the drag rather than one frame back into it.
#[test]
fn a_drag_is_taken_back_in_one_press() {
    let mut h = History::new(Uuid::new_v4());
    h.change("", "A");
    h.drag("A", &["B", "C", "D", "E"]);
    assert_eq!(h.showing(), "E");

    assert_eq!(h.press(false).as_deref(), Some("A"), "the whole drag, at once");
    assert_eq!(h.press(false).as_deref(), Some(""), "and the change before it");
}

/// Four hundred writes should not cost four hundred rows to reverse, or the log
/// would grow faster the more of it you took back.
#[test]
fn reversing_a_drag_writes_one_row_per_path() {
    let sam = Uuid::new_v4();
    let mine = Uuid::new_v4();
    let mut log = Vec::new();
    let mut before = String::new();
    for step in ["A", "B", "C", "D"] {
        let mut written = op(
            Some(sam),
            path(&["show", "name"]),
            serde_json::json!(step),
            Some(serde_json::json!(before)),
        );
        written.gesture = Some(mine);
        log.insert(0, written);
        before = step.to_string();
    }

    let run = next_to_undo(&log, sam);
    assert_eq!(run.len(), 4, "the gesture is all four writes");
    let inverses = inverses_of_run(&run);
    assert_eq!(inverses.len(), 1, "and one write puts it back");
    assert_eq!(inverses[0].value, serde_json::json!(""), "to before the drag began");
}

/// A gesture across a selection touches many paths, and all of them come back.
#[test]
fn a_gesture_across_a_selection_puts_every_fixture_back() {
    let sam = Uuid::new_v4();
    let mine = Uuid::new_v4();
    let mut log = Vec::new();
    for light in ["spot1", "spot2", "spot3"] {
        for level in [0.2, 0.7] {
            let mut written = op(
                Some(sam),
                path(&["programmer_values", light, "value"]),
                serde_json::json!(level),
                Some(serde_json::json!(if level == 0.2 { 0.0 } else { 0.2 })),
            );
            written.gesture = Some(mine);
            log.insert(0, written);
        }
    }

    let inverses = inverses_of_run(&next_to_undo(&log, sam));
    assert_eq!(inverses.len(), 3, "one per fixture, not one per pointer event");
    for inverse in &inverses {
        assert_eq!(inverse.value, serde_json::json!(0.0), "back to where the drag started");
    }
}

/// A gesture that made something and then named it needs only the delete. Putting
/// the old name back into a fixture that is about to go is a row in the log
/// describing a state nobody will ever see.
#[test]
fn a_field_written_into_something_the_gesture_made_needs_no_undoing() {
    let sam = Uuid::new_v4();
    let mine = Uuid::new_v4();
    let id = Uuid::new_v4();

    let mut created = op(
        Some(sam),
        path(&["fixtures", "__create"]),
        serde_json::json!({ "id": id, "name": "Untitled" }),
        Some(serde_json::Value::Null),
    );
    created.gesture = Some(mine);
    let mut named = op(
        Some(sam),
        entity_path("fixtures", id, "name"),
        serde_json::json!("Spot 4"),
        Some(serde_json::json!("Untitled")),
    );
    named.gesture = Some(mine);

    let log = newest_first(vec![created, named]);
    let inverses = inverses_of_run(&next_to_undo(&log, sam));

    assert_eq!(inverses.len(), 1, "the delete covers the rename");
    assert_eq!(inverses[0].path, entity_path("fixtures", id, "__delete"));
}

/// Two fixtures added in one gesture are two writes to the same `fixtures/__create`
/// path. Keyed by the path they would collapse into one, which deletes the first and
/// leaves the second standing — a half-undone gesture, and the worst of both.
#[test]
fn two_things_made_in_one_gesture_both_go_away() {
    let sam = Uuid::new_v4();
    let mine = Uuid::new_v4();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();

    let log = newest_first(
        [first, second]
            .iter()
            .map(|id| {
                let mut created = op(
                    Some(sam),
                    path(&["fixtures", "__create"]),
                    serde_json::json!({ "id": id, "name": "Spot" }),
                    Some(serde_json::Value::Null),
                );
                created.gesture = Some(mine);
                created
            })
            .collect(),
    );

    let inverses = inverses_of_run(&next_to_undo(&log, sam));
    let deleted: Vec<_> = inverses.iter().map(|i| i.path.clone()).collect();
    assert_eq!(deleted.len(), 2, "both, not one");
    assert!(deleted.contains(&entity_path("fixtures", first, "__delete")));
    assert!(deleted.contains(&entity_path("fixtures", second, "__delete")));
}

/// A gesture that moved a fixture and made another has to be unpicked in the order
/// it was tied: the move first, the delete last. The other way round, the move
/// writes into a hole.
#[test]
fn a_gesture_is_unpicked_newest_first() {
    let sam = Uuid::new_v4();
    let mine = Uuid::new_v4();
    let moved = Uuid::new_v4();
    let made = Uuid::new_v4();

    let mut nudged = op(
        Some(sam),
        entity_path("fixtures", moved, "name"),
        serde_json::json!("Spot 9"),
        Some(serde_json::json!("Spot 1")),
    );
    nudged.gesture = Some(mine);
    let mut created = op(
        Some(sam),
        path(&["fixtures", "__create"]),
        serde_json::json!({ "id": made, "name": "Spot 4" }),
        Some(serde_json::Value::Null),
    );
    created.gesture = Some(mine);

    // Created first, renamed after — so the delete is the older of the two and has
    // to come last on the way back out.
    let log = newest_first(vec![created, nudged]);
    let inverses = inverses_of_run(&next_to_undo(&log, sam));

    assert_eq!(inverses.len(), 2);
    assert_eq!(inverses[0].value, serde_json::json!("Spot 1"), "the name goes back first");
    assert_eq!(inverses[1].path, entity_path("fixtures", made, "__delete"), "then it goes away");
}

/// A drag put back is a drag again: redo is one press too, not one per path.
#[test]
fn a_drag_is_put_back_in_one_press() {
    let mut h = History::new(Uuid::new_v4());
    h.change("", "A");
    h.drag("A", &["B", "C", "D"]);

    assert_eq!(h.press(false).as_deref(), Some("A"), "undo the drag");
    assert_eq!(h.press(true).as_deref(), Some("D"), "and it comes back where it ended");
    assert_eq!(h.press(true), None, "with nothing left to put back");
}

/// The case that made gestures need a key of their own rather than an operation id.
/// Reversing a drag writes one row against four, so three of the drag's operations
/// have nothing pointing at them — and if undo looked at operations it would find
/// one of those three still standing and take the same drag back again.
#[test]
fn a_reversed_drag_stays_reversed() {
    let mut h = History::new(Uuid::new_v4());
    h.change("", "A");
    h.drag("A", &["B", "C", "D"]);

    assert_eq!(h.press(false).as_deref(), Some("A"), "undo the drag");
    assert_eq!(h.press(false).as_deref(), Some(""), "the next press moves on");
}

/// A gesture id from one operator is not a way into another's history.
#[test]
fn a_gesture_belongs_to_whoever_made_it() {
    let sam = Uuid::new_v4();
    let alex = Uuid::new_v4();
    let mine = Uuid::new_v4();

    let mut sams = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("")));
    sams.gesture = Some(mine);
    let mut alexs = op(Some(alex), path(&["show", "name"]), serde_json::json!("B"), Some(serde_json::json!("A")));
    alexs.gesture = Some(mine);

    let log = newest_first(vec![sams.clone(), alexs.clone()]);
    let run = next_to_undo(&log, sam);
    assert_eq!(run.len(), 1, "Sam takes back Sam's half of it");
    assert_eq!(run[0].id, sams.id);
}

/// A write with no gesture is a gesture of one, so nothing about the ordinary case
/// changed — including for rows written before gestures existed.
#[test]
fn a_write_with_no_gesture_stands_alone() {
    let sam = Uuid::new_v4();
    let first = op(Some(sam), path(&["show", "name"]), serde_json::json!("A"), Some(serde_json::json!("")));
    let second = op(Some(sam), path(&["show", "name"]), serde_json::json!("B"), Some(serde_json::json!("A")));
    let log = newest_first(vec![first, second.clone()]);

    let run = next_to_undo(&log, sam);
    assert_eq!(run.len(), 1);
    assert_eq!(run[0].id, second.id);
    assert_eq!(gesture_key(run[0]), second.id, "keyed by itself");
}
