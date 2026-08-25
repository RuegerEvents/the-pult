# Roadmap

State of the system and what to work on next. Reconstructed from the code on 2026-08-25, then reconciled against [SPEC.md](SPEC.md).

The spec is the product. This is the build order for getting there, and right now the gap is very wide: what exists is a synchronised show-state engine with cues and playback. The spec's 3D programmer, geometric selections, phasers, event system, and waveform timecode are all still ahead.

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
| Playback engine | Working. Fades, active-cue tracking, and FollowAfter cues at 40 Hz. |
| Output plugins | Not started. `infra/connectors/mod.rs` is a two-line stub, so nothing reaches a light. |
| WASM plugins | Not started. `infra/plugins/mod.rs` is a stub. |
| 3D programmer | Not started. The largest piece of the spec and none of it exists. |
| Selection, effects, events, timecode | Not started. No schema for any of them yet. |

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

### 3. Playback engine (done)

`model::playback` fades captures into `Fixture::live_values`, marks the played cue active, and fires `FollowAfter` cues. It is a pure state machine driven by the engine's own tick, so its tests run a four-second fade in microseconds.

`Timecode` follows still do nothing. The spec wants waveform-based timecode with beat grids rather than plain SMPTE, so this should wait for that design rather than get a stopgap.

`Show::is_running` is still unused. Its meaning is a product decision, not a code one.

### 4. Output plugin layer (next)

The spec is explicit that this is a plugin layer, not a DMX-shaped core: output plugins translate high-level data into whatever protocol a fixture speaks, DMX among them, and network-based communication is preferred over DMX-centric workflows.

So the first piece is the trait and the registry, not Art-Net. An output plugin takes fixture state and a `FixtureType`, and emits protocol frames. Art-Net is then the first implementation of it, mapping `live_values` through `FixtureType::parameters` to channels at 40 Hz. sACN after that.

Two things from the spec to design in from the start rather than retrofit:

- Send parameter changes, not continuous full frames. Playback already skips fixtures that did not move, but DMX itself needs a full frame per universe, so the change-only path has to live above the protocol.
- Fixtures are meant to behave like processing nodes that can preload upcoming playback data. That argues for handing a plugin a description of what is coming, not only the current frame.

### 5. Fixture and patch UI

There is no way to patch a rig from the UI at all. Anything real needs this, and the spec's 3D programmer will need the same underlying data.

### 6. Sync catch-up and conflict handling

Can run in parallel with the output work.

A peer that drops and reconnects gets a fresh full snapshot. There is no replay. The `oplog` table exists and `OperationRequest` and `OperationBatch` are declared but stubbed. `VectorClock` is merged on receive and then never read, so concurrent edits resolve by arrival order.

Also missing: heartbeat timeout handling, and leader re-election. `LeaderChanged` is declared in the protocol and never sent.

### 7. Housekeeping

- `crates/pult-schema/bindings/` is tracked in git but is ts-rs intermediate output. It should be ignored like `frontend/src/lib/generated/` is, otherwise running codegen keeps producing untracked files.
- Two unused `futures::StreamExt` imports and one unused `leader_node_id` binding produce warnings on every build.
- Five `a11y_autofocus` warnings from `svelte-check`, in `SequenceRunner.svelte` and `ShowPanel.svelte`.

### 8. WASM plugins

Nothing else depends on it, and the plugin API should be designed against a system that already plays back cues and drives output.

## Further out

Everything below is in the spec and has no schema, no code, and no design yet. Listed so the near-term work does not paint itself into a corner.

**Fixture positions.** The spec wants axial (position plus direction vector) or positional (XYZ) coordinates on every fixture, with tracking data feeding in later. `Fixture` has no position field at all. This is small to add and everything spatial depends on it, so it is the cheapest thing on this list to get right early.

**Selection as a geometric query.** Selections are meant to be generated from the rig by geometric functions and re-evaluated as the rig changes, not stored as fixture lists. That is a query language, and it needs positions first.

**Effects and phasers.** Derived from the 3D selection with modifiers that can themselves be dynamic. Needs selection.

**3D programmer.** Rig view, fixture puppeteering, quicksheets. The biggest single piece of the product and entirely absent.

**Event system and node graph.** Sensor and network triggers, delays, reactive playback. Cuts across everything.

**Waveform timecode and "timecode without timecode".** Beat grids, markers, live audio analysis for band sync. Should subsume the `Timecode` follow mode rather than sit beside it.

**Open control interfaces.** OSC, MIDI, and control surfaces alongside the existing WebSocket API.
