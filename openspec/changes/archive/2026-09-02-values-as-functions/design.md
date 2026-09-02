## Context

See `proposal.md` — Why, for the measurement. The structural facts that decide the
approach, all verified against the tree:

- **The objects are already sufficient.** `RunningFade { from, to, t0, duration_ms,
  easing, cue_id }` (`types/effect.rs:199`) anchors in absolute console unix ms.
  `RunningEffect` is the same. `playback.rs:212`'s `value_at` is already a pure
  function of one of them and a moment; it just takes a `std::time::Instant` rather
  than the millisecond the object carries.
- **They already reach the browser.** `live_effects` and `live_fades` are LOCAL, and
  LOCAL means synced to connected frontends. The browser is already being sent what
  it needs to evaluate; it is also being sent the answers.
- **`pult-schema` cannot be the shared crate.** Its dependencies include `sqlx`,
  `tokio`, `inventory` and `ts-rs`. It will not build for a browser target.
- **The plugins' wasm toolchain is the wrong one.** `wasm32-wasip2` components run in
  wasmtime *on the host*. Code that has to run inside a page is
  `wasm32-unknown-unknown` with `wasm-bindgen`. Saying "we already have a wasm
  toolchain" is true and irrelevant; this is a second one.
- **The frontend surface is small**: 47 references to `live_values`, in about seven
  real source files.
- **Connectors already hold the objects.** `OutputPlugin::send(patch, changed)` gets
  whole `Fixture`s, live maps included, so a connector already has everything it
  needs to evaluate.

## Goals / Non-Goals

**Goals:**

- One evaluator, two compilation targets, no second implementation in any language.
- A value is the same number everywhere it is asked for, for the same instant.
- The engine has no periodic work left that is proportional to the rig.

**Non-Goals (design-level):**

- No change to how fades and effects are *decided* — which cue is live, what a Go
  does, how the programmer takes priority. Only where the arithmetic runs.
- No attempt to make the browser authoritative about anything. It evaluates for
  display; the station evaluates for output.

## Decisions

### A new leaf crate holds the types and the maths

`crates/pult-render` (name to settle at implementation): the evaluable types —
`ParameterValue`, `RunningFade`, `RunningEffect`, `EffectSpec`, `Curve`, `Easing`,
`Shape`, `SpeedMaster` — and the functions over them, extracted from
`model/playback.rs` and `types/effect.rs`. Dependencies: `serde`, `uuid`, and nothing
that touches an OS.

`pult-schema` takes a dependency on it and **re-exports the types under their current
paths**, so every existing `use pult_schema::types::effect::RunningFade` keeps
working and the move is invisible to the rest of the workspace. This is the same
courtesy `fixture-groups` paid when `SelectionQuery` moved.

*What does not move:* deciding which cue is live, building a fade from a capture,
overlay priority bookkeeping — everything in `playback.rs` that is about the show
rather than the arithmetic. The crate is the part a browser could sensibly run.

*Instants become milliseconds.* `value_at(&self, now: Instant)` becomes
`value_at(&self, now_ms: u64)`, which is what the objects are anchored in anyway. The
`Instant` in today's signature is why the maths is not already portable.

### The browser gets it as `wasm32-unknown-unknown` + `wasm-bindgen`

A thin `pult-render-wasm` wrapper exposes a batch entry point and is built into
`frontend/src/lib/` beside the TypeScript `pult-codegen` already writes.

*Batch, not per-fixture.* The call is "evaluate these parameters at this moment" and
returns a packed result. A JS→wasm crossing per fixture per frame would replace a
protocol cost with a boundary cost, which is the mistake this change is fixing one
level up.

*Alternative rejected: a TypeScript twin.* It is what `fixture-groups` did for
`SelectionQuery`, held together by `testdata/selection-queries.json`, and that entry
records the standing cost honestly. The surface here is an order of magnitude larger —
easings, curves, step lists, spread, phase, direction, width, master rates, priority,
home fallback, split fades — and the failure mode is worse: a drift shows as the
screen disagreeing with the lamps, which an operator cannot work around and may not
notice until it matters.

*Alternative rejected: keep pushing computed values to the browser only.* It keeps one
evaluator with no wasm, and it keeps 40 Hz of socket traffic per connected client,
stepped motion on screen, and `live_values` in the schema — which is most of what this
change is for.

### The show clock advances monotonically from one wall reading

*Decided during implementation, when moving fades onto console milliseconds broke
thirteen tests and showed the console had two clocks that do not advance together.*

`now_ms()` took `SystemTime::now()` afresh on every call. Everything the console is
doing is anchored to that number and evaluated against it, so a step in it steps every
running fade and every effect at once — and `SystemTime` does step, whenever NTP
corrects a drift or a laptop wakes up. Effects have had that exposure since they
existed; it is not one to hand to fades as well now that they share the arithmetic.

So the show clock reads the wall clock **once** and advances monotonically from there.
Still a unix millisecond, so it replicates and anchors a cue exactly as before, and two
stations still agree by agreeing on the anchors they replicate rather than on their
clocks.

