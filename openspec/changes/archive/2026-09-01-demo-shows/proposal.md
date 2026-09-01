## Why

Task 29 measured what a tick costs and found a real bug doing it — `ShowView`
scanned the fixture slice for every lookup, which made the tick quadratic in the
size of the rig. Nothing noticed until 2000 fixtures were in front of it. Three
things about that are still true and are the reason for this change.

- **The rig it was measured on is gone.** It was ad-hoc, built by hand, described
  in the roadmap as a table and nowhere as a thing you can run. There is no
  `[[bench]]`, no criterion, no fixture generator: the numbers in the roadmap can
  be re-derived but not reproduced.
- **The instrumentation is gone too.** `crates/pult-backend/src/engine/mod.rs:835`
  calls `self.playback.tick(...)` and nothing times it. Whatever produced "2.0 ms
  (8% of the 25 ms budget)" was added to get the number and taken out again, so
  the next person to ask starts from nothing.
- **The only show anybody can start in one command has five fixtures and three
  cues.** `scripts/demo-seed.mjs` seeds three dimmers and two heads. That is the
  right size for looking at the UI and the wrong size for every question about
  what the console costs — and the bug task 29 found was invisible at that size
  by construction, because the scans only happen on ticks that do work.

`multithreading` is the next change and it is judged entirely on numbers: the
backlog's own note is "do the cheap win, then measure again before adding
threads". It cannot start without a rig to run and a number to read, and neither
exists. Building both once, in the repo, is cheaper than building them ad-hoc
again — and the thing that gets built is also what `system-stats-panel` wants and
what makes a quadratic scan visible the next time one is introduced.

## What Changes

- **The demo has sizes.** `scripts/demo.sh --size <small|big|huge>`, passed
  through to `demo-seed.mjs`. `small` is exactly today's show and stays the
  default, so every existing invocation seeds what it seeded before. `big` is
  roughly 500 fixtures, 60 cues, several sequences and an effect up — the size at
  which task 29 saw 24% of a core. `huge` is roughly 2000 fixtures, 300 cues,
  several stage plans and effects running — the size at which the tick stops
  keeping 40 Hz, which is the interesting one, because the console's behaviour
  there is a claim the roadmap makes ("a slow tick loses smoothness, not
  correctness") and nothing tests it.
- **A station publishes what its own tick costs.** New SYNCED fields on
  `Station`, beside `cpu_percent` and `computes_fixtures`, which is already the
  row where a station says how hard it is working. Two numbers, not one: the
  whole tick, and the `Playback::tick` part of it. Task 29's finding was that
  those differ by roughly three times and the cheap win is in the *other* half —
  the per-fixture `apply_local`, broadcast and output push — so a single figure
  would hide the thing `multithreading` is about to go after.
- **Seeding stays entirely over the WebSocket API.** `demo-seed.mjs` says in its
  own header that nothing in it is privileged and that this is the point: it
  speaks the protocol the frontend speaks, so drift fails loudly. A 2000-fixture
  seed is the largest exercise of the write path anything in the repo performs,
  which makes keeping it on that path worth more than the time it costs. What
  changes is that writes go in flight together instead of one awaited round trip
  at a time, and `demo-ws.mjs`'s fixed 5 s per-request timeout becomes something a
  large seed can raise.
- **`scripts/demo.sh --measure`** brings a station up on the chosen preset, lets
  it settle with an effect running, and prints the table task 29 printed by hand —
  preset, fixtures, cues, tick, playback share, CPU — read from the station's own
  row over the same WebSocket API rather than from an external profiler. One
  command, a number, and the same number the Stations panel would show.

## Capabilities

### New Capabilities

- `observability/tick-cost`: A station measures how long its own tick takes and
  publishes it the way it already publishes CPU and memory — including what it
  reports when a settled show is not ticking at all, and what it reports when the
  tick is over budget, which is the case the measurement exists for.

### Modified Capabilities

None. The size presets and the `--measure` runner are development tooling: they
add no requirement about how the console behaves, and specs describe behaviour.
The station measuring itself is the one part of this change that is product
behaviour, and it is the one part with a spec.

## Non-goals

- **No CI budget and no regression gate.** The backlog asks whether tick cost per
  preset should be recorded in CI; the answer here is not yet. A threshold needs a
  number that holds still, shared runners do not give one, and a gate that flaps
  gets disabled — which is worse than no gate. Revisit once `multithreading` has
  moved the numbers and we know how stable they are.
- **No criterion, no `[[bench]]`.** The thing worth measuring is a whole station
  under load, not a function in isolation; a harness that runs `Playback::tick`
  without the engine around it would miss the half of the cost that task 29 found.
- **Nothing about what the tick does changes.** No per-key `live_values` writes,
  no partitioning, no threads. Those are `multithreading`, and this change exists
  so that they can be judged.
- **No panel.** Reading tick cost in the UI is `system-stats-panel`. This change
  puts the number in the row it will read; showing it is that change's business.
- **The presets are not a seeded showfile format.** There is no `--write` that
  produces a `.pult` file offline. Everything goes through a running station.

## Impact

- `crates/pult-schema/src/types/station.rs` — new SYNCED fields; `serde(default)`
  so an older peer's row still deserialises. Requires
  `cargo run -p pult-codegen -- generate`.
- `crates/pult-backend/src/engine/mod.rs` — timing around the tick handler, and
  around the `playback.tick` call inside it.
- `crates/pult-backend/src/infra/stations.rs` — carries the new numbers into the
  row it already publishes every couple of seconds.
- `scripts/demo-seed.mjs` — size presets and generated rigs; `scripts/demo-ws.mjs`
  — pipelined writes and a settable timeout; `scripts/demo.sh` — `--size` and
  `--measure`.
- Answers half of an open question in `system-stats-panel` ("extend `Station` rows
  vs. a new LOCAL stats collection") in favour of extending the row, on the
  grounds that a station is already the sole authority on its own numbers there.
