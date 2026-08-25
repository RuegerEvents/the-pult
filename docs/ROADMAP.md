# Roadmap

State of the system and what to work on next. Reconstructed from the code on 2026-08-25.

## Where the system stands

| Layer | State |
|---|---|
| `pult-schema` + `pult-macros` | Working. The derive macro generates entity meta, patch/create types, accessors, and SQL. All 30 workspace tests live here. |
| `pult-codegen` | Working and idempotent. TypeScript types, the `data.ts` proxy, and the SQL migration all come from the `EntityMeta` and `CommandRegistration` inventories. |
| Showfile (SQLite) | Working. Load and save are registry-driven and enumerate no entity types. |
| WebSocket API | Working. Path-pattern subscribe, set, call, and broadcast fan-out. |
| Session discovery | Working. mDNS advertise and browse, create, join, leave. |
| Peer sync | Partial. TCP handshake, a full snapshot when a peer joins, and live `SyncedBroadcast` fan-out. Nothing else. |
| Frontend | Working for show, session, sequences, and cues. The typed proxy runs end to end. |
| DMX output | Not started. `infra/connectors/mod.rs` is a two-line stub. |
| Playback engine | Not started. |
| WASM plugins | Not started. `infra/plugins/mod.rs` is a stub. |

## Task list

Ordered. Each task is meant to end in its own commit with tests passing.

### 1. Cover the engine with tests (done)

Nothing outside `pult-schema` had a single test. The path dispatch and lifecycle routing in `ShowEngine::apply_set` is the most intricate code in the backend and the least protected, and task 2 rewrites it. Write the tests first so the rewrite has something to fail against.

Covers set, get, create, delete, command dispatch, LOCAL vs SYNCED vs PERSISTED routing, ordering, broadcasts, the snapshot round trip, and peer operations.

Writing them found a bug: deleting a sequence or a fixture silently did nothing and reported success, because the generic field-patch arm in `apply_set` came before the `__delete` arm and serde dropped `"__delete"` as an unknown field.

### 2. Drive engine dispatch from the registry (done)

Was the most important task. `CLAUDE.md` promises that adding an entity collection means editing `ShowState` and nothing else. `engine/mod.rs` breaks that promise in four places:

- `ShowState::get_by_path` hand-matches every collection key
- `ShowEngine::apply_set` has per-entity arms for patch, `__create`, and `__delete`
- `ShowEngine::apply_entity_result` matches on the entity key again
- `ShowState::FRONTEND_PATHS` is a hand-kept list

The drift is already visible. `FixtureType` has a `#[pult(table = "fixture_types")]` and a full schema, but the backend never mentions it. It is not in `ShowState`, never loads, and no path reaches it. It exists only as a TypeScript file.

`ShowState` now holds entities as JSON keyed by table and routes every read and write through `EntityMeta`. No entity type is named in `engine/mod.rs`, and `FixtureType` works with no code written for it.

Two more breaks of the same rule turned up and are fixed. Accessor path keys were camelCase while the wire is snake_case, so every field write through the Rust accessor API silently did nothing. And `ShowDataRoot`'s collection accessors were hand-written, listed three of the five tables, and pointed `fixture_types` at a table that has never existed.

Two things left open, both worth doing, neither blocking:

- Collection order is deterministic in memory but not persisted. After a reload the order comes from sorted UUIDs rather than creation order. Sequences need an order column, or the showfile needs to preserve row order.
- Commands do not write to SQLite. That is right for `go_next`, which moves SYNCED playback state and should not touch the disk on every Go press, but a command that changes a PERSISTED field would not survive a restart.

### 3. Playback engine (next)

`go_next` moves an index. That is the whole of playback today. These fields are modelled and unused:

- `Cue::fade_in_ms`, `Cue::fade_out_ms`
- `Cue::follow_mode` including `FollowAfter` and `Timecode`
- `Cue::is_active`
- `Fixture::live_values`
- `Show::is_running`

Build a tick loop that interpolates `Cue::captures` into `Fixture::live_values` over the fade time, marks cues active, and fires follow cues on schedule. `live_values` is SYNCED, so peers and frontends both see the output without new plumbing.

### 4. DMX output

Art-Net first, sACN after. Read `Fixture::live_values`, map through `FixtureType::parameters` to DMX channels, and send universes at a fixed rate. Until this exists the project cannot control a light.

### 5. Sync catch-up and conflict handling

Can run in parallel with 3 and 4.

A peer that drops and reconnects gets a fresh full snapshot. There is no replay. The `oplog` table exists and `OperationRequest` and `OperationBatch` are declared but stubbed. `VectorClock` is merged on receive and then never read, so concurrent edits resolve by arrival order.

Also missing: heartbeat timeout handling, and leader re-election. `LeaderChanged` is declared in the protocol and never sent.

### 6. Fixture and patch UI

The frontend has panels for show, session, and sequences. Patching a rig is not possible from the UI at all. Depends on task 2 for `FixtureType` to exist in the backend.

### 7. Housekeeping

- `crates/pult-schema/bindings/` is tracked in git but is ts-rs intermediate output. It should be ignored like `frontend/src/lib/generated/` is, otherwise running codegen keeps producing untracked files.
- Two unused `futures::StreamExt` imports and one unused `leader_node_id` binding produce warnings on every build.
- Five `a11y_autofocus` warnings from `svelte-check`, in `SequenceRunner.svelte` and `ShowPanel.svelte`.

### 8. WASM plugins

Last. Nothing else depends on it, and the plugin API should be designed against a system that already plays back cues and outputs DMX.
