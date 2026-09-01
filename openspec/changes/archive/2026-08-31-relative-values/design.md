## Context

See proposal.md — Why. What shapes the approach:

- `EngineCommand::Set` in `engine/mod.rs` does five things in order: read
  `previous` for undo, `apply_set`, record the write, append to the oplog,
  broadcast, and hand to sync. Every one of them takes **the path and value as
  given**. So anything relative that survives past the top of that arm is
  relative in the oplog and relative on the wire, which would make two stations
  disagree.
- `apply_set` already dispatches on path shape, with `__create` and `__delete` as
  verbs in the last segment. Adding a verb is an established move.
- The programmer is `programmer_values`, SYNCED, one row per (fixture,
  parameter). Its id is *derived* from the pair rather than minted, which is what
  makes two consoles grabbing one fader write one row. That derivation exists
  twice — `frontend/src/lib/programmer.ts` and
  `plugins/command-line/core/src/ids.rs` — with tests pinning them to each other
  by literal example.
- What a fixture is actually showing is `Fixture::live_values`, which the tick
  renders from playback and then the overlay covers. It is already "programmer if
  held, else playback": there is no second stack to consult.

## Goals / Non-Goals

**Goals**

- One place in the system knows the word "relative", and it is upstream of
  everything that records or replicates.
- No new WebSocket message, no new plugin host function, no new permission.
- The programmer can be nudged by somebody who is not already holding the key,
  because that is the ordinary case.

**Non-Goals**

- Making `apply_set` generic over "operations on values". One verb that adds is
  the whole ask; an expression language is not.

## Decisions

### Resolution happens at the front door, and the rest of the system is untouched

A short `resolve_relative(&self, path, value) -> Result<(Path, Value)>` runs at
the top of the `EngineCommand::Set` arm, *before* `authorship.previous` is read.
It answers an ordinary absolute path and value; everything after it is unchanged
code that has never heard of `__by`.

That single placement buys the whole of the spec's second requirement for free:
`previous` is the absolute before, the oplog row is an absolute write, undo
reverses it by writing `previous` back, the broadcast carries an absolute, and
`sync.broadcast_synced` sends an absolute. **A peer never resolves anything**,
which is the only way two stations can end up holding the same number.

*Alternative considered:* resolving inside `apply_set`. Rejected — `apply_set` is
downstream of `previous` and upstream of nothing; the logging and sync in the
caller would still see the relative path.

*Alternative considered:* a station RPC, the way `selection.resolve` is one.
Rejected for the opposite reason to that one: `selection.resolve` is a read and
must not write history, and this is a write that **must** be in history, be
attributed and be undoable. Going through `Set` is what gets all three without
writing any of them.

### The verb is `__by`, in two shapes

```
[table, ref, field, "__by"]          value: <delta>
["programmer_values", "__by"]        value: { fixtureId, parameterKind, by }
```

The first is the primitive: relative to the field's current value. The second
exists because the programmer's ordinary case is *not* holding the key yet —
`dim +10` on a fixture nobody has touched has no row to name — and because what
it must be relative to is the fixture's live value rather than a row that does
not exist.

The second shape means the engine names one collection by hand. That is a real
cost and worth being honest about: it is the same kind of naming `apply_set`
already does for `outputs` and `flow_nodes`, and it does not break the rule it
looks like it breaks — **adding a new collection still needs no edit here**. What
would be intolerable is the engine needing an edit per entity type; one entity
with a grabbing rule of its own is not that.

*Alternative considered:* requiring the caller to create the row first and then
nudge it. That puts the initial read back on the client, which is the race the
change exists to remove — and it is worst exactly where it is most likely, two
people reaching for the same fader at once.

### The arithmetic lives in `pult-schema`, beside `ParameterValue`

`ParameterValue::nudged(delta) -> Result<ParameterValue, String>`: `Float` and
`Int` add and clamp, `Color` adds per channel and clamps each, `Bool` and `Text`
refuse by name. Floats are normalised 0..1 throughout the console (the command
line divides percent by 100 and clamps), so that is the range clamped to.

In the schema rather than the backend because it is a fact about the type, and
because the frontend's `by()` and any plugin doing its own arithmetic should be
reading one definition of what "ten percent brighter" does at the top of a fader.

### `programmer_entry_id` becomes the schema's

The engine now needs the derivation, and a third copy of a rule that already has
two is not acceptable. It moves to `pult-schema` beside `ProgrammerValue`, whose
doc comment already explains why the id is derived. `plugins/command-line/core`
cannot depend on `pult-schema` — the plugins workspace builds guests for
`wasm32-wasip2` and the console's schema does not belong in that graph — so its
copy stays, and so does the frontend's. The pinned-example tests that already
hold those two to each other gain the Rust one, so all three break loudly
together or not at all.

### A held effect refuses

`Overlay` holds a key as either a value or an effect, never both. Nudging a shape
would have to mean nudging its offset, which is a different feature wearing the
same word, so a `__by` against a key held as an effect is an error naming the
reason. The check is in the resolution step, which can see the overlay.

### The grammar gets a sign, not a new word

`at +10` and `at -10`, and the same after a selection. The parser's `number()`
becomes a signed read that reports whether a sign was written, because `at 10`
and `at +10` must not be the same command — `+10` is a nudge and `10` is a
destination. `Command::Intensity` and the `at` of `Command::Select` carry a level
that is either.

The NL plugin needs nothing: it speaks by emitting command-line text, so "a bit
darker" becomes `at -10` and the audit trail is one grammar deep, which is the
`nl-show-context` option the backlog preferred.

## Risks / Trade-offs

- **A relative write racing a fade.** The value it lands on is whatever the fade
  had reached at that instant, which is what grabbing a fader has always meant —
  `Overlay::beneath` goes on following underneath, so a later release still lands
  where playback got to. → Not a defect; documented in the requirement's wording
  ("what is showing").
- **The engine naming `programmer_values`.** → Bounded and argued above; a test
  asserts that a new collection needs no edit here, which is the property that
  actually matters.
- **Three copies of the id derivation.** → Two exist today and this makes three
  because the plugins workspace cannot depend on the schema. Mitigated by the
  pinned literal examples, which now cover all three; a change to any one breaks
  two suites.
- **`__by` looks like it should compose** — `__by` on `__create`, `__by` on a
  whole entity. → It does not, and the resolution step rejects those shapes with
  a message rather than doing something surprising.
- **Clamping to 0..1 assumes normalised floats.** True everywhere in the console
  today, and `ParameterDefinition` carries no range to consult. → If ranges
  arrive (they are a `gdtf-import` question), the clamp reads them instead; the
  arithmetic is in one function.

## Migration Plan

Nothing to migrate. `__by` is a new path shape; no showfile holds one, and no
operation ever will, because resolution happens before the log. A station on an
older build rejects a `__by` path with "path not found" rather than doing
something wrong — and it never receives one from a peer, since peers only ever
see absolutes.

## Open Questions

- Whether `by` should eventually have a multiplying sibling ("half as bright").
  Out of scope (proposal — Non-goals); it would be another verb reading the same
  resolution step, so nothing here forecloses it.
