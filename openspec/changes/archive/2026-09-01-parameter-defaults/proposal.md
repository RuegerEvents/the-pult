## Why

Nothing in the console ever puts a parameter back. `live_values` only grows:
`emit()` (`crates/pult-backend/src/model/playback.rs:447`) merges a tick's changes
onto the map a fixture already has, and no path removes a key or writes one back
down. Three consequences, all of them visible to an operator:

- **There is no way to say "stop".** A sequence has Go and Go To Cue and nothing
  else. Turning it off is not an act the console has, so a look built by a
  sequence outlives it — the cue's `is_active` goes false and the values it faded
  to sit there with nothing claiming them.
- **Clearing the programmer on an untouched fixture lands on a hardcoded zero.**
  `Overlay::assert` seeds what is underneath a newly grabbed key with
  `zero_like` (`playback/programmer.rs:120`), a zero of the same variant, when
  nothing was live before the grab. A fade started on a key with no prior value
  does the same (`playback.rs:353`). Neither asks the fixture what that parameter
  rests at, though the fixture knows.
- **`ParameterDefinition::default_value` already exists** and is derived from what
  an OpenHaunt node said about its own ports. Only the connectors read it, and
  only as a fallback for a key absent from `live_values`, so a house light that
  defaults to on is on until the first thing that touches it — and then it is
  never on again.

`nudge_programmer` (`engine/mod.rs:1373`) already needed this and reached for
`default_value` to get it, which is the third reader of the same idea and the sign
it should be a concept rather than three fallbacks.

## What Changes

- **A parameter has a home value.** Its fixture's override where one is set, and
  its type's `default_value` otherwise. One resolution, in `pult-schema`, used by
  the engine, by playback, and by whatever asks next — not evaluated a second time
  in the browser, which is the standing cost `fixture-groups` paid and this change
  declines to pay again.
- **A fixture can override its type's default.** `Fixture::home_values`, PERSISTED
  and keyed like `live_values`, empty on every fixture that has nothing to say.
  PERSISTED because a house light that defaults to on is a fact about the rig,
  travels with the show, and cannot live on `FixtureType` — that is derived from
  the device and re-derived whenever the device describes itself again.
- **A sequence can be taken off.** `Sequence::off` sets `active_cue_index` to
  `None`, and playback puts the keys that sequence could drive back to home. Which
  keys those are is read from the show — the parameters its cues capture — rather
  than remembered, so every station computes the same set and a station that
  joined this evening computes it too.
- **BREAKING: Go at the last cue stays at the last cue.** Today `go_next` wraps to
  `None`, so "the operator ran out of cues" and "the operator turned it off" are
  the same state and playback cannot tell them apart. After this change, `None`
  means off and nothing else reaches it. Running out of cues holds what is
  showing, which is what `playback.rs:283` says it wants to do and can now do by
  leaving the last cue active rather than by leaving values behind.
- **An operator can send a selection home.** A path verb, `__home`, beside
  `__by`, `__create` and `__delete`: `["programmer_values", "__home"]` with
  `{ fixtureId, parameterKind }` puts the programmer on that key at its home
  value. Homing is a programmer act, which is how it replicates, how it undoes,
  and how Clear takes it back. The command line gets `home` beside `full` and
  `out`, so the natural-language plugin can answer "put it back" with no access to
  the show — the same argument `relative-values` made for `at +10`.
- **`zero_like` is deleted.** Both of its callers become the home value, and the
  function's own comment ("the dark, home, or off value") stops being a guess.
- **Going home can take time.** `Show::home_fade_ms`, PERSISTED, default 0 — which
  is exactly what a programmer clear does today, so nothing an operator has got
  used to changes until they ask for it. Seeded from a new station preference the
  way `history_depth` already is: the desk decides what a *new* show starts with
  and then stops mattering, because two stations fading the same rig home over
  different times is not a preference but a disagreement about the output
  (`types/show.rs:36`).

## Non-goals

- **No tracking.** "Not controlled" here means no cue of any sequence that is on
  captures that parameter and the programmer does not hold it. It does not mean
  "not tracked from an earlier cue", and this change does not introduce cue-to-cue
  inheritance. A sequence's own cues are read as a set, which is deliberately
  coarser than tracking and needs none of it.
- **No priority stack rewrite.** Home sits under playback as a value, not as a
  layer with its own arbitration. The one priority rule stays the one task 14
  wrote: for every parameter the programmer holds, the programmer wins.
- **`live_values` keeps its keys.** A key is not removed to mean "at home"; the
  home value is written into it. Panels read that map to say what a fixture is
  doing, and an absent key reads as unknown rather than as at-default —
  `commonValue` (`frontend/src/lib/programmer.ts:124`) skips what is not there.
- **No cue-out fades.** `Cue::fade_out_ms` and `ParameterCapture::fade_out_ms` are
  declared and nothing reads them. Wiring them up is a change of its own, and
  `home_fade_ms` deliberately does not pretend to be it.
- **No park interaction.** A parked (locked) programmer value survives Clear by
  design and survives a home the same way. "Release all" in the values panel means
  un-park and keeps meaning that; going home is a different word for a different
  act.
- **No per-type editing.** A show cannot rewrite a fixture type's
  `default_value`. The override is per fixture, where nothing re-derives it.

## Capabilities

### New Capabilities
- `programming/home-values`: what a parameter rests at when nothing is driving it,
  where that value comes from, the acts that reach it — taking a sequence off,
  sending a selection home, releasing a key nothing was under — and what is
  guaranteed not to reach it.

### Modified Capabilities
- `programming/relative-values`: its requirement that a relative write is resolved
  against "what the priority stack is showing" left the bottom of that stack
  unsaid, and the implementation quietly used the type's `default_value`. It
  becomes the home value, which is the same answer for every fixture with no
  override and the right one for the fixtures that have one.

## Impact

- **`crates/pult-schema`** — `Fixture::home_values`; `Show::home_fade_ms`; the
  home resolution over a fixture and its type; `Sequence::off` and the changed
  `go_next`; the `__home` verb on the accessors.
- **`crates/pult-backend`** — `fixture_types` in `ShowView` so playback can
  resolve a home value at all; homing on a sequence going off; `zero_like`
  removed; `__home` resolved beside `__by` at the top of the `Set` arm;
  `default_value_of` in `engine/mod.rs` replaced by the shared resolution; the new
  station preference.
- **`plugins/command-line`** — `home` as a first word beside `full` and `out`.
  `sequence 1 off` needs no grammar work: entity commands come from the catalogue.
- **`frontend/`** — a Home button in the values panel and an Off in the sequence
  runner; `home_values` in the patch UI so an override can be set; nothing that
  resolves a home value for itself.
- **Docs** — `docs/ROADMAP.md` gains a task; `openspec/BACKLOG.md`'s
  `parameter-defaults` entry becomes a pointer, and its "does this need tracking
  first" question is answered no.
