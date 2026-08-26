# Roadmap

State of the system and what to work on next. Reconstructed from the code on 2026-08-25, then reconciled against [SPEC.md](SPEC.md).

The spec is the product. This is the build order for getting there, and the gap is still wide: what exists is a synchronised show-state engine with cues, playback, output, and an event system. The spec's 3D programmer, geometric selections, phasers, and waveform timecode are all still ahead.

## Where the system stands

| Layer | State |
|---|---|
| `pult-schema` + `pult-macros` | Working. The derive macro generates entity meta, patch/create types, accessors, and SQL. All 30 workspace tests live here. |
| `pult-codegen` | Working and idempotent. TypeScript types, the `data.ts` proxy, and the SQL migration all come from the `EntityMeta` and `CommandRegistration` inventories. |
| Showfile (SQLite) | Working. Load and save are registry-driven and enumerate no entity types. |
| WebSocket API | Working. Path-pattern subscribe, set, call, and broadcast fan-out. |
| Session discovery | Working. mDNS advertise and browse, create, join, leave. |
| Peer sync | Works and converges. Handshake, bidirectional catch-up from the oplog, live fan-out, heartbeat liveness and latency, vector-clock conflict resolution, and leader failover. Stations publish themselves and are visible in the UI. |
| Frontend | Working for show, session, sequences, cues, and patch. The typed proxy runs end to end. Vitest covers the pure helpers; components are untested. |
| Playback engine | Working. Fades, active-cue tracking, and FollowAfter cues at 40 Hz. |
| Output plugins | Working for Art-Net, sACN, and OpenHaunt nodes, several at once. Configured from the `outputs` collection and editable while the show is up, with per-output status in the UI. Flags only seed an empty showfile. |
| Flows | Working. The spec's node graph, evaluated as a graph: sources, conditions, boolean logic, delays and actions, with live state on every node. Replaced `triggers`. |
| Devices / events | Working. OpenHaunt nodes are discovered over mDNS and adopted as fixtures; their inputs land in `live_values`; flows turn those into cues. Tested end to end against `tools/openhaunt-sim`, which is all there is until there is firmware. |
| WASM plugins | Not started. `infra/plugins/mod.rs` is a stub. |
| 3D programmer | Not started. The largest piece of the spec and none of it exists. |
| Selection, effects, timecode | Not started. No schema for any of them yet. |

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

One thing left open, worth doing, not blocking:

- Commands do not write to SQLite. That is right for `go_next`, which moves SYNCED playback state and should not touch the disk on every Go press, but a command that changes a PERSISTED field would not survive a restart.

### 3. Playback engine (done)

`model::playback` fades captures into `Fixture::live_values`, marks the played cue active, and fires `FollowAfter` cues. It is a pure state machine driven by the engine's own tick, so its tests run a four-second fade in microseconds.

`Timecode` follows still do nothing. The spec wants waveform-based timecode with beat grids rather than plain SMPTE, so this should wait for that design rather than get a stopgap.

`Show::is_running` is still unused. Its meaning is a product decision, not a code one.

### 4. Output plugin layer (done for Art-Net)

The spec is explicit that this is a plugin layer, not a DMX-shaped core: output plugins translate high-level data into whatever protocol a fixture speaks, DMX among them, and network-based communication is preferred over DMX-centric workflows.

So the first piece is the trait and the registry, not Art-Net. An output plugin takes fixture state and a `FixtureType`, and emits protocol frames. Art-Net is then the first implementation of it, mapping `live_values` through `FixtureType::parameters` to channels at 40 Hz. sACN after that.

`OutputPlugin` and `OutputManager` are in place, `connectors::dmx` renders fixtures and their types into universes, and `connectors::artnet` puts ArtDmx on the wire. Art-Net skips universes whose 512 bytes have not changed and refreshes every 800 ms, so an idle rig stays off the network.

sACN followed in task 11, as predicted: `connectors::sacn` is one file beside `artnet.rs`, and the dedup and refresh bookkeeping both protocols need moved into `connectors::dmx` rather than being written twice.

Still to do here:

- The spec wants fixtures to preload upcoming playback data, which means handing a plugin a description of what is coming rather than only the current frame. Nothing here does that yet.

### 5. Fixture and patch UI (done)

Fixture types with their parameters, and a fixture table with name, type, universe, address, position, and live values. New fixtures land at the next free address, and channel overlaps within a universe are highlighted.

`Fixture::position` went in at the same time, which also turned up two things worth knowing: an added column needed a migration path for existing showfiles, and reading an optional column panicked on NULL. Both fixed.

Left open here:

