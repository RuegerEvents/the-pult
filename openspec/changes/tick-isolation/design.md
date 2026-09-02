## Context

See `proposal.md` — Why, for the measurement. The structural facts that shape the
approach:

- **One actor, one queue.** `ShowEngine::run` is a `tokio::select!` over an mpsc of
  depth 256 and a 25 ms ticker. Whichever branch is taken runs to completion before
  the other can be looked at, so any awaited work in a command arm is tick jitter.
- **`ShowState` holds JSON** (`serde_json::Map` keyed by table) precisely so that
  `engine/mod.rs` names no entity type. `get_by_path` returns a **clone**.
- **`EntityMeta` already carries what code generation would need**: `entity_name`,
  `table_name`, `is_singleton`, and — the one that turns out to matter —
  `field_lifecycles`, which says which fields of an entity are LOCAL.
- **`pult-codegen` already writes Rust-adjacent artifacts into this crate**: the SQL
  migration at `main.rs:277`. Generating a Rust module here is a new file, not a new
  idea.
- **`Fixture` mixes two clocks**: `address`, `position`, `home_values` change when
  somebody patches; `live_values`, `live_effects`, `live_fades` change forty times a
  second. They are one struct, which is why re-reading the patch costs the live data.
- **Output is already isolated.** `OutputHandle::push` is a `try_send` that drops
  when behind. That is the pattern; the rest of this is applying it inwards.

## Goals / Non-Goals

**Goals:**

- Playback's deadline is not shared with work that has no deadline.
- Reading the show costs the tick nothing that grows with the rig.
- The isolation survives somebody later adding a slow await to the engine, because
  that is exactly how the current state arose.
- Every property the JSON substrate buys — a new collection costing no hand edit,
  serde-derived snapshots, generic path verbs — comes out unchanged.

**Non-Goals (design-level):**

- No change to the WebSocket protocol, the WIT contract, or the showfile format.
- No lock shared between the playback thread and the engine on a per-tick path.
  A lock is a way for one to wait for the other, which is the thing being removed.

## Decisions

### JSON stays the authority; playback gets a typed view derived from it

This refines the shape asked for — *hold the whole structure in memory as its own
types* — rather than adopting it whole, and the reason is that the engine serves two
masters with opposite preferences.

- **Writes arrive as JSON** — from the socket, from a plugin, from a peer.
- **Broadcasts leave as JSON**, one per write, on the hot path.
- **Playback wants types**, and is the only reader that does.

Storing JSON and converting for playback pays once per *change to show data*.
Storing types and converting for the wire pays on every write and every broadcast —
which is more often, and on the path that already works. So: `ShowState` keeps its
`serde_json::Map`, and a **`PlaybackView`** holds the whole show typed beside it,
rebuilt per collection when that collection changes.

The whole show *is* held in memory as its own types. It is derived rather than
authoritative, which is what keeps the wire cheap.

*Alternative considered and recorded rather than dismissed:* a fully typed
`ShowState` with generated path dispatch. It is the cleaner object, it removes the
view's invalidation surface entirely, and `{T}Patch` / `{T}Accessor` from
`pult-macros` already exist to make field-level writes typed. It is worth
reconsidering if broadcasts ever stop being per-write JSON — at that point the
argument above reverses. It is not worth doing first, because it rewrites the
engine's write path to win a read path that the view already wins.

### A LOCAL field write does not invalidate the view

The trap this design exists to avoid: if the view is invalidated by *any* write to a
collection, then playback's own frame — which writes `live_values` for every fixture
that moved — invalidates `fixtures` every tick, and the next tick rebuilds all two
thousand of them. That is the current cost with extra steps, and it is exactly how a
naive cache would have failed here.

The way out is *not* `EntityMeta::field_lifecycles`, which was the first answer and
is wrong: `live_effects` and `live_fades` are declared LOCAL but **`live_values` is
declared SYNCED** (`types/fixture.rs:269`), so a rule keyed on the declared field
lifecycle would invalidate `fixtures` on every tick and win nothing — the exact
failure it was written to avoid.

The rule is keyed on the lifecycle the **write** carries instead. Playback applies
its output through `apply_local`, which passes `Lifecycle::Local` whatever the field
declares, and that is available at the call site with no lookup. So: a write applied
as LOCAL does not invalidate the view. Patching a fixture rebuilds `fixtures`; a fade
does not touch the view at all.

Worth resolving separately: a field declared SYNCED that is only ever written LOCAL
is either a mislabel or a replication path nothing uses. Every station derives its own
live values from replicated cue state, so there is nothing for the SYNCED declaration
to do — but this change should not quietly redefine it, and a wrong label here is a
trap for the next reader.

This names no entity type and no field: it asks the registry. A new collection, or a
new live field on an existing one, is covered by declaring its lifecycle, which is
already required.

*What playback does about the live values it no longer reads:* it owns them. They are
derived from what playback itself computed last tick, so reading them back out of the
engine was always a round trip to fetch its own output.

### Playback runs on a thread, reading a `watch` channel

`std::thread::Builder::new().name("pult-playback")`, outside the tokio worker pool.
A thread rather than a task because the guarantee has to hold against a future
blocking call on the runtime — the failure being fixed here is that everything was on
one executor and nobody meant it to be.

