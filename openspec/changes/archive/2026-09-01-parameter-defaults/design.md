## Context

See proposal.md — Why. What matters for the approach:

- `live_values` is written only by playback, through `PlaybackEffect::SetLiveValues`,
  and applied with LOCAL lifecycle even though the field is SYNCED: every station
  computes the same values from replicated cue state rather than being told them.
  So **anything that puts a value back has to be derivable from replicated state**,
  or the stations will disagree about the rig.
- `ShowView` carries sequences, cues, fixtures, programmer values and speed
  masters. It does not carry fixture types, so playback cannot currently see a
  `default_value` at all.
- `zero_like` has exactly two callers: the seed for what is underneath a newly
  grabbed programmer key, and the start of a fade on a parameter with no live
  value.
- `nudge_programmer` already resolves a default through `default_value_of`, which
  walks fixture → type → parameter in the engine's state.
- A sequence with `active_cue_index: None` is today reachable two ways — never
  started, or gone past its last cue — and the two are indistinguishable.
- `active_cue_index` is SYNCED, not PERSISTED, so every sequence is inactive when a
  show is opened.

## Goals / Non-Goals

**Goals:**

- One resolution of "what does this parameter rest at", reachable from the engine,
  from playback, and from a plugin, evaluated once.
- An act that stops a sequence, whose effect every station computes for itself.
- No new priority layer and no ownership bookkeeping.

**Non-Goals:**

- Deciding what a cue *out* time means. `fade_out_ms` stays unread.
- A second home-value evaluator in TypeScript. The frontend asks; it does not
  resolve.
- Making `live_values` sparse. See proposal.md — Non-goals.

## Decisions

### The home value is resolved in pult-schema, once