- ~~`subscribeDeep` re-fetches a whole collection on any change beneath it.~~ Done in `caed1bc`: an update names the path that changed, so the local copy is patched rather than re-read, and only a cold start or an entity the client has never seen costs a round trip. What was left after that was not the network but the work done per delivery — `clashingFixtures` compared every fixture against every other one, on every tick of every fade. It sweeps per universe now: a 500-fixture rig went from 32 ms to 0.9 ms per second of ticks, and stopped growing quadratically.
- Position can only be set to the origin from the UI. Editing coordinates, and the axial form, wait for the 3D view.

### 6. Sync catch-up and conflict handling (done)

Peer identity in `HelloAck`, heartbeat liveness with a 16-second timeout, vector-clock conflict resolution, catch-up from the oplog in both directions, and leader failover. Twenty-three tests run real nodes over real TCP.

Leader election needs no messages: the leader publishes the membership, and every survivor removes the lost node from the same list and picks the lowest remaining id. Lowest id rather than freshest state, because catch-up runs both ways on connect, so whoever leads ends up with everything either side had.

One bug found later, worth writing down because it cost an afternoon and looked like something else entirely. `read_frame` reads a length and then a body, which makes it not cancel-safe, and it sat directly in `run_peer_loop`'s `select!`. When a heartbeat tick won that race the half-finished read was dropped, the bytes it had already taken went with it, and every frame after that landed at the wrong offset — so the connection died and never came back. Rare, load-dependent, and indistinguishable from flaky tests. Reading now happens in its own task and the loop selects on a channel, which is cancel-safe.

Two latent deadlocks turned up while chasing it, both the same shape: a bounded channel awaited from inside the loop that drains it. Dialling a peer no longer runs its handshake inside the event loop, and `fan_out` no longer waits for a peer's outbox. A peer whose outbox fills is now dropped rather than waited for — it reconnects and catches up from the oplog, which is better than the alternative of quietly not sending it a write.

What this does not do:

- The election assumes every survivor received the same membership list. A node that joined and never heard a membership update, or one partitioned from the rest, can pick differently. Real partition tolerance means a consensus protocol, which is a design decision rather than a coding one.
- Followers do not automatically reconnect to the new leader. They find it again through mDNS once it advertises, which is a delay rather than a break.
- The oplog is never pruned. It grows for the life of a showfile.

### 7. Housekeeping (done)

`cargo build --workspace` and `npm run check` are both at zero warnings, so a new one is visible rather than buried. `crates/pult-schema/bindings/` is untracked now.

The one exception is ts-rs printing `failed to parse serde attribute` for the generated Patch structs' `skip_serializing_if`. That is inside ts-rs.

### 8. WASM plugins

Nothing else depends on it, and the plugin API should be designed against a system that already plays back cues and drives output.

### 9. Output configuration in the web UI (done)

Outputs are `--artnet` flags read once at startup. Nothing in the data model knows an output exists, so an operator cannot see where the show is going, let alone change it without restarting the backend.

Add a PERSISTED `outputs` collection — `OutputConfig { id, name, kind: Artnet | Sacn | OpenHaunt, target, universes, enabled, node_id: Option<NodeId> }` — and build the `OutputManager`'s plugins from it instead of from the command line. `OutputManager` then has to add and remove plugins when the collection changes, which it cannot do today: it takes its `Vec<Box<dyn OutputPlugin>>` once at construction. The flag stays, as a way to seed one output into an empty showfile.

An *Outputs* tab lists them, adds them, and enables or disables them, with per-plugin LOCAL status beside each: last send, frames per second, error count. That status is the reason to do this at all — right now a mistyped Art-Net address is silent.

`node_id` says which station runs the plugin. It is the first ownership concept in the system, and what task 10 displays.

sACN is not part of this. It lands earlier, in task 11, as a sibling of `artnet.rs`.

All of that is in. `OutputManager` reconciles against the collection rather than taking its plugins once, so an output can be re-addressed while the show is up — and it keeps the socket when only the name changed, because rebuilding it resets the dedup cache and puts a redundant frame on the wire for a label edit. Status is LOCAL and measured over a one-second window; `interval` fires immediately, which the first version divided by, reporting a thousand frames a second.

`node_id` turned out to matter immediately rather than in task 10. `outputs` is PERSISTED, so it replicates — and without an owner every station would send the same universes, which is two copies on the wire. The UI fills in the local station and *Every station* is a deliberate choice.

The flags stay as a way to seed an empty showfile, and refuse to add anything to a show that already has outputs.

Left open: nothing decides what happens to an output whose station leaves the session. It simply stops, which is right for a duplicate and wrong for the only path to the rig. That belongs with task 10's partitioning question rather than here.

### 10. Station view (done)

What exists today is thinner than it looks:

- ~~`NodeId` is generated fresh on every process start.~~ Done, and not optional once task 9 landed: an output names the station that sends it, so a fresh id per start meant a saved output belonged to nobody and silently stopped sending. `infra::identity` records it beside the showfile — beside rather than inside, because copying a showfile must not clone a station's identity.
- `SyncManager` knows its `members` and `peers`, but they are reachable only through test-only commands. Nothing publishes them.
- Heartbeats go out every 5 s with a 16 s timeout, and the ack carries a `seq` that is never matched against a send time. Liveness is known; latency is not.
- There are no system stats of any kind.
- Every node computes every fixture. `playback_tick` runs the same fades everywhere, which is what makes output deterministic without extra messages, and also means "which node drives what" has no answer yet.

Add a SYNCED `stations` collection that each node publishes about itself — `Station { node_id, hostname, is_leader, sync_addr, cpu_percent, mem_used, mem_total, uptime_s, output_plugins, computes_fixtures, last_seen }`, refreshed every couple of seconds via `sysinfo`. Round-trip time comes from matching `Heartbeat { seq }` to `HeartbeatAck { seq }` in `infra/sync/peer.rs`, published as a LOCAL `peers` list by the node that measured it, because latency is a property of a link rather than of a station. And persist `NodeId` in the showfile so a station keeps its identity across restarts.

A *Stations* tab then shows who leads, what the latency to each peer is, cpu and memory per node, which output plugins run where, and which fixtures each node computes.

That last column is honest but dull for now: every node computes every fixture, so it is all-or-nothing until parameter computation is partitioned. Partitioning it is the follow-up — the interesting version is a node driving only the fixtures on the outputs it owns, with a defined answer for what happens when that node drops out.

All of that is in. `Station` is SYNCED, and each node writes only its own row, so the collection converges with nobody arbitrating it: a station is the only authority on its own memory usage. It is reported as `computes_fixtures` over `total_fixtures` rather than a flag, so the number already says something true and will say something more interesting once the work is split.

Latency is LOCAL, not SYNCED, and that turned out to be the right call for a reason worth recording: measured across a two-node session the same link read 0.22 ms from one end and 0.33 ms from the other. They are two different paths. A single shared number would have been an average of two things nobody asked about.

Pruning is the leader's job alone — two nodes deleting each other's rows on different schedules is a fight rather than a cleanup. A station goes grey in the UI after three missed reports and its row is removed after thirty seconds of silence.

Persisting `NodeId` came first and separately, because task 9 had already made it urgent: an output names the station that sends it, and a fresh id per start meant a saved output belonged to nobody and silently stopped sending.

Left open: nothing partitions fixture computation, so the fixture column is all-or-nothing; and a station's row says what it is doing rather than what it should be doing, so there is still no answer for an output whose station has left.

### 11. OpenHaunt nodes and the event system (done)

Covers the spec's *Event-Based Control & Automation*: sensors and switches drive playback, and the console drives things that are not lights.