The view reaches it through `tokio::sync::watch`, already used in this codebase for
`PeerLinks`. `watch::Receiver::borrow()` is synchronous, so the playback loop needs no
runtime, and the engine publishing a new view never waits for a reader. No new
dependency; `arc_swap` would do the same job and would be one.

*Scheduling:* the loop sleeps to the next absolute 25 ms boundary rather than
sleeping 25 ms, so a slow tick does not push every later tick out. Where a boundary
has already passed, it is skipped rather than run late — playback computes from the
wall clock, so skipping is exactly the graceful failure the spec requires and running
late is not.

### The tick emits one frame, and the engine fans it out

Playback sends **one** `PlaybackFrame` per tick — the moved fixtures' live state, cue
activations, and any follow-cue Go — instead of the two thousand `apply_local` calls a
fade makes today. One message across the thread boundary, one queue slot, one command
for the actor to handle.

Fan-out stays inside the engine: the frame is applied entry by entry and broadcast per
path exactly as now, which is what keeps the WebSocket protocol untouched. The 2.2 ms
of applying is not the problem and is not what this is trying to move.

### Persistence moves behind one writer, and the reply still waits for it

A writer task owns the pool and takes work on an mpsc. The actor hands over a write
and a `oneshot`, and forwards the answer to whoever asked. Ordering is free: one
consumer, and the pool is `max_connections(1)` already.

**A client's write is still acknowledged only once it is durable.** The actor stops
waiting for the disk; the client does not. That distinction is the whole point — an
acknowledged write means the same thing after this change as before, and the spec's
non-goal about durability holds literally. What is gone is the disk being between the
ticker and its deadline, and with playback off this loop entirely, an actor that waits
is no longer a show that waits.

### Admission is per source, and backpressure is waiting, not failing

`EngineHandle` gains a source tag — plugin, client, peer, station — and the engine
takes work from a bounded queue per source, round-robin. A source at its limit is made
to `await`, not answered with an error: a plugin told "too fast" would report a broken
station, and a browser would show a failed edit, when the truth is that it is being
paced.

Playback is not in this scheme, because it is no longer in the queue at all. That is
the ordering guarantee: an operator pressing Go is a client write, applied by the
actor, published into the view, and read by the next tick — at most one tick of
latency, which is what it is today.

## Risks / Trade-offs

- **The view is a second copy of the show, and copies drift** → it is derived on
  write from the one authority, never written to directly, and rebuilt per collection
  rather than patched, so there is no merge to get wrong. A test asserts the view and
  `ShowState` agree after a long random write sequence.
- **Playback reads a view up to one tick stale** → it already did. `read_collection`
  ran at the top of the tick, so a write landing mid-tick was seen next tick then too.
- **A rebuild of `fixtures` is now on the write path** → patching one fixture
  re-deserialises that collection. At 2000 fixtures that is the 33 ms this change is
  removing, moved to an action that happens by hand a few times an hour. If it ever
  matters, the fix is patching the view entry rather than rebuilding the collection,
  which is a smaller change made later against real numbers.
- **Two thousand live values crossing a thread boundary per tick** → one allocation
  and one message; measure it, because if it is not comfortably under the 2.2 ms the
  in-process version costs, the frame wants to carry a reusable buffer.
- **A hostile-load test is a timing test, and timing tests flake** → it asserts on the
  published `TickCost`, not on wall-clock in the test process, and its threshold is
  the 25 ms budget rather than a tuned number. `demo-shows` deliberately declined to
  put a tick budget in CI for this reason; this one is a *guarantee* rather than a
  performance figure, and if it cannot be made stable it belongs in the same "run it
  by hand" category — decided when it exists, not now.
- **Nothing forces future code to stay off the tick** → the thread is the enforcement.
  A slow await added to the engine tomorrow delays commands, not frames.

## Migration Plan

No migration. The showfile, the protocol and the plugin contract are untouched, so a
station on this build and one on the previous build interoperate and open each
other's showfiles.

`cargo run -p pult-codegen -- generate` gains a Rust output; `CLAUDE.md` already
tells everyone to run it after a schema change, and CI builds with `--locked` so a
stale generated file fails the build rather than drifting quietly.

The order the work lands in matters more than usual, because each step is separately
measurable:

1. The view, with playback still on the actor — the 93% should disappear here.
2. The frame, still in-process — two thousand messages become one.
3. The thread — the isolation, with the two above making the boundary affordable.
4. Disk and admission — the rest of what can stall it.

## Open Questions

- **Whether `Fixture` should be split** so live state is its own collection rather
  than three LOCAL fields on a mostly-static entity. The lifecycle-aware invalidation
  above makes it unnecessary for this change, and the split would reach the frontend,
  every panel reading `fixture.live_values`, and the sync layer. Worth asking again if
  a second reader of live state appears.
- **Whether the view should be published per collection or whole.** Whole is simpler
  and the rebuild is per collection either way; per-collection saves an `Arc` clone
  per unchanged table per publish, which is nothing until it is.
- **What the admission budgets actually are.** Round-robin with a per-source depth is
  the shape; the numbers want the hostile-load test to exist before they mean
  anything.
