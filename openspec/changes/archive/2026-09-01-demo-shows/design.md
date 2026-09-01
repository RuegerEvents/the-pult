## Context

See `proposal.md` — Why. The constraints that actually shape the approach:

- **The engine is one actor with a 256-deep command channel** (`engine/mod.rs:614`)
  and a 25 ms ticker (`model/playback.rs:38`). Its timer arm calls
  `push_output_config`, then `playback_tick`, then `flows_tick`.
- **`playback_tick` early-returns when there is nothing to do** (`engine/mod.rs:803`):
  no work in playback and no state change since the last one. This is what "a
  settled show stops ticking" means — the timer still fires, the work does not
  happen. It is also the only place the spec's "absent rather than zero" can come
  from cleanly.
- **The whole cost is inside `playback_tick`.** It reads the collections it needs,
  computes effects through `Playback::tick`, then applies each one — a write, a
  broadcast and an output push per fixture that moved. Task 29's finding was that
  the compute is the small half.
- **`StationReporter` already publishes one row about this station every 2 s**
  (`infra/stations.rs:24`), replacing it whole rather than patching it, and is a
  separate task from the engine actor.
- **`demo-seed.mjs` awaits one round trip at a time** through `demo-ws.mjs`, which
  has a hard 5 s per-request timeout and no concurrency.

## Goals / Non-Goals

**Goals:**

- One command produces a rig of a stated size, and one command produces a number
  for it, both reproducible by someone who was not here when they were written.
- The number distinguishes the compute half from the apply half, because that is
  the distinction `multithreading` will act on.
- The measurement survives this change — it lives in the product, not in a patch
  someone applies and reverts.

