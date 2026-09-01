## Why

A fade is already an object. `RunningFade { from, to, t0, duration_ms, easing,
cue_id }` carries an absolute anchor in console unix ms and needs nothing else to be
evaluated; `RunningEffect` is the same shape. The comment on the function that builds
one already says what it is for: *"This fade, described well enough for somebody else
to run it."* One output path already takes that offer —
`connectors/openhaunt.rs:162` reads `fixture.live_effects` and ships the object to
the node, which evaluates it locally, so an effect leaves the console as one message
instead of forty a second.

Everywhere else, the console evaluates the object forty times a second and stores the
answer in `Fixture::live_values`. That store is the entire cost of running a show:

| 2005 fixtures, one tick | |
|---|---|
| Computing every value | **0.07 ms** |
| Reading state in order to compute | 33.8 ms |
| Writing the answer back as state | 2.2 ms |

The tick is not expensive because it evaluates. It is expensive because the answer
becomes state — to be read, written, versioned, broadcast, and read again. Stop
storing it and 99.8% of the tick goes with it, along with the reason for the engine to
have a tick at all.

`live_values` is SYNCED for a reason that has been measured and retired: the belief
that computing is the hard part, so one station should compute and the rest should be
told. At 0.07 ms for two thousand fixtures, telling costs orders of magnitude more
than recomputing, and at 40 Hz across a session it would be the largest thing on the
network. It is already half-vestigial — playback writes through `apply_local`, i.e. as
LOCAL whatever the field declares, so the only thing SYNCED achieves is putting a
stale sample in the snapshot a joining station is handed immediately before it
recomputes.

## What Changes

- **A live value stops being stored.** `Fixture::live_values` is removed. What a
  parameter is doing right now is a function of the objects already in state — the
  fades and effects driving it, the programmer over the top, the home value
  underneath — and the wall clock. Every consumer evaluates it for itself, at the
  rate it actually needs.
- **The evaluator is one implementation, compiled twice.** A new leaf crate holds the
  evaluable types and the maths, with no `sqlx`, `tokio` or `inventory` in it — which
  is why it cannot simply be `pult-schema`, whose dependency list rules out a browser
  target. Native for the station, its connectors and its plugins; **`wasm32-unknown-
  unknown` via `wasm-bindgen`** for the browser. This is a *second* wasm toolchain,
  not the one the plugins use — those are `wasm32-wasip2` components run by wasmtime
  on the host, which is the wrong tool for code that has to run inside a page.
- **The browser evaluates what it is showing.** Panels stop reading a value and start
  asking for one at a moment. A rig of two thousand with forty on screen evaluates
  forty, at animation frame rate rather than 40 Hz, so the picture gets *smoother*
  while the socket goes quiet during a fade.
- **The browser learns the station's clock.** The objects are anchored in console
  time, so a browser evaluating locally has to know its offset or it will run every
  fade early or late. This is the one genuinely new mechanism the change needs.
- **Output connectors own their own rate.** They already receive the objects in the
  patch and already know their protocol's needs — DMX dedups and drops to an 800 ms
  refresh when idle (`connectors/dmx.rs:119`), OpenHaunt sends an object once. Each
  evaluates on its own loop from what it was last pushed, and the engine pushes only
  when the *show* changes rather than when a value does.
- **Flows keep a sampler, and it is small.** A `Watch` node does edge detection on a
  parameter, which cannot be done without sampling. `watched` is already a gated set —
  the gate exists because 40 Hz across every fixture was too much — so this samples
  only what something is actually watching.
- **The engine stops ticking.** With nothing to materialise, the 25 ms timer in
  `ShowEngine::run` has no work: no `read_collection` per tick, no `apply_local` per
  fixture, no broadcast per value. The engine becomes what it should have been, a
  thing that answers questions and applies writes.
- **What is sensed stays state.** `Contact`, `Temperature`, `Humidity` and the rest
  arrive *from* devices and are not functions of time. The split is written into the
  spec so it stays clean: driven outputs are functions, sensed inputs are state.

## Capabilities

### New Capabilities

- `playback/derived-values`: What a parameter is doing is evaluated, not stored — who
  evaluates, from what, at what rate, agreeing on which clock, and what remains state
  because it is sensed rather than driven.

### Modified Capabilities

- `observability/tick-cost`: its requirements describe an engine tick with a playback
  half and an applying half. After this change there is no engine tick and nothing is
  applied; what has a deadline is the output frame. The figures move to it rather than
  being deleted — the reason they exist, that a console should be able to say what
  running a show costs it, is untouched.

## Non-goals

- **No change to what a value *is*.** The same fade produces the same number at the
  same instant. If a single frame differs, that is a bug in this change.
- **No new visual quality.** Smoother motion falls out of evaluating at frame rate,
  but beams that look like light is `rig-viewer-fidelity`, which this unblocks rather
  than does.
- **No explicit save, and no deferred persistence.** That is `showfile-management`,
  and it has its own hard problem in live replication.
- **Disk stays where it is**, and so does the single engine queue. Those are the parts
  of `tick-isolation` that survive, and they share no decision with this.
- **No partitioning of computation across stations.** Task 10's question stays open.
  It was always about a workload one station could not carry, and this changes what
  that would have to be.
- **The plugin contract does not change.** A plugin that wants a current value gets
  the evaluator natively, through the same host functions it uses now.

## Impact

- **New crate** for the evaluable types and the maths, extracted from
  `pult-backend/src/model/playback.rs` (750 lines, of which the evaluation is a
  subset) and `pult-schema/src/types/effect.rs`. `pult-schema` depends on it and
  re-exports, so nothing else moves.
- **New wasm build step** and a generated artifact under `frontend/src/lib/`, beside
  the TypeScript that `pult-codegen` already writes. CI has to build it, and
  `--locked` means a stale artifact fails rather than drifts.
- `crates/pult-schema/src/types/fixture.rs` — `live_values` removed; codegen rerun.
- `crates/pult-backend/src/engine/mod.rs` — the tick goes.
- `crates/pult-backend/src/infra/connectors/` — each connector gains a loop.
- `crates/pult-backend/src/model/flows.rs` — the watched-parameter sampler.
- **Frontend: 47 references across about seven real source files** — `stage.ts`,
  `programmer.ts`, `patch.ts`, `Quicksheet.svelte`, `Rig3D.svelte`,
  `PatchPanel.svelte`, `FixtureTypeEditor.svelte`. Smaller than it sounds.
- **BREAKING: the showfile and the wire both lose a field.** `live_values` was SYNCED
  and PERSISTED-adjacent in the snapshot; a station on an older build syncing with one
  on this build will not agree about it. Since both stations already compute their own,
  the practical effect is nil — but it is a field removal and the sync path has to
  tolerate it in both directions.
