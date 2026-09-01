## Why

Task 30 made a selection a question about the rig instead of a list of ids, and
then had nowhere to keep one. `SelectionQuery` lives in
`frontend/src/lib/selection.ts`, which says so in a comment: "If saved groups
ever become show data, the types move to `pult-schema` then and the backend gets
an evaluator beside this one." Everything an operator builds — "every mover on
the downstage truss", "the four specials" — is thrown away when the browser tab
closes, and no plugin, command line or peer station can name it.

A group is show data. Two operators at two consoles hold different fixtures, but
they work on the same rig, and "the downstage movers" is a fact about that rig
rather than about whoever is looking at it.

## What Changes

- **A PERSISTED `groups` collection** in `pult-schema`, each row a name plus a
  `SelectionQuery` — the query, not the ids it currently resolves to. That is
  the whole point of task 30 and the reason a group survives a re-patch.
- **The query types move to `pult-schema`.** `Term`, `Combine`, `Clause`,
  `Order` and `SelectionQuery` become Rust types with `#[derive(TS)]`, and
  `frontend/src/lib/selection.ts` imports them from `./generated/` instead of
  declaring them. **BREAKING** for nothing shipped: the wire shape of a query is
  new, and no showfile holds one yet.
- **A Rust evaluator** in `pult-schema` beside the types: the same pure function
  over a rig that `evaluate()` already is. The frontend keeps its own, because a
  cone being dragged has to re-evaluate per frame and cannot afford a round trip.
- **A shared corpus** of `(rig, query, expected ids)` cases, checked in as JSON
  and read by both a Rust test and a vitest test. Two evaluators is the cost of
  this change; a corpus that fails on either side is how it stays paid.
- **A station RPC to resolve a group to fixture ids**, so the command line and
  any plugin can address one. Adding it to `api/rpcs.rs` makes it callable from
  the WebSocket, callable from plugins, and visible to introspection at once.
- **The command line learns `group`**: `group 3`, `group "movers" at 50`, and
  `group 3` as a term beside `fixture 1 thru 5`. The backlog guessed this came
  free from introspection; it does not. `fixture` is a keyword in
  `plugins/command-line/core/src/parse.rs` and `group` has to be one too.
  Generic entity addressing does give `group 3 name "Movers"` and
  `create group` for free.
- **The frontend saves and recalls**: the selection panel gets *Save as group…*,
  and recalling a group sets the query store to the group's query, so the
  recalled selection stays live rather than becoming a list.

## Non-goals

- **Groups do not replace the live selection.** What is selected right now stays
  a Svelte store, unreplicated and unpersisted. Two operators sharing one
  selection is a different feature and a worse one.
- **No `InGroup` term.** A query referencing another group would give
  composition and would need cycle detection, an evaluation order, and an answer
  for what deleting a referenced group does. Recall-then-refine already covers
  the workflow; liveness through a reference can be a later change.
- **No boolean tree.** The clause list stays as task 30 built it. This change
  moves the type, it does not grow it.
- **Groups are not a playback or masters concept.** No group-scoped submaster,
  no group intensity. This change stores a question and answers it.
- **No migration of existing shows.** There is nothing to migrate; shows without
  a `groups` table simply have none.

## Capabilities

### New Capabilities
- `rig/groups`: saved fixture groups as show data — the query language as a
  specified, replicated shape; what a group is; how a group is resolved to
  fixtures, by the frontend, by the backend, and by a plugin; and what happens
  to a group when the rig underneath it changes.

### Modified Capabilities

None. No existing capability's requirements change: `plugins/*` gains a callable
RPC but no new rule, and `users/identity` and `history/retention` are untouched
(a group edit is an ordinary attributed, undoable write, which is already what
they require of every PERSISTED row).

## Impact

- **`crates/pult-schema`** — a new `types/group.rs` holding `Group` and the query
  types, an evaluator, and the shared corpus test. `types/mod.rs` gains the
  module.
- **`crates/pult-backend`** — `api/rpcs.rs` gains one RPC and `LocalRpcDeps`
  gains a way to read the rig, which today it has no reason to hold. Nothing in
  `engine/` changes: a new collection needs no edit outside `pult-schema`.
- **`frontend/`** — `selection.ts` loses its type declarations and keeps its
  evaluator; the selection panel gains save/recall; a groups list has to live
  somewhere (its own panel, or a section of the existing selection panel — a
  design question).
- **`plugins/command-line`** — the grammar gains one keyword; `core` gains a
  variant on `Command::Select` and completion for it.
- **Codegen** — `cargo run -p pult-codegen -- generate` after the schema lands,
  which is what puts `SelectionQuery` into `frontend/src/lib/generated/`.
- **Docs** — `docs/ROADMAP.md` gains a task; `openspec/BACKLOG.md`'s
  `fixture-groups` entry becomes a pointer to the archived change.
