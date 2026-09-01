## 1. What a delta does to a value

- [x] 1.1 Add `ParameterValue::nudged(delta: f32) -> Result<ParameterValue, String>`
      to `crates/pult-schema/src/types/fixture.rs`: `Float` and `Int` add and clamp,
      `Color` adds per channel and clamps each, `Bool` and `Text` refuse by name.
      Verify unit tests cover each variant, both clamps, and the two refusals.
- [x] 1.2 Move the programmer entry id derivation into `pult-schema` as
      `programmer_entry_id(fixture_id, key)`, beside `ProgrammerValue`. Verify a test
      pins it to the same literal examples
      `plugins/command-line/core/src/ids.rs` and `frontend/src/lib/programmer.test.ts`
      already pin, so all three break together.

## 2. Resolving at the front door

- [x] 2.1 Add `resolve_relative(path, value)` to the engine, called at the top of the
      `EngineCommand::Set` arm *before* `authorship.previous` is read. Handle
      `[table, ref, field, "__by"]` against the field's current value. Verify a test
      nudges a cue's `fade_time` and reads back the sum.
- [x] 2.2 Handle `["programmer_values", "__by"]` with `{ fixtureId, parameterKind, by }`:
      derive the row id, resolve against the programmer's value where it holds the key
      and the fixture's `live_values` where it does not, and patch or create the row.
      Verify tests for both cases, including one where playback is showing a value and
      the programmer holds nothing.
- [x] 2.3 Refuse the shapes that do not compose — `__by` on `__create`, on a whole
      entity, on a path that is not there — with a message naming the path. Verify a
      test asserts the message and that nothing was written.
- [x] 2.4 Refuse a nudge against a key the programmer holds as an effect, with a
      message. Verify a test holds a shape, nudges, and finds the effect still running.
- [x] 2.5 Verify by test that the oplog row for a relative write is an *absolute*
      write: its path has no `__by`, its value is the resolved number, and its
      `previous` is what was there before.
- [x] 2.6 Verify by test that undo after a relative write restores the previous value,
      and that redo does not apply the delta twice.

## 3. Two stations

- [x] 3.1 Add a test that a relative write on one station reaches a peer as an
      absolute operation, and that a peer whose local value differed still ends up
      holding the same number.
- [x] 3.2 Verify by test that two relative writes of the same delta move the value by
      twice the delta — neither is lost.

## 4. The accessors

- [x] 4.1 Add `by(delta)` to `FieldAccessor` in `crates/pult-schema/src/handle.rs`,
      writing the `__by` path. Verify `cargo build` is warning-free and a doc comment
      says what it is relative to.
- [x] 4.2 Add `by` to `LeafProxy` and `createDataProxy` in
      `frontend/src/lib/ws/proxy.ts`. Verify `npm run check` is clean and a vitest
      case asserts the path it sends.
- [x] 4.3 Point `frontend/src/lib/programmer.ts` at nothing new — its own derivation
      stays, since the browser cannot import Rust — but verify its pinned test still
      matches the schema's literals.

## 5. The command line says how much

- [x] 5.1 Give `command-line-core` a level that is either a destination or a change:
      a signed read in `parse.rs` that reports whether a sign was written, carried by
      `Command::Intensity` and by `Select`'s `at`. Verify parse tests cover `at 10`,
      `at +10`, `at -10`, `full`, and `fixture 1 thru 5 at +10`, and that `at 10` and
      `at +10` do not produce the same command.
- [x] 5.2 Hint the signed form in completion beside `at`. Verify a completion test
      offers it.
- [x] 5.3 Teach the executor to write `programmer_values/__by` for a signed level and
      an absolute for a destination. Verify
      `cargo test -p pult-backend --test plugins` passes and a test runs
      `fixture 1 at 50` then `at +10` and finds the fixture at 60%.
- [x] 5.4 Verify a signed level against a fixture the programmer is not holding takes
      the key and starts from what playback is showing.

## 6. Finishing

- [x] 6.1 Verify a test asserts adding a new collection needs no edit to the
      resolution step — the property the `programmer_values` special case must not
      cost.
- [x] 6.2 Run the full gate: `cargo test`, `cd plugins && cargo test`,
      `cd frontend && npm test`, `npm run check`, `cargo build` — all clean and
      warning-free.
- [x] 6.3 Document `__by` in `docs/PLUGINS.md` beside `data.set`, and add the roadmap
      task recording why resolution sits above the oplog and what the engine naming
      `programmer_values` does and does not cost.
- [x] 6.4 Turn the `relative-values` entry in `openspec/BACKLOG.md` into a pointer to
      the archived change, and answer `nl-show-context`'s option (b) in its entry.