**Non-Goals (design-level, beyond the proposal's):**

- No third figure separating "reading the collections" from "applying the
  effects". Two figures satisfy the spec's requirement, which is that not
  everything be attributed to playback. The read cost is separable by comparing
  presets rather than by counting it, and a third counter stays cheap to add.
- No sampling or histogram. Mean and max over a 2 s window, which is what the row
  is published on anyway.

## Decisions

### The numbers go on `Station`, SYNCED, as one optional struct

`Option<TickCost>` on `Station` beside `cpu_percent`, carrying mean, max, the same
pair for the playback part, and the number of ticks the window contained.

*Why the `stations` row.* It is already "a station's own account of itself",
already published on an interval, already the sole-authority-per-row pattern that
needs no arbitration, and already replicated — so `--measure` can read every
station's figures from whichever one it is connected to, and `system-stats-panel`
gets the number without a second mechanism. This answers half of that item's open
question ("extend `Station` rows vs. a new LOCAL stats collection") in favour of
the row.

*Why SYNCED rather than LOCAL.* A session's stations do the same work on different
hardware, and the interesting reading during a two-station demo is the pair. LOCAL
would make `--measure` connect to each station separately to learn what the
`stations` collection would have told it.

*Why one `Option<struct>` rather than four optional fields.* The spec requires
"not ticking" to be distinguishable from "ticking instantly", and one absence is
one thing. Four independent `Option`s admit states the machine was never in — the
same argument `StationReporter` already makes for replacing the row whole.

*Alternative rejected: a new LOCAL `stats` collection.* It buys nothing this
change needs and it multiplies the places a station describes itself. If later
stats do not fit a row (a ring buffer of recent ticks, say), that is when a
collection earns its place.

### The engine accumulates into atomics; the reporter drains them

An `Arc<TickStats>` of relaxed `AtomicU64`s — sum, max and count of microseconds,
for the whole tick and for the playback part — shared between the engine and
`StationReporter`. The engine adds to it at the end of each tick it actually
performs. The reporter swaps the counters to zero when it publishes, so "mean over
the window" and "worst in the window" both fall out of a drain, and a window with
a count of zero publishes `None`.

*Why atomics.* The spec requires that measuring not scale with the rig and not
produce a replicated write per tick. Three relaxed adds and a compare-exchange for
the max are constant and are noise beside a tick measured in milliseconds.

*Alternative rejected: a new `EngineCommand` to ask for the stats.* It puts the
reporter's request on the same queue as the writes it is trying to measure, so the
act of measuring queues behind the thing being measured.

*Alternative rejected: a `Mutex`.* A lock on the tick path contended by a timer
every 2 s, to protect three integers.

### "The tick" means `playback_tick`, and says so

Timing starts after the early return at `engine/mod.rs:803` and ends when the
apply loop is done. `push_output_config` and `flows_tick` are outside the number.

*Why exclude them.* Two reasons, and the second is the load-bearing one. They are
not proportional to the rig, so they measure a different question. And they run
whether or not playback had work — so including them would mean every timer firing
is a tick, "a station that is not ticking" would never be observable, and the
spec's requirement about a settled show would have nothing to attach to.

The consequence to state plainly: this number is what playback costs, not what the
process costs. What the process costs is `cpu_percent`, in the same row, which is
why both are printed side by side.

### Seeding stays on the WebSocket API, with a bounded window

Writes go in flight together, but through a bounded window (order of 64 in flight),
not an unbounded `Promise.all`.

*Why bounded.* The engine's command channel is 256 deep. Firing two thousand
writes at once does not go faster; it fills the channel, and the backpressure then
shows up as `demo-ws.mjs`'s fixed 5 s timeout rejecting requests that were only
waiting their turn. So the timeout becomes settable and the window keeps the socket
inside what the actor can absorb.

*Why not a direct showfile write for the big presets* (the alternative considered
and declined): it is a second way to build a show, it skips the engine, the oplog
and every validation on the way in, and it can drift from the real write path
silently — which is exactly the failure `demo-seed.mjs`'s header says the script
exists to prevent. A 2000-fixture seed is the largest exercise of the write path
anything in this repo performs, and that is worth more than the minutes it costs.

### The presets are shaped like a rig, not multiplied

- **Addresses spread across universes.** 2000 six-channel heads is roughly 24
  universes; a preset that put them all in universe 1 would exercise the DMX dedup
  cache wrongly and would not be a rig anyone could patch.
- **A cue captures a slice, not everything.** 300 cues × 2000 fixtures would be
  600,000 captures, which measures JSON rather than lighting. Each cue takes a
  fraction of the rig, which is both smaller and more like a real cue stack.
- **At least one effect is up in the seeded state**, because a station with no
  effect running settles and stops ticking, and a preset that cannot be measured
  without manual intervention is not the preset this change is for.
- **Plans go over HTTP, not the WebSocket.** A `StagePlan` names an asset by
  sha256, so seeding one means POSTing image bytes to the asset endpoint. That is
  the same public API by a different verb, not an exception to the rule above.

### `--measure` reads the row, it does not profile

`scripts/demo.sh --size <preset> --measure` brings a station up, seeds, sets the
show running, waits out a few report intervals so the window it reads is a settled
one, then reads `stations` over the same WebSocket API and prints the table. No
external profiler and no privileged access, so the number it prints is the number
the Stations panel will show and the number a peer sees.

## Risks / Trade-offs

- **Seeding `huge` takes minutes even windowed** → progress output while it runs,
  and `--keep` already exists to carry a seeded show over so nobody pays it twice
  in a session.
- **A mean over a 2 s window hides an overrun** → the max is published beside it
  and the tick count says how much of the window was busy. This is the reason the
  spec asks for both.
- **The atomics are on the tick path** → relaxed ordering, constant count, and a
  drain that touches them once every 2 s. Worth confirming on the `huge` preset
  that the figure does not move when measurement is switched off, since that
  preset is the one where it would show.
- **`huge` writes a large oplog while seeding** → bounded by `history_depth`
  already; the pruner does not know or care that these writes came from a script.
- **Someone reads the tick figure as "what the console costs"** → it is not; it
  excludes flows and output config. Mitigated by printing it beside `cpu_percent`
  and by saying so where it is printed.

## Migration Plan

Additive and backwards-compatible. The new fields are `#[serde(default)]` on
`Station`, so a peer running an older build sends a row that still deserialises and
reads as "not reported" — which the spec requires anyway for a station with nothing
to say. `cargo run -p pult-codegen -- generate` after the schema edit.

No impact on the sync protocol: `ShowState` is serde-derived over collections, so a
new field on an existing entity rides along with nothing enumerating it. No impact
on the WIT contract: a plugin learns entities through runtime introspection, so the
field is visible to plugins without a contract version bump.

`--size` defaults to `small`, which is today's show unchanged, so every existing
`scripts/demo.sh` invocation seeds exactly what it seeded before.

## Open Questions

- **What `big` and `huge` are exactly** — 500/2000 fixtures and 60/300 cues are
  task 29's sizes and a guess respectively. The right numbers are the ones where
  the console is comfortable, working hard, and past keeping 40 Hz; they can be
  tuned once the presets exist without touching the spec or the approach.
- **Whether the tick figure wants a share-of-budget beside it** (`7.9 ms (32%)`, as
  the roadmap table prints it). Presentation only, and `TICK` is a constant the
  reader already has, so this can be settled when the table is written.
