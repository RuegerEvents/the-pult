# Tasks

## 1. The schema

- [x] 1.1 Add `Fixture::home_values` (PERSISTED, `HashMap<String, ParameterValue>`, `#[serde(default)]`) and verify with a test that a fixture written before the field existed still loads with an empty map
- [x] 1.2 Add the home-value resolution over a fixture, its type and a parameter kind in `types/fixture.rs`; verify with tests for the override, for the type's `default_value` when there is no override, and for a parameter the type does not have
- [x] 1.3 Add `Show::home_fade_ms` (PERSISTED, `#[serde(default)]`, default 0) with a clamp beside `clamp_history_depth`; verify a show written before the field existed loads at 0
- [x] 1.4 Add `Sequence::off` as a `#[pult_command]` clearing `active_cue_index` and re-anchoring `went_at`, and change `go_next` to stay on the last cue instead of wrapping to `None`; verify by replacing the wrap test with one asserting Go at the last cue leaves it active, and one asserting `off` leaves no cue active
- [x] 1.5 Add `home()` to the Rust field accessor (`handle.rs`) and to `frontend/src/lib/ws/proxy.ts` beside `by()`, so both path APIs stay the same shape; verify the TypeScript type appears on `LeafProxy`
- [x] 1.6 Run `cargo run -p pult-codegen -- generate` and verify `frontend/src/lib/generated/` builds with no hand edits

## 2. The engine

- [x] 2.1 Recognise `["programmer_values", "__home"]` with `{ fixtureId, parameterKind? }` at the top of the `Set` arm beside `__by`, resolving to ordinary programmer writes — one parameter when named, every output parameter of the fixture when not; verify with engine tests that a peer and the oplog see absolute values and never the verb
- [x] 2.2 Replace `default_value_of` with the shared resolution from 1.2 so a relative write to an undriven parameter honours a fixture's override; verify with a test that `__by` on a fixture with an override starts from the override
- [x] 2.3 Refuse `__home` on an input parameter and on a parked programmer value, leaving both unchanged; verify with tests naming each
- [x] 2.4 Add `Preferences::home_fade_ms` with sane bounds and seed `Show::home_fade_ms` from it when a show is created; verify a new show carries the station's value and an existing show is untouched by it

## 3. Playback

- [x] 3.1 Add `fixture_types` to `ShowView` (read in `playback_tick`) with a `home_value` lookup on the view; verify the existing playback tests still pass with the new constructor argument
- [x] 3.2 Replace both `zero_like` callers with the home value and delete `zero_like`; verify with a test that clearing the programmer on a fixture nothing had driven lands on the type's default, and one that a first fade on an undriven parameter starts there
- [x] 3.3 Home a sequence's parameters on a `Some → None` transition, using the key set from design.md — every parameter its cues capture, minus those another live sequence's cues capture, minus those the programmer holds; verify with tests for a lone sequence, two sequences sharing a fixture, a held parameter, and a show being opened (no transition, so nothing homes)
- [x] 3.4 Fade home over the show's `home_fade_ms` instead of snapping when it is non-zero; verify with a test sampling the fade early, in the manner task 40 fixed the node-sim test
- [x] 3.5 Verify across stations: two stations in one session, a sequence taken off on one, both showing the same live values afterwards — including a station that joined after the sequence had run several cues

## 4. The command line

- [x] 4.1 Add `home` as a first word beside `full` and `out`, emitting `__home` for the selection; verify with parser tests and a grammar test that `home` appears in the first-word completions
- [x] 4.2 Verify `sequence 1 off` parses with no grammar change, from the catalogue, with a test beside `entity_commands_come_from_the_catalog_not_the_parser`
- [x] 4.3 Add `home` to the plugin's help topics and verify the help test covers it

## 5. The frontend

- [x] 5.1 Add a Home button to the values panel that sends the selection home, next to Clear; verify by hand that it holds home values and that Clear gives them back
- [x] 5.2 Add Off to the sequence runner beside Go, and verify a sequence taken off shows no active cue
- [x] 5.3 Let the patch UI set and clear a fixture's home override per parameter; verify by hand that the override survives a reload and that clearing it returns the row to the type's value
- [x] 5.4 Add the show's home fade time to the settings panel beside the history depth, and verify it writes to the show rather than to the station
- [x] 5.5 Run `npm run check` and `npm test` and verify both are clean

## 6. Documentation

- [x] 6.1 Add a paragraph to `CLAUDE.md` on the home value and the `__home` verb, beside the `__by` paragraph; verify it says where the resolution lives and that the browser does not do it
- [x] 6.2 Add the task to `docs/ROADMAP.md` with the decisions and the traps, and turn `openspec/BACKLOG.md`'s `parameter-defaults` entry into a pointer that answers its "does this need tracking first" question
- [x] 6.3 Note the Go-at-the-last-cue change in `CHANGELOG.md` as a breaking behaviour change

## 7. Verification

- [x] 7.1 Run `cargo test`, `cd plugins && cargo test`, `cd frontend && npm test` and `npm run check`; verify all pass with zero warnings from both `cargo build` and `svelte-check`
- [x] 7.2 Run `scripts/demo.sh --two` and verify by hand: build a look on a sequence, take it off on one station, and see both stations put the rig back to its home values together
