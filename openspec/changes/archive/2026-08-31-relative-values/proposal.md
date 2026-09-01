## Why

Every write in the console is absolute. There is no way to say "ten percent
brighter", only "at 62%", and three things want the other one:

- The natural-language plugin cannot honour "a bit darker". It is granted no data
  access on purpose (`commands = false` in its manifest — everything goes through
  the command line), so it has no current value to be relative to, and giving it
  one would weaken the safety story and grow the prompt with the rig. The
  backlog's `nl-show-context` lists "add relative syntax so the model never needs
  the value" as the option that keeps the one-grammar, one-audit-trail property.
- The command line has `at 80` and no `at +10`, which is the second thing anyone
  types on a console.
- Encoders and fan, when they arrive, are relative by nature.

The question that has kept it out is where a relative write turns into an
absolute one. On the client it is racy — two operators nudging one fader read the
same value and both write the same answer, and one nudge is lost. Task 14 already
built the priority stack that says what is showing; this change makes the engine
read it.

## What Changes

- **A path verb, `__by`.** `["cues", id, "fade_time", "__by"]` with `1.5` means
  "one and a half more than whatever that is now". It sits beside `__create` and
  `__delete`, so it arrives over the existing `Set` message and needs no new
  protocol.
- **Resolved at the front door, and nowhere else.** The engine rewrites a `__by`
  write to an absolute path and value *before* it reads `previous`, applies it,
  logs it, broadcasts it or replicates it. **A peer never sees a relative
  operation** — which is the point: a peer applying "+10%" to its own copy would
  diverge. Undo, the showfile, the history panel and the sync layer are untouched
  and know nothing about any of this.
- **The programmer can be nudged without already holding the key.**
  `["programmer_values", "__by"]` with `{ fixtureId, parameterKind, by }` finds
  or creates the row for that parameter and resolves against what is showing —
  the programmer's value if it holds the key, and the fixture's live value if it
  does not. That is task 14's stack, read rather than re-implemented.
- **`programmer_entry_id` moves into `pult-schema`.** The derived id that makes
  two consoles writing one fader converge is currently implemented twice, in
  `frontend/src/lib/programmer.ts` and `plugins/command-line/core/src/ids.rs`,
  and the engine now needs it too. Three would be too many; the schema is where
  the rule belongs, and the other two stay pinned to it by the tests that already
  pin them to each other.
- **The command line learns signed levels**: `at +10`, `at -10`,
  `fixture 1 thru 5 at +10`. Which is all the NL plugin needs, since it speaks by
  emitting command-line text.
- **`.by()` on both accessors**, Rust and TypeScript, so the two path APIs stay
  the same shape.

## Non-goals

- **No new priority rule.** "Relative to what is showing" is task 14's stack,
  already rendered into `live_values`. This change reads it and adds nothing to
  it.
- **No fan.** A spread of different deltas across a selection is a separate
  feature that needs the selection's order to mean something to the backend, and
  it can be built on this.
- **No multiplicative deltas.** `by` adds. "Half as bright" is a different verb
  and nobody has asked for it; nothing here forecloses one.
- **No relative on a held effect.** A key the programmer holds as a shape refuses
  a nudge with a message. Nudging a shape means moving its offset, which is a
  different thing wearing the same word.
- **No relative cue values.** A cue that stores "+10%" rather than a level is
  tracking, and tracking is its own design.
- **No encoder or wheel bindings.** This is the write; the hardware that would
  want it is not in the tree.

## Capabilities

### New Capabilities
- `programming/relative-values`: what a relative write means, what it is relative
  to, where it becomes absolute, and what the rest of the system is guaranteed
  never to see.

### Modified Capabilities

None. No existing capability's requirements change. `history/retention` and
`users/identity` already require every attributed write to be undoable and
attributed, and a relative write is an ordinary absolute write by the time either
of them sees it — which is the design rather than a coincidence.

## Impact

- **`crates/pult-schema`** — the canonical `programmer_entry_id`; `.by()` on
  `FieldAccessor`; the arithmetic for adding a delta to a `ParameterValue`, which
  is where the type rules (float, int, colour; not bool, not text) live.
- **`crates/pult-backend`** — a resolution step in the `EngineCommand::Set` arm
  of `engine/mod.rs`, and the two `__by` path shapes. Nothing in the sync layer,
  the oplog, or undo.
- **`plugins/command-line`** — a signed level in the grammar and in completion;
  the executor uses `__by` instead of computing a level itself.
- **`frontend/`** — `by()` in `ws/proxy.ts` and its type in `LeafProxy`;
  `programmer.ts` imports the id derivation rather than implementing it.
- **Docs** — `docs/ROADMAP.md` gains a task; `docs/PLUGINS.md` gains the verb;
  `openspec/BACKLOG.md`'s `relative-values` entry becomes a pointer, and
  `nl-show-context` gets its option (b) answered.