[OpenHaunt/node](https://github.com/OpenHaunt/node) is a PoE-powered modular I/O node — one carrier, one plug-in module — advertising itself over mDNS as `_openhaunt._tcp.local` with an HTTP/JSON control API and MQTT for events. Its guiding principle is that a node is discovered, not configured. There is no firmware yet, so this is built against the written protocol, and what the-pult assumes gets fed back into those docs.

What this delivers:

- Discovery. Nodes appear in a *Devices* panel with their module type, capabilities, and a mains warning where the descriptor says the module switches mains. Adopting one creates a fixture.
- Fixtures that are not DMX. `Fixture::universe`/`dmx_address` become `FixtureAddress::{ Dmx, OpenHaunt }`, and `ParameterDefinition` gains a direction and a binding, so a parameter can be an *input* on a *port* rather than an output on a DMX channel. A contact closure and a temperature reading are then ordinary fixture parameters in `live_values`, replicated like everything else.
- An MQTT broker embedded in the leader (rumqttd). On adoption the-pult POSTs its own broker address to the node, so nothing external has to be installed and the "discovered, not configured" principle holds. Only the leader drives devices, gated the same way `GoNext` already is.
- Input to trigger to cue. A PERSISTED `triggers` collection with a source, a condition, an action and a delay, evaluated by a pure state machine in the engine's own tick next to playback.
- Output to relays, LED strips, and OLED displays through an `OpenHauntOutput` plugin, plus sACN unicast for the DMX gateway module — which is task 4's remaining sACN work, done here because the gateway needs it.
- `tools/openhaunt-sim`, a simulator implementing the node side of the protocol, so the whole path is covered by tests without hardware on the bench.

All of that is in. `FixtureAddress` and `ParameterDirection`/`ParameterBinding` went in first, with the two migration paths they needed — a hand-written `Deserialize` for the JSON column and `showfile::upgrades` for the real ones. `types::openhaunt` is the only place that knows what a module id means. `DeviceManager` browses, adopts, and drives; `SetLiveValue` merges an input inside the engine actor and replicates it; `model::triggers` evaluates the rules in the engine's own tick beside playback.

What it leaves open:

- ~~The node-graph UI.~~ Task 12.
- Per-pixel WS2812. A strip is one colour and one brightness.
- OSC and MIDI as trigger sources. `TriggerSource` is an enum with one variant so they can be added beside `Parameter` without touching anything else.
- RDM, which the gateway module's `caps` advertises and nothing here uses.
- A running fade and a `SetParameter` trigger writing the same key: last writer wins, and it is the fade, because it writes on every tick. Documented rather than solved — deciding what *should* happen is a product question.
- The broker is started once per process and never stopped. A node adopted by a previous leader is re-configured on promotion, but a follower keeps a broker it started while it was leading.

Assumptions made about the protocol, to be fed back into the OpenHaunt docs since there is no firmware to check them against: `/api/v1/config` takes `{ mqtt: { broker }, dmx?: { protocol: "sacn", universe } }` and persists it; the mains flag is descriptor bit 6, reachable through `GET /api/v1/info` as `module.flags` (a `mains=1` TXT key would let the panel warn without the round trip); input events are `{ state, edge, ts }` and sensor readings `{ value, unit, ts }` on the same `input/<n>` topic; output payloads are `{ state }` for a relay, `{ r, g, b }` and `{ brightness }` on ports 0 and 1 for a strip, and `{ text }` for a display; `POST /api/v1/state` takes `{ outputs: { "<n>": payload } }`; `status` is the literal `online`/`offline`, retained, with `offline` as the will; health is `{ uptime_s, temp_c, poe_class, errors }`; the DMX module lists `sacn` in `caps` and listens on unicast 5568 for its configured universe; and TXT `sn` matches the instance short serial, one module per node.

### 12. The node graph (done)

Covers the spec's *Node-Based Workflow*: "visually connect triggers, events, playback
actions and automation logic".

`triggers` is gone. A rule was a source, a condition, a delay and an action in a row,
which is a four-node chain — so the graph replaced it rather than sitting beside it,
and `showfile::upgrades` redraws every existing trigger as a flow on the next open.
One evaluator, one meaning, and the thing a row could not say — two contacts into an
`And` — is now drawable.

`flows`, `flow_nodes` and `flow_edges` are three PERSISTED collections rather than two
`Vec`s on one entity, so dragging a node patches one row instead of rewriting the
graph and two operators moving two different nodes both keep their work. Adding all
three needed no edit in `engine/mod.rs`: task 2's registry-driven dispatch held.

The design decision everything else follows from is that a port carries either a
**level** or a **pulse**. A level stays put; a pulse is an instant. Sources emit
levels, `And`/`Or`/`Not` combine them, and a `Condition` is the only thing that turns
one into the other — by noticing a *change*. That asymmetry is what stops a warm room
firing a cue forty times a second, and having it in the type system means the editor
refuses the connection rather than the evaluator refusing the graph.

Node `active` is SYNCED, so a graph lights up as signals pass through it on every
console watching. That is the reason to draw it at all: a diagram that shows its own
state is an instrument.

Two things turned up while building it:

- A button press is a write to `last_fired_at`, not a message. That fell out of
  wanting a press to work from a tablet: the leader is the only node that fires
  anything, and a replicated field change already reaches it by the path everything
  else takes.
- A `Watch` node offered every driven parameter and could never fire for any of
  them, because playback applied fades with LOCAL lifecycle and never queued an
  input event. A cue's own output is show state like any other, so fades now reach
  the flow tick — gated by the set of parameters some `Watch` actually names, since
  this runs at 40 Hz per fixture in a fade.

Left open:

- Nothing decides what a cycle *means*. It terminates rather than hanging, which is
  right for a drawing mistake and not an answer for a graph that wants feedback.
- An action still loses to a running fade writing the same key. Task 11 documented
  it; drawing it does not settle it.

## Further out

Everything below is in the spec and has no schema and no code yet. Listed so the near-term work does not paint itself into a corner.

**Selection as a geometric query.** Selections are meant to be generated from the rig by geometric functions and re-evaluated as the rig changes, not stored as fixture lists. That is a query language, and it needs positions first.

**Effects and phasers.** Derived from the 3D selection with modifiers that can themselves be dynamic. Needs selection.

**3D programmer.** Rig view, fixture puppeteering, quicksheets. The biggest single piece of the product and entirely absent.

**Waveform timecode and "timecode without timecode".** Beat grids, markers, live audio analysis for band sync. Should subsume the `Timecode` follow mode rather than sit beside it.

**Open control interfaces.** OSC, MIDI, and control surfaces alongside the existing WebSocket API.
