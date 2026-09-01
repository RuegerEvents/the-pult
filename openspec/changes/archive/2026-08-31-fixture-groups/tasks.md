## 1. The query language becomes schema

- [x] 1.1 Add `crates/pult-schema/src/types/group.rs` with `SelectionTerm`,
      `SelectionCombine`, `SelectionClause`, `SelectionOrder` and
      `SelectionQuery`, tagged and named so they serialize exactly as the
      TypeScript in `frontend/src/lib/selection.ts` does today; register the
      module in `types/mod.rs`. Verify `cargo test -p pult-schema` passes and a
      round-trip test deserializes a hand-written JSON query of each term kind.
- [x] 1.2 Give `SelectionOrder::Manual` an `order: Vec<Uuid>` and verify a
      serde test proves an empty order round-trips as `{"kind":"Manual","order":[]}`.
- [x] 1.3 Add the `Group` entity in the same file — `id`, `name`, `query`, all
      PERSISTED, `#[pult(table = "groups")]` — and verify
      `cargo test -p pult-backend` still passes with nothing outside
      `pult-schema` edited.
- [x] 1.4 Run `cargo run -p pult-codegen -- generate` and verify
      `frontend/src/lib/generated/` gains `Group`, `GroupCreate`, `GroupPatch`,
      `SelectionQuery` and the term/order types, and that `data.ts` gains
      `groups`.

## 2. The evaluator, twice, against one corpus

- [x] 2.1 Write `testdata/selection-queries.json`: a small rig plus cases of
      `{ query, previous?, expected }` covering every term kind, all three
      combines in sequence, every order, unplaced fixtures in a geometric order,
      a `Manual.order` naming a deleted fixture, and a `Manual.order` that misses
      a fixture the query newly matches. Verify the file parses as JSON.
- [x] 2.2 Implement `evaluate(query, fixtures, previous: Option<&[Uuid]>)` in
      `pult-schema`, porting the geometry and the clause fold from
      `selection.ts`. Verify a Rust test drives every corpus case and passes.
- [x] 2.3 Point `frontend/src/lib/selection.test.ts` at the same corpus file and
      verify `npm test` drives every case through the TypeScript `evaluate()`
      and passes.
- [x] 2.4 Change `selection.ts` to import the generated types and re-export them
      under the old local names, deleting its own declarations, and teach its
      `evaluate()`/`sortSelection()` the `Manual.order` fallback. Verify
      `npm run check` is clean and no panel imports changed.

## 3. Resolving a group from a station

- [x] 3.1 Add an `EngineHandle` to `LocalRpcDeps` and update its three
      construction sites (`lib.rs`, `api/ws/mod.rs`, the plugin manager). Verify
      `cargo build` is warning-free.
- [x] 3.2 Rewrite the header of `api/rpcs.rs` to say these are the calls that
      answer rather than change, replacing the "calls against LOCAL state"
      framing that the engine handle outgrows.
- [x] 3.3 Add the `group.resolve` entry to `LOCAL_RPCS` and its arm in
      `dispatch`, answering ordered fixture ids and erroring by name on an
      unknown group id. Verify `the_table_and_the_dispatcher_agree` still passes
      and a new test resolves a seeded group and gets an error for a made-up id.
- [x] 3.4 Verify by test that resolving a group writes no operation: the oplog
      length is unchanged across a resolve.

## 4. The command line learns `group`

- [x] 4.1 Generalise `Command::Select` in `plugins/command-line/core` from
      `Vec<(SelOp, Range)>` to a target enum carrying either a fixture range or a
      group `Target`. Verify the existing parse tests pass unchanged in meaning
      and new ones cover `group 3`, `group "movers"`, `fixture 1 thru 5 + group 2`
      and `group 3 at 50`.
- [x] 4.2 Add `group` to completion beside `fixture`, sourcing names from the
      introspection catalogue. Verify a completion test offers `group` and then
      the show's group names.
- [x] 4.3 Implement the group arm in `plugins/command-line/src/lib.rs`: return
      the group's query as a selection effect, and call `group.resolve` only when
      the line also sets a level. Verify `cargo test -p pult-backend --test plugins`
      passes and a test executes `group 1 at 50` against a station with a seeded
      group.
- [x] 4.4 Verify naming a group that does not exist leaves both the selection and
      the programmer untouched and returns an error line.

## 5. Selection effects can carry a query

- [x] 5.1 Document the `query` form of the selection effect where the SDK and
      `docs/PLUGINS.md` describe `ExecResponse.effects`. (`effects` is deliberately
      untyped JSON, so there was no field to add — narrowing it to a struct would
      have taken away the property that a surface can ignore an effect it does not
      know.) Verify `cd plugins && cargo build` is warning-free.
- [x] 5.2 Route both `ConsoleSurface.svelte` and `BarSurface.svelte` through one
      `applySelectionEffect` in the selection store, preferring `query` over
      `fixtureIds` and dropping the hand order when a query arrives. Verified by
      `npm run check` clean and store tests covering both shapes, the preference,
      and a recalled query keeping up with a fixture patched afterwards.

## 6. Saving and recalling in the frontend

- [x] 6.1 Add a groups section to `SelectionPanel.svelte`: the show's groups,
      *Save as group…* on the current selection, recall on click, rename and
      delete. Verify `npm run check` is clean.
- [x] 6.2 Bake the hand order into `Manual.order` when saving a group whose
      order is `Manual`, and verify a vitest case proves the saved query resolves
      to the dragged order with no store behind it.
- [x] 6.3 Verify by hand in `scripts/demo.sh`: save a group, patch a fixture that
      matches it, and see it join the group on recall.

## 7. Two stations agree

- [x] 7.1 Add a backend test that a group written on one station reaches a peer
      and resolves to the same ids in the same order on both.
- [x] 7.2 Verify a group delete followed by an undo restores the group on both
      stations, and that the rename shows up attributed in the history.

## 8. Finishing

- [x] 8.1 Run the full gate: `cargo test`, `cd plugins && cargo test`,
      `cd frontend && npm test`, `npm run check`, `cargo build` — all clean and
      warning-free.
- [x] 8.2 Add the roadmap task to `docs/ROADMAP.md` recording the `Manual` order
      trap and the two-evaluators-one-corpus decision; update `docs/SPEC.md` if
      it claims groups do not exist.
- [x] 8.3 Turn the `fixture-groups` entry in `openspec/BACKLOG.md` into a pointer
      to the archived change, the way `plugin-datastores` and `default-user` read.
