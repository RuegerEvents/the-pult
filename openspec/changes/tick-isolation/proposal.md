## Status: on hold, largely superseded

Do not apply this. It is a good plan for an architecture we have decided to leave.

`values-as-functions` (see `openspec/BACKLOG.md`) makes a live value something each
consumer evaluates rather than something the engine stores, and that removes most of
this change's reason to exist rather than optimising it:

- **The typed `PlaybackView`** — unnecessary. It exists to make a per-tick read of the
  show cheap; there is no per-tick read once nothing materialises values into state.
- **One frame per tick** — unnecessary. There are no per-tick writes to batch.
- **Playback on its own thread** — mostly unnecessary. The loop moves into each output
  connector, which already has one, and the engine has no periodic work left except
  the small sampling that flow `Watch` nodes need.

What survives, and is worth doing on its own terms whatever happens above:

- **Disk off the actor.** An operator's edit should not wait behind another's write.
  Lower priority than it looked, since the disk is no longer anywhere near the show.
- **Per-source admission.** A plugin should not be able to crowd out an operator.
- **A figure for what the frame costs.** But `observability/tick-cost` describes a
  tick that would no longer exist, so what it measures has to be re-decided rather
  than extended — which is one of the open questions on `values-as-functions`.

The analysis below stands and is why the successor exists; the measurement in it is
the reason the plan changed. Re-scope this against whatever `values-as-functions`
decides, rather than trimming it now — the parts that survive depend on answers that
change has not given yet.

## Why

The show runs on one actor, and everything else the station does runs on it too.
`demo-shows` built the instrument that shows what that costs, and pointing it at
2005 fixtures produced a number that changes what this work is:

| Rig | Whole tick | Reading the show | Computing | Applying |
|---|---|---|---|---|
| huge — 2005 fixtures | 35.2 ms | **33.8 ms (93%)** | 0.07 ms (0.2%) | 2.2 ms (6%) |

`playback_tick` calls `read_collection` six times a tick — fixtures, cues,
sequences, fixture types, programmer values, speed masters. Each one clones a
collection out of `ShowState` as `serde_json::Value` and then deserialises it
whole into a `Vec<T>`. Two thousand fixtures, each carrying four `HashMap`s of
live state, go through that forty times a second, so that playback can read the
patch data that has not changed since the show was opened.

**So the backlog's framing was wrong.** `multithreading` proposed parallelising
the render across cores. The render is 0.07 ms. Threads would win nothing.

Three things can stall the show today, and the reads are only the first.

- **The tick stalls on its own reads.** 35 ms against a 25 ms budget, and it grows
  with the rig.
- **The tick stalls on disk.** `persist`, `oplog::append` and `order::save` are
  awaited *inside* the actor's command arm (`engine/mod.rs:1899`, `:2152`,
  `:1995`), against a pool of `max_connections(1)`. The `tokio::select!` cannot
  reach the ticker until the write returns, so an fsync is tick jitter.
- **The tick stalls behind everything else.** Plugins reach the engine through the
  same `EngineHandle` a browser does (`host_impls.rs:207`), into one 256-deep
  channel with no priority. A plugin in a write loop, a browser fetching the whole
  show, and playback all queue together, and playback is what has a deadline.

One part of the station already got this right and is the model for the rest:
`OutputHandle::push` is a `try_send` that drops when the consumer is behind,
documented as "Never blocks the engine … the next tick carries the same state"
(`connectors/mod.rs:82`). Frames keep leaving whatever the engine is doing. That
property should hold for playback itself.

## What Changes

- **The show is held as its own types, and the typed structure is generated.**
  `ShowState` keeps JSON today so that no entity type is named in `engine/mod.rs`
  — the rule that makes a new collection cost no edit outside `pult-schema`. That
  rule is about *hand-maintained* lists, and `pult-codegen` already emits a Rust
  artifact into this crate (the SQL migration, `pult-codegen/src/main.rs:277`) as
  well as the frontend's proxy types. So the typed collections and the path
  dispatch over them are generated from the `EntityMeta` inventory: adding a
  collection still costs nothing but a codegen run, and the tick stops
  deserialising the show to read it.
- **Playback runs on its own thread**, outside the tokio worker pool, holding an
  `arc_swap`ped snapshot of the static show and owning the live state it writes.
  A thread rather than a task because the guarantee has to survive somebody later
  awaiting something slow on the runtime — which is how this happened the first
  time.
- **The tick writes once, not once per fixture.** Today a fade calls `apply_local`
  per moved fixture: two thousand messages, two thousand oplog decisions and two
  thousand broadcasts per tick. Playback emits one batch, which is also what makes
  a thread boundary affordable.
- **Disk leaves the actor.** Persistence and the oplog move behind a single writer
  task with an ordered queue, so a write is still ordered and still durable but no
  longer between the ticker and its deadline.
- **Command traffic is bounded per source.** A plugin, a peer and a browser get
  their own budget rather than sharing one queue first-come-first-served, so no
  one of them can crowd out the others. Playback is out of this contest entirely,
  having left the queue.
- **The tick reports where its time went.** `TickCost` gains the third figure this
  change was designed from — reading, computing, applying — so the next person
  gets the split without adding a counter by hand and reverting it.

## Capabilities

### New Capabilities

- `playback/isolation`: The show keeps its tick whatever else the station is doing
  — what may and may not delay playback, what happens to work that cannot keep up,
  and what a station reports about it.

### Modified Capabilities

- `observability/tick-cost`: the requirement that a station measures the whole tick
  and the playback part separately becomes three figures rather than two — reading,
  computing, applying. The two-figure split proved that computing was not the cost
  and could not say what was; the answer, reading at 93%, was found only by adding a
  counter by hand and removing it again, which is the work the published figures
  exist to save.

## Non-goals

- **No parallel render, and no partitioning across stations.** The render is
  0.07 ms of a 35 ms tick. Task 10's question about partitioning computation
  between consoles stays open and stays unanswered by this; when it is worth
  asking again the numbers will be different, and this change is what makes them
  different.
- **No change to what playback computes.** Same values, same anchoring to the wall
  clock, same output. If a single frame differs, that is a bug in this change.
- **No new durability guarantee.** Writes stay as durable as they are now; they
  stop being on the tick's path. A write that was acknowledged before still is.
- **Nothing about the browser's own cost.** A 2000-fixture rig is hard on the
  frontend too, and that is `rig-viewer-fidelity`.
- **No priority for one operator over another.** Fairness here is between *kinds*
  of work, not between people.

## Impact

- `crates/pult-schema/src/registry.rs` — `EntityMeta` may need to expose enough for
  codegen to name the Rust type of each collection; `entity_name` and `table_name`
  are already there.
- `tools/pult-codegen` — a new generated Rust module for the backend, beside the
  migration it already writes.
- `crates/pult-backend/src/engine/mod.rs` — `ShowState`'s storage and its path
  get/set; the actor loop; the tick moving out.
- `crates/pult-backend/src/model/playback.rs` — the thread, the snapshot it reads,
  the batch it emits.
- `crates/pult-backend/src/infra/showfile/` — the writer task; the pool's single
  connection is the reason ordering is free.
- `crates/pult-backend/src/infra/plugins/host_impls.rs` and `api/ws` — per-source
  budgets.
- **BREAKING for nothing external.** The WebSocket protocol, the WIT contract and
  the showfile format are untouched: this is where the work happens, not what is
  said. A peer on an older build sees no difference.