The base is `std::time::Instant`, not `tokio::time::Instant`, and that is not a detail:
tokio's is per-runtime, so a test binary running several runtimes would read one global
base against several unrelated clocks and get nonsense. **The consequence to know is
that `tokio::time::pause()` no longer fast-forwards a fade.** A test that wants one to
advance has to let real time pass or drive `Playback` directly — which is why
`taking_a_cue_fades_the_fixture_up` now runs a one-second fade in real time instead of
a four-second one in virtual time.

The alternative — a `ShowClock` owned by the station and passed to whoever needs it —
is the better object and was rejected for reach rather than shape: `went_at`'s fallback
is inside a registered command in `pult-schema`, which has no station to ask. Worth
revisiting if a second thing ever needs to control the clock.

### The browser must learn the station's clock, and say when it has not

The objects are anchored in console unix ms. A browser evaluating against
`Date.now()` runs every fade out by however wrong its own clock is, silently, because
every individual value looks plausible.

So: an offset, estimated the way a round-trip time is — send with a local timestamp,
compare against the station's reply, keep the best-of-several. `PeerLink::rtt_ms`
already measures exactly this shape between stations, and the OpenHaunt clock topic
and `went_at` anchoring are the project's prior art for shared time.

Two things this must not do. It must not present a value before the offset exists —
the spec requires saying so instead, because a plausible wrong number is worse than a
visible gap. And it must re-establish rather than drift when a clock steps, which
means the estimate is maintained, not taken once at connect.

*This is the genuinely new mechanism in the change*, and the one most likely to
produce a bug that only shows up on somebody else's laptop.

### Connectors own their rate; the engine pushes the show, not the values

`OutputPlugin::send(patch, changed)` becomes a connector that holds the last patch it
was given and runs its own loop over it. The engine pushes when the *show* changes — a
new cue is live, a fade starts, a fixture is patched — rather than when a value does,
which after this change it never separately does.

This is close to what the two ends already do: OpenHaunt sends an object and lets the
node run it; DMX dedups and refreshes every 800 ms when idle. What changes is that DMX
gains a timer of its own instead of being driven by the engine's.

### Flows sample, in proportion to what is watched

`queue_watched_changes` already exists and is already gated by a `watched` set,
because 40 Hz for every fixture was too much even when the values were free to read.
It becomes a small sampler over that set. Nothing watched, nothing sampled.

### `live_values` is removed rather than deprecated

Leaving it in place and unwritten would be worse than removing it: every reader would
keep compiling and silently see nothing move. Removing it makes every consumer a
compile error or a `svelte-check` error, which is the whole list of things to fix,
produced by the tools rather than by grep.

## Risks / Trade-offs

- **A wasm blob in the frontend bundle** → it is types and arithmetic, not a runtime;
  measure it, and if it is not small, that is a signal the crate boundary was drawn
  too generously.
- **Debugging is worse than TypeScript** → true, and the mitigation is that the same
  code is debuggable natively in Rust with the same inputs. A disagreement between
  screen and lamps becomes a test, not a browser session.
- **Two build steps that can go stale** → CI builds with `--locked` and the generated
  artifact is checked, so a stale one fails rather than drifts. Same rule as the
  existing codegen.
- **Clock offset wrong in a way nobody notices** → the spec forbids showing a value
  before the offset exists, which turns the silent failure into a visible one. Worth a
  test that deliberately skews a client clock and asserts agreement.
- **A field disappearing from the wire and the showfile** → both directions of sync
  have to tolerate it. Since every station already computes its own, the practical
  effect is nil, but an older peer sending the field must not be rejected and a newer
  one omitting it must not surprise an older peer.
- **The browser evaluating only what is visible could get it wrong about what is
  visible** → a panel that under-reports what it is showing displays stale values.
  Prefer evaluating a superset cheaply over tracking visibility precisely.

## Migration Plan

The showfile and the wire both lose a field. A showfile written before this change
opens with `live_values` present and ignored; serde's `deny_unknown_fields` is not in
use, so this is a non-event on load. On the sync path, both directions must tolerate
the field's presence or absence, which is a test rather than a mechanism.

The order that keeps it reviewable, each step independently checkable:

1. Extract the crate, re-export from `pult-schema`, change nothing else. The
   workspace should build and every test pass with no behaviour changed.
2. Move `Instant` to `u64` ms in the extracted maths. Still no behaviour change.
3. Station-side: connectors evaluate, engine stops materialising, `live_values` goes.
   Native only — the browser is broken at this point and that is expected.
4. The clock offset, on its own, with a test that skews a client.
5. The wasm build and the frontend panels.
6. The measurement moves to the output frame.

## Open Questions

- **What the crate is called**, and whether the extracted types keep living in
  `pult-schema`'s module paths forever or eventually move properly. Re-export is right
  for this change; a later tidy is a separate decision.
- **Whether the browser should evaluate for the `values` panel at frame rate at all**,
  or whether a numeric readout wants a deliberately slower rate so the digits are
  readable. A display question, settled when it is in front of someone.
- **How much of a superset the 3D viewer should evaluate** — everything in the scene,
  everything in the show, or everything that could enter frame. Answerable with the
  `huge` preset once the wasm path exists.
