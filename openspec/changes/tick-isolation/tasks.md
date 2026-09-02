## 1. A typed view of the show

- [ ] 1.1 Add a `pult-codegen` output that emits a Rust module into `pult-backend` holding one typed collection per registered entity, generated from the `EntityMeta` inventory beside the SQL migration it already writes; verify a fresh `cargo run -p pult-codegen -- generate` produces a module that compiles and that adding a throwaway entity to the schema makes it appear with no hand edit
- [ ] 1.2 Build `PlaybackView` over that module — the whole show typed, rebuilt per collection from `ShowState` — with a `version` per table; verify by a test that a view built from a `ShowState` round-trips to the same values the JSON holds
- [ ] 1.3 Invalidate a collection's view entry only when a **non-LOCAL** field of it is written, asking `EntityMeta::field_lifecycles` rather than naming fields; verify by a test that writing `live_values` on a fixture leaves the `fixtures` view version untouched while writing `address` bumps it
- [ ] 1.4 Have `playback_tick` read the view instead of calling `read_collection` six times; verify every existing playback and engine test still passes unchanged
- [ ] 1.5 Verify the win on the bench that motivated it: `scripts/demo.sh --measure --size huge` shows the tick's reading figure no longer dominating, and record the before/after

## 2. Three figures instead of two

- [ ] 2.1 Extend `TickCost` with the reading figure so the split is reading/computing/applying, keeping mean and worst for the whole tick; verify by a unit test that the three account for the whole tick between them and that an empty window still reports nothing rather than three zeroes
- [ ] 2.2 Update `scripts/demo-measure.mjs` to print the three-way split; verify against all three presets
- [ ] 2.3 Verify a station running an older build still deserialises — the field is additive and defaulted, as `tick_cost` itself was

## 3. One frame per tick

- [ ] 3.1 Introduce a `PlaybackFrame` carrying a tick's whole result — moved fixtures' live state, cue activations, follow-cue Gos — and have the engine apply it entry by entry, broadcasting per path exactly as now; verify the WebSocket traffic a client sees is byte-identical for a fade by comparing captured broadcasts before and after
- [ ] 3.2 Replace the per-fixture `apply_local` calls in `playback_tick` with one frame; verify by a test that a fade over many fixtures produces one engine command per tick rather than one per fixture
- [ ] 3.3 Verify the applying figure has not regressed on `huge`, since fan-out moved rather than went away

## 4. Playback on its own thread

- [ ] 4.1 Publish `PlaybackView` over a `tokio::sync::watch` and have the engine update it after applying a write that invalidated a collection; verify by a test that a write is visible to a `borrow()` from a non-async context on the next publish
- [ ] 4.2 Move the playback loop onto a named `std::thread` outside the runtime, scheduling to absolute 25 ms boundaries and skipping a boundary already passed rather than running it late; verify by a test that a deliberately stalled tick does not push subsequent ticks out of phase
- [ ] 4.3 Verify values are unchanged across the move: a cue fade sampled at a series of wall-clock instants produces the same levels before and after, and `two_stations_that_took_the_cue_at_different_moments_still_agree` still passes
- [ ] 4.4 Verify the tick survives a blocked runtime: a test that occupies every tokio worker thread with a blocking sleep and asserts playback still ticks and output still leaves

## 5. Disk off the actor

- [ ] 5.1 Move `persist`, `oplog::append`, `order::save` and `delete_one` behind a single writer task owning the pool, the actor handing over work and a `oneshot`; verify by a test that writes reach the showfile in the order they were applied
- [ ] 5.2 Keep a client's write acknowledged only once durable, by forwarding the writer's answer; verify by a test that a `Set` that fails to persist still reports the failure to the caller, and that a reply never arrives before the row is on disk
- [ ] 5.3 Verify the actor no longer awaits the pool anywhere in a command arm, by inspection and by a test that a deliberately slow writer does not delay an unrelated `Get`

## 6. No source starves another

- [ ] 6.1 Give `EngineHandle` a source tag — plugin, client, peer, station — and take work per source round-robin from a bounded queue each; verify by a test that a source flooding its queue does not delay another source's write
- [ ] 6.2 Make an over-quota source wait rather than receive an error; verify by a test that a plugin writing far beyond its budget sees every write succeed, slowly, and receives no error
- [ ] 6.3 Verify a plugin cannot make an operator wait: a test with a plugin writing continuously and a client write asserted to complete promptly

## 7. The guarantee, under load

- [ ] 7.1 Write the hostile-load test: the `huge` preset running, a plugin writing continuously, a client reading every collection repeatedly, and the showfile being written, all at once; assert on the station's published `TickCost` that no tick exceeded the 25 ms budget and that the window contains a full complement of ticks
- [ ] 7.2 Verify each load source alone as its own case, so a failure says which one broke it
- [ ] 7.3 Verify the whole thing by hand once with `scripts/demo.sh --measure --size huge` and record the numbers beside the ones this change started from
- [ ] 7.4 Decide, with the test in front of you, whether it is stable enough for CI or belongs beside `demo-shows`' hand-run measurement; record which and why

## 8. Writing it down

- [ ] 8.1 Update the architecture notes in `CLAUDE.md` — the typed view, what invalidates it, the playback thread, and the rule that the engine may not await the disk; verify the commands quoted there still run as written
- [ ] 8.2 Add the roadmap entry, carrying the before/after numbers and the correction that the cost was reading rather than applying; verify it says plainly why threads were not the answer to the speed and were the answer to the isolation
- [ ] 8.3 Mark `multithreading` done in `openspec/BACKLOG.md`, recording that partitioning across stations remains open and that task 10's question is untouched; verify the Order section is renumbered and the `→` prerequisites still resolve forward

## 9. The whole thing

- [ ] 9.1 Verify `cargo test` and `cd frontend && npm test && npm run check` pass with zero warnings from either build
- [ ] 9.2 Verify a showfile written before this change opens unchanged, and that a station on this build and one on the previous build sync a show between them
- [ ] 9.3 Verify `scripts/demo.sh --two --size big` still brings up two stations that agree on the rig, since playback moving threads is exactly the change that could make two consoles disagree
