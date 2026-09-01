## 1. A station measures its own tick

- [x] 1.1 Add a `TickCost` type and `Option<TickCost>` to `Station` in `crates/pult-schema/src/types/station.rs` — mean and max for the whole tick, the same pair for the playback part, and the tick count for the window — with `#[serde(default)]`; verify by a unit test that a `Station` JSON written without the field deserialises with the field `None` and every other field intact
- [x] 1.2 Run `cargo run -p pult-codegen -- generate` and verify `frontend/src/lib/generated/` picks up the new type with no hand edits and `cd frontend && npm run check` stays clean
- [x] 1.3 Add a `TickStats` accumulator of relaxed atomics (sum, max, count in microseconds, for the whole tick and the playback part) with a `drain()` that resets and returns `Option<TickCost>`; verify by unit tests that a drain of an untouched accumulator is `None`, that mean and max are right over a known set of samples, and that a second drain immediately after is `None` again
- [x] 1.4 Thread an `Arc<TickStats>` from `pult_backend::start` into both the engine and `StationReporter`; verify the workspace builds with zero warnings
- [x] 1.5 Time `playback_tick` in `crates/pult-backend/src/engine/mod.rs` — the outer span starting *after* the early return at line 803 and ending when the apply loop finishes, the inner span around the `self.playback.tick(...)` call — and record both into the accumulator; verify by a test that a station running an effect records ticks and that the playback figure is less than or equal to the whole-tick figure
- [x] 1.6 Publish the drained figures from `StationReporter::measure`; verify by a test that a station whose show has nothing running publishes a row with tick cost absent rather than zero, and that a station that was ticking and is then taken off publishes absent on the next report rather than repeating its last figure
- [x] 1.7 Verify by a test over two stations in one session that each row carries its own figures, that neither station writes the other's, and that a peer row arriving without tick figures is accepted with the rest of the row intact

## 2. Seeding a rig worth measuring

- [x] 2.1 Give `scripts/demo-ws.mjs` a settable per-request timeout and a bounded-window helper that keeps a fixed number of writes in flight (order of 64, under the engine's 256-deep command channel); verify the existing small seed still runs unchanged through it
- [x] 2.2 Add `--size <small|big|huge>` to `scripts/demo-seed.mjs`, defaulting to `small`; verify that `small` seeds exactly what it seeds today — same fixture types, five fixtures, three cues, one sequence, one speed master, two flows — by running it against a fresh station and comparing the summary line
- [x] 2.3 Generate the `big` rig: fixtures addressed across as many DMX universes as their channel counts need, positioned rather than null, several sequences whose cues each capture a slice of the rig rather than all of it, and at least one effect left up so the station does not settle; verify by seeding a fresh station and reading the collections back over the API
- [x] 2.4 Generate the `huge` rig by the same shape at the larger size; verify the seed completes against a fresh station and report how long it took, since that number decides whether the windowed API path was the right call
- [x] 2.5 Seed stage plans for `huge` by POSTing generated image bytes to the asset endpoint and creating `stage_plans` rows against the returned sha256; verify the plans open in the Plan panel
- [x] 2.6 Verify all three presets seed cleanly against a fresh `.demo/` and that a re-run on a non-empty show still declines to touch it, as the script does today

## 3. Getting a number out

- [x] 3.1 Pass `--size` through `scripts/demo.sh` to the seeder and document it in the header comment that doubles as `--help`; verify `scripts/demo.sh --help` prints it
- [x] 3.2 Add `scripts/demo.sh --measure`: seed the chosen preset, set the show running, wait out enough report intervals for the window read to be a settled one, then read `stations` over the WebSocket API and print preset, fixtures, cues, whole tick, playback share and CPU; verify it prints a table for `small`
- [x] 3.3 Run `--measure` on all three presets and record the numbers; verify `huge` reproduces task 29's shape — playback the smaller half, and the tick past its 25 ms budget at the top size
- [x] 3.4 Verify measurement is not paying for itself: compare the `huge` tick figure against the same run with recording disabled and confirm the difference is inside the noise

## 4. Writing it down

- [x] 4.1 Note `--size` and `--measure` in the Running section of `CLAUDE.md`; verify the commands there run as written
- [x] 4.2 Add the roadmap entry for this task in `docs/ROADMAP.md`, carrying the measured numbers per preset and the decision that "the tick" excludes flows and output config; verify it names what the figure is not, since that is the way it will be misread
- [x] 4.3 Mark `demo-shows` done in `openspec/BACKLOG.md` with a pointer to the archived change, and record that it answered `system-stats-panel`'s open question in favour of extending the `Station` row; verify the Order section still lists the item and the entry text says where the answer went

## 5. The whole thing

- [x] 5.1 Verify `cargo test` and `cd frontend && npm test && npm run check` pass with zero warnings from either build
- [x] 5.2 Verify `scripts/demo.sh --two --size big` brings up two stations that each publish their own tick figures, since a single-console check would not have exercised the requirement that they not be aggregated
