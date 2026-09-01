## 1. Extract the evaluator, change nothing

- [x] 1.1 Create the leaf crate with the evaluable types — `ParameterValue`, `RunningFade`, `RunningEffect`, `EffectSpec`, `Curve`, `Easing`, `Shape`, `SpeedMaster` — moved out of `pult-schema/src/types/effect.rs` and `fixture.rs`, depending on nothing that touches an OS; verify `cargo tree -p <crate>` shows no `sqlx`, `tokio`, `inventory` or `ts-rs`
- [x] 1.2 Move the arithmetic — `value_at`, `progress`, easing and interpolation, the effect shapes — out of `model/playback.rs`, leaving the show bookkeeping behind; verify by moving the existing tests with it and having them pass unchanged
- [x] 1.3 Re-export every moved type from its current `pult-schema` path so no other crate edits; verify the workspace builds with zero warnings and `cargo test` passes with no test modified
- [x] 1.4 Change the evaluation signatures from `std::time::Instant` to `u64` console-unix-ms, which is what the objects are anchored in; verify by a test that a fade evaluated at a series of millisecond stamps gives the values the `Instant` version gave

## 2. The station stops materialising

- [ ] 2.1 Give each output connector its own loop over the last patch it was pushed, evaluating rather than reading; verify by a test that a connector emits a moving value across a fade without the engine writing anything between frames
- [ ] 2.2 Push from the engine when the *show* changes rather than when a value does; verify by a test that a fade in progress produces no engine writes and no broadcasts
- [ ] 2.3 Replace `queue_watched_changes` with a sampler proportional to the `watched` set; verify by a test that a flow watching one parameter of a 2000-fixture rig samples one, and that watching nothing samples nothing
- [ ] 2.4 Make "what is this parameter doing now" available where the station needs it — storing into a cue, `__set_home`, the plugin data host functions; verify by tests that each gives the value for the moment it asked, mid-fade
- [ ] 2.5 Remove `Fixture::live_values` and let the compiler enumerate the consumers; run codegen and verify the Rust side builds with zero warnings
- [ ] 2.6 Remove the engine's 25 ms timer once nothing needs it; verify a settled show and a running show both do no periodic engine work, by asserting no writes occur across a fade
- [ ] 2.7 Verify the values are unchanged: a cue fade sampled at a series of instants gives the same numbers as before this change, and `two_stations_that_took_the_cue_at_different_moments_still_agree` still passes

## 3. Everyone agrees on the clock

- [ ] 3.1 Add a clock-offset estimate between a client and the station it is connected to, maintained rather than taken once, in the shape `PeerLink::rtt_ms` already uses between stations; verify by a test that an offset is established over a connection with an artificial delay
- [ ] 3.2 Make a client with no offset yet decline to present values rather than present wrong ones; verify by a test that a client evaluates nothing before its offset exists and says so
- [ ] 3.3 Verify a skewed clock still agrees: a test that deliberately offsets a client's clock and asserts its evaluated values match the station's for the same instants
- [ ] 3.4 Verify a stepped clock re-converges without a reload

## 4. The browser evaluates

- [ ] 4.1 Add the `wasm32-unknown-unknown` + `wasm-bindgen` wrapper exposing a batch entry point — "these parameters, this moment" — and a build step writing into `frontend/src/lib/`; verify the artifact loads in a page and returns a value
- [ ] 4.2 Verify the boundary is not the new cost: a benchmark evaluating a realistic on-screen set per frame, compared against the per-fixture call it replaces
- [ ] 4.3 Rewire the frontend consumers — `stage.ts`, `programmer.ts`, `patch.ts`, `Quicksheet.svelte`, `Rig3D.svelte`, `PatchPanel.svelte`, `FixtureTypeEditor.svelte` — from reading a value to evaluating one; verify `npm run check` is clean and the existing vitest suites pass
- [ ] 4.4 Verify agreement across runtimes: a corpus of fades, effects and programmer states evaluated natively and in wasm, asserted equal — the guard that replaces the twin this change declined to write
- [ ] 4.5 Verify the picture improved rather than merely moved: a fade on the `huge` preset shows no per-value WebSocket traffic, and motion is drawn at frame rate

## 5. What has a deadline now

- [ ] 5.1 Move the published figures from the engine tick to the output frame, per connector, keeping mean and worst and the absent-not-zero rule; verify by tests that a station with no outputs reports absent and one with two connectors reports each separately
- [ ] 5.2 Update `scripts/demo-measure.mjs` to print per-connector frame cost; verify against all three presets
- [ ] 5.3 Verify the win end to end: `scripts/demo.sh --measure --size huge` against the numbers this change started from — 35.2 ms whole, 33.8 ms reading, 0.07 ms computing

## 6. Both directions of a mixed session

- [ ] 6.1 Verify a showfile written before this change opens, with the removed field ignored
- [ ] 6.2 Verify sync tolerates the field in both directions: a station on this build receiving a row that carries `live_values`, and a station on the previous build receiving one that does not

## 7. Writing it down

- [ ] 7.1 Update `CLAUDE.md` — the evaluator crate and its two targets, that a driven value is evaluated and a sensed one is stored, the clock offset, and that connectors own their rate; verify the commands quoted there run as written
- [ ] 7.2 Add the roadmap entry with the before/after numbers and the reasoning chain that got here — that the tick was never a concurrency problem; verify it records why the TypeScript twin was declined
- [ ] 7.3 Mark `values-as-functions` done in `openspec/BACKLOG.md`, record what it leaves open for `rig-viewer-fidelity` and `tick-isolation`, and renumber the Order; verify the `→` prerequisites still resolve forward

## 8. The whole thing

- [ ] 8.1 Verify `cargo test` and `cd frontend && npm test && npm run check` pass with zero warnings from either build
- [ ] 8.2 Verify `scripts/demo.sh --two --size big` brings up two stations that agree on the rig, and that a browser on each shows the same thing at the same moment
- [ ] 8.3 Verify the reference plugins still load and run, since the evaluator moved out from under the host functions they use