A free function over a fixture, its type, and a parameter kind, in
`crates/pult-schema/src/types/fixture.rs`: the fixture's `home_values` entry for
that key if present, else the type's `ParameterDefinition::default_value`, else
nothing (the fixture's type does not have that parameter).

Callers: the engine's `__by` resolution (replacing `default_value_of`), the
engine's `__home` resolution, and playback through the view. Three callers is
exactly the number that made this a concept rather than a fallback.

**No TypeScript twin.** `fixture-groups` pays for two evaluators of
`SelectionQuery` because a cone being dragged re-evaluates per frame and cannot be
a round trip; nothing here is on that path. The values panel already carries the
*type's* `default_value` per row as a display fallback and keeps doing so, which
is the only place a browser needs a number, and it is a fallback for an empty
readout rather than an answer about the rig. Every act that puts a value somewhere
goes through the station.

### `__home`, a path verb, resolved at the front door

`["programmer_values", "__home"]` with `{ fixtureId, parameterKind? }`, beside
`__by`. Resolved at the top of the engine's `Set` arm, before `previous` is read,
so the oplog, the broadcast and the sync layer see ordinary absolute programmer
writes and a peer receives the value rather than the verb — the same property
`relative-values` established and for the same reason.

`parameterKind` is **optional**: omitted, it means every output parameter of that
fixture, enumerated by the station from the fixture's type. That is what lets the
command line and the natural-language plugin ask for home without reading the rig,
and it keeps the enumeration in one place rather than in every client.

Alternatives considered. A `home` command on `fixtures` — rejected: it would write
programmer rows from a command on another table, and the derived programmer row id
is exactly the thing the verb needs. A station RPC — rejected: RPCs are reads by
convention (`selection.resolve` is deliberately not a command), and this writes.

### Off derives its key set from the show, not from what it watched

Taking a sequence off homes the parameters captured by **any cue of that
sequence**, minus those captured by a cue of another sequence that is on, minus
those the programmer holds.

The obvious alternative — remember, per sequence, which keys it has actually
written since it went on — was rejected because that memory is per station. A
console that joined at the interval never ran act one's cues and would take fewer
parameters home than the console that did, which is a divergence in the output
with no way back. Reading the cues instead is stateless, identical everywhere, and
correct for the same reason: a parameter no cue of any live sequence captures is a
parameter nothing is driving.

The cost is over-reach in one direction — a cue that never ran still contributes
its captures to the set — and the result is the same value, because nothing was
driving that parameter either. The rule errs toward leaving a value alone rather
than toward homing it: a parameter another live sequence merely *could* drive is
left where it is.

### `go_next` stops at the last cue

The only way to make "off" a distinguishable state without a second field for it.
`None` becomes unambiguous — the sequence was taken off — and playback homes on a
`Some → None` transition.

Two things fall out. The comment at `playback.rs:283` about not going dark because
the operator ran out of cues gets a better implementation than the one it
describes: the last cue stays active and owns its values, instead of the values
lingering under a cue that has been marked inactive. And a follow on the last cue
stops there rather than ending the sequence.

Alternative considered: a SYNCED `is_off` beside `active_cue_index`, leaving
`go_next` alone. Rejected — two fields encoding one thing, and a peer could
replicate them into a state that means nothing.

Opening a show is not an off: every sequence loads with no active cue, and
playback sees `None → None`, which is not a transition. Homing fires on a
transition out of a cue, never on the initial read.

### Home is written into `live_values`, not signalled by absence

Removing the key would be cheaper in the engine — the connectors already fall back
to `default_value` for an absent key — but the panels read that map to say what a
fixture is doing, and `commonValue` skips a key that is not there. Absence would
read as "unknown" exactly where the operator asked for "at home". So the home
value is written like any other value, and `emit`'s existing comparison means a
rig already at home writes nothing.

### The fade time is show data, seeded from a station preference

`Show::home_fade_ms`, PERSISTED, default 0; `Preferences::home_fade_ms` decides
what a *new* show starts at and then stops mattering.

This is the `history_depth` pattern, and `types/show.rs:36` already wrote the
argument for it: a default that kept applying would let two stations give
different answers about the same show. For undo that is a disagreement rather than
a preference; for a fade home, where both stations are driving the same rig, it is
a disagreement the audience can see. The desk still gets the knob it wants — it
sets it for the shows it creates.

Snapping is the default because a programmer clear snaps today, so no operator has
to be told that anything changed.

### `fixture_types` joins `ShowView`

One more `read_collection` in `playback_tick`. Fixture types are a handful of rows
where fixtures are thousands, so this is not the per-tick cost task 29 was
measuring; the view builds no index over them beyond by-id.

## Risks / Trade-offs

- **Go at the last cue is a behaviour change on every existing show.** → It is the
  half of this change that cannot be opt-in, and it is stated as BREAKING in the
  proposal. Nothing outside the console depends on the wrap: the frontend reads
  `active_cue_index` to render, and the one test that asserts the wrap is asserting
  the behaviour being replaced. The new behaviour is the safer one — a rig holds
  rather than being left in a state nothing claims.
- **Off homes more than an operator expects, and a house goes dark.** → Home is the
  fixture's own resting value, not zero, which is the whole point of the override.
  A rig where that is wrong is a rig whose overrides are wrong, and they are
  editable in the patch UI.
- **A parameter two sequences share is never homed while either is on.** → Correct
  by the rule and conservative in the right direction; taking both off homes it.
- **Two stations mid-fade when the show's `home_fade_ms` is edited.** → They finish
  the fade they started, and the next one agrees. The same is already true of every
  cue time edited while it runs.
- **`home_values` is another opaque map keyed by a parameter key string.** → It is
  keyed exactly like `live_values`, by the one key derivation there is, so the two
  cannot drift apart without the tests noticing.

## Migration Plan

No data migration. `home_values` and `home_fade_ms` are absent from every existing
showfile and default to empty and zero, which is behaviour identical to today
except at the two places `zero_like` used to answer — and there the new answer is
the fixture's own, which is the fix.

A session must not mix builds across this change: an older station would still
wrap `go_next` to `None`, which a newer station reads as the sequence having been
taken off and homes on. Same rule as `history-pruning`, and for the same kind of
reason.

## Open Questions

- Whether the patch UI should offer "take the fixture's current output as its
  home", which is how an operator would actually set a house light's override.
  Convenience over the same field; it changes no requirement and no task
  boundary.
