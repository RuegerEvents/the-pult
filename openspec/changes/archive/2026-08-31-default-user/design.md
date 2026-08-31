## Context

See `proposal.md` — Why. The mechanism this has to fit into is small and already
built, which is what makes the change mostly a question of *where* rather than
*what*:

- `users` is a PERSISTED collection (`crates/pult-schema/src/types/user.rs`) with
  three fields and no seeding anywhere.
- `Operation::is_undoable()` is
  `user_id.is_some() && previous.is_some() && !is_command_path(&self.path)`
  (`crates/pult-schema/src/events/operation.rs:196`), and the History panel reads
  `recent_by_people`, whose SQL filter is `WHERE user_id IS NOT NULL`. So an
  unattributed write is non-undoable and invisible to history with nothing edited
  — the property task 35 leans on for a plugin store's default.
- The frontend's `userId` is a `writable` seeded from `localStorage`, `null`
  before anybody has said, and `beUser(id)` both stores it and tells the socket
  via `showClient().identify(id)` — the backend attributes per connection.
- `load_from_showfile` (`engine/mod.rs:1575`) rebuilds `self.state` from the
  `EntityMeta` registry. `apply_snapshot` (`engine/mod.rs:1609`) replaces state
  wholesale on joining a session and clears `path_clocks`.
- `create_entity` (`engine/mod.rs:1292`) validates, persists and inserts with **no
  existence check**. Nothing stops a second create for an id that is already
  there.

## Goals / Non-Goals

**Goals:**

- Undo works on a show nobody has configured, on the first change, with no
  browser having been asked anything.
- Two stations opening one show hold one default user, not two.
- No new mechanism: no new collection, no new lifecycle, no sync change, no WIT
  change.

**Non-Goals:**

- Anything the proposal's Non-goals section rules out.
- Making the default user a special row the engine guards. It is ordinary; the
  only special thing about it is that its id is known in advance.
- Changing `Authorship::user_id` from `Option`. The engine's own writes stay
  nobody's, which the spec requires and task 35 depends on.

## Decisions

### The id is a fixed constant, not derived from the show

`User::DEFAULT_ID` is a `const Uuid` in `pult-schema`, the same value in every
show.

The requirement is only that every station computes the same id for the same
show. A UUIDv5 over the show's id would satisfy it and would match how a
`PluginDatum` id is derived from `(plugin_id, store, key)` — but it buys nothing
here and costs two things. First, a v5 needs the `Show` row to exist and carry an
id at the moment of seeding, and the load path makes no such promise for an empty
showfile; a constant needs nothing. Second, and the reason this is not a
close call: the **frontend** has to know the id to work as it before the `users`
collection has arrived, and a constant it can hold is the only version of that
which is not a round trip.

Ids are only ever compared within one show, so the same value appearing in every
showfile collides with nothing.

*Alternative rejected:* deriving from `node_id`, which would make one default per
station. That contradicts the argument `user.rs` opens with — identity is chosen
rather than derived from the machine, because a desk and a tablet are one person
— and would put a login-shaped fact into a file that travels.

### The frontend holds the constant too, as `USER_COLOURS` already is

`frontend/src/lib/users.ts` already carries a copy of the schema's colour list,
with the reason written down: "a browser needs one the moment somebody types a
name, and a round trip to ask what colour they should be would be a round trip to
learn a constant." The default user's id is that argument again, and stronger —
the browser needs it *before* the first write, not merely promptly, and adopting
it must not wait for the `users` collection to arrive or there is a window in
which a change is unattributable and therefore permanently un-undoable.

So the constant is written in both places, and a test asserts they agree by
reading `users.ts` from the Rust side. Duplication with a guard, rather than
duplication with a comment.

*Alternative rejected:* serving it from `GET /api/config` beside the station id
and version. Correct-looking, and it reintroduces the window: `identify` would
have to wait on a fetch.

### Seeding happens in the engine, after the load, and only when absent

