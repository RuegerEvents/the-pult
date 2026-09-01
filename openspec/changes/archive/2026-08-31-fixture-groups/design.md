## Context

See proposal.md — Why. What matters for the approach is where things are today:

- `SelectionQuery` and `evaluate()` are TypeScript, in
  `frontend/src/lib/selection.ts`. The types are structurally simple: a list of
  clauses, each a `Combine` and a `Term`, plus an `Order`.
- The live selection is a Svelte store (`stores/selection.ts`) holding the
  query, with a separate `handOrder` store holding the order an operator dragged
  the panel into. `selection` is derived from `(query, fixtures, handOrder)`.
- A plugin surface can already move the selection: `ExecResponse.effects` may
  carry `{ selection: { fixtureIds } }`, which `ConsoleSurface.svelte` and
  `BarSurface.svelte` turn into `setQuery(idsQuery(ids))` — a *frozen* list.
- A plugin's `host::call` goes to `api/rpcs.rs` if the method is in
  `LOCAL_RPCS`, and to `engine.call` otherwise. `engine.call` only ever means
  `<table>.<command>`: it mutates an entity and writes an operation.
- There is precedent for a Rust implementation mirrored in TypeScript:
  `frontend/src/lib/effects.ts` mirrors `crates/pult-backend/src/model/effects.rs`,
  and both assert the same numeric table (`effects.test.ts`, "the numeric table,
  again").

## Goals / Non-Goals

**Goals**

- One meaning for a query, provable rather than asserted.
- Resolving a group is a read, and stays a read — no oplog entry, no operation.
- The wire shape of a query after codegen is byte-identical to what the
  frontend uses today, so no panel changes because a type moved.

**Non-Goals**

- Removing the TypeScript evaluator. It stays, and the design is about keeping
  it honest rather than about deleting it.
- Changing how the live selection is stored or shared.

## Decisions

### The evaluator is written twice, and a checked-in corpus is the price

The frontend re-evaluates the query on every change to `(query, fixtures,
handOrder)`. Dragging a box or a cone across the rig changes the query per
frame, so evaluation is on the interaction path. A round trip to the station per
frame is not an option, and neither is caching a result that is supposed to
follow the rig.

So: `evaluate()` exists in Rust (in `pult-schema`, beside the types) and in
TypeScript, and a corpus of `(rig, query, previous, expected ids)` cases lives in
one JSON file that both test suites read. A case that disagrees fails on the side
that is wrong, at the commit that made it wrong.

*Alternatives considered.* A single Rust evaluator called over the WebSocket —
rejected for the drag. Compiling `pult-schema` to WASM for the browser — a large
new build dependency for one pure function, and it would put a second copy of the
schema in the bundle. Duplicating the cases in both test files — that is two
corpora, which is the problem again one level up.

*Where the corpus lives:* a new top-level `testdata/selection-queries.json`.
Neither crate owns it, which is the point; both read it by relative path.

### The query types move to `pult-schema` with a `Selection` prefix

`Term`, `Clause` and `Order` are fine names in a file about selection and bad
names in a crate that exports 112 flat TypeScript files. They become
`SelectionTerm`, `SelectionClause`, `SelectionOrder`; `SelectionQuery` and
`Combine`... `Combine` also gets the prefix, for the same reason.

`frontend/src/lib/selection.ts` re-exports them under the old local names
(`export type { SelectionTerm as Term }`), so no panel changes and the diff stays
about the thing that changed.

The serde and ts-rs shapes must reproduce today's TypeScript exactly:

- `SelectionTerm` and `SelectionOrder` are `#[serde(tag = "kind")]`, matching
  `{ kind: 'Sphere', centre, radius }`.
- `Combine` is a plain unit enum — `'Add' | 'Keep' | 'Drop'` already.
- The axis is an enum rendered lowercase (`'x' | 'y' | 'z'`).
- `descending` is `#[serde(default)] #[ts(optional)]`, so it stays
  `descending?: boolean` and an absent field is `false`.
- `Vec3` is the one already in `types/fixture.rs`. `Ids` holds `Uuid`s, which
  ts-rs writes as `string`.

`svelte-check` at zero warnings is what catches any of this being wrong.

### `Order::Manual` carries the order

This is the trap in the change. `Manual` today means "whatever order the
operator dragged into", and that order lives in a frontend store —
`handOrder` — not in the query. A group saved with a `Manual` order would
therefore resolve to a different order on a station that has never seen that
drag, which the spec forbids.

So `SelectionOrder::Manual` carries `order: Vec<Uuid>`: the ids in the order
somebody put them in, with anything the query newly matches appended. Saving a
group with a hand order bakes the current order into the query. A group is then
deterministic by construction rather than by everyone remembering to freeze it.

The evaluator takes `previous: Option<&[Uuid]>`, which wins over `Manual.order`
when given. That is what lets the live selection keep `handOrder` as a store —
an in-flight drag is not a fact about the show and has no business being written
into the query on every mouse move — while a saved group, which has no store
behind it, gets its order from the query itself. Corpus cases carry `previous`
so both behaviours are covered on both sides.

### Resolution is a station RPC, not an entity command

`selection.resolve` goes in `crates/pult-backend/src/api/rpcs.rs` with the other
station RPCs, taking `{ "groupId": "<uuid>" }` and answering an ordered array of
fixture ids.

It is **not** called `group.resolve`, and the reason is a trap worth writing
down: the command line's parser checks RPC prefixes *before* collection names, so
an RPC named `group.*` takes the word `group` out of the grammar — `group 1` then
parses as "an unknown command on the `group` RPC" rather than as a selection. An
RPC's prefix is a reserved word, and naming one after a collection quietly deletes
that collection's spelling. That one edit makes it callable over the WebSocket, callable from
plugins, and visible to introspection — which is the requirement about
discovery, met by the mechanism that already exists.

Not a `#[pult_command]`: a command deserializes an entity, mutates it, and hands
the engine a new entity to apply, which writes an operation. Resolving a group
changes nothing and must not appear in anybody's history or undo stack.

`LocalRpcDeps` gains an `EngineHandle`, since resolution needs the rig. That
widens the premise of `api/rpcs.rs`, whose header says these are "calls against
LOCAL state". The honest reframe is the one the code already implies: these are
the calls that are *not* entity commands — the ones that answer rather than
change. The header gets rewritten to say that, and the test holding `LOCAL_RPCS`
and the dispatcher to each other is unaffected.

*Alternative considered:* an RPC taking a whole `SelectionQuery` rather than a
group id, which a paperwork or NL plugin might one day want. Rejected for now:
the command line has no syntax for writing a query as JSON, and "no such group"
is a real error the group-id form can give and the query form cannot. The query
form is a small addition on top later if something asks for it.

### A selection effect can carry a query

`ExecResponse.effects.selection` gains an optional `query` beside `fixtureIds`.
When the command line selects a group it returns the group's *query*, and the
surface sets it directly — so `group 3` typed into the command line leaves
exactly what recalling the group in the panel leaves: a live selection, not a
snapshot of it.

`fixtureIds` stays and keeps its meaning for `fixture 1 thru 5`, which is a list
by nature. Both surfaces (`ConsoleSurface.svelte`, `BarSurface.svelte`) learn the
new field; a surface that only knows `fixtureIds` ignores an effect it does not
understand, which is what an optional field buys.

The plugin still needs ids when the line also sets a level (`group 3 at 50`),
because it writes `programmer_values` per fixture. That is what `group.resolve`
is for, and it is the only path that needs it.

### The grammar generalises the select target

`Command::Select` today is `ops: Vec<(SelOp, Range)>`. A range of fixture
numbers becomes one variant of a target:

```
SelectTarget::Fixtures(Range) | SelectTarget::Group(Target)
```

`Target` is the addressing the grammar already has for entities (by index or by
name), so `group 3` and `group "movers"` both parse, and
`fixture 1 thru 5 + group 2` composes without a second code path. Completion
gains `group` beside `fixture`; the group names come from the catalogue the
plugin already builds from introspection.

### Groups live in the selection panel

A section of `SelectionPanel.svelte`: a list of the show's groups, *Save as
group…* on the current selection, and a click to recall. Not a new panel — a new
panel has to be dragged into a tile before anybody sees it, and every preset
layout would need editing to make groups discoverable. A group is only ever
wanted where the selection is.

## Risks / Trade-offs

- **Two evaluators drift.** → The corpus, read by both suites. Any new term or
  order added later has to land a case in it, and a term implemented on one side
  only fails on the other.
- **`Manual` order is subtly wrong in some path nobody tested.** → Corpus cases
  with and without `previous`, including a query whose `Manual.order` names
  fixtures that no longer exist and fixtures the query newly matches.
- **The ts-rs output does not match what the panels expect**, e.g. `descending`
  becoming required. → `npm run check` at zero warnings fails the build rather
  than shipping a runtime shape mismatch.
- **`LocalRpcDeps` gaining the engine invites the next non-command call to go
  there too**, until `api/rpcs.rs` is a junk drawer. → The rewritten header says
  what belongs: calls that answer rather than change. That is a narrow rule, not
  an open door.
- **Resolution is a round trip for the plugin**, so `group 3 at 50` is two calls
  where `fixture 1 thru 5 at 50` is one. → It is one extra call on a keypress,
  not on a frame; the alternative is a third evaluator inside the plugin.

## Migration Plan

Nothing to migrate. No showfile holds a group, so there is no old shape to read.
A show written by this build and opened by an older one has a `groups` table the
older build ignores; a show written by an older build and opened by this one has
no groups, which is a show with no groups.

Rollback is dropping the change: shows keep a `groups` table nothing reads.

## Open Questions

- Whether a group should eventually be usable *inside* a query (`Term::InGroup`)
  for live composition. Deliberately out of scope here (proposal — Non-goals);
  nothing in this design forecloses it, since a term is one more variant and one
  more corpus case, plus cycle detection.
- Whether the selection panel's group list wants ordering or folders once a show
  has forty groups. Answerable after somebody has forty groups.