The seed goes where `load_from_showfile` finishes, checking `self.state` for the
id and doing nothing if it is there. Not in `lib.rs` beside
`seed_outputs_from_flags`, though the spirit is the same ("anything already
configured wins") — the engine owns `state` and the seed has to read it before
deciding.

Only when absent, because `create_entity` does not check. An unconditional create
at every load would rewrite the row on every start and replicate "Operator" over
a name somebody chose.

`apply_snapshot` needs no seed of its own: a snapshot comes from a station that
has loaded the show, so it already carries a default user. Re-seeding after a
snapshot would only reintroduce the clobber this decision exists to avoid.

### The seed is unattributed, like the engine's other writes

Nobody asked for it, so `Authorship::none()`. It does not appear in history and
cannot be undone, which is right: an operator pressing Ctrl-Z on a fresh show
should reach their own first change, not the console's act of inventing them.

### Sign out becomes a fall back, and `beUser(null)` stops being reachable

The store keeps `beUser(id: string | null)` — the null path is what clears
`localStorage`, and clearing it is exactly what "stop working as this person"
means. What changes is that nothing calls it with `null` and then leaves the
client as nobody: the UserBar's Sign out clears the stored identity *and* adopts
the default, in that order, so the next visit to this browser starts as the
default rather than as the person who signed out.

### The two "say who you are" messages go away, and one nudge replaces them

`undo.ts`'s toast and `HistoryPanel.svelte`'s empty state describe a state that
can no longer be reached, so they are deleted rather than reworded. The UserBar
gains the opposite thing: while the client is working as the default *and*
nobody has chosen, the chip shows the operator's name with a visible affordance
to identify. The existing `--live` bordered "unknown" styling is the right
vocabulary and stays — what it means changes from "nothing can be taken back" to
"this is everyone's undo history until you say who you are".

Once somebody has chosen — including choosing the default deliberately — the
nudge is gone. That distinction is client state, not show data: a `localStorage`
flag beside `pult.user`, since whether *this browser* has been told is not a fact
about the show.

## Risks / Trade-offs

- **Two operators silently share one undo history.** → This is the cost of the
  default and the reason the nudge is a requirement rather than a nicety. It is
  strictly better than today, where neither of them can undo at all, and the
  affordance to fix it is one click in the top bar.

- **A station seeding offline can revert a rename.** A station whose copy of the
  show predates the default user, loading while disconnected, creates it; if
  another station had meanwhile renamed it, the two writes are concurrent and the
  tie is broken by the sync layer rather than by intent. → Bounded and
  self-healing: the ids match, so the worst case is one row whose name reverts to
  "Operator" and somebody renames it again. There is no duplicate user, no split
  undo history, and no lost operation. Engineering around it would mean making
  the seed wait for a session that may never come, which breaks the headless case
  the seed exists for.

- **Deleting the default user leaves the show without one until the next load.**
  → Accepted, and specified. Guarding the row would make it special, and the
  point is that it is not. A client working as a deleted user is the same
  already-possible state as a client working as any deleted user.

- **The constant lives in two languages.** → The guard test. If it drifts, the
  test says so; without the test this is the sort of duplication that is correct
  for a year and then quietly is not.

## Impact on pult-schema, the WIT contract and the sync protocol

- **pult-schema**: two constants (`User::DEFAULT_ID`, the default name) and no
  field changes. No migration, no `ShowState` work, no codegen consequences
  beyond running `cargo run -p pult-codegen -- generate` for form's sake — the
  `users` collection already exists, already persists and already replicates.
- **WIT contract**: none. A plugin's view of users is whatever introspection
  already reports about the `users` collection, and that collection's shape is
  unchanged.
- **Sync protocol**: none. The default user is an ordinary PERSISTED row and
  travels by the machinery already carrying every other one. The only sync-shaped
  question is the concurrent-seed case above, which the existing per-path conflict
  resolution answers without being told anything new.

## Migration Plan

There is nothing to migrate and nothing to roll back to. An existing showfile
gains a row on its next open; a station running an older build against a showfile
that has one sees an ordinary user it did not create, which is a state it already
handles, since any peer could have created one. Downgrading is safe.

Operations already written with `user_id: None` stay non-undoable. That is
history rather than a defect, and the spec says so.
