# Roadmap

State of the system, what has been built, and what to work on next. Reconstructed from the code on 2026-08-25, then reconciled against [SPEC.md](SPEC.md).

The spec is the product. This is the build order for getting there, and the gap is still wide: what exists is a synchronised show-state engine with cues, playback, output, and an event system. The spec's 3D programmer, geometric selections, phasers, and waveform timecode are all still ahead.

## Where the system stands

| Layer | State |
|---|---|
| `pult-schema` + `pult-macros` | Working. The derive macro generates entity meta, patch/create types, accessors, and SQL. All 30 workspace tests live here. |
| `pult-codegen` | Working and idempotent. TypeScript types, the `data.ts` proxy, and the SQL migration all come from the `EntityMeta` and `CommandRegistration` inventories. |
| Showfile (SQLite) | Working. Load and save are registry-driven and enumerate no entity types. |
| WebSocket API | Working. Path-pattern subscribe, set, call, and broadcast fan-out. |
| Session discovery | Working. mDNS advertise and browse, create, join, leave. |
| Peer sync | Works and converges. Handshake, bidirectional catch-up from the oplog, live fan-out, heartbeat liveness and latency, vector-clock conflict resolution, and leader failover. Stations publish themselves and are visible in the UI, with what each machine and each link is costing since task 49. |
| Frontend | Working for show, session, sequences, cues, patch, the programmer, effects and speed masters. A tiled workspace of resizable panels replaced the sidebar and tabs; layouts are saved in the showfile. Panels that can change the show open read-only behind an Edit toggle and are sized for a finger. The typed proxy runs end to end. Vitest covers the pure helpers; components are untested. Since task 49 a page also reports what it is itself costing — frame rate, evaluator time, clock offset — which the System panel shows beside every station's. |
| Playback engine | Working, and no longer a tick. Playback decides *what is driving* each parameter — fades and effects anchored on the cue's `went_at` — and publishes the descriptions; nothing stores what they are worth. A pass happens when the show changes, so a fade in progress costs the engine nothing. |
| Output plugins | Working for Art-Net, sACN, and OpenHaunt nodes, several at once. Each holds the last patch it was pushed and draws its own frames out of it at its protocol's rate, evaluating rather than being handed values. Configured from the `outputs` collection and editable while the show is up, with per-output status and per-connector frame cost in the UI. Flags only seed an empty showfile. |
| Stage view | Working. A ground plan is uploaded, calibrated against something of known length, and fixtures are dragged onto it — then the same rig in 3D from front of house, beams and all. Since task 47 it draws the *drawing* too: trusses and objects out of an MVR, from their own meshes, with a Layers panel to show and hide parts of it. Every beam is still one cone at one angle; the geometry and the beam angle a GDTF import brings are stored and not yet drawn. Nothing can be moved or placed in either view yet. |
| Rig interchange | Working. MVR in and out: fixtures with their positions, trusses and objects with their meshes, layers and classes, all keyed by the uuid the file uses so a re-import updates the rig rather than doubling it. A fixture's place is a transform with a signed scale, relative to whatever it hangs off. What an import no longer mentions is reported, never deleted. No scene editing yet. |
| Fixture definitions | Working. GDTF in and out, with modes, breaks, wheels, emitters, physical data and the geometry tree; the archive is kept whole and exports byte for byte. The GDTF Share is searchable and importable behind a station credential. A type the console derived from a node or somebody typed in is unchanged and still has an implicit mode. |
| Flows | Working. The spec's node graph, evaluated as a graph: sources, conditions, boolean logic, delays and actions, with live state on every node. Replaced `triggers`. |
| Devices / events | Working. OpenHaunt nodes are discovered over mDNS and adopted as fixtures; their inputs land in `sensed_values`; flows turn those into cues. A port that says it can trace a shape is handed one descriptor instead of forty messages a second. Tested end to end against `tools/openhaunt-node-sim` and, since task 22, against real firmware on an ESP32. |
| WASM plugins | Working. wasmtime component runtime with a WIT contract, permissions, hot reload, plugin-to-plugin calls and runtime introspection of the schema registries. Two reference plugins in `plugins/`: a command line (grammar built from introspection, console panel with completion and spans) and natural-language control (an LLM over the plugin's own gated HTTP, executing through the command line). Plugin UI is built-in surfaces or plugin-shipped web components. `docs/PLUGINS.md` is the author guide. |
| 3D programmer | Working in outline. A shared programmer buffer beats playback, and pan and tilt are puppeteered by grabbing a ring, an arc, or the beam spot on the floor — in the rig and on the plan. Effects are in, and a selection is a question about the rig rather than a list. |
| Selection | Working as a query over the rig: by type, name, sphere, box or the spec's radial cone, built up by adding, narrowing and removing, and ordered along an axis or outwards from a point. Re-evaluated as the rig changes, so a fixture patched under a live selection joins it, and read against a *world* position, so a light on a truss is where the truss put it. Saved as groups since task 30. |
| Effects | Working. One primitive covers a shape and a step list, running from the programmer or a cue, at its own rate or a speed master's. Rendered identically on every station from replicated state, and handed to a node that can trace it for itself. No amplitude fade into one yet. |
| Undo / history | Working, per person and across their clients, and by gesture rather than by write — one drag is one Ctrl-Z. The oplog carries the author, the previous value and what an operation reverses, so undo is a query over it rather than a stack — which is what lets a tablet take back what the desk did. A History panel shows what everyone changed. Nothing prunes the log. |
| Timecode | Not started. `FollowMode::Timecode` exists and nothing produces one. |
| Distribution | Working. The frontend is built into the binaries that serve it, the console and the simulator each have a Tauri desktop app, and tagging builds all four for Linux x86_64 and aarch64, macOS arm64 and Windows. Nothing is signed and nothing auto-updates. |

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

### 8. WASM plugins (done)

Nothing else depends on it, and the plugin API should be designed against a system that already plays back cues and drives output.

That deferral paid for itself. The API landed as one WIT package (`wit/pult-plugin.wit`, wasmtime 48 + wasm32-wasip2 components) with JSON on every edge, so no entity type appears in the contract — a plugin learns the schema from introspection host functions that serve the `EntityMeta` and `CommandRegistration` inventories at runtime, plus a station-RPC table that `handle_local_call` was refactored into (`api/rpcs.rs`) so those six calls stopped being invisible. `PluginManager` mirrors `OutputManager`: an actor, per-plugin instance actors under it, LOCAL `plugins` state telling every frontend what runs. Manifest permissions gate everything — data access, commands, an outbound-HTTP host allowlist enforced in the host's `send_request`, env passthrough by name. Epoch interruption traps a guest that runs five seconds; a changed file reloads its plugin while the show is up, which is the node-sim's stop-and-start-fresh applied to plugins.

Two rules fell out of debugging rather than design. The manager never awaits guest code — a dependency's `init` calling back through the manager deadlocked the first version; now instances init on their own tasks, readiness is a message, and calls to a still-loading dependency queue in its mailbox, which makes mailbox creation order the whole of load sequencing. And plugin writes carry the caller's identity: the surface context's user attributes every write of a call, gathered under one gesture, so a CLI command that fans over a selection is one Ctrl-Z.

Plugin UI is both ways at once: *surfaces* the console draws (a command-line panel, a one-line bar — the plugin implements `surface.exec`/`complete`/`help`, pure Rust, spans and completions included) and *web components* the plugin ships as JavaScript served from its directory. Panel ids are `plugin:<id>:<panel>`, so saved layouts survive a missing plugin the way they survive an unknown panel.

The reference plugins double as the examples: `plugins/command-line` derives its entire grammar from introspection (and reimplements the frontend's derived programmer-entry id, pinned in both test suites so the two FNV implementations can only drift loudly); `plugins/natural-language-control` depends on it, fetches its grammar for the prompt, and runs whatever the model answers back through it — the command line is the safety boundary, and the model gets no other hands. `scripts/build-plugins.sh` builds the separate `plugins/` workspace; `scripts/demo.sh --plugins` loads everything; `docs/PLUGINS.md` teaches it.

Left open: no resource limits beyond the epoch deadline (memory is unbounded), plugins are not release artifacts, and the `is_public` flag on commands still has no reader.

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
- `tools/openhaunt-node-sim`, a simulator implementing the node side of the protocol, so the whole path is covered by tests without hardware on the bench.

All of that is in. `FixtureAddress` and `ParameterDirection`/`ParameterBinding` went in first, with the two migration paths they needed — a hand-written `Deserialize` for the JSON column and `showfile::upgrades` for the real ones. `DeviceManager` browses, adopts, and drives; `SetLiveValue` merges an input inside the engine actor and replicates it; `model::triggers` evaluates the rules in the engine's own tick beside playback.

**A node describes itself, and this console carries no module catalogue.** The first cut of this had a table in `types::openhaunt` from module id to fixture type, which meant a node newer than the console — or anybody else's module — was a thing the console had to be taught. That is backwards from the principle the project shares with [SHIFTY](https://github.com/oshifty/vision): only the device knows what it is and how it is best controlled. So `GET /api/v1/info` carries a `ports` list in E1.73 UDR's vocabulary — `access`, `dataType`, `unit`, `minimum`/`maximum`/`default`, plus a small `class` hint — and `openhaunt::fixture_type_from` turns that into a fixture type at adoption. A `class` the console recognises becomes the kind it has semantics for; anything else becomes `ParameterKind::Named`, so a device can declare a parameter nobody here has a word for and it still patches, still programs, still stores into a cue. A node that describes nothing is refused rather than guessed at, and the fixture type's id is derived from the description, so two identical modules share one type and firmware that changes its ports gets a fresh one instead of silently mismatching.

What it leaves open:

- ~~The node-graph UI.~~ Task 12.
- Per-pixel WS2812. A strip is one colour and one brightness.
- OSC and MIDI as trigger sources. `TriggerSource` is an enum with one variant so they can be added beside `Parameter` without touching anything else.
- RDM, which the gateway module's `caps` advertises and nothing here uses.
- A running fade and a `SetParameter` trigger writing the same key: last writer wins, and it is the fade, because it writes on every tick. Documented rather than solved — deciding what *should* happen is a product question.
- The broker is started once per process and never stopped. A node adopted by a previous leader is re-configured on promotion, but a follower keeps a broker it started while it was leading.

Assumptions made about the protocol, to be fed back into the OpenHaunt docs since there is no firmware to check them against: `GET /api/v1/info` carries `ports` — one entry per terminal with `port`, `name`, `access`, `dataType`, and optionally `unit`, `minimum`/`maximum`/`default` and `class` — and `dmx` only on a node that forwards a universe; `/api/v1/config` takes `{ mqtt: { broker }, dmx?: { protocol: "sacn", universe } }` and persists it, and accepts `dmx` only where the description declared one; the mains flag is descriptor bit 6, reachable through the same `/info` as `module.flags` (a `mains=1` TXT key lets the panel warn without the round trip); input events are `{ state, edge, ts }` and readings `{ value, unit, ts }` on the same `input/<n>` topic, with `unit` a UDR name; output payloads follow the port's `dataType` rather than the module — `{ state }` for a boolean, `{ value }` for a number, `{ r, g, b }` for a colour, `{ text }` for a string, and no `{ brightness }` anywhere; `POST /api/v1/state` takes `{ outputs: { "<n>": payload } }`; `status` is the literal `online`/`offline`, retained, with `offline` as the will; health is `{ uptime_s, temp_c, poe_class, errors }`; the DMX module lists `sacn` in `caps` and listens on unicast 5568 for its configured universe; and TXT `sn` matches the instance short serial, one module per node. `mod` and `caps` in the TXT record stay an inventory shortlist — filtering and the mains warning — never the basis for control.

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

### 13. The stage view (done)

`Fixture::position` went in with the patch in task 5 and could only ever be set to
the origin, so the rig had coordinates nobody could enter and nothing drew. Two
views now read them, and one of them puts them there.

**Assets.** The first bytes in a system that was otherwise all fields. A ground plan
is a few megabytes, and putting it in the oplog would put a copy in every operation,
every snapshot and every catch-up — so `assets` is a blob table beside the show,
addressed by the sha256 of its own contents, with `POST /assets` and
`GET /assets/{sha}` beside the WebSocket that was until now the entire HTTP surface.

Content addressing carries more weight than it looks. The id *is* the check, so a
station that has never seen a plan fetches it from one that has and verifies what
came back; the same drawing uploaded twice is stored once; and the response can be
cached for ever because its contents cannot change. `Station` gained `http_addr` so
there is somewhere to fetch from, and a relayed request is answered locally or not at
all, so a ring of consoles cannot forward one request round between them.

PDFs never reach the backend. Page one is rasterised in the browser, which keeps a
document engine out of both stage views and off the main bundle.

**The plan.** `StagePlan` is the drawing plus the two numbers that make it a map:
where its top-left corner sits in the room, and how many metres one pixel covers. The
second comes from clicking two points and saying how far apart they really are, which
is the whole of calibration. Fixtures are dragged onto it and their live colour and
level fill the symbol, so the view is both where the rig is and what it is doing.

**Axes.** This is the first place in the system that had to commit: **Y up, X to the
right seen from front of house, Z downstage towards the audience.** Z is chosen
rather than inherited, and two things agree on it — a ground plan is drawn with the
audience at the bottom of the page, and the 3D camera looks up −Z. A plan therefore
lies on the floor with no flip anywhere.

**The rig in 3D.** Threlte over three.js, opening at the FOH perspective the spec
calls primary. Fixture bodies, a beam cone each, and a spot light so the floor shows
the state as well as the air. Both views read `lib/stage.ts`, so they cannot disagree
about where anything is.

Left open:

- Pan is taken as 540° about the way a fixture hangs where the type says nothing —
  task 45 made that a fallback rather than the answer — because `FixtureType` carried
  no real ranges. Right about which way a head is swinging and wrong in detail for
  any particular one.
- Nothing prunes an asset. A replaced plan's bytes stay in the showfile.
- The plan is one drawing on one plane. Sections, multiple decks and a plan per
  level are all the same schema and none of the UI.
- MVR and GDTF. `StagePlan` and the asset store are the two things an import needs,
  so nothing here is in its way.

### 14. The programmer and cue editing (done)

The console could play a cue but nobody could *make* one. `Cue.captures` was written
as `[]` at creation and never touched again, no control anywhere set a fixture value,
and the spec's §Programming — the programmer buffer, parking, the store menu,
puppeteering pan and tilt in 3D — was the part the roadmap called the biggest single
piece.

**The priority rule.** The decision everything else follows from: **for every
parameter the programmer holds, the programmer wins over playback**, until that value
is cleared or stored. This is the first explicit priority rule in the system, and it
is the standard one — MA and ETC both work this way.

What it does *not* do is stop the cue. Fades keep running underneath, and what they
would be showing is kept current as they do, so a value released or stored lands
where playback has got to rather than snapping back to where the cue was when the
operator grabbed it. That is the difference between an override and a freeze, and it
is what lets a look be built during a fade.

Anything else that writes a live value — a flow action, an input off a device — is
not fought with. It writes; the overlay notices on the next tick, treats what it
wrote as the new value underneath, and covers it again. The open question task 11
recorded ("an action loses to a running fade") is still open and still a product
question; what is settled is that neither of them beats the operator's hands.

**Where it lives.** `programmer_values` is a SYNCED collection — replicated to peers
and frontends, never persisted. A showfile that reopened asserting somebody's
half-finished look over playback would be a fault, not a feature. Entry ids are
*derived* from the fixture and the parameter key rather than minted, so two consoles
grabbing the same fader write the same row and converge instead of leaving two rows
that take turns reaching the output.

**Storing and editing.** A store menu shows what is about to be written — fixture,
parameter, value, one checkbox each, as the spec asks — with merge or replace, a new
cue or an existing one, and a *keep the programmer* option. Editing a cue is load,
tweak, Update: the cue is read into the buffer and taken, changed there, and written
back on Update. Not live editing — a cue that rewrote itself as a fader moved would
have no way back from a mistake, and would be doing it on every console at once.
`Show::editing_cue` is SYNCED so the second console shows the same banner rather than
quietly storing over the first one's work.

**Puppeteering.** The spec asks for pan and tilt by grabbing the axis, "just like it
would behave in real life". In 3D a selected head wears a pan ring, a tilt arc lying
in the plane it is currently panned to, and a disc where its beam lands on the floor;
dragging any of them projects the pointer ray onto the plane that axis turns in and
reads the angle off it. The plan view gets the same beam-spot handle, plus a level
ring, in world units. Both write through the programmer, never into `live_values`.

A ring is *turned*, not aimed: the axis moves by however far the pointer has gone
round since it took hold, from wherever the axis already was. Reading the absolute
angle instead — which is what the first version did — snapped the head to the pointer
the moment the ring was touched, which is not what taking hold of a yoke does. The
turn is added up one move at a time, each wrapped to the short way round, so a drag
that passes behind the fixture keeps counting instead of flipping.

The beam-spot handle is capped at a distance worth drawing. A beam near the
horizontal lands arbitrarily far away and one above it never lands at all, so the
honest answer is a point tens of metres off the plan — and a drag that flattened the
beam sent the handle off the screen mid-gesture, which read as the handle simply
coming away in your hand.
`interactivity()` had never been installed, so click-to-select in 3D had been dead
since the view was built; `OrbitControls` became `CameraControls` so that picking a
fixture can animate the camera to it.

Two things about the 3D view worth writing down because both cost an afternoon:

- The orbit controls and a gizmo hear the same `pointerdown` on the same element, and
  by the time any handler could call the camera off it has already begun to move. The
  press is now taken in the *capture* phase, before the event reaches the canvas at
  all, and raycasts the gizmos itself.
- An `<HTML>` overlay with `pointerEvents="auto"` takes pointer events over its whole
  layout box — which, when the panel is moved by a CSS transform, is not where the
  panel is drawn. An invisible rectangle below the quicksheet was eating every click
  on the beam spot. The wrapper is `none` now and the panel itself is `auto`.

And one about the plan view, worth writing down because the symptom pointed
nowhere near the cause. Clicking a fixture filled most of the room with a
white-and-blue rounded band. Nothing in the DOM was that size — every shape measured
a few pixels — and `elementsFromPoint` found nothing there at all, which is the clue:
it was not an element. It was Chrome's own focus ring on the fixture group, which is
focusable, drawn in the element's own coordinates. On a plan those are metres, so the
ring was several metres thick and shaped like the bounding box of the fixture's beam.
`svg :focus { outline: none }`, and keyboard focus is shown by the ring the symbol
already has.

Two things were changed while chasing it that are worth keeping on their own terms: a
full-turn level ring is drawn as two half-arcs, because an arc that ends where it
began does not say which circle it meant; and every stroke in that view is measured in
screen pixels through `vector-effect: non-scaling-stroke`, which is what a hairline
wanted rather than a dozen widths each computed as some fraction of the view.

**`u64` is not a `bigint` on this wire.** ts-rs maps every 64-bit integer to `bigint`,
which would be right if the wire could carry one; it is JSON, so a `u64` arrives as a
`number`. The declared type was a lie about every value that turns up, and
`error_count === 0n` is `false` for the `0` that does — so a working sACN output
reported itself unhealthy for ever. pult-codegen now rewrites `bigint` to `number` on
the way out, which is one place rather than an attribute to remember on every `u64`
somebody writes.

And one about the engine. A subscriber watching `show` never heard a field write to
it: patterns are matched against the path a write names, and `show/editing_cue` does
not match `show`. Collections already answered this by broadcasting the collection
after a create or a delete; singletons now do the same. Entities deliberately do not —
that path is `fixtures/<id>/live_values` at forty a second during a fade, and sending
the whole rig each time is what `subscribeDeep` exists to avoid.

Left open:

- **Every drag is an oplog row.** Writes are coalesced to one per animation frame per
  parameter and skipped when the value has not changed, which makes a fader drag tens
  of rows rather than hundreds. It is a reduction, not a fix: the oplog is still never
  pruned, and that is where this belongs.
- **Pan and tilt travel are constants** — 540° and 270° about the way a fixture hangs
  — because `FixtureType` carries no real ranges. Right about which way a head is
  moving, wrong in detail for any particular one. Centring the travel on the hung
  direction also lets a head hung straight down fold 135° past vertical and point up
  and backwards, which no real one does; per-type ranges are what settles it.
  *(Task 45: half settled. `stage.ts`'s `travelOf` reads the type's own `physical`
  range where a GDTF import gave it one, and the constants are the fallback for a type
  that never said. The folding-past-vertical part is untouched: what a head can reach
  is a fact about its geometry, not about how far it turns.)*
- Parking survives Clear and Store, as the spec asks. Nothing yet parks a value
  *across* a showfile reload, because nothing SYNCED does.
- The programmer holds a parameter or it does not. There is no partial override, no
  release time, and no touch-sensitive designer fader — that last is its own item in
  the spec and needs hardware to mean anything.

### 15. The workspace (done)

The main page was a fixed sidebar and six tabs, so the 3D rig, the values and the cue
list could never be seen at once — which is exactly what programming needs.

Panels now live in a tree of splits and tab groups: drag a tab to any edge of any tile
to divide it, or to the middle to stack it, and drag the gutters to resize. Six
presets ship built in and are always available; an arrangement worth keeping is saved
into the show as a PERSISTED `layouts` row, so the console next to it opens the same
way.

Two decisions worth recording:

- **Layouts are the show's; which one you are looking at is yours.** Two operators at
  two screens plainly want different tiles up, so the active layout and any unsaved
  rearranging live in `localStorage` rather than in the showfile. Rearranging never
  saves on its own — otherwise a busk on a spare screen would rewrite the layout
  everyone else is using.
- **The schema does not know what a panel is.** `LayoutNode` holds panel ids as plain
  strings, and one file in the frontend turns an id into a component. A layout saved
  by a newer build opens on an older one with the unknown panel simply missing.

The tree keeps two invariants after every operation: no split contains a split running
the same way, and no split has fewer than two children. The first matters because a
gutter drags the two tiles either side of it, and nesting would make some gutters move
tiles that are not next to them.

Panels that were the sidebar — show, session, devices — are ordinary panels now, and
the stage tab's two halves became two panels so both can be open together.

Left open:

- A panel can only be open in one tile at a time. Two views of the same plan at
  different zooms is a reasonable thing to want and is not possible.
- Nothing is responsive. A tree of tiles on a phone is a tree of very small tiles, and
  the spec asks for tablets and phones.

### 16. Something to install (done)

Everything up to here was two processes started by hand. `cargo run -p pult-backend`
served `/ws` and `/assets` and nothing else, the console was Vite on another port
reaching across origins to a hardcoded `ws://localhost:7700`, and there was no
release, no CI, no README and no LICENSE. A console nobody can install is a
program, not an instrument.

**The frontend is in the binary.** `rust-embed` over `frontend/build`, served as
the router's fallback so `/ws` and `/assets` are still matched first. One artifact
is the whole console.

The decision that everything else follows from is what that does to *where the
backend is*. The page now comes from the station, so the socket is on the origin
the page came from, and the question the `?port=` query string existed to answer
stops being a question. `endpoint.ts` is the only place that decides, `?port=`
survives as a way to name a second station on the same host, and `/api/config`
answers what a page genuinely cannot work out for itself — which station this is
and what version it is running. It is deliberately not asked *before* connecting:
a console must not wait on a request to make its first one.

The build is precompressed, so the `.br` beside every file was squeezed once here
rather than on every request from every tablet in the room.

**The console is a desktop app.** `crates/pult-gui` is a window around
`pult_backend::start`, which meant splitting `pult-backend` into a library and a
thin binary — worth doing on its own, since `main.rs` had been the only definition
of what starting a station meant.

The window points at `http://localhost:<port>`, the server it has just started,
rather than at a copy of the frontend bundled beside it. That is the choice worth
recording. It means the app and the tablet in the rig are the same page from the
same origin, there is one frontend to build, and the desktop build cannot drift
from the one everybody else uses — which is the MA web-remote arrangement, and
the reason the app defaults to port 7700 rather than asking the OS for one. It
also means Tauri's IPC is not available to that origin without a capability
naming it, so there is no native file dialog yet.

A second console on one machine is an ordinary thing to want, so a taken port
falls back to any port and says so in the title bar rather than refusing to start.

**The simulator has a window.** `tools/openhaunt-node-sim-gui`, and the thing it fixes
is small and real: `openhaunt-node-sim` can only be driven by typing at its stdin, and
`scripts/demo.sh` does not connect one — which is why the input node has to be
started with `--auto 2500`. Here a contact is a button.

It talks to the node over Tauri's IPC rather than over anything on the wire. The
sim implements the node side of the OpenHaunt protocol and nothing else, and a
debug UI is not part of that protocol. The one thing added to the sim itself is a
`Snapshot` — everything a node knows about itself in one value — because a window
should not have to subscribe to five channels and stitch them together. The panel
writes the port layout out again from the same documents rather than importing it
from `pult-schema`, for the reason the sim exists at all.

**Releases.** `.github/workflows/release.yml`, modelled on the one in
`Dein-Ticket-Shop/printer-client-rust`. Four products for Linux x86_64 and
aarch64, macOS arm64 and Windows x86_64, plus `.deb` for the server and whatever
Tauri bundles per platform. The repository going public is what makes the arm64
half cheap: `ubuntu-24.04-arm` runners are free there, so a Raspberry Pi build is
a native build rather than a cross-compilation exercise.

Two things about it worth knowing:

- **The frontend is built in its own job, once.** The generated TypeScript is not
  in the repository — it comes from the schema — so codegen has to run before npm
  does. Building it once also means every artifact ships byte-identical assets.
- **No `package` × `platform` matrix.** GitHub merges an `include` entry whose keys
  are absent from the base matrix into *every* combination, which is not what
  reading that matrix suggests. One job per platform builds both products.

Two footguns found while building it, both of which cost a window that was simply
blank:

- `devUrl` in a Tauri config wins over `frontendDist` in a **debug** build, so
  `cargo run` loaded a dev server that was not running. Removed; both profiles now
  use the bundled panel.
- `rust-embed`'s `$CARGO_MANIFEST_DIR` interpolation needs a feature flag it does
  not have by default. A plain relative path resolves against the crate root and
  wants nothing.

And four more that only a real tag could find, since every one of them passed
locally:

- The changelog headings were the bracketed Keep a Changelog form. The action
  matches `## 0.0.1` or `## v0.0.1` and nothing else.
- `npm ci` failed on Linux and nowhere else. npm's tree differs by platform, and
  generating the lockfile on macOS left `@napi-rs/wasm-runtime`'s own dependencies
  unresolved — they existed only pinned under `@rolldown/binding-wasm32-wasi`.
  The lockfile is generated in the image CI uses now, and `npm ci` is checked on
  Linux x64, Linux arm64 and macOS.
- `tauri build` takes cargo's arguments after a `--`.
- `beforeBuildCommand` runs from a directory Tauri infers from `frontendDist`
  rather than from the config's own, so `npm --prefix ui` landed one level too
  deep. The panel is built by the workflow instead, which depends on no inference.
- `uploadWorkflowArtifacts` is on tauri-action's main branch and not on the `v0`
  the workflow pins, so bundles that built were discarded. Collected from its
  `artifactPaths` output instead — with the backslashes turned round and the
  carriage returns stripped, because on Windows jq ends its lines with one and
  `cp` then goes looking for a file whose name ends in a carriage return.
- `generate-sbom` finds a version in a virtual workspace manifest, under
  `[workspace.package]`, but no name. It has to be told one.
- Building two packages in one job with the same action twice does not leave two
  archives behind: the second clears the first out of `target/<triple>/release/`.
  Every `pult-backend` archive went missing from a release that otherwise looked
  finished, and `upload-artifact` said so in one line — *"there will be 2 files
  uploaded"* — and carried on. Every upload is `if-no-files-found: error` now,
  because a release that is quietly missing its main product is worse than one
  that fails.

That last one had the same cause as the stray `pult-backend.d` in the archives,
and as the second copy of the binary in there that made every download twice the
size it needed to be: **the archive was cargo's output directory**, wiped and
re-tarred wholesale. `scripts/package-binaries.sh` stages what belongs in a
release by naming it — the binary, the licence, the readme — and it is a script
rather than workflow steps so that what goes into an archive can be checked on a
laptop before it is checked on a tag.

The cost of finding them was six tags. The alternative — a Docker container
reproducing the Linux half — found the worst one in a minute, and is worth
reaching for before pushing the next one.

Left open:

- **Nothing is signed or notarized.** macOS and Windows both warn on first run.
  The workflow is laid out so this is adding secrets and a few steps.
- **No auto-update.** `latest.json` and a Tauri updater keypair are the next step.
- **32-bit Raspberry Pi is not built.** The arm64 runner is aarch64, which is
  64-bit Raspberry Pi OS.

### 17. Three fields nothing ever read (done)

`Show.is_running`, `Show.active_sequence` and `Fixture.active_preset` were in the
schema from the first sketch of it and never acquired a reader. Nothing in the
backend ever set `active_preset` to anything but `None`. `active_sequence` was
never written at all — a sequence became active by having an `active_cue_index`,
which is where playback actually looks. And `is_running` had a button in the Show
panel that toggled it and one that nothing consulted: the engine ticks whenever
`Playback::has_work()` says there is work, which is a better answer to "is the
show running" than a flag an operator can get wrong.

Removing them costs nothing, which is the point worth recording. All three are
SYNCED, and a SYNCED field has no SQL column — `db.rs` loads by named columns and
the generated migration is unchanged byte for byte after the edit, which the
codegen run proves rather than asserts. There is no `deny_unknown_fields` anywhere
in `crates/`, so a showfile or a peer that still mentions them deserialises fine
with the field dropped on the floor. An old oplog row naming one is skipped with a
warning on catch-up, in `apply_peer_operation`, which already handles a path that
no longer resolves.

The lifecycle test that proved a SYNCED field is not persisted was written against
`is_running`, so it now runs against `editing_cue` — the SYNCED field on `Show`
that does have a reader, and the one whose loss on reload would actually be felt.

### 18. The shape of an effect (done)

Types only, no engine: `EffectSpec`, `SpeedMaster`, and the fields on `Cue`,
`Sequence`, `Fixture` and `DiscoveredDevice` that carry them. Nothing renders yet,
which is the point — the wire format and the storage format are settled before
anything depends on them.

**One primitive for sine and chase both.** A shape on intensity and a red-green-blue
chase look like separate features and are separate features on most consoles. They
differ only in what a cycle position maps to: a `Curve::Shape` reads a level out of a
function and scales it between `low` and `high`, while `Curve::Steps` looks the
position up in keyframes that carry real `ParameterValue`s. How fast, which way,
where in the cycle this fixture sits — the same question either way, so it is asked
once, in one envelope. That is why `EffectSpec` has no `kind` field.

**Phase is a number, spread is a memory.** `EffectSpec::phase` is this fixture's
absolute offset, worked out when the operator applied it. `Spread` records *how* they
asked, so the GUI can re-apply the same arrangement to a different selection, and the
engine never reads it. Rendering is then a pure function of one entry rather than of
an entry plus its position in a selection that may since have changed — which matters
because two stations must not disagree about what "third of five" means.

**Where an effect is measured from.** `EffectSpec::t0` is `Some` in the programmer,
set when it was applied, and `None` in a stored capture, where the cue's new
`Sequence::went_at` is the anchor. So `go_next` and `go_to_cue` now take an optional
`at`, and the frontend passes `Date.now()`. Commands run per station from the same
arguments, and that is the whole mechanism: a Go carrying its time makes every
console anchor the cue at one millisecond rather than at whenever its own actor got
to the message. A `Rate::Master` ignores both and uses the master's own `t0`, which is
what keeps every effect on one master in step.

**Tempo is a replicated fact.** `SpeedMaster` is mixed lifecycle like `Cue`: name,
bpm and multiplier persist, `running` and `t0` do not. Every tap and every bpm edit
rewrites `t0` along with the tempo, so a tempo change is a bounded step in phase
rather than a drift that grows — the new rate and the anchor it is measured from
arrive together. A `t0` of 0 after a reload is a defined anchor, not a missing one.

**The first LOCAL entity field.** `Fixture.live_effects` and `live_fades` are LOCAL,
which the macro has supported since task 2 and nothing had used. Every station works
them out for itself from replicated state, so broadcasting them would be sending each
console a slower copy of what it already has. They exist for the two readers that
cannot compute them: an output plugin deciding whether to hand a shape to a node
instead of streaming samples at it, and a panel drawing where each fixture sits in
its cycle. The generated migration confirms it — `speed_masters` appears with its
four PERSISTED columns and `fixtures` gains none.

**The trap worth naming.** `fixture_type_id` is a UUIDv5 over the serialised
`NodeDescription`, so a port's effect capability must not be a field on
`PortDescription`: firmware that started advertising effects would give every adopted
node a fresh fixture type and orphan every parameter already patched against the old
id. `effect_capability_from` reads it out of the raw `/info` body instead, the way
the mains flag already is, and a test pins the id across a description that gains an
`effects` block.

No migration anywhere. `captures` is one JSON column, `#[serde(default)]` covers the
new keys, and a cue or sequence written before any of this reads back with no effect
and no anchor.

### 19. The effect pass (done)

`model/effects.rs` renders; `playback.rs` decides what is running and what wins.
Nothing has left the console yet — an effect is still turned into a stream of values
like everything else — but the console now has something periodic in it.

**Two clocks, because there are two questions.** `Playback::tick` takes both an
`Instant` and a wall-clock millisecond now. A fade's progress is an elapsed duration,
and `Instant` is the only clock that cannot go backwards underneath one. An effect's
phase is a position on a clock that every station shares, and `Instant` is not shared
with anything — it is not even comparable between two processes on the same machine.

**The anchor is the cue, not this station's arrival.** `start_cue` places the
sequence's `went_at` on the monotonic clock and starts the cue's fades from there, so
a console that processed the Go 600 ms late starts its fade 600 ms in rather than at
the beginning. Before this, two stations were visibly out of step for the length of
every fade. For an effect it would have been worse: out of step for good, because an
effect never arrives anywhere to resynchronise at.

**Precedence, which falls out of the write order.** The overlay writes last, so the
rule is: a programmer effect beats a cue effect beats a cue value, and a plain
programmer value covers all of them. That last one is how an operator takes one light
out of a chase by grabbing its fader, and it is why `Overlay::held` became an enum
rather than gaining a second map. A cue effect under a held key keeps rendering into
`beneath`, exactly as a fade already did, so releasing lands on where the chase has
got to rather than where it was when the fader was grabbed.

**What the plugins are told, and what they are not.** `SetLiveEffects` and
`SetLiveFades` describe *why* a value is moving, and only for the winner on each key.
A plain programmer value over a chase produces no entry at all, and the absence is the
message: it is what tells a node to stop tracing a shape and take values again. Unlike
`emit`, this one keeps a cache of what it last said, because these two fields are
LOCAL and playback is their only writer — there is no other hand for the cache to be
wrong about, which is exactly the thing that made a cache wrong for `live_values`.

**Determinism, concretely.** The inputs to a rendered value are the spec, the speed
master, `went_at` and the wall clock, and all but the clock are replicated. Nothing
accumulates per station, so there is no drift to accumulate. A tempo edit rewrites bpm
and anchor together and is re-resolved on the next tick, which makes it a bounded step
in phase rather than a slide. The rate is resolved every tick rather than stored
resolved, so editing a master reaches every effect following it without anything
having to go looking for them.

One thing worth writing down because it was got wrong first: an effect the programmer
holds with no anchor of its own falls back to the epoch, not to now. Anchoring on the
current tick moves the anchor forward exactly as fast as time passes, and the effect
sits perfectly still.

Left open:

- **No amplitude fade into an effect.** An effect starts at full size the moment its
  cue goes. Fading a chase up is a real thing to want and it needs a second envelope.
- **Inter-station clock skew is assumed, not measured.** Every station is taken to be
  NTP-close. A 50 ms skew on a 2 Hz chase is a tenth of a cycle, which is visible. The
  peer heartbeat already measures round-trip time in `infra/sync/peer.rs`, so an offset
  could ride along on it; nothing does yet.
- **A tick never idles while an effect runs**, on every station, and a 500-fixture rig
  is 500 writes a tick. `emit` drops the no-ops, but the work is still done. This is
  the same question task 10 left open about partitioning, arriving from another
  direction.

### 20. Telling a node the shape instead of the samples (done)

The console side of the wire. A node that can trace a shape for itself is handed a
description and then left alone; one that cannot sees exactly what it always saw.

The arithmetic is the argument. A three second fade at 40 Hz is a hundred and twenty
MQTT messages to a node that could have been told "go to 1.0 over three seconds"
once. A chase is worse: it never stops. `drive_the_ports` now sends each port the
least it needs to hear — a shape, or a timed `set`, or the stream of values — and the
tests mostly assert what was *not* sent.

**Capability is per port and per shape.** A relay that can chop a square wave has no
way to trace a sine, and a port that lists its shapes lets the console find that out
without trying. Absent means the old behaviour, so nothing is negotiated and firmware
that has never heard of any of this is unaffected. It is read out of the raw `/info`
body rather than through `PortDescription`, for the reason task 18 records, and a
test now pins that end to end by adopting the same node twice — once advertising,
once not — and asserting one fixture type.

**Two messages, in order, to stop.** Taking a light out of a chase is `{"clear":true}`
and then a value, and the plugin drops its record of what the port was last sent so
the value goes out even when it looks unchanged. The node has gone back to holding
whatever the shape left it on, which is not what the console last recorded sending it.
Either message alone leaves the light wrong.

**A clock, because a phase needs one.** `openhaunt/clock`, retained, once a second
while this station is driving. Retained matters: a node connecting between ticks gets
an answer on subscribe rather than rendering against a guess for up to a second.
`seq` counts up so a node can tell a fresh sample from a retained one replayed after
the broker restarted. This is the first periodic thing in `DeviceManager::run`, which
until now was entirely event-driven.

Only the driving station publishes it. A follower putting its own idea of the time on
the leader's broker would give every node two answers, and the entire point of the
topic is that there is one.

**A timed `set` degrades to an untimed one.** The destination sits at the top level in
the port's ordinary payload shape and the timing rides beside it, so a node that
ignores `fade_ms` still lands on the right value — just immediately. That is the
fallback the whole design leans on: unknown keys are harmless, so no version has to be
agreed.

Nothing on the console renders differently. Art-Net, sACN and the universe cache still
read `live_values` and are untouched.

### 21. The simulated node renders for itself (done)

The other end of task 20. `openhaunt-node-sim` now advertises what each of its ports
can trace, accepts a descriptor, and animates on its own while nothing on the network
says anything to it.

**Written from the documents, not shared with the console.** `src/motion.rs` is a
second implementation of the five shapes, the five easings and the cycle arithmetic,
and it deliberately imports nothing from `pult-schema`. That is the reason the
simulator exists: two implementations that agree because they were both written from
the protocol prove the protocol is unambiguous, and two that agree because they share
a module prove nothing. Both test suites assert the same numeric table, and
`test_curve.c` in the firmware will be the third.

The one shared constant is the clock topic, and the end-to-end test asserts the two
crates' spellings of it match — a shape is only as good as the clock under it, and a
clock published where nobody is listening is no clock at all.

**Every write to a port goes through one place.** `Node::write_port` is the single
answer to "where is this port", used by a `set` off MQTT, by `POST /state`, and by
`run_renderer` forty times a second. So `GET /api/v1/state` and the panel both see a
port that is genuinely moving rather than a description of one that might be, and the
end-to-end test can prove the node moved by reading it twice.

**A `set` always cancels.** A console that has decided to send a value has taken the
port back; a shape still running underneath would overwrite it on the very next tick.
Clearing an effect leaves the port exactly where the shape had got to, which is why
the console follows a clear with a value — this node has no opinion about where a
stopped chase should leave things, and inventing one would be inventing state the
console does not know about.

**The clock estimate is smoothed and slew-safe.** The first live sample is taken
outright; later ones move a fifth of the way, because the error being corrected is
one-way network latency and a jump straight to each sample would jog a running effect
by however much that varied. A retained sample only ever seeds: the broker replays it
on subscribe and it was published at an unknown time in the past. A `seq` that goes
backwards means the broker restarted, and the estimate starts again rather than being
dragged through a number from before the gap.

**The editor can lie, and the config checker says so.** A `readonly` port that
advertises effects is a promise nothing will ever ask this node to keep, and a string
port that lists shapes is claiming there is something between two strings for a sine
to trace. Both are `problems()`, not deserialisation failures, so a config file with
one still loads into the editor to be fixed. `configs/fog-machine.json` and
`configs/mirror.json` both advertise now, and the mirror's engraved line advertises
steps and no shapes, which is the honest answer for a string.

### 22. The firmware renders too (done)

Two commits in [the firmware repo](https://github.com/OpenHaunt/node), not this one:
a pure-C core with host tests, then the integration. `oh_curve.c` is the third
implementation of the numeric table, after `model/effects.rs` and the simulator's
`motion.rs`, and the three share no code by design.

**What the bench showed.** A 0.5 Hz sine, applied from a running the-pult to an
adopted ESP32-1732S019, is *one* MQTT message and then twelve seconds of silence
while the strip port moves on its own. At 40 Hz that window used to be 480 messages.
A three second ease-in-out fade arrives as one timed `set` and the node walks it:
0.003, 0.10, 0.33, 0.66, 0.90, 1.0.

**The clock is not a formality.** Between an NTP-synced node and an NTP-synced
console on one LAN, the measured offset settled around -100 ms. That is well over a
tenth of a cycle on a 2 Hz chase, and it is exactly the skew `openhaunt/clock` exists
to absorb — the assumption that two NTP clients agree closely enough to share a phase
turns out to be worth checking, and does not hold.

**A plain `set` always cancels.** Stopping an effect is a clear and a value, and
`MqttLink::publish` spawns a task per publish, so the two can arrive either way
round. On the bench they did: the value landed before the clear. Both orders leave
the port in the right place, because a value takes the port back on its own — which
was designed in rather than discovered, but it is good to have watched it happen.

**A node upgraded in the field advertises nothing until its module is re-applied.**
The description is stored, and one written before the `effects` bits existed has none.
`oh vmod preset led` fixes it; a real module's EEPROM would carry the new field.
Worth knowing before wondering why the console is still streaming values at a node
whose firmware plainly supports better.

Three smaller things the compiler and the bench found, in the order they hurt:
a `-Wformat-truncation` error the host clang build does not produce but the Xtensa
GCC one does; a 1 KiB default MQTT RX buffer that would have dropped a sixteen-step
chase silently, now 4 KiB; and a `display_dirty` flag added and then removed on the
spot, because the display already marks itself dirty on any `OH_EVENT` and the field
had no reader — which is the same fault task 17 spent a commit removing.

### 23. View by default, edit on purpose (done)

The rule, the shared controls it needs, and the Patch panel as the worked example.

**Why a lock at all.** A console is a tablet gaffer-taped to a truss as often as it
is a desk with a mouse, and every panel written so far is two things at once: a view
of the show and a way to change it. On a laptop that costs nothing. On a tablet, a
delete button eight pixels from a row selector is a fixture unpatched during a show.
So a panel that can change the show opens read-only and says so, and the operator
unlocks it deliberately.

**The toggle is in the tile chrome, not in the panel.** It is the same control with
the same meaning everywhere it appears, and a panel's own buttons are exactly what it
must not be mistaken for. `PanelMeta.editable` marks which panels get one; closing a
panel locks it again, so reopening it an hour into a show does not put an unlocked
delete under a thumb with nothing having said so.

**Controls are removed from the DOM, not disabled.** A greyed-out button invites a
second, harder press. One that is not there says what it means. In view mode the
Patch panel is text cells and the row selectors — because selecting a fixture changes
nothing about the show and is what an operator does most.

**`styles/controls.css`, beside `tokens.css` and with the same policy.** Tokens say
what the colours are; this says what buttons, inputs and rows look like. Panels
written before it keep their own styles rather than being churned. `--hit: 44px` is
the target every converted control is built to, but a control reaches it with padding
rather than by *being* 44 pixels — a row of 44px faders puts four on a tablet screen.
The exception is the fader itself, which is dragged rather than pressed and so has to
be the size of the gesture: that is what `--fader-h` is for.

**Two kinds the picker was missing.** `Raw` and `Named` were left off the parameter
list on the reasoning that neither is chosen by name — a raw channel is addressed by
its binding, and a named parameter comes from a device describing itself. Both
arguments are right about where those kinds *come from* and wrong about who has to
type one: a light nobody has written a profile for is mostly raw channels, and
building its type by hand was impossible without editing JSON. So `kindFromLabel`
takes a name, and `kindOption` joins `kindLabel` — the two answer different questions
and a `<select>` needs the other one. A named parameter shows its own name, because
the word "Named" tells an operator nothing about which port they are looking at.

**Two things the browser found.** Per-parameter `default_value` had a schema field, a
reader in the output plugins and no way to set it; it is a `ValueControl` in the type
editor now, so a moving head can rest centred rather than hard left. And seeding a
fresh show broke: `fixtures/__create` refused a body that omitted `live_effects`,
which no client can know. LOCAL fields are `#[serde(default)]` now — a field the
station works out for itself is not one a client should have to name.

### 24. Speed masters (done)

A tempo several effects follow, and a big button to tap it with.

**Tap writes the tempo and the anchor together.** That pairing is the whole reason a
tempo change is a bounded step in phase rather than a slide: every station re-resolves
from the new bpm measured from the instant of the tap, so they all land in the same
place rather than each drifting on from wherever it happened to be. Editing the bpm by
hand re-anchors for the same reason, and starting a stopped master starts its beat
*here* rather than resuming a cycle that has notionally been running the whole time it
was off.

**The average is over the run, not the last gap.** A hand is not a metronome, and at
120 bpm a single 40 ms slip is 10 bpm — plainly audible. Anything before a two-second
pause is dropped, because that is the operator stopping rather than tapping very
slowly, and counting the pause as an interval would drag the tempo down for the next
several taps. The panel shows how many taps are in the run, which is the difference
between a tempo you trust and a number that appeared.

**Tap and Run/Stop stay live when the panel is locked.** Both are done during a show,
at speed, and a lock that stopped an operator following a tempo change is a lock
nobody would leave on. What the lock covers is renaming, the multiplier, and delete.

One beat clock for the panel rather than one per master: a dozen masters would be a
dozen animation frames doing the same arithmetic against the same millisecond.

`beatPhase` wraps rather than truncating, so a master whose anchor is slightly in the
future — which clock skew between two consoles makes ordinary — puts its dot at the
top of the beat rather than at a negative position.

Found by driving it: the create form used `autofocus`, which browsers apply on page
load and not reliably to an input that has just been inserted. Every other panel here
uses the `focusOnMount` action, and now so does this one.

### 25. The effects panel (done)

Where a chase is built. No Edit toggle — this panel *is* an editor, and it writes to
the programmer, which is the scratch buffer.

**The waveform is the panel.** An operator asking for "a chase across these six"
wants to see six dots moving round one curve, not read six phase numbers. The dots
are drawn from the same arithmetic `model/effects.rs` renders with, so they are where
the lights are; a browser that worked it out differently would be drawing a lie, and
`effects.test.ts` asserts the same numeric table the engine's tests do.

**Spreads are where "make them chase" becomes a number per fixture.** Even, Chase,
Reversed, Centre out, Wings, Groups and Random, each tested at one, four and five
fixtures — a selection of one has to survive every one of them, because "chase these"
with one light selected is an ordinary thing to do by accident and dividing by n − 1
would give NaN. Chase is `i / n` rather than `i / (n − 1)` on purpose: the last
fixture should be one step short of the first, not on top of it, or a four-light
chase looks like three.

**A random spread is a seed, not a roll.** `Math.random()` would make the phases a
fact about this browser at this moment, which is exactly what they must not be. The
seed is stored and the phases are rebuilt from it, so two consoles chase identically
and a reload does not reshuffle the rig. Reseeding is how you ask for a different
arrangement.

**One `effect_id` across the selection**, so the panel can gather a selection's worth
of specs back into one editable effect rather than the operator finding six unrelated
sines to change one at a time. The anchor is set on Apply, not on opening the panel:
one set when the panel opened would start the effect part way through its first cycle.

`setEffect` writes straight through rather than staging like `setValue`. Applying an
effect is one deliberate act, not a fader being dragged, so there is no burst to
coalesce — but it does clear any pending value for the same key first, or a drag
still in flight would land after it and cover it.

**A chip in the values panel, not a number.** The value under an effect is only where
it falls back to, so showing it would show a number the light is not at. The chip says
what has the parameter — `sine · 0.5 Hz` — and opens the effects panel, for which
`revealPanel` is new: bring a panel to the front wherever it lives, or open it if it
is not on screen. Where in the workspace a panel sits is not something an operator
should have to know to follow a link.

Two things the browser found. The parameter picker bound a null while the effect used
a fallback, so it drew a blank box over a working effect. And `bpm` is an `f32`, so a
tapped 56.1 comes back as 56.099998474121094 and a number input shows every digit —
rounded for display now, which is a console that can count.

### 26. Cue timing, and what a cue is actually doing (done)

The backend has had per-capture fade, delay and follow-after since task 3, and the
frontend has never had anywhere to type them. It does now, along with the two things
fractional cue numbers were always for.

**A cue list says what was asked for; the running strip says what is happening.**
During a three second fade or a running chase those are different things, and the
difference is exactly what an operator wants to see. It reads `live_fades` and
`live_effects` — the LOCAL fields task 19 added — and shows only what *this cue* put
there: a programmer effect over the top is the operator's, not the cue's, and saying
otherwise would be a lie. Fades are blue and arrowed because they are on their way
somewhere and will stop; effects are amber because they will not.

**Insert, at last.** `Cue.number` has been a float since the first sketch and nothing
ever inserted, so cues were numbered by counting. `insertNumber` takes the midpoint,
which is how a list survives being inserted into repeatedly without renumbering
everything below — and appending to a list that ends at 4.75 gives 5, not 5.75.

**Dragging a cue changes the order and not the numbers.** `cue_ids` is `ordered` in
the schema, so that is what a drag rewrites. Renumbering would make a cue an operator
calls "cue 5" stop being cue 5 because somebody moved cue 2, which is the one thing
cue numbers exist to prevent.

**GO and reset stay live when the panel is locked.** Running a show is what the panel
is for. What the lock covers is rewriting the cue list while it is being run from:
rename, delete, insert, reorder, and the timing — a fade time changed by a mis-hit is
a look arriving at the wrong moment with nothing on stage to say why.

The store menu gains a fade, a delay and a curve per capture, cue-level fade in and
out, and a follow mode. Zero on a capture means "use the cue's", which is what
`start_cue` already did, so leaving every row alone gives exactly the behaviour the
console had before there was anywhere to type these.

`DEFAULT_FADE_MS` replaces the hardcoded `500` and the `0, 0, 0` that every stored
capture used to get.

### 27. Devices, plans, positions and a flow's name (done)

The remaining gaps where the backend had a field and the frontend had no way in.

**A device's detail.** Address, host, firmware and protocol version, module id and
serial — and a port table with a *Can trace* column reading the capability the node
advertised in `/info`. Behind a disclosure because it is what you look at when
something is wrong and clutter the rest of the time. Adopt and Forget go behind the
lock; Find and Select stay live, because neither changes the show.

**More than one plan.** A show has as many as it has rooms, and the panel showed the
first and offered no way to reach the others — worse, the upload button *replaced*
the plan you had, so a second room cost you the first. There is a picker now, `New
plan` adds, and `stores/stage.ts` holds which one this browser is looking at so the
3D rig draws the same room's floor. Not show data, for the same reason the layout is
not: two operators at two screens want different rooms up.

**Plan rotation.** `stage.ts` has rotated plans since positions landed and nothing
could set the angle. A drawing squared up to the page is rarely squared up to the
room.

**Positions by typing.** Dragging in the plan is right for a whole rig at once and
useless for the one light that has to be at exactly 4.2 metres because the drawing
says so. `PositionEditor` gives x, trim and z — trim, because that is the word on the
plot an operator is copying — and a checkbox that turns a `Point` into an `Axial` one
with a direction vector. `splitPosition`/`joinPosition` keep the two forms from
drifting apart: they are the same fact with one detail added, and an editor should
not make somebody choose a variant before they can type a number. A new direction
points straight down, because a light with no aim yet is hanging, and aiming it at
the origin would point an upstage light across the room.

**A flow's name.** How you find one in a list of twenty, and there was no way to
change it. Double-click, behind the lock beside delete.

Found by driving it: the maximised view draws its own chrome and had no Edit toggle,
so filling the screen with a panel — exactly when there is room to edit it — was the
one place the lock could not be undone. And the device row was a flex row, which laid
the new detail panel out *beside* the name instead of beneath it; it is a grid now.

That leaves, from the gap list: device rename and configuration, blind, highlight and
fan, go-back, release and rate, and a timecode source. All of them want backend work
first except blind, which wants a second programmer buffer.

### 28. The demo, and what is left (done)

`scripts/demo.sh` starts a third node on `configs/fog-machine.json`, whose fog output
advertises every shape — so the demo has something to point at when it claims an
effect leaves the console as one message. Its "Try, in order" gains the effect, the
tap, the fade and the read-only patch, and now ends at thirteen steps rather than
eight.

`scripts/demo-seed.mjs` seeds a 120 bpm master at half speed and a third cue,
*Possession*, with a colour sine on the two heads half a cycle apart. Verified by
running it: both resolve to 1.0 Hz off the master, and the two heads' blue channels
sum to 1.00 at every reading, which is what "half a cycle apart" means when you can
see it.

Two stale things removed. The state table said effects and timecode were "not
started" with no schema; they are now separate rows because one is done and the
other is not, and `FollowMode::Timecode` has existed unimplemented since task 3.
And task 16 still listed "the simulator's panel cannot restart the node" as open,
which `Stopper` fixed.

**What is left of the GUI gaps**, all of which want something before they can be
built rather than being merely unwritten:

- **Device rename and configuration.** A node's name comes from its TXT record and
  `POST /api/v1/config` takes one. Nothing asks.
- **Blind.** Programming a look nobody sees needs a second programmer buffer, and
  the priority rule in `playback/programmer.rs` currently has exactly one overlay.
- **Highlight and fan.** Highlight wants a temporary override above the programmer;
  fan wants the spread arithmetic from task 25 applied to values rather than phases,
  which is a smaller job than it was before that landed.
- **Go back, release, and rate.** Going back needs the cue *before* this one on the
  same anchor arithmetic; rate is a live multiplier on a running fade, which the
  `Fade` struct has no room for yet.
- **A timecode source.** `FollowMode::Timecode` is matched and ignored. It should
  wait for the beat-grid work rather than sit beside it.

Task 8 — the WASM plugin runtime — is still the largest thing not started, and is
next.

### 29. What a tick actually costs (done)

Task 19 left this open as a worry rather than a number: an effect never arrives
anywhere, so a station running one never idles. Before effects existed a settled show
stopped ticking; now a show with a chase up ticks at 40 Hz for as long as it is up, on
every station. Measuring it found a real bug.

**`ShowView` scanned the fixture slice for every lookup**, which made the tick
quadratic in the size of the rig. `emit` looked up each fixture that moved, and
`live_value` looked one up for every key a fade or an effect started on — both by
walking the slice. Nothing noticed while a settled show stopped ticking, because the
scans only happen on ticks that do work, and before effects there were very few of
those in a row. The struct already indexed cues by id and did not index fixtures; it
does now. A thousand fixtures went from 29% of the tick budget to 16%.

**The numbers, release build, one effect across the whole rig:**

| Rig | `Playback::tick` | Process CPU |
|---|---|---|
| idle | — | 0.2% |
| 500 fixtures | 2.0 ms (8% of the 25 ms budget) | 24% of one core |
| 2000 fixtures | 7.9 ms (32%) | ~137%, over one core |

`Playback::tick` is the small half. The rest is the engine doing one `apply_local`,
one broadcast and one output push per fixture that moved — which is the cost task 19
actually named, and it is linear.

**The failure mode is graceful, and by construction.** At 2000 fixtures the process
is over one core and the tick cannot keep 40 Hz — but the effect still reaches the
top and bottom of its range on time, because a value is computed from the wall clock
rather than accumulated from the last one. A slow tick loses smoothness, not
correctness, and two stations under different load still agree. That is the same
property that makes stations agree at all, arriving somewhere it was not designed for.

Left open, and unchanged: nothing partitions fixture computation, so every station
computes every fixture. Task 10 named that and it is still the answer for a rig where
24% of a core is not acceptable. The obvious cheaper win first is that `emit` clones a
fixture's whole `live_values` map per tick and the engine then serialises it whole; a
per-key write would cut both. Neither is worth doing on these numbers.

### 30. Selection as a question about the rig (done)

The spec has asked for this since the first read and said why: a selection should be
*generated* from the rig by geometric functions and re-evaluated as the rig changes,
"useful for festivals, changing fixtures". Task 14 built the list of ids that comes
first. This is the query underneath it.

**A list of ids is a photograph of a rig that has since been rebuilt.** "The four
movers on the downstage truss" is still true after somebody patches a fifth; a list
of four ids is not. Verified the way it matters: with "every Spot" selected, patching
a third Spot took the count from two to three with nobody touching the panel.

**Read left to right, because that is how an operator says it.** A query is a list of
clauses that each add, narrow or remove — "all the movers, of those the downstage
ones, but not the broken one". A boolean tree would be more general and nobody wants
to type one. Nothing here forecloses a tree if a query ever needs one.

**Hand-picking is a query too.** A click builds an `Ids` clause, so clicking and a
geometric selection are the same kind of thing and combine: shift-click one more
light onto "every Spot" and the geometry still picks up the next Spot patched. There
is one representation rather than a list *and* a query with rules about which wins.

**Order is part of the selection, not decoration.** An effect spreads along it, so
this is what makes a chase run left to right rather than in patch order. Ties break
by name, so two fixtures at the same point come out the same on two consoles rather
than in whatever order each happened to list the rig.

**An unplaced fixture fails every geometric term**, and sorts last rather than to the
origin — where it would sit in the middle of the rig pretending to be somewhere. That
is why `Everything` and `OfType` exist: they are how a rig in flight cases is
reachable at all.

**`pruneSelection` is gone.** It existed because a list could hold a fixture that had
left the rig; a query cannot. The panel that called it no longer needs to know the rig
changed. That is the shape of the whole change: a question does not go stale.

Left open: a query cannot be saved. Groups are the obvious next thing and the moment
they exist the types belong in `pult-schema` rather than the frontend, with an
evaluator beside this one. Until something needs that, one implementation is better
than two that can disagree — the comment at the top of `selection.ts` says so, so the
next person knows it was a decision.

### 31. Taking it back (done)

Every console has an Oops key and every operator reaches for it before they can name
what they meant to do. This is that, plus the shared list of what everyone has been
doing, which turns out to be the more useful half on a two-operator tech.

**There is no undo stack.** The oplog already holds every write in order, and a
second list beside it would be a second thing to keep in step. So an operation
carries three more fields — who asked for it, what was there before, and which
operation it reverses — and undo is a query over the log. Three consequences fall
out rather than being built: an undo replicates to peers like any other write, so a
second station is not left showing a value that has been taken back; the same
person's other client sees it; and redo is undoing an undo, so one mechanism covers
both.

**Undo is per person, not per browser.** That was the ask, and it is the reason the
log rather than a stack: an operator with a desk and a tablet is one person, and
either device takes back what the other did. A browser-local stack could not do that
however carefully it was written. So the show gained a `users` table, a client says
who it is when it connects and again on every reconnect, and every write it makes is
attributed. A socket that came back anonymous would keep working and quietly stop
being undoable, which is the kind of fault nobody notices until they press Ctrl-Z.

**What undoes: everything editable, and nothing that moves lights.** The programmer
counts, patch counts, cue timing counts. A `goNext` does not. Commands are excluded
in `Operation::is_undoable` rather than at the call site, so a new command is
non-undoable by default — the safe direction. An operator who pressed Ctrl-Z
expecting to take back an edit would not thank a console that jumped the sequence
instead; going back a cue is a different gesture and has a different name.

**Undo and redo are told apart by chain depth, not by a flag.** An operation's depth
is how far along a chain of reversals it sits: a change is 0, an undo of it 1, a redo
of that 2. Undo takes the newest thing in effect at even depth, redo at odd. The
first attempt used "does it point at something" and quietly took the change away
again when you redid twice — an undo wearing the wrong label. And a fresh change ends
the redo branch, checked separately, because putting an old value back on top of work
done since is not what anybody means by redo.

**Whether something is still in effect is recursive.** An operation is reversed when
something points at it that is *itself* still in effect, so an undo that has since
been redone no longer hides what it undid. Flat set membership says a change stays
undone after it has been put back, and the second undo of a run then finds nothing to
do.

**The window counts edits, not rows.** Undo reads the most recent 500 authored
operations. Counting every row instead would have made it a window of about a quarter
of an hour, because a station writes its own telemetry into the log twice a second —
so a rename made twenty minutes ago would have stopped being undoable while its
author was still thinking about it. The same filter is what makes the History panel
readable: it lists what people did, and the console's own doing does not appear,
because nobody did it.

**Seeing is shared even though undoing is not.** The History panel shows everyone,
colour-coded and named, and marks which entries are yours to take back. On a
two-operator tech the useful question is usually "what just happened", and the answer
is often somebody else. Undos appear as themselves rather than being tidied away,
which makes the list a true account.

Left open at the time: a group undo, which is task 32. Nothing prunes the log yet,
so a long show's history grows without bound. And a user is a name and a colour, with
no notion of a station being signed into: two people at one desk have to remember to
switch.

### 32. One drag, one press (done)

Task 31 shipped with a hole in it: dragging a fader wrote a few hundred operations
and Ctrl-Z took back the last one. Right for a rename, useless for a move. This
closes it.

**Only the client knows where an act begins.** The backend sees a stream of writes,
and no amount of guessing at the gaps between them tells a drag from two quick edits
— a slow drag has longer pauses in it than a fast pair of clicks. So the frontend
says: everything written between a pointer going down and coming up carries one
gesture id, stamped on the message in one place, `PultWsClient.set`. A drag reaches
the socket through the programmer's staging, a path proxy and a panel's own handler,
and any of the three remembering to pass a gesture along would eventually be the one
that forgot.

**A gesture is not closed when the pointer lifts.** The programmer stages a move and
writes it on the next frame, and a fan across twenty fixtures is twenty round trips
after that, so a gesture that ended with the pointer would leave its own tail outside
it — and one drag would want three presses to take back. It closes 400 ms later
instead. Closing late is free: the only thing a stale id could spoil is the *next*
gesture, and beginning one replaces the id outright. The same timer gives a held
arrow key a gesture, which is a drag with no pointer to say when it stopped.

**An ordinary write is a gesture of one, keyed by its own id.** That is what keeps
`undo.rs` a single code path rather than two, and it is why a row written before
gestures existed still behaves exactly as it did. `undoes` came to name a gesture
rather than an operation for the same reason, and needed no migration to do it.

**Reversing a gesture writes one operation per thing it touched, not per operation.**
Otherwise taking back four hundred writes would put four hundred rows in the log, and
the log would grow faster the more of it you took back. The value it goes back to is
the one from before the *first* write at that path, which is what "before the drag"
means.

Three rules found by getting them wrong first. **A create is keyed by what it made,
not by where it was written** — two fixtures patched in one gesture are two writes to
the same `fixtures/__create` path, and collapsing them by path deleted one and left
the other standing. **A field written into something the gesture created is dropped**,
because the entity is going away and putting a value back into it first describes a
state nobody will ever see. And **the paths are unpicked newest first**, or a delete
runs before the rename above it and the rename writes into a hole.

**A gesture had to become the unit of `in_effect` and `depth` too.** Reversing a drag
writes one row against four hundred, so three hundred and ninety-nine of the drag's
operations have nothing pointing at them — and undo, looking at operations, found one
of those still standing and took the same drag back again.

Two things fixed on the way past. A peer received `user_id` and `previous` but not
`undoes`, so an undo replicated to another station landed in its log as a fresh
change and the next Ctrl-Z there took back the wrong thing; all four fields travel
now, as one `Authorship`. And the codegen owns `001_initial.sql`, so the oplog's
columns are declared in `tools/pult-codegen/src/main.rs` — a hand-edit to the
migration is overwritten by the next `generate`.

**And a gesture is what finally let the log stop growing at forty rows a second.** A
write inside one replaces that gesture's earlier write to the same path rather than
landing beside it: the row keeps the value the drag started from and takes the one it
ended on, which is exactly the pair undo wants and exactly what a peer catching up on
that path needs. Two seconds of dragging across a selection of twenty went from 2,400
rows to 20. The row takes the new sequence number as well as the new value, and that
is the part that makes it safe rather than merely smaller — catch-up asks for
everything past a sequence number, so a row that kept its first one would be
invisible to a peer that had already caught up mid-drag and would leave it sitting at
whatever value the drag was passing through when the two of them last spoke. Only
within one gesture, because two separate edits to the same path are two things
somebody did and folding them would leave the second with nothing to go back to. And
never a create, for the same reason creates are keyed by what they made.

Left open: a gesture is per client, so two operators dragging the same fader from two
consoles interleave into two gestures that each own half the writes. Undo still does
the right thing for each of them, but "before the drag" means before *their* first
write, not before the pair started. Nothing prunes the log. And nothing outside the
pointer controls opens a gesture — a panel that writes several fields from one button
could, and none does yet.

### 33. Two kinds of setting (done)

The console had no settings. A show had a name; everything else about how a station
behaved was a flag decided before it started and unchangeable from the desk. The
first thing that actually needed one was how far back Ctrl-Z reaches, and it turned
out to need both kinds at once.

**A show setting travels with the show.** `Show.history_depth` is PERSISTED and
replicates, because two consoles working one show have to agree about how far back
undo goes — a station-local number would not be a preference, it would be a
disagreement, and the two desks would give different answers about the same press.

**A console setting belongs to the machine somebody is sitting at.** A
`preferences.toml` in the platform's config directory, read and written over
`GET`/`PUT /api/preferences`. It decides what a *new* show starts with and then stops
mattering, which is exactly what keeps the first rule true. Machine-wide rather than
beside the showfile, which is the opposite of task 17's identity file and for the
opposite reason: an identity must not travel with a copied showfile, and a preference
about new shows has no showfile to sit beside. It is stateless in the router, so a
second console on the same machine sees a change without either being restarted and
there is no copy in memory to go stale.

**The number is changes, not presses.** An undo is a change too and shares the window
with the ones it reverses, so a run of them meets itself around half way: five hundred
changes is on the order of two hundred and fifty consecutive presses. Found by writing
a test that expected ten presses from a window of ten and got five. The panel says so,
because a number whose meaning has to be derived is a number people will get wrong.

**`#[serde(default)]` does not reach the showfile.** A SQL read does not go through
serde, so the new column — added nullable, as the additive pass must — came back as
zero on every existing show, and the clamp would have quietly left them with a depth
of ten. Fixed with a backfill in `upgrades.rs`, which is what that module is for, plus
a `const _: () = assert!(...)` tying the literal in the SQL to the constant the rest
of the console uses, so changing one without the other is a build error rather than a
surprise a year later.

Left open: this is one setting, and the panel is shaped for more. Nothing else has
moved into it yet — the output, session and device panels still own their own
configuration, which is right while each is a page of its own and will stop being
right when the second console-level preference arrives. The preferences file has no
version field.

### 34. A show carries its plugins (done)

A plugin was a directory on one
station's disk named by `--plugins`, which is right for developing one and
wrong for everything else — a show authored with the command line's panels in
its layout degraded on every other console in the session, and installing on a
ten-station rig was ten copies.

**A `plugin_packages` roster is PERSISTED, and the bundle is an asset.** The row
names its bundle by the sha256 of the zip; the bytes go in the same
content-addressed store task 13 built for stage plans, so `fetch_from_peers`,
the digest check and the dedup were all already written. Adding the collection
needed no edit outside `pult-schema`, which is task 2's promise still holding
thirty-two tasks later.

**The digest covers the archive, not the component.** A plugin is a manifest
*plus* a component *plus* the scripts a panel loads; hashing the wasm alone
would leave two thirds of it unversioned, and a changed panel script would be
indistinguishable from no change at all.

**Stations reconcile, keyed by `(plugin_id, sha256)`.** The diff is a pure
function in `roster.rs`, so the interesting cases are tested without a station,
a showfile or a wasm engine. The action worth having a name for is `Publish`: a
row carries a display name and a stage hint as well as a digest, and editing
those must not restart the plugin. That is task 9's lesson about outputs
arriving from another direction — rebuilding a live thing for a label edit put a
redundant frame on the wire, and here it would be a plugin restarting mid-show.

**A fetch never happens inside the event loop**, for the reason task 8 recorded:
the manager awaiting something that can call back into it is how the first
version deadlocked. It runs on its own task and reports back as a message, and
the test stands up a peer that accepts the request and then says nothing.

**`Fetching` is a state, not a failure.** A station that has just joined and is
downloading is working; saying "failed" would send an operator looking for a
fault that is not there.

**A `--plugins` directory beats the show**, on that station only. Otherwise
somebody editing a plugin on a console joined to a session would silently be
running the show's build — the most confusing thing the runtime could do to the
person least able to explain it.

**Configuration is three layers**, most specific winning: the manifest, the
show's row, then `[plugins.<id>]` in the station's `preferences.toml`. Station
last because what a station overrides is what is true of that machine and no
other, and a show cannot know which console holds the key. A plugin is handed
its configuration in `init` and never again, so a change is a restart.

**The trust assumption, stated rather than left implicit.** Opening a showfile
runs the plugins it carries: no approval step, and the manifest's permissions
are granted by opening the file. The sandbox, the epoch deadline and every
permission gate still bound it; what they do not bound is a plugin that
legitimately has `data = "read-write"` and an HTTP allowlist. So the Plugins
panel prints each plugin's permissions in words, and an approval gate is a
recorded open question with the schema laid out so adding one is a field and a
status rather than a redesign.

Three bugs the tests found, in ascending order of how much they mattered:

- The disk-override flag was set after an early return that fires when the plan
  is empty — which is exactly when the only roster row is the overridden one.
  The flag was never set in the one case it exists for.
- `PUT /api/preferences` built a fresh `Preferences` from the request body,
  which would have erased every plugin's station configuration — an operator's
  API keys among them — on the next change to the history depth.
- **A station that successfully fetched a bundle never ran it.** The
  placeholder row saying "fetching" already carried the digest, so the next diff
  saw a match and decided there was nothing to do. Every earlier test either had
  the cache already warm or had the fetch fail, so none of them went down the
  path the whole change exists for. Found by writing the two-station test last
  rather than first, which is the wrong order and is why it survived that long.

Left open: install over HTTP carries no user, so it is a change nobody can take
back; nothing prunes an unreferenced bundle from the asset store, the same way
nothing prunes a replaced stage plan; and the `stage` hint groups the panel and
gates nothing.

### 35. A plugin can remember things (done)

`lifecycle.init` ran on every start and every reload and nothing survived one,
so no plugin could offer a saved macro, a remembered provider or a snippet
library. Now a manifest declares `[[stores]]`, each `scope = "show"` or
`scope = "station"`, and four calls — `get`, `set`, `delete`, `keys` — reach
them. Declaring the store is the permission: a plugin can address no store it
did not declare and no other plugin's, because the host derives where the data
lives from the plugin id and the store id rather than from anything the guest
passes.

Three decisions worth keeping, and one thing that turned out to be false.

**`scope`, not `lifecycle`.** In this codebase LOCAL means *not persisted* —
state a station holds and shares with its frontends and which does not survive
a reload. A station-scoped store is the opposite: persistent and not
replicated, a fourth combination the enum has no name for and which `identity`
and `preferences` already are two instances of. Naming the axis `scope` says
the true thing — whose data is this — and leaves `Lifecycle` alone.

**Attribution is the switch, so nothing learned what a plugin is.** The design
was going to teach `Operation::is_undoable` to exclude `plugin_data` and add a
matching filter to the History panel. Neither was needed: `is_undoable` already
requires a user and the history reads `WHERE user_id IS NOT NULL`, so a write
made with `user_id: None` is non-undoable and invisible with nothing edited in
`pult-schema`, in the oplog's SQL, or in the frontend. That made the *opt-in*
nearly free as well — a store declaring `undoable = true` gets its writes
attributed to the operator instead, so a macro the operator asked to save
undoes like any other edit while a cache does not. The gesture is kept either
way, because coalescing keys on the gesture rather than on the user. And a
write with no operator behind it stays unattributed however the store is
declared, because `ctx.userId` is absent — the rule falls out of the mechanism
instead of being a second one.

**A key's identity is its row's identity.** `PluginDatum`'s id is a UUIDv5 over
`(plugin_id, store, key)`, not a fresh v4. `create_entity` takes the id from
the value the caller supplies, so a random one would have meant two stations
each writing `macros/opening` created two rows holding the same key — not a
conflict the vector clock resolves, but a duplicate it has no reason to notice,
and a plugin reading back two values for one key. The spec said "last writer
wins by vector clock", which is only true if both stations write the same
entity.

**The WIT version bump could not work as designed, and the package moved to
1.0.** Adding an interface was supposed to take the package from `0.1` to
`0.2`, with `API_VERSION` becoming a supported-versions list. It fails, and not
because of the new import: a component's imports are stamped with the package
version, so a `0.1` guest asks for `pult:plugin/data@0.1.0` and a `0.2` host
offers `@0.2.0` — nothing resolves, and wasmtime's semver-compatible linking
cannot help because under semver a `0.x` minor bump *is* breaking. At `0.x` the
contract could never have grown without stranding every plugin already in a
showfile. So the package is `1.0.0` and the manifest's `api` is a **floor**:
same major, station's minor at least the plugin's. Verified rather than
assumed — a `1.0` component does run on a `1.1` host — and
`scripts/check-api-compat.sh` reproduces the check, because a wasmtime upgrade
could quietly change the answer.

Two smaller things the tests pinned down. Within-gesture coalescing leaves
**two** oplog rows for ten writes to one new key, not one: the first write is a
create, and `fold_into_the_gesture` refuses creates because every create in a
collection shares the `__create` path and folding two would lose a row. And
`describeChange` names ids from fixtures, cues and sequences, so an undoable
store write would have rendered `plugin data → a1b2c3 → value` until the panel
learned to call a row by its plugin, store and key.

Also settled: an earlier draft worried that every browser would receive every
plugin's store, since `frontend_paths()` is derived and has no opt-out. That
was wrong. Subscriptions are demand-driven and per collection, reference-counted
by `stores/show.ts`, so a browser gets a store only if a panel asks for it. The
real cost of choosing an entity is the showfile, the backups and the snapshot a
joining station swallows — which is what the quotas are sized against.

Left open: nothing prunes the oplog, which store writes add to and telemetry
already dominates at twice a second; there is no change notification, so a
plugin holding an undone value in memory learns about it on its next read; and
a plugin that changes its own data's shape is on its own, as any application is.

### 36. A show always has somebody (done)

Undo shipped in task 31 and did not work on a new show. `users` was a PERSISTED
collection that nothing ever seeded, so a fresh showfile had none; the frontend's
`userId` started `null`, "before anybody has said"; and `Operation::is_undoable()`
requires `user_id.is_some()`. The first change an operator made therefore carried
`Authorship { user_id: None }` and could never be taken back — not later, and not
once they finally said who they were. Three pieces of UI existed to describe that
hole rather than fix it: a toast reading "Say who you are first", a history panel
empty state saying the same, and a chip bordered in `--live` whose tooltip was
"Nobody is signed in — changes cannot be taken back". All three are gone.

**There is no no-user now.** The engine seeds a default user at the end of
`load_from_showfile`, so it happens with no browser attached — a station runs
headless, and plugins and station RPCs write too. And the WebSocket write path
falls back to that user for a socket that never identified, so the guarantee does
not rest on a well-behaved client.

**The id is a fixed constant, and that is the load-bearing decision.** A UUIDv5
over the show's id would have matched how a `PluginDatum` derives its own, but a
v5 needs the `Show` row to exist at seed time and the load path promises no such
thing for an empty showfile. The stronger reason is the frontend: it has to be
working as *somebody* before its first write, and anything it had to fetch — or
wait for the `users` collection to deliver — would leave a window in which a
change is attributed to nobody, which is the exact bug. A constant has no window.
So `frontend/src/lib/users.ts` holds it too, as it already held `USER_COLOURS`
for a weaker version of the same argument, and a Rust test reads that file and
asserts the two agree. Duplication with a guard rather than duplication with a
comment; the guard was checked by breaking it.

**One per show, not per station.** `user.rs` opens by arguing that identity is
*chosen* rather than derived from the machine, because a person's desk and tablet
are both them — deriving the default from `node_id` would have contradicted the
reason the type exists, and put a login-shaped fact into a file that travels. So
two stations opening one show converge on one row.

The trap was `create_entity`: it validates, persists and inserts with **no
existence check**. An unconditional seed at every load would rewrite the row every
start, and on a second station replicate "Operator" over a name somebody chose.
The seed is conditional, and two tests hold it down — one that three loads write
no oplog rows at all, one end to end across two stations.

Left open, and accepted rather than solved: a station whose copy of the show
predates the default user, loading while disconnected, creates it concurrently
with another station's rename, and the sync layer breaks that tie rather than
intent. Bounded and self-healing — the ids match, so the worst case is a name
reverting to "Operator" and somebody renaming it again. There is no duplicate
user and no lost operation. Engineering around it would mean making the seed wait
for a session that may never come, which breaks the headless case it exists for.

Two smaller things. The seed is unattributed, like the engine's other writes, so
an operator pressing Ctrl-Z on a fresh show reaches their own first change rather
than the console's act of inventing them. And *Sign out* survives — it was a real
gesture for the end of a session — but lands on the default rather than on
nobody, and forgets the stored identity so the next visit does not resume as
whoever left.

### 37. The log has an end (done)

Nothing deleted an operation. `oplog.rs` had `append`, `since`, `len` and
`recent_by_people` and no `DELETE` anywhere in the backend, so a showfile grew for
as long as it was ever used. `history_depth` was already the number that should
have bounded it and bounded the wrong thing: the `History` command clamped its
limit to it and `recent_by_people` passed it as a SQL `LIMIT`, while everything
past it stayed on disk, invisible and unreachable.

Two facts made it a cost rather than untidiness. **The bulk of the table is not
what anybody did** — a station replaces its own row every two seconds
(`REPORT_INTERVAL`) as a SYNCED whole-row write, so it is logged: around 43,000
rows per station per day, each carrying a whole `Station` struct. And **catch-up
read the whole table**: `since` selects every row with no `WHERE` and filters in
Rust, so every reconnecting peer deserialized the entire log to find the handful
of operations it missed.

**Two retentions, counted differently on purpose.** Authored rows are bounded by
the show's `history_depth`, because that is already its promise about how far
Ctrl-Z reaches. Unattributed rows — nothing undoes them, they never reach the
history panel — are bounded by a *duration*, a station preference defaulting to an
hour. One rule over both would have broken what `recent_by_people`'s filter exists
for: at a row every two seconds, five hundred rows is a few minutes of edits. A
count for one and an age for the other is not an inconsistency: `history_depth`
counts changes because an operator counts changes, and an absence is a duration.

**The floor is the part that makes deleting safe, and it is a seq per node.**
Catch-up compares a peer's vector-clock entry for the node that *wrote* each row,
so "everything up to here is gone" is only meaningful about one node's sequence; a
single pruned-before timestamp would have compared two different kinds of thing.
`operations_since` gained its third reason to answer `None` — it already had "an
empty clock" and "replaying most of the log is a slow snapshot" — and the snapshot
path it falls back to is the one every joining station already takes. Without it a
peer behind the cut receives the surviving rows, sees no error, and believes it is
caught up.

The floor is written **before** the rows go, because the two failure directions
are not symmetric: a floor recorded for rows that were not deleted costs
unnecessary snapshots and stays correct, while rows deleted with no floor recorded
is the silent half-answer. And a floor only ever rises — `MAX` in the upsert, so
two prunes racing cannot interleave into a lower value.

**Pruning is local and never replicated.** Two stations legitimately hold
different amounts of history and each serves catch-up from what it has. Making
deletion a replicated operation would have invented a new kind of write for the
sync layer and let one station's disk pressure delete everyone's history.

**It runs on open and then every thousand appends, spawned rather than awaited.**
The engine is one actor, and this is the only place in it that issues a `DELETE`
over what can be a million rows — awaiting that would be a stalled tick, which is
a far worse failure than a log briefly too long. A flag stops a second prune
starting beside one already running, since two concurrent deletes racing on the
floor is the one way to get its ordering wrong. Driven by appends rather than a
timer because what should pace the work is how much has been written; a timer
wakes to do nothing on an idle show and still lands mid-burst on a busy one.

**The first open of a long show was measured** rather than guessed, since it is
the largest cut this will ever take and it happens at startup: a log of 25,160
rows — a fortnight of two stations' telemetry plus five thousand edits — took
**36 ms** to cut down to 500. The measurement is an `#[ignore]`d test rather than
a threshold, because a number asserted there would fail on a slower disk without
saying anything true.

**`since` was deliberately left alone.** Its missing `WHERE` looks like an obvious
win and is not: the predicate is a vector clock, so the query would be
`(node_id = ? AND seq > ?) OR ...` built per request from the asking peer's clock,
plus every row from a node that clock has never heard of. A query whose shape
depends on the number of peers, to replace a filter that is only expensive because
the table is unbounded, would be treating the symptom of the thing being fixed in
the same change. The retention bounds the table, so it bounds the read.

Two consequences worth holding on to. `HISTORY_DEPTH_MAX` now bounds what is
*kept* as well as what is read, so a larger value is a larger showfile rather than
only a longer query — its comment said "nothing prunes the log yet" and has been
rewritten. And the History panel shows its oldest entry as a boundary: the rows
past it are deleted rather than merely unlisted, and a list that simply ended
would read as a bug.

Left open at the time: nothing vacuumed the showfile, so SQLite kept the freed pages
and the file stayed at its high-water mark — a `showfile-management` question, and not
what this was paying down. **Closed by task 52**, which vacuums on open when more than
a quarter of the file is free: opening is the one moment nothing else is using it. **A station running a build without this change can
short-change a peer from a pruned showfile**, since it serves catch-up without
consulting a floor it does not know about; a session should not mix builds across
it.

### 38. A question worth keeping (done)

Task 30 made a selection a *question* about the rig — "every mover on the downstage
truss" — and then had nowhere to keep one. The types lived in
`frontend/src/lib/selection.ts`, which said so: if saved groups ever became show
data, they would move to `pult-schema` and the backend would get an evaluator beside
this one. Both have happened. `groups` is a PERSISTED collection whose row is a name
and a `SelectionQuery`, and nothing outside `pult-schema` was edited to make it
replicate, persist, appear in `data.ts`, or reach a plugin.

**A group stores the question, not the answer.** Resolving it reads the rig as it is
at that moment, so a fixture patched this afternoon is in this morning's group with
nothing re-saved, and a deleted one leaves it without anything having to prune. That
is the whole reason task 30 made a selection a query in the first place, and storing
the ids would have thrown it away at the moment it finally mattered.

**The evaluator is written twice, and the price is a checked-in corpus.** The
frontend re-evaluates on every change to the query, and a box or a cone dragged
across the rig changes it per frame — evaluation is on the interaction path, and a
round trip per frame is not available. So `evaluate` exists in `pult-schema` and in
`selection.ts`, and `testdata/selection-queries.json` holds one rig and 21 cases that
both test suites read. A case that disagrees fails on the side that is wrong, at the
commit that made it wrong. It is the arrangement `model/effects.rs` and
`frontend/src/lib/effects.ts` already had for the same reason.

**The trap was `Manual` order.** "Whatever order the operator dragged into" lived in
a Svelte store *beside* the query — deliberately, since an in-flight drag is not a
fact about the show. A group has no store behind it: it is read on a station that
never saw that drag, so a group saved left-to-right would have come back in patch
order everywhere else. `SelectionOrder::Manual` now carries the ids, `asSavedQuery()`
bakes the hand order in on the way out, and the evaluator takes `previous:
Option<&[Uuid]>` — `None` means "use what the query carries", and an *empty* hand
order is not the same as no hand order. That distinction is a corpus case, because it
is exactly the kind of thing that is right for a month and then quietly wrong.

**Resolving is a read, so it is a station RPC and not a command.** A
`#[pult_command]` mutates an entity and writes an operation; asking what is in a
group must not appear in anybody's history or undo stack, and a test asserts the
oplog length is unchanged across five resolves. That put a read in `api/rpcs.rs`,
whose header said these were "calls against LOCAL state" — the honest reframe, now
written there, is that these are the calls that *answer* rather than *change*.
`LocalRpcDeps` gained an engine handle to do it.

**And it is called `selection.resolve`, not `group.resolve`.** The command line's
parser checks RPC prefixes before collection names, so an RPC named `group.*` takes
the word `group` out of the grammar: `group 1` parsed as an unknown command on the
`group` RPC. **An RPC's prefix is a reserved word in the command line**, and naming
one after a collection deletes that collection's spelling. Worth remembering the next
time an RPC is added.

The backlog guessed that `group 3 at 50` came free from introspection. It did not:
`fixture` is a keyword in `parse.rs` and `group` had to become one too.
`Command::Select` now carries a `SelectTarget` — a range of the rig or one group —
and either word switches what a bare number counts, so `fixture 1 thru 5 + group 2`
composes with no second code path. Generic entity addressing did give
`rename group 1 "Movers"` for nothing.

**A selection effect can carry a query.** `group 3` on its own hands the browser the
group's *question*, so what the command line leaves is exactly what clicking the
group in the panel leaves — a live selection rather than a photograph of one. Mixed
lines (`fixture 1 + group 2`) resolve to ids instead, because a group's own clauses
may narrow or subtract and appending them would narrow the whole line. `at` needs the
ids either way, which is the one thing `selection.resolve` is called for.

Left open: a group cannot appear *inside* a query (`Term::InGroup`), which would give
live composition and needs cycle detection and an answer for deleting a referenced
group. Recall-then-refine covers the workflow; nothing here forecloses the term.

### 39. How much, rather than what to (done)

Every write was absolute. There was no way to say "ten percent brighter", only "at
62%", and the thing that had kept it that way was not the arithmetic but the
question of *where* a relative write becomes an absolute one. On the client it is
racy in exactly the case that matters: two operators reaching for one fader read
the same number, compute the same destination, and one of the two nudges is lost.

**Resolution happens at the front door and nowhere else.** `resolve_relative` runs
at the top of the `EngineCommand::Set` arm, before `authorship.previous` is read.
Everything below it — the apply, the oplog, the broadcast, `sync.broadcast_synced`
— is code that has never heard of the verb. That one placement is what buys the
whole property list: `previous` is the absolute before, the log holds a
destination, undo reverses it by writing `previous` back, and **a peer receives the
number rather than the delta**. It has to: a peer adding ten percent to whatever it
was showing would part company with the station that sent it on the first press.

The verb is `__by`, beside `__create` and `__delete`, so it arrives over the
existing `Set` message and needed no new protocol, no new host function and no new
permission. `data.set(path + "__by", delta)` is a relative write from a plugin, and
`.by()` on both accessors is the same thing from Rust or the browser.

**Two shapes, and the second one names a collection.**
`[table, ref, field, "__by"]` is the primitive: relative to what that field says.
`["programmer_values", "__by"]` with `{fixtureId, parameterKind, by}` exists
because the programmer's ordinary case is *not already holding the key* — `at +10`
on a light nobody has touched has no row to name, and what it has to be relative to
is then what playback is showing rather than a row that does not exist. So the
engine names one collection for a reason of its own. That is a real cost and it was
taken deliberately: the rule it looks like it breaks is "adding a collection needs
no edit outside pult-schema", and it does not break that — a test nudges a
`speed_masters` field, which the resolver has never heard of, to keep it honest.

What it resolves against is task 14's stack **read rather than re-implemented**:
the programmer's own value where it holds the key, `Fixture::live_values` where it
does not, and the fixture type's `default_value` where nothing has ever driven the
parameter. There is no second priority rule anywhere in this.

**A shape refuses.** `Overlay` holds a key as either a value or an effect, never
both, and nudging a shape would have to mean moving its offset — a different
feature wearing the same word. So does a switch and so does a line of text:
`ParameterValue::nudged` lives in the schema and says no by name, because quietly
doing nothing is worse than an error an operator can read.

**The trap was integers.** `nudge_json` first tried to answer `4500.0` for a
`fade_in_ms` of 3000 plus 1500, and `u64` will not take a float — a nudge on a
timing field failed at the patch rather than at the arithmetic. An integer field
stays one.

`programmer_entry_id` moved into `pult-schema`, where `ProgrammerValue`'s doc
comment already explained why the id is derived. There are now three
implementations of it — the frontend cannot run Rust and the plugins workspace
builds guests for `wasm32-wasip2`, which the console's schema does not belong in —
and all three are pinned to the same two literal examples, so any change to one
fails two suites.

The command line gained a sign rather than a word: `at 10` and `at +10` are
different commands, and `Level::To` / `Level::By` keeps them apart from the parser
down. `+10` is one token — the tokenizer only splits a *lone* sign off — so the
sign is read off the number's text, which is the sort of thing that is right for a
month and then quietly wrong. And the natural-language plugin needed no change at
all: it speaks by emitting command-line text, so "a bit darker" is now `at -10` and
the audit trail stays one grammar deep. That is `nl-show-context`'s option (b),
answered without giving the model any show data.

Left open: no fan, no multiplying sibling, and no relative value stored in a cue —
that last is tracking, which is its own design.

### 40. Four flaky tests, and one of them was the console (done)

`cargo test` failed about one run in three, in a different place each time. None of
it was flaky code in the usual sense; three were tests lying about their isolation,
and the fourth was the console giving a wrong answer that only a busy machine made
visible.

**A process has one environment.** `StationStore::open` read `PULT_PLUGIN_DATA` at
open time, and two stations inside one program cannot each have their own that way —
whichever set it last decided for both. `stores.rs` had already met this and worked
around it with a `OnceLock`, complete with a comment that a file per test would be
"a race over one variable rather than the isolation it looks like". Worse than the
flake: `plugins.rs` starts two stations and only one set the variable, so the other
opened the **developer's real `plugin-data.db`** and wrote to it. `Config::plugin_data`
now names the file, the way `showfile` and `plugin_dirs` already do, and the
environment variable is the fallback for a station started from a shell.

**A test that samples a fade has to sample it early.** The node sim's timed-set test
started a 400 ms fade, slept 200 ms and asserted the level was between the ends. A
saturated machine spends longer than that between the sleep and the read, so the
fade had finished and the assertion read "it jumped" when it had done nothing of the
kind. Three seconds, sampled a sixth of the way in: two and a half seconds of
overrun would now be needed to fool it. The test's own doc comment had said "a three
second fade" all along.

**Connecting is not free on a loaded machine.** Eighteen stations come up in
`roster.rs`, and a connect to a listener whose accept queue has backed up times out
rather than being refused — `ETIMEDOUT` on `127.0.0.1`, arriving as a panic in
whichever test was unlucky. The HTTP helpers are now as patient about connecting as
`eventually` already was about state, with a two-second connect timeout so a dropped
SYN is retried in a moment rather than after the operating system's minute.

**And the one that was not a test problem at all.** The same load made a station
fail to fetch a plugin bundle from a peer, and what it told the operator was *"no
station in this session has the bundle"* — because `fetch_from_peers` folded a
transport error into the same `continue` as a peer answering 404. Those are opposite
diagnoses: one sends you to install the bundle somewhere, the other sends you to the
network. `Fetched` now says which, and `begin_fetch` **asks again** when somebody
could not be reached, four times backing off from a quarter second.

That retry is a real behaviour change and worth stating plainly: nothing re-drove a
fetch, so a station that came up while a peer was still loading its own show had a
permanently failed plugin until the roster changed again. Only an unreachable peer
is worth asking twice — peers that all answered "no" will go on answering no.

The trap in writing it was that `break` left the *previous* attempt's message in
place, so a fetch that ended in "nobody has it" reported "could not reach one
station". Every arm sets the answer before deciding whether to go round again.

Reviewing that retry turned up the thing underneath it. **Every station publishes
its own row into `stations`**, and there were two `peer_addresses`: the one in
`api/rest` filtered self out, and the one in `infra/plugins` did not — while its doc
comment said "the other stations". So a fetch asked *itself* for a bundle it had
just established it did not have, spending a round trip to be told 404 by its own
HTTP server. Harmless-looking until the retry, which made a self-ask that failed
under load cost four attempts instead of one, and which is what put "could not reach
1 station" in front of an operator about their own console. There is now one
`peer_addresses`, in `assets.rs`, and both callers use it.

Three smaller things from the same reading. The constant's comment said "under two
seconds" for backoffs summing to 3.75; it sleeps after the last attempt no longer,
since there is nothing left to wait for; and an `Err` from `fetch_from_peers` is
this station's disk failing to store what came back, not a network, so it stops
rather than asking the same disk the same question four times.

The retry is held by a test that fails without it: a peer that drops the first
caller's connection and serves properly after. Closing a connection before any
response is a transport error rather than an answer, which is the shape of the real
thing and costs milliseconds rather than the ten-second timeout that black-holing
the port would have.

### 41. Where a parameter rests (done)

Nothing in the console ever put a parameter back. `emit()` merged each tick's changes
onto the map a fixture already had and no path ever wrote one back down, so a look
built by a sequence outlived the sequence, and clearing the programmer on a fixture
nothing had touched landed on a hardcoded zero of the right shape. `default_value`
had been sitting on every parameter since the node protocol arrived — derived from
what the device said about its own ports — read only by the connectors, and only as a
fallback for a key that was absent.

**The home value.** A parameter's own resting place: the fixture's `home_values`
override if it has one, and its type's `default_value` otherwise. Resolved once, in
`pult-schema`, because three callers wanted it — a relative write starting from
nothing, sending a selection home, and the programmer letting go of a key nothing was
under. No TypeScript twin: `fixture-groups` pays for two evaluators because a cone
being dragged re-evaluates per frame, and nothing here is on that path. The values
panel still carries a type's `default_value` per row, but that is a display fallback
for an empty readout rather than an answer about the rig.

The override is PERSISTED and on the *fixture*, not the type. A house light that
comes up when nothing is controlling it is a fact about this rig, and a type is
derived: the node describes its ports again, the console rebuilds it, and an override
living there would go with it.

**`__home`, a third path verb.** `["programmer_values", "__home"]` with
`{fixtureId, parameterKind?}`, beside `__by` and resolved in the same place — the top
of the `Set` arm, before `previous` is read — so the oplog, the broadcast and every
peer see ordinary absolute programmer writes. Omitting the kind means every output
parameter of that fixture, enumerated by the station, which is what lets the command
line's `home` and the natural-language plugin ask for it without reading the rig. The
same argument `relative-values` made for `at +10`.

One verb is several writes, which the `Set` arm did not previously allow: it now
resolves to a `Vec` and loops. And when a user's single request expands to more than
one write, the engine mints a **gesture** so one Ctrl-Z takes back the whole fixture
rather than one parameter of it.

**Off, and the change underneath it.** `Sequence::off` was the easy half. The hard
half was that `go_next` wrapped to `None` at the end of the list, so "the operator ran
out of cues" and "the operator turned it off" were the same state and playback could
not tell them apart. **Go at the last cue now stays on the last cue** — a breaking
behaviour change, and the better one: the comment at `playback.rs:283` had always
wanted a light not to go dark for want of cues, and leaving the last cue *active*
does that honestly, where leaving the values behind under a cue marked inactive did
not. A second SYNCED field saying "off" would have been two fields encoding one thing.

**What a release releases** is read from the show, never remembered: the parameters
any cue of that sequence captures, minus those a cue of another live sequence
captures, minus those the programmer holds. The tempting alternative — remember what
this sequence has actually written since it went on — is per station, and a console
that joined at the interval would release less than the one that ran act one. That is
two rigs with no way back. Reading the cues is stateless, identical everywhere, and
right for the same reason: a parameter no cue of any live sequence captures is a
parameter nothing is driving. It errs conservatively in both directions — a cue that
never ran still contributes, and a parameter another sequence merely *could* drive is
left alone.

**How long it takes** is `Show::home_fade_ms`, PERSISTED, zero by default so nothing
an operator was used to changed. Show data rather than a station preference, against
the first instinct, because `types/show.rs` had already written the argument for
`history_depth`: a station default that keeps applying lets two stations give
different answers about one show. For undo that is a disagreement; for a rig two
consoles are both driving, it is one the audience can watch. The station preference
decides what a *new* show starts with and then stops mattering.

Three things fell out of the reading. `zero_like` is gone, and with it the console's
last guess about where a parameter belongs; where nothing can say — a fixture whose
type has gone — a cue now lands rather than fading from a zero nobody vouched for.
`parameter_key` moved into `pult-schema` beside `programmer_entry_id`, because the
home resolution is the fourth thing that needed it and three spellings of one key was
already one too many. And `home_values` needed a showfile upgrade of the same shape
as `history_depth`'s: the additive pass adds a column nullable, and a JSON column read
as NULL is a parse failure rather than an empty map.

### 42. Four things left open (done)

Not a feature but a sweep: the four "left open" notes the last three changes wrote
down, closed together because each was one decision that had already been argued and
deferred rather than a design nobody had started.

**A cue fades two ways.** `Cue::fade_out_ms` and `ParameterCapture::fade_out_ms` had
been declared since cues existed and read by nothing; task 41 explicitly declined to
say what a cue *out* time meant. It means the classic split: a capture takes the out
time where its destination is below where it starts, and the in time otherwise, with
a capture's own time beating the cue's on each axis independently.

Two decisions inside that. **Zero out means "this cue does not split its fade"**,
not "snap" — an unset field cannot become an instruction, or every show ever written
would start slamming its levels down. And **only a value with an order can be coming
down**: `Float` and `Int`. A colour has three orders and no agreed way to rank them,
a relay has none, and a fixture whose parameter changed kind mid-show is a mistake
rather than a direction — all of those take the in time. A colour fading to black
*looks* like a fade out, and treating it as one would be the console inventing a
ranking to give some cues a time nobody asked for.

The cost, found by a test rather than by reading: `a_cue` in the engine tests carried
`fade_out_ms: 3000` from before the field was inert, so `an_intensity_cue(_, _, 0)` —
a cue asking to snap — began taking three seconds to come down. Data written when a
field meant nothing is data that means something now, and the fixture was the thing
that was wrong.

**Where it rests, from where it is.** `["fixtures", "__set_home"]` with
`{fixtureId, parameterKind?}`, the fourth path verb and the exact inverse of the
third: `__home` sends a parameter to where it rests, this makes where it rests be
wherever it is now. Which is how an operator actually sets a house light's — aim it,
look at it, keep it — where the Home column in the patch panel wants a number typed
in about a light that is already right in front of them.

A verb rather than a plain write to `home_values`, for the reason `__home` is one: a
browser could read `live_values` and write the map itself, and the command line and a
plugin with no data access could not. The whole argument of `__by` and `__home` is
that a caller able to *act* should not have to be a caller able to *read the rig*.
One write of the whole map, because `home_values` is one JSON column and taking a
fixture should be one Ctrl-Z. A parameter putting nothing out is not taken — refused
by name when it was the one asked for, quietly skipped when the whole fixture was.

**A station that gave up asks again.** `plugin-fetch-retry` left a fetch failing
permanently until somebody touched the show, which meant the console holding the
bundle walking in five minutes late changed nothing and the operator's move was to
edit the roster until the reconcile ran — a workaround for a show that was never
wrong.

What re-drives it is **somewhere new to ask**, and nothing else. The manager watches
`stations` beside `plugin_packages`, compares the peer *addresses* against what it
last saw, and re-drives only failed fetches and only on growth. A station leaving is
not new information; neither is a heartbeat, which is why the filter is in the
watcher rather than the handler — a station row carries cpu, memory and a timestamp,
and waking the manager for each of those per console per second to learn nothing is
the version of this that would have been worse than the bug. No timer, because a
station that is not there will not be there in thirty seconds either, and a session
has as many re-drives in it as it has consoles arriving.

It does not contradict `ASK_PEERS_TIMES`' "an answer is an answer". That rule is
about asking *the same stations* twice inside one fetch. A station that was not there
has not answered.

**A plugin can be told.** `plugin-datastores` left a plugin holding a value in memory
learning about an undo on its next read, which for anything cached is never.
`store.subscribe(store)` closes it, delivered through the **existing**
`lifecycle.on-update` as `[store, key]` with a null value for a key that was
forgotten — one token space with `data.subscribe`, so a plugin that uses both tells
them apart the way it already does.

Built on the engine's broadcast rather than a hook in `show_store_set`, which is the
decision that matters. A hook would see only this station's guest writing — exactly
the case a plugin does not need telling about — where the broadcast also sees an
operator's undo, another station's copy of the same plugin, and a showfile catching
up. So a plugin hears its own echo, which is easy to ignore because it knows what it
just wrote, and hears the three things it could not otherwise learn at all.

The mechanical wrinkle: a `plugin_data` row is a UUIDv5 over `(plugin, store, key)`
and that is one-way, so a field write broadcasting `plugin_data/<id>/value` says
nothing about which key it is. The subscription task keeps `id -> key`, seeded at
subscribe and kept up as rows arrive — which is also what lets a *delete* report the
key it was, since by then the row is gone. A create or delete broadcasts the whole
collection, so those are a diff rather than a lookup.

A station-scoped store hands back a dead token: this machine's file, this plugin its
only writer, nothing to report. Said with a token rather than an error, the way
`data.subscribe` answers a plugin with no data permission.

**And the contract moved to `pult:plugin@1.1.0`** — the first exercise of the floor
rule task 35 went to 1.0 for. Adding a function to an imported interface is
additive; a plugin built against 1.0 never calls it. That claim was checked rather
than trusted, and not with the synthetic bump `check-api-compat.sh` performs: the
reference plugin was built from the previous commit in a throwaway worktree, its
imports confirmed as `pult:plugin/data@1.0.0`, and run against this station. It comes
up Running. `store-probe` is the one manifest that moved its floor to `1.1`, because
it calls the new function and a 1.0 station could not serve it — which is what a
floor is for.

### 43. A show with a size, and a tick that says what it cost (done)

Task 29 measured what a tick costs and found a real bug doing it. Three years of
that work survived as a table in this document and nothing else: the rig was ad-hoc,
the instrumentation was added to get the number and taken out again, and the only
show anybody could start in one command had five fixtures in it. `multithreading` is
judged entirely on numbers, so it needed both back before it could begin.

**The demo has sizes.** `scripts/demo.sh --size small|big|huge`. `small` is the
hand-made show and the default, so every existing invocation seeds exactly what it
did before; `big` and `huge` add a generated rig on top — 500 or 2000 six-channel
heads addressed across as many universes as they need, a stack of cues over several
sequences, and effects left running so the station does not settle.

Two shaping decisions. **A cue captures a slice, not the rig**: 300 cues times 2000
fixtures is 600,000 captures, which measures JSON rather than lighting, and a real
cue stack does not touch everything in every cue either. And **the fixtures are
placed**, because half of a tick is what leaves the console once something has moved,
and because a rig with no positions draws nothing in the panel most likely to be the
reason somebody wanted a big rig.

**Seeding stayed on the WebSocket API.** `demo-seed.mjs` says in its own header that
nothing in it is privileged and that this is the point, and a 2000-fixture seed is
the largest exercise of the write path anything in this repo performs — worth more
than the time it costs. What changed is that writes go in flight together through a
bounded window of 64 rather than one awaited round trip at a time. Bounded, because
the engine is one actor behind a channel 256 deep: firing two thousand writes at once
does not go faster, it fills the channel, and the backpressure then arrives as a
per-request timeout on writes that were only ever waiting their turn. A `huge` seed
takes 43 s in release, 119 s in debug. `--keep` exists so nobody pays it twice.

**A station publishes what its own tick costs**, in the `stations` row beside
`cpu_percent` — which answered `system-stats-panel`'s open question (extend the row,
or a new LOCAL stats collection?) in favour of the row, on the grounds that a station
is already the sole authority on its own numbers there. Accumulated into relaxed
atomics by the engine and drained by the reporter every couple of seconds, so a
figure always describes the window just gone. Six nanoseconds a tick, measured, which
is what "measuring does not change what it measures" had to mean.

**Two figures, not one, and this is the finding.** The tick has two halves that scale
differently: computing what playback wants, and everything else — reading the show it
needs, then applying the result as one write, one broadcast and one output push per
fixture that moved. Task 29 put the split at roughly one to three. It is far past
that:

| Rig | Tick | Worst | Playback | Share of tick | CPU | Ticks/2 s |
|---|---|---|---|---|---|---|
| small — 5 fixtures | 0.42 ms | 1.00 ms | 0.03 ms | 7% | 3% | 52 |
| big — 505 fixtures, 4 stacks up | 10.6 ms | 12.2 ms | 0.20 ms | 1.9% | 55% | 80 |
| huge — 2005 fixtures, 12 stacks up | 65.2 ms | 70.0 ms | 0.73 ms | 1.1% | 115% | 30 |

Release build, this laptop, every sequence driven to a cue carrying an effect. At two
thousand fixtures **playback is one percent of the tick** and the other ninety-nine is
the engine around it. A single figure would have credited all of it to playback and
sent the next optimisation to the wrong code.

**And a third counter, added afterwards for one run, says where that ninety-nine
actually goes.** Not where this entry first assumed:

| Rig | Whole tick | Reading the show | Computing | Applying |
|---|---|---|---|---|
| huge — 2005 fixtures | 35.2 ms | **33.8 ms (93%)** | 0.07 ms (0.2%) | 2.2 ms (6%) |

`playback_tick` calls `read_collection` six times — fixtures, cues, sequences, fixture
types, programmer values, speed masters — and each one clones the collection out of
`ShowState` as `serde_json::Value` and then deserialises it whole into a `Vec<T>`. Two
thousand fixtures go through that forty times a second. **Applying the effects is six
percent of the tick; reading the show to compute them is ninety-three.**

That is the correction worth having before `multithreading` starts, because it says
the work is not threads: parallelising a render that costs 0.07 ms would win nothing,
and the cheap win task 29 named — `emit` cloning a fixture's whole `live_values` map —
is in the six percent. The engine re-deserialising the show at 40 Hz is the cost, and
it is not a concurrency problem at all.

**Mean and worst, because the tick has a budget.** An average over a couple of seconds
answers a different question and hides an overrun happening several times a second.
The `huge` row is ten times over its 25 ms budget and the failure is still the graceful
one task 29 described: 30 ticks in a two-second window instead of 80, so the picture
loses smoothness, and nothing loses correctness, because a value is computed from the
wall clock rather than accumulated from the last one.

**"The tick" is `playback_tick`, and the definition is load-bearing.**
`push_output_config` and `flows_tick` share the timer arm and are outside the number.
Not because they do not cost anything, but because they run *whether or not playback
had work* — so a number including them would make every timer firing a tick, and "this
station is not ticking" would stop being a state anything could report. Which matters,
because a settled show stops ticking and a window with no ticks in it publishes
**nothing rather than zero**: zero reads as "instant" when what happened is that
nothing happened. That distinction is why the field is an `Option<TickCost>` rather
than a struct of zeroes, and it is what a station taken off has to be able to say
instead of republishing the last figure it managed to measure.

The trap, found by a test that was wrong before the code was: "settled" means no
playback work *and* nothing written since the last tick. A write bumps `state_version`,
so exactly one tick runs after it to reconcile. A console being programmed therefore
reports a cost; only one left alone reports nothing.

What this number is not, said here because it is the way it will be misread: it is
what *playback* costs, not what the process costs. That is `cpu_percent`, in the same
row, which is why `--measure` prints both and says so underneath.

**No CI budget.** The backlog asked; the answer is not yet. A threshold needs a number
that holds still, shared runners do not give one, and a gate that flaps gets disabled,
which is worse than no gate. The A/B above is the shape of the problem: two *identical*
runs of `huge` varied by more than a percentage point of CPU and fifteen milliseconds
of tick. Revisit once `multithreading` has moved the numbers and we know how stable
they are.


### 44. Values as functions (done)

Task 43 measured a tick and the number came back wrong-shaped. At 2005 fixtures a
tick cost 35.2 ms, and of that **33.8 ms was reading the show, 2.2 ms was applying the
answer, and 0.07 ms was computing it**. The conclusion written down then was that the
tick was not a concurrency problem. It was not a *computation* problem either. It was
that the answer became state.

The console evaluated a fade forty times a second and stored the result in
`Fixture::live_values`, and every one of those samples was then read, written,
versioned, broadcast and read again. Stop storing it and 99.8% of the tick goes with
it, along with the reason for the engine to have a tick at all.

**A fade was already an object.** `RunningFade { from, to, t0, duration_ms, easing }`
anchors in absolute console milliseconds and needs nothing else to be evaluated;
`RunningEffect` is the same shape. One output path already took that offer — an
OpenHaunt node advertising `transitions` is handed the description and left to it, so
a three-second fade leaves the console as one message instead of a hundred and twenty.
This change is that offer taken everywhere.

**What the numbers came to**, on the same machine, `--release`, `--size huge`:

| 2005 fixtures | before | after |
|---|---|---|
| whole | 35.2 ms per tick, 40 Hz | **2.86 ms per output frame**, 34 Hz |
| reading the show | 33.8 ms | — |
| applying it | 2.2 ms | 0.26 ms (emitting) |
| computing it | 0.07 ms | 2.60 ms (evaluating) |
| updates to a connected browser | thousands a second | **0 in four seconds** |

The evaluating figure went *up*, and that is not a regression: the old 0.07 ms rendered
only what had moved since the last tick, and the new 2.60 ms works out every parameter
of every patched fixture from scratch, every frame, plus assembling twenty-four
universes. Paying it thirty-four times a second instead of paying thirty-five
milliseconds forty times a second is the trade. Eleven percent of the frame budget
against a hundred and forty percent of the tick budget.

The last row is the one an operator feels. A cue on a 2000-fixture rig used to put a
few thousand messages a second onto every connected console; it now puts none at all,
because nothing about the rig changes while a fade runs.

**One evaluator, compiled twice, and no TypeScript twin.** The arithmetic moved to
`crates/pult-render` — `serde` and `uuid` and nothing that touches an OS — and is
linked natively by the station and compiled to `wasm32-unknown-unknown` by
`crates/pult-render-wasm` for the browser. This is a *second* wasm toolchain: the
plugins are `wasm32-wasip2` components run by wasmtime on the host, which is the wrong
tool entirely for code that has to run inside a tab.

The alternative was what `fixture-groups` did for `SelectionQuery`: write it twice and
hold the two together with a corpus. It was declined, and the reason is the size of the
surface and the shape of the failure. Easings, curves, step lists, spread, phase,
direction, width, master rates, priority, home fallback and split fades are an order of
magnitude more than a selection query, and a drift between them shows up as the screen
disagreeing with the lamps — which an operator cannot work around and may not notice
until it matters. What is guarded instead is that the two *compilations* agree:
`testdata/driven-values.json` is 47 cases read by a native Rust test and by a vitest one
that loads the wasm build and asks it the same questions.

**A landed fade stays.** `live_fades` stopped meaning "in flight" and started meaning
"what is driving this parameter", including the fades that have arrived. That follows
from the removal rather than being a decision beside it: once nothing stores the number
a fade landed on, the finished fade is the only thing that remembers it, and evaluating
one gives exactly that constant. An effect that stops without anything taking its key is
*parked* the same way — a fade of no length at the value it was showing — because
stopping a chase should freeze the look, which is what leaving the number in a map used
to do by accident.

**Connectors own their rate; the engine pushes the show.** `OutputPlugin::send` gained
a moment and lost the assumption that somebody had already computed the values. Each
connector holds the last patch it was pushed and draws its own frames out of it — the
DMX family at 40 Hz while anything is moving and at its 800 ms keep-alive when nothing
is; an OpenHaunt node told once. The engine pushes when the *show* changes, which after
this change is the only kind of change there is.

**The 25 ms timer went, and what replaced it is a deadline.** `ShowEngine::run` now
sleeps on the soonest thing it actually has to do — a follow cue coming due, a *Watch*
node wanting a sample — and on nothing at all when there is neither. A settled station
and a station running a chase are now the same amount of work: none. The one thing that
still samples is edge detection, which cannot be done from a function; it was already
gated by a `watched` set and is now *proportional* to it, so a graph watching one lamp
of two thousand costs one evaluation a sample rather than two thousand thrown away.

**The browser has to learn the station's clock, and say when it has not.** This is the
one genuinely new mechanism the change needed. The objects are anchored in console time,
so a page evaluating against an unadjusted `Date.now()` runs every fade out by however
wrong its own clock is — and does it silently, because every individual value is
plausible. The offset is estimated the way a round-trip time is, kept as the best of
five samples, and maintained rather than taken once, because clocks step. The rule that
makes the failure visible rather than silent: **a client with no offset yet presents
nothing**. `consoleNow()` answers `null`, and a panel shows a gap.

**What stayed state.** `Fixture::live_values` did not become nothing; its two halves
were separated. What a device *reported* — a contact, a temperature, a humidity — is
now `sensed_values`, and it stays SYNCED for the reason it always was: the wire it came
off is attached to this station and nobody else can work it out. Driven outputs are
functions of time; sensed inputs are facts the console was told. Removing the old field
rather than deprecating it was deliberate — an unwritten `live_values` would have left
every reader compiling and silently seeing nothing move, where a removal makes the
compiler and `svelte-check` produce the list of things to fix.

**What a tick cost became what a frame costs.** The published figures moved from the
engine to the output frame, one entry per connector, keeping the mean and the worst and
the rule that a window with no frames in it reports **nothing rather than zero**. Per
connector because their rates are their own: Art-Net at 40 Hz beside a node told once
are not two samples of one number.

**The trap, for whoever is next.** The show clock advances monotonically from one wall
reading, so `tokio::time::pause()` does not fast-forward a fade. A test that needs one
to advance has to let real time pass or drive `Playback` directly. Several tests here
run a short fade in real time for exactly that reason.

**And one bug found while verifying it, fixed here because it was in the way.**
`scripts/demo.sh --two` could not sync at all on this machine: the two stations
discovered each other over mDNS, joined the session, and never connected.
`infra/session/mod.rs` took `info.get_addresses().iter().next()` — and that is a
`HashSet`, so it was not the first address the other station advertised but whichever
the hash order gave, differently on different runs. Half the time that was a
link-local `fe80::` address, which cannot be dialled at all: it is only meaningful
together with the interface it was learned on, and mdns-sd does not say which that was,
so the `SocketAddr` built from it carries no scope id and the connect fails with "No
route to host".

Two halves to the fix, because ranking alone would still be a guess. `reachable_at`
orders what was advertised by how far it reaches — the network, then the segment, then
this machine — and drops what cannot be dialled rather than ranking it last, since
spending a timeout on it only delays the address that was going to work. And
`sync::dial` works *down* that list with a two-second budget each, because which address
reaches a given machine is not something either end can know on its own.

And the half of it that was not about addresses at all: `Join` used to answer `Ok`
before the dial had been tried, so a session that could not be reached was one the UI
said had been joined — the Sessions panel has had a toast for that since it was written
and it could never fire. It waits now, and answers what happened, naming every address
it tried. Safe to wait on despite the deadlock the sync manager is careful about: that
manager spawns the dial and goes on draining its channel, so nothing the session actor
waits for is waiting on it. Bounded at three seconds across every candidate, split
between them, because somebody is on the other end of the answer — an answer that
arrives after the caller has given up is the same as no answer.

### 45. GDTF (done)

A fixture type was derived data. An OpenHaunt node describes its own ports and the
console builds a type from that; the demo seed writes a few by hand; the editor lets
somebody type one in. Nothing else could make one, which is why the rig view drew every
beam at the same hardcoded angle and why `stage.ts` carried a `PAN_TRAVEL = 540`
constant that is right for no particular head.

GDTF is the same idea as a file. This task reads one, writes one, and grew the schema
by what a real fixture definition turns out to contain.

**`crates/pult-gdtf` is a format library and knows nothing about this console.**
`quick-xml`, `serde`, `zip`, `uuid`, `thiserror`, and no pult crate — which is what
lets it be tested against other people's files with no station anywhere near it. The
translation into the schema lives in `crates/pult-backend/src/infra/interop/gdtf/`,
where it can be tested against a show. It writes as well as reads, and that is the
reason it exists rather than a crate off crates.io: `gdtf` 0.3.0 reads and cannot
write, and the console exports its own types.

**A mode is a detail of addressing; the parameter list is what the light can do.**
`FixtureType::dmx_modes` holds a layout per mode — a footprint per DMX break, and per
channel the parameter it drives, which break it is in, its one to four byte offsets,
its default at its own width, and its named ranges. `FixtureAddress::Dmx` grew from
`{universe, address}` to `{mode, breaks}`, because a fixture with a separate dimmer
break sits in two spans that need not be in the same universe, and because universe 1
address 1 does not say what the bytes mean until the mode does.

**A type with no modes still has one, computed rather than written.**
`FixtureType::mode` builds an implicit `"Default"` from the legacy `Dmx { channel }`
bindings where any parameter still carries one, and from parameter order otherwise —
one byte each, three for a colour, in the order the connector used before layouts
existed. Computed and not a load-time rewrite, and the reason is the SQLite read path:
`from_columns` reads each column on its own and unwraps, so a deserialize-time rewrite
would never see the other columns it needs and a NULL one would panic. `ParameterBinding::Dmx`
therefore survives as a read-only legacy variant that new code never writes, and every
showfile, every demo seed and every peer written before this opens unchanged.

**Migrations, in the three places this codebase has learned they go.**
`FixtureAddress` and `ParameterDefinition` grew hand-written `Deserialize`s that accept
the old shapes, because both are JSON columns with nothing to alter. The seven new
non-`Option` columns on `fixture_types` got a row in `upgrades.rs`, because the additive
pass adds a column nullable and `from_columns` panics on a NULL one. And both are
tested by loading the old shape rather than trusted, because an `Option` column that
fails to parse becomes `None` without an error.

**A colour is one parameter and several channels.** `ColorAdd_R`, `_G`, `_B`, `_W`,
`ColorSub_C` and the rest are all the fixture's colour; a reader that made three
parameters would give an operator three faders where every other console gives a
picker. So `ParameterDefinition::emitters` carries the dies, each channel of a mode
names the one it drives, and `pult_render::color` gets from the one to the other —
compiled twice like the rest of the evaluator, and held to its native half by
`testdata/color-mix.json`. RGB passes through, white and amber and lime take the
largest multiple of their own measured colour that fits under the target, CMY is one
minus the component each flag removes, and `ParameterValue::Color` grew an `overrides`
map for the head whose white is warmer than its file says.

**Where the browser had to be given the answer rather than work it out.** The packed
frame is four floats per parameter, and a map of per-emitter levels does not fit — so
`pult-render-wasm` gained two exports beside `evaluate`, `color_overrides` and
`emitter_levels`, for the colour control alone. Widening the stride would have made
every frame of every rig pay for a panel open on one screen.

**The file is the record; the row is a reading of it.** An imported `.gdtf` is kept
whole in the asset store and `FixtureTypeSource::Gdtf` points at it by sha256, so a
later version of this console reads more out of the same bytes rather than asking
anybody to download anything again — and `GET /api/export/gdtf/{id}` hands back the
archive byte for byte. A type the console made for itself exports as a generated file
instead, deliberately minimal: inventing a beam angle or a weight would be writing a
lie another console would then act on.

**And it is never rebuilt behind the operator's back.** `FixtureTypeSource` exists for
one reason: `fixture_type_from` runs again whenever a node re-describes itself, and
doing that to an imported type would throw the file away.

The GDTF Share is behind a login that lives in the station's preferences and never in
the show — a showfile travels, and a password in one travels with it. Three things
about it are worth having written down. **Its login answers 200 with an HTML page when
the credentials are wrong**, so success is decided by the body and never by the status.
**Its list is tens of megabytes and unfiltered**, so it is fetched once, cached in
memory and beside the preferences, searched locally, and re-fetched daily or on demand.
**Its session goes idle after about two hours**, so an unauthorised answer logs in again
and retries once — once, because a second refusal after a fresh login is not something
retrying fixes.

Both import paths go through `interop::apply`, which is where the rules about writing
live: the plan is built by a pure function before anything is stored, so a body that is
not a GDTF leaves neither an asset nor a row behind; every write carries one gesture, so
an import is one Ctrl-Z; and a write that fails takes the rest back, which is only
honest because of the first rule.

**What real files taught this reader, once it was pointed at the Share.** Everything
below was written strictly first, passed against the three fixtures checked in beside
it, and failed on the first file downloaded from gdtf-share.com.

- **A DMX value is often a bare number.** The spec's grammar is `value/bytes` and three
  of the first five files pulled off the Share write `1` where it says `1/1`. A bare one
  is one byte, and refusing it refused the whole fixture.
- **`"None"` is a value.** The spec uses the literal string where an attribute has
  nothing to say — `Highlight="None"` on a channel with no highlight — and a reader
  that only accepted `255/1` there refused the whole fixture.
- **`-2147483648` appears in unsigned fields.** `i32::MIN` as a "not set" sentinel, in
  a `WheelSlotIndex` the spec calls a count. Every number in the object model is now
  read leniently and anything unreadable becomes absent, which is what it meant.
- **`"N/A"` appears where the Share's own API declares a float.** A rating nobody has
  given. Strictly typed, one such row failed the deserialization of *the entire list*,
  so the console answered "the GDTF Share answered something this console could not
  read" for every search. The list is now read row by row and an unreadable row costs
  that row.
- **A Share login is a user name, not an email**, and the field was `type="email"`,
  which a browser refuses.
- **Nobody types a fixture's name the way its uploader spaced it.** "mega pointe" found
  nothing, because the Share calls it "Robin MegaPointe". The search matches word by
  word and ignores spacing and hyphens on both sides.
- **Seven files answer to "megapointe" and one of them is Robe's.** The Share says which
  — `uploader` is `"Manuf."` on exactly one — and that beats any ranking this console
  could invent. An earlier attempt ranked by *where* the words matched and got "robe
  mega pointe" backwards, putting a third party's copy above the manufacturer's,
  because the copy had the word "Robe" in its own name.
- **The outermost geometry's model is the base plate, not the fixture.** Reading it for
  `dimensions_m` gave a Robin MegaPointe a height of 9.5 cm. It is the envelope across
  every part at the place its own geometry puts it: 0.40 × 0.45 × 0.25 m, against 22 kg
  and a 39-channel mode, which is a fixture somebody could put on a rider.

Two traps worth recording. **`ParameterBinding::Dmx { channel: u8 }` capped a mode at
255 channels** — gone with modes, offsets are `u16`. And **grouping the frame by channel
instead of by parameter costs what colour costs**: a flat list of placed channels
evaluates an RGBW head's colour four times a frame, and mixing the whole fixture to pick
one level out of the answer allocates a vector of names per channel per frame. Both
showed up as the frame cost roughly doubling in a debug build and were fixed before
they reached a release one. Measured at 505 fixtures, `--release`: 0.93 ms a frame
against 0.90 ms before, and 0.72 ms evaluating against 0.74 ms — unchanged, which is
what the fix was for.

The lesson under all of those is one the corpus job exists for and could not deliver on
its own: **this reader is only as good as the files it has been pointed at.** Three
hand-written fixtures proved the arithmetic and proved nothing about the shapes real
files take. `scripts/fetch-interop-corpus.sh` with a real Share login is what found
every item above.

The corpus it fetches is five files and each is there for a corner: an RGBWAUV par with
six emitters, the MegaPointe, an Astera PixelBar with a hundred and sixty-five modes and
its cells behind geometry references, a MAC Aura XIP, and a Sunstrip. All five parse,
rewrite stably, agree with the `gdtf` crate about every mode and offset, and import —
the Astera with two warnings out of its 165 modes, which is the "kept at its own
offsets" fallback saying so rather than dropping a channel.

The script itself was wrong in the same way the reader was, and worth recording because
it is the more embarrassing half: it fetched two XSDs and an MVR sample from URLs that
do not exist, reported the 404s, and carried on — a corpus job that looks like it ran.
That repository publishes the spec as Markdown, has no XSD and no samples, and there is
no public collection of MVR files to point at, so MVR material now comes from
`PULT_MVR_SAMPLES` and the script says plainly when it has none. Its Share rids were
invented too; they are real ones now, verified, with a note saying what each is for.

`.github/workflows/ci.yml` is new, because there was none: the default suite, the
frontend, and an `interop-corpus` job that runs `scripts/fetch-interop-corpus.sh` and
the `#[ignore]`d tests over other people's files. The corpus is gitignored; what is
checked in is `testdata/gdtf/`, three small fixtures written here — a dimmer, an RGBW
mover with two modes and two breaks and a 16-bit pan whose fine byte is six slots away
from its coarse one, and a four-cell bar that describes one cell and references it four
times, which is the whole reason a mode's channel list is not its footprint.

```
cargo test -p pult-gdtf                              # the format library
scripts/fetch-interop-corpus.sh                      # other people's files
cargo test -p pult-gdtf -- --ignored                 # against them
curl -X POST http://localhost:7700/api/import/gdtf \
     -H 'content-type: application/vnd.gdtf+zip' --data-binary @head.gdtf
```

### 46. Three flaky roster tests, and none of them was flakiness (done)

`cargo test` failed on `tests/roster.rs` about half the time, and always on a
different test, which is the shape that gets a suite labelled flaky and re-run rather
than read. Three separate causes, none of them timing noise.

**A fetch can take forty-two seconds and the test waited twenty.** A station that
cannot reach a peer asks four times with a ten-second HTTP timeout each, plus backoff.
`eventually` waited a flat twenty seconds. Whether a test passed depended on whether a
dead address refused the connection quickly or hung — which is a property of the
machine, not of the console. The budget is now named where it is spent —
`assets::PEER_ANSWERS_WITHIN` and `plugins::GIVING_UP_TAKES_AT_MOST` — and the test
waits on *that* plus a margin, so changing either constant cannot bring it back.

**One test waited on the wrong thing.** `a_station_arriving_re_drives_a_fetch_that_gave_up`
waited for the failure reason to stop saying "no station" and then asserted the bundle
was on the disk. A reason that has cleared is a fetch that has *started*. It waits for
the fetch to finish now.

**And the connect helper gave up in five and a half seconds.** Ten attempts with a
rising backoff, which is plenty on an idle machine; with every core pinned, twenty-one
tests' worth of stations starting at once all failed there together, and the suite read
as the console being broken rather than the box being loaded. A thirty-second wall-clock
budget replaces the attempt count.

Verified by running the suite eight times with every core pinned, which used to fail
about half the time and now does not fail at all. Worth writing down because the first
two were real facts about the console that only the tests knew: how long a station can
sit in *Fetching* before it says what happened is a number an operator watches, and it
was written in one place and waited on in another.

### 47. MVR: a rig somebody drew (done)

GDTF is a fixture. MVR is the *rig*: where every light hangs, what it hangs off, what
truss that is, what the truss is made of, and which layer of the drawing it belongs to.
This reads one, writes one, and grew the schema by what a real drawing turns out to
contain.

**`crates/pult-mvr` is a format library and knows nothing about this console**, like
`pult-gdtf` beside it — and it depends on that one, for the fixture definitions inside
an archive and for the millimetre Z-up to metre Y-up conversion the two formats share.
A second copy of that conversion is the one bug that would show up as the screen
disagreeing with the lamps.

**A place in the rig is a transform, and the scale is signed.** `Fixture::position` was
a point, or a point and a direction. That is enough to draw a beam and not enough to
draw a rig: a truss is somewhere, it is turned to face somewhere, things hang off it,
and moving it should move them. So a position is now a `Transform` — metres, XYZ Euler
degrees, and a scale — *relative to whatever the fixture hangs off*.

The scale is signed because twenty-one of the forty-three trusses in the first real
file this was pointed at have a basis whose determinant is −1: the drawing mirrored
them. No rotation is a reflection, so an unsigned decomposition brings a mirrored truss
back as some rotation that puts it nearly right, with its bolt holes on the wrong side
and nothing in the numbers admitting it. The reflection is pulled onto X.

**Five new collections**, all keyed by the uuid the file uses: `scene_objects` (with
`Group` for the handle that moves a truss and its lights together), `layers`, `symbols`
— because ninety-five objects in one drawing share ninety-five symbol definitions and
the meshes are the bulk of the archive — `classes`, and `named_assets`, which exists
because a `.3ds` asks for `tx603.jpg` by that string and a content-addressed store has
no names in it.

**Composing a parent chain is worked out twice**, `types/scene.rs` and
`frontend/src/lib/scene.ts`, for the reason `SelectionQuery` is evaluated twice:
dragging a truss re-composes every child per frame and cannot be a round trip.
`testdata/transforms.json` holds them together. Its `matrices` half starts from a
matrix as a file writes one and is read by `pult-backend`, which is where `pult-mvr`
and `pult-schema` meet without either depending on the other.

Consequence: **a geometric selection term reads a world position**, so `evaluate` takes
the scene objects as well as the fixtures. The selection corpus proves it without being
told to — a fixture whose own row puts it at the origin sorts after the one at x=3,
because the truss it hangs off is at x=8.

**Every uuid the file uses is the id the row gets.** An imported fixture's `id` *is* its
MVR uuid, so a re-import updates the drawing rather than doubling it, with no lookup
table to keep, and an export writes the ids back without inventing any. A fixture
*type* is the exception, keyed by the GDTF's own `FixtureTypeID`, because a drawing can
name one definition twice.

The file wins on a re-import — transform, address, mode, name, layer, parent — and what
an earlier import left in a layer this one no longer mentions is **listed under
`missing` and never deleted**: somebody may have taken that light out on purpose. A
fixture whose GDTF the archive does not carry gets a placeholder type rather than being
dropped, so the address, the mode and the place survive until somebody supplies the
real file.

**And the round trip is the test.** Every real file in the corpus, imported, written
back out and read again, gives the same fixtures at the same addresses in the same
modes: 36, 46 and 46 of them.

**Before any of it, showfiles stopped being a migration target.** `upgrades.rs`, both
hand-written legacy `Deserialize` shims and `ParameterBinding::Dmx` are gone. While the
console is in development nobody is carrying a season's work in one, and a migration is
a promise about every shape the data has ever had. What replaces it is a refusal that
says what is wrong, and it needs two checks because a showfile fails two ways. A stamp
— `PRAGMA user_version` against `SCHEMA_GENERATION` — catches a changed *shape*, which
nothing else can see: an `Option` column that fails to parse becomes `None` with no
error. And a scan for a required column nothing filled in catches the additive pass's
own hole, where a new non-`Option` field's column is NULL on every row and
`from_columns` panics mid-open. `add_missing_columns` stays: adding a field is free.

The hand editor's per-parameter channel is derived now rather than typed. Where a
parameter sits belongs to a mode, and a type made by hand has the implicit one, which
lays its parameters out in the order they are listed.

**What real files taught this reader.** Everything below was written strictly first and
failed on a file somebody actually exported.

- **The first file is not well-formed XML.** A grandMA export ends with a NUL byte
  after `</GeneralSceneDescription>`, and every strict parser refuses the whole document
  over one byte nothing reads.
- **A `GDTFSpec` is spelled three ways.** grandMA writes `Vendor@Product` bare;
  Vectorworks writes it with `.gdtf`; and a zip's central directory does not always
  decode a name the way the XML spells it — an ARRI Orbiter with a degree sign in its
  name comes out of the archive as `15┬░` and out of the XML as `15°`. The lookup walks
  down from exact to letters-and-digits-only and says which rung answered. It does
  **not** warn about the extension rung: that is how one whole family of exporters
  spells a spec, and a line per fixture is a report nobody reads.
- **`Color="nan,nan,nan"` is in a real Robe file**, on a colour wheel's black slot, and
  Rust parses "nan" into a number quite happily. What followed was worse than a refusal:
  the NaN reached the schema, `serde_json` wrote it as `null`, and the fixture type was
  stored as a row that could never be read back. Silent loss, with no bad data to blame.
  Reachable through `/api/import/gdtf` too, so it was a latent defect in task 45's path
  as much as in this one.
- **Every `Option<u32>` in the schema was losing its value.** `#[derive(PultSchema)]`
  stores an optional field as JSON text but gave the column the *inner* type's affinity,
  so `Option<u32>` declared INTEGER, SQLite converted the text `101` to the number 101
  on the way in, the text-based reader found no text on the way out, and the field read
  back as `None`. `Fixture::fixture_number` was the first optional number the schema had
  ever had, which is why nothing caught it before.
- **`order` is a SQL keyword** and the generated `CREATE TABLE` does not quote one. A
  `Layer::order` column fails to open the show; it is `sort_order`.
- **Two fixture types can honestly want the same file name.** One drawing carries the
  same Robe head twice — two `FixtureTypeID`s, one product name — and written under one
  archive entry they become one type on the way back in, with half the rig repatching
  itself. A name already taken gets a number, in id order, so two exports of one show
  write the same names. Worth recording that the guess going in was wrong: those two
  files were assumed to be the same file under two names, and they are not. Same byte
  count, different sha256, different `FixtureTypeID` — genuinely two definitions, and no
  keying strategy collapses them.

- **And `scripts/demo.sh` was broken for two commits without a test noticing.** The
  seed still wrote a `ParameterBinding::Dmx`, which the schema no longer has, so the
  demo came up empty — and nothing in `cargo test` runs the seed. It was found by
  running `--measure`, which is the only thing in the repository that does. Worth
  knowing: the seed is a client of the write path like any other, and the suite does
  not exercise it.

**Two more that only the second implementation could find**, both in the pair that
turns an aim into a rotation and back. `atan2(-0, -0)` is −π, so a light hung straight
down — which has no bearing to speak of — was stored as turned all the way round, and
the epsilons that came back out of the angles then read as a bearing of 45°: every beam
in the demo pointed the wrong way. And `facing` returned `-0` components, which flips
any bearing taken off one. Neither is visible from one side; writing the browser's half
against the same corpus is what surfaced them.

**The browser draws it.** `geometry.ts` loads a mesh once per sha and clones it per
object. A `.3ds` is Z-up and is turned in that one place; its textures resolve through
`named_assets` and three.js's own URL modifier, so nothing below that line knows assets
are addressed by hash; and a file the loader refuses becomes a placeholder box, because
a rig view that goes blank over one bad mesh in two hundred is worse than one with a
box in it. A mirrored instance gets its own material — negative scale reverses winding.
The Layers panel shows and hides parts of the drawing, per browser, and hiding one takes
its objects out of the plan and the rig **and nowhere else**: a hidden fixture still
takes a cue, still answers a group, and is still in the patch.

And the beam maths now reads a *world* position: a light on a truss somebody turned
points where the truss points it.

**Measured, and the honest reading is "unchanged".** At 505 fixtures, `--release`:
0.24 ms a frame and 0.19 ms evaluating on one run, 0.36 ms and 0.29 ms on the next.
Task 45's baseline was 0.93 ms and 0.72 ms. Both of these are well under it, and the
right conclusion is *not* that this task made anything four times faster — nothing in
it touches the render path. The spread between two consecutive runs on an idle machine
is 50%, which is larger than any change this work could have caused, so the number that
matters is that nothing leaked into the frame. Worth remembering the next time a figure
here is quoted as a comparison: one run of `--measure` is not a benchmark.

The browser has its own figure now, in the rig view's toolbar, and it is a different
number for a different thing: **8.3 ms a frame** with both corpus rigs loaded — 138
fixtures, 150 objects and about a hundred meshes. That is read by hand, because
`--measure` starts no browser on purpose.

Checked against the corpus through the actual UI, which is the thing the tests cannot
say. A grandMA export of a Moulin Rouge show imports as 102 rows, its fixtures in the
patch under the mode names the file uses — `63: DIM RGBAWS DIM RGBAWS`, universe 1,
channels 37 to 148. A Vectorworks drawing of a festival stage imports as 352 more and
draws as a rig: trusses with their lattice, the towers holding them up, the deck, and
the fixtures hanging off them in rows.

```
scripts/fetch-interop-corpus.sh                             # with PULT_MVR_SAMPLES set
cargo test -p pult-mvr -- --ignored                         # the format library
cargo test -p pult-backend --test mvr_corpus -- --ignored   # and what it becomes here
curl -X POST http://localhost:7700/api/import/mvr \
     -H 'content-type: application/vnd.mvr-scene+zip' --data-binary @rig.mvr
curl -o rig.mvr http://localhost:7700/api/export/mvr
```

What it does **not** do is edit. Nothing can be moved, rotated, parented or placed in
either view yet, fixture bodies are still markers, and gobo images are still not
extracted. Those are `scene-editing` and `gdtf-share-panel-polish`, both of which this
task's asset pipeline is what was blocking.

### 48. The console shows its own log (done)

`tracing` wrote to stdout and nothing captured it, so **on every way of running this
that is not a terminal, the log did not exist**: `pult-gui` wrote to a stdout nobody
was looking at, a packaged `.app` had nowhere to write it at all, and a browser on the
network — which is a whole console by design — had no access to any machine's stdout.
Plugins were logging into that void too. `wit/pult-plugin.wit`'s `logging.log` promises
an author that their message "lands in the station's log", and it did, and the log was
nowhere. That was the argument for doing this first: it is the audience with no
workaround.

**A line is a function of what is driving it, and the log is the exception.** Almost
everything else in this console is evaluated on demand; a log is an append-only stream
of things that already happened. So the shape had to be found rather than inherited,
and the one open question at the top of the entry — where the lines live — had a wrong
answer that looked obvious. A LOCAL ring in `ShowState` beside `output_status` would
have been uniform with every other LOCAL path, and it would have **rewritten and
rebroadcast the entire buffer as JSON on every line**.

What it turned out not to need was a new protocol shape either. `UpdateBroadcast`
(`engine/mod.rs`) is a plain `broadcast::Sender<(Path, Value)>` in `AppState`, and
`ws_registry.broadcast_update` matches *any* path against a session's subscription
patterns — neither requires the path to exist in `ShowState`. So appends ride the
existing `Update` message on the `logs` path, gathered for `COALESCE_MS` so a burst of
two hundred lines is two messages, and pushed **without going through the engine
actor** — which is the property that matters, because queueing diagnostics behind
whatever the console is busy with is exactly wrong at the moment somebody is reading
them. The backlog comes from a `log.tail` station RPC. A browser with the panel closed
subscribes to nothing and costs nothing, so task 44's "no updates to a browser during a
fade" is still true.

**The subscriber cannot be built inside the station.** `tracing_subscriber::init` is
once per *process* and a station is a library a process may start more than one of —
`tests/stores.rs` and `tests/plugins.rs` each run three. So `logging::install` builds
the whole subscriber, `fmt` layer and `EnvFilter` unchanged, with the capture layer
beside them, and hands back a `LogHandle` that both binaries put in `Config` as a
`#[serde(skip)]` field. A station given none simply has no log, which is what every
existing test wanted and why none of them changed. `logging::detached` is the same
thing with nothing feeding it, for a process that already has a subscriber.

That split found a real bug on the way. `start` was reading preferences and setting the
levels from them **per station**, which silently overwrote whatever the caller had
asked for. Preferences are one file per machine and `install` is the one call per
process; that is where the two line up, and `detached` gets exactly the levels it was
given.

**Two levels, not one, and the second was the user's idea rather than the entry's.**
`log_level` is what this station keeps — the ring, the panel, the file. `peer_log_level`
is what it puts on the sync link, `warn` by default. So a peer's warnings and errors
always arrive without anyone asking, and nobody's `debug` crosses the network that is
also carrying the show. A booth watching the roof station can ask for more
(`SyncMessage::LogRaise`, protocol 5), and:

**A raise is clamped to what the peer itself captures.** If the roof is keeping `info`,
its `debug` events are dropped by its own layer before anything could forward them, so
no ask can produce them. Reaching *past* that would mean one console changing what
another keeps in its ring and writes to its file, which is not a thing a log panel
should be able to do. `publish_level_for` is the whole rule and is in one place,
because it fails silently in both directions: too low and an escalation shows nothing,
too high and a station publishes what it never kept.

**And nothing expires.** The first draft of this called the raise "per-connection state
that must unwind correctly" and assumed a TTL and a renewal timer. Reading the code
said otherwise: `ws_registry.remove_session` already fires when a browser goes, so the
ask is recomputed from who is actually watching and the peer is told the new answer —
including "nobody". A booth that dies takes its TCP connection with it, and the sync
layer's dead-link timeout reaps that. There is nothing to expire because nothing
outlives its connection, and the timer that was nearly built would have been a second
mechanism for a problem the first one already solved.

**A source is a field, not a prefix.** `host_impls.rs` used to interpolate
`[plugin:<id>]` into the message text. It now records `plugin = %id` as a `tracing`
field, which `LogSource` lifts into `Station | Plugin(id) | Browser(session)`, so the
panel's per-plugin filter reads a field rather than parsing text — a message that
merely contains a bracket cannot defeat it, and there is a test that says so. `fmt`
still prints it, as `plugin=<id>` at the end of the line, so a terminal and
`.demo/backend.log` lose nothing but the position.

**The browser reports itself.** `window.onerror` and `unhandledrejection` go through a
`log.report` RPC, deduped and rate-limited so a panel throwing every frame is one line
and a count rather than five thousand lines that push out everything explaining why.
They cross to peers like any other line, because the tablet at the back of the room is
the console nobody is watching. That also gave system-stats-panel the precedent it
needed: a browser reporting on itself to the station now has a working path, and
task 49 used exactly it.

**Ordering is honest rather than exact.** Each line carries the emitting station's own
`seq` and clock. `(node_id, seq)` is what lets the browser merge the `log.tail` backlog
with the live stream — they overlap by construction — and it is what makes a dropped
line *visible*: the panel says "1,204 lines from roof-2 did not arrive" instead of
quietly skipping them. Across stations the merge is by `at_ms`, which is only as close
as their skew allows, and finding that out is what opened the `station-clock-offset`
entry below.

Two traps worth keeping.

**Two runs of the integration test are not one.** Three of the five failed under
`cargo test` and passed in isolation, and the failures were fixed sleeps: the raise is
asynchronous, so a test that says "raise, sleep 200 ms, assert" is asserting on how
fast the machine is. They poll now. The negative assertion needed the same treatment
in reverse and one thing more — after a withdrawal the withdrawal is *in flight*, and
lines said in that window should still arrive, so the test asserts "goes quiet and
stays quiet for three rounds" rather than "is quiet at once".

**A tail's limit applies after the level, not before it.** Asking for the last two
`warn` lines out of a ring holding six `debug` ones has to walk back past them, or the
panel shows nothing and looks broken. There is a test for that, because the natural
implementation gets it wrong.

```
cargo test -p pult-backend --lib logging      # the ring, the levels, the file
cargo test -p pult-backend --test logs        # two stations over a real sync link
cd frontend && npm test                       # merge, gaps, and the report throttle
```

### 49. What the console costs, what the machine costs, and what is on the wire (done)

Task 44 gave a station the ability to say what its own output frames cost and put the
figures on the `stations` row, where **nothing in the frontend read them**. And the
number that was missing entirely was the browser's: since the engine lost its tick a
console *is* a page evaluating a rig in wasm on every animation frame against a clock
it had to estimate, and no instrument anywhere could see one. The machine struggling in
a room where every station is comfortable is the tablet at the back of it.

**The panel split into two panels rather than growing.** The Stations panel was already
a table of nine columns mixing two questions — who is in the session, and what each
machine costs — and the browsers had to go somewhere. So `stations` keeps the network:
hostname, leader, sync address, outputs, fixture share, heard-ago. The new `system`
panel takes what each machine costs — the console's own processor and memory, the
machine's around it, what is on the wire, and a line per output connector — and puts
the browsers underneath them. **Latency is deliberately in both**, because it is the one figure that
answers both questions. The Setup preset became a 2×2 grid to hold the extra tile:
outputs beside system, stations beside show and session.

**A browser is not a station and must not appear in `stations`.** That collection is one
row per node, written by the node about itself and replicated; a tab that closes has to
leave nothing behind. So `clients` is a new LOCAL path, a map keyed by the *short*
session id — the same eight characters `LogSource::Browser` already carries, so a
warning in the System Log and a row in the System panel are recognisably the same tab.

**The open question the entry carried was whether a browser's figures replicate, and
the answer is no — but the exception does.** Task 48 answered the neighbouring question
for a *log line* with "yes, at a quieter threshold". A fault is occasional and a frame
rate is every second, and that difference is the whole of it: a row per browser per
report crossing the sync link for ever is a stream nobody reads, on the same network as
the Art-Net. So the continuous figures stay with the station serving the page, the way
`peers` does, and what crosses is the *exception* — a window under 20 fps, or one frame
over 100 ms, becomes a `warn` through the `log.report` path that already reaches every
console. The useful property survives at the moment it is useful.

**Measured in the loop that already exists.** `stores/output.ts` evaluates the rig once
per animation frame, so the frame time and the evaluating half are taken there rather
than in a loop of this task's own — a second `requestAnimationFrame` loop would keep a
page rendering purely to prove that it can, which is exactly the wrong thing to do to
the tablet being diagnosed. The consequence is that **a page drawing nothing measures
nothing**, and says so: `frames` is `None`, and the panel prints "drawing nothing"
rather than a frame rate of zero, for the same reason an idle connector carries no
`FrameCost` at all.

The frame figure is the **gap between frames**, not the work done inside one. A page
whose own work takes 2 ms and which is nonetheless served a frame every 200 ms is a
page that is stuttering, and only the gap says so.

**The clock offset is read, not re-measured.** `ws/clock.ts` already maintains an
estimate the page is evaluating against, and this reports that one. A second estimate of
the same quantity would be a second answer to it, and the panel is meant to show what
the page is actually using — which is also the figure that says whether anything else it
shows can be trusted, since a page that has placed itself wrongly draws every fade out
by exactly that much, plausibly.

**And a figure gets a line, kept by whoever is looking at it.** Every report is one
*closed window* and nothing on the wire carries a series — a growing series on a
replicated row being the one thing this entry said a row cannot hold. So the history is
the reader's: `frontend/src/lib/trace.ts` keeps the last sixty readings the tile
actually witnessed, two minutes at the report interval, and draws a sparkline beside
each connector's mean frame and each browser's frame rate. A line therefore starts empty
when the tile is opened and covers only what that tile saw, which the panel says out
loud rather than implying a record it does not have.

Two rules in there that the obvious implementation gets wrong. A trace deduplicates by
the *window's stamp* rather than by value, because a station that has gone quiet is
still being rendered with its last figure — taking every render as a reading would draw
a flat line, which reads as a machine steadily working rather than one that has stopped
talking. And a sparkline is scaled **from zero**, not from its own lowest point: these
are costs and rates, and a frame time bouncing between 4.0 and 4.1 ms has to read as
flat rather than as an alarming sawtooth. A browser's line is drawn against 60 fps
rather than against its own best, which is what makes two browsers' lines comparable.

Three things that had to be decided rather than assumed.

**A page cannot name its own key.** The station fills in `session` and stamps `at_ms`
rather than believing what arrived: a browser's clock is the thing in question here, and
a tab that could name its own key could write over another tab's row. There is a test
that says so. Which leaves the page not knowing which row is itself — so `client.report`
**answers the key it landed under**, and that is the only way a browser learns its own
session id.

**A row is a reading, so it is dropped when it stops being one.** Unlike task 48's log
raise — which could be left to expire with its connection, because it is an *ask* — a
client row is the last thing a page said, and a socket can stay open long after the page
stopped saying anything. So a disconnect is the usual end of a row and a sweep is the
other, at **ninety seconds** rather than sixty: a browser throttles a backgrounded tab's
timers to roughly one a minute, and pruning at the throttle would make the tablet at the
back of the room flicker out of the list and back on alternate sweeps.

**Reporting is not something a panel opts into.** Every page reports every two seconds
whether or not anybody has the System panel open anywhere, because the browser worth
knowing about is precisely the one with nobody in front of it. One WebSocket message per
browser per two seconds, matching `REPORT_INTERVAL` so a station row and a client row
are the same age.

**And what is on the wire, which is four figures and not one.** The entry listed
network throughput among what was missing, and the word turns out to name four
different things measured in four different places — so the panel shows them apart
rather than adding them up:

- **What each connector put out**, counted in `Frame` at the point the packet is
  handed to the socket. *After* the dedup, which is the whole value of it: the DMX
  family skips a universe whose image has not changed and is not yet due a refresh,
  so a settled rig sends a fraction of what its universe count suggests, and a figure
  derived from the patch would hide exactly the optimisation it is there to show.
- **What crossed each peer link**, counted around the socket rather than at the twelve
  places that write a frame. `protocol::Counted` wraps the `TcpStream` before it is
  split, so the handshake, the catch-up batches, the heartbeats and a raised log are
  all in the figure and none of them had to remember to say so. `S: Unpin` rather than
  a pin projection, which is what let it be forty lines and no new dependency.
- **What the station sent each browser**, counted in the socket's own send task. This
  is the one figure a page cannot supply — no browser API says how many bytes arrived
  on a WebSocket — so it sits beside `session` and `at_ms` as something the station
  fills in, over the window between two of that page's reports.
- **What the machine's interfaces carried**, which is not the console at all.

**And the machine itself, which is the other half of every figure on the row.**
`cpu_percent` and `mem_used` are *this process* and deliberately so — a console sharing
a box with something else should report what it is costing, not what the box is. The
sentence that finishes is `MachineStats`: global CPU across every core, memory and swap
in use, the load average, how long the machine has been up as against how long this
backend has, free space on **the volume the showfile is written to**, and the warmest
sensor the machine exposes. The pair is the point. A station at 4% on a machine at 96%
is not a comfortable station; it is one about to be starved by something nobody is
looking at, and until now no console could say so about itself.

**A process percentage and a machine percentage are not the same unit**, and putting
them side by side is how you find that out. `sysinfo` reports a process's CPU as a
share of *one core* — a multi-threaded console can exceed 100% — and the machine's as a
share of all of them, so an unlabelled 15.2% beside an unlabelled 6.4% reads as the
console using more than the box it is in. The panel says "of a core" and "of 18 cores"
and spells the comparison out in words: *this console is 0.8% of it*. The whole value
of publishing the pair is that comparison, so an axis it is silently wrong on would
have been worse than not publishing it.

Three of those were chosen for what ends a show rather than what slows one. A full
disk is a show that cannot be saved. Swap in use on a console is a fade that will
stutter. And a temperature is the honest answer for a station in a truss-mounted case
in a roof void, which is a thermal question long before it is a processing one — the
warmest sensor rather than one named, because what a machine calls its packages differs
per platform and per vendor while the question does not.

That is where `systemstat` was looked at and not taken: `sysinfo` is already a
dependency, already refreshed by this reporter every window, and has all of it behind
`network`, `disk` and `component` features that were simply switched off. A second
crate for figures the first one already had would have been a second thing to keep
working on four release targets.

**A relative showfile path resolves to no volume at all.** A mount point is absolute,
so `starts_with` matches none of them and the disk figure comes out as a plausible
zero — and a relative showfile is the ordinary case rather than a corner one, since
`demo.sh` passes `.demo/demo.db` and a console started from its own directory passes a
bare name. The path is absolutised once, in the reporter's constructor, by resolving
the *directory* and putting the file name back: canonicalizing the file itself fails
when the show is about to be created, which is exactly when a console is started. The
test that caught it is the one that asserts the machine answers at all, which is the
argument for asserting that rather than trusting a platform layer to be non-empty.

Two traps in it. **Loopback has to come out**, or a laptop running `demo.sh` counts
every byte it sends itself twice, once each way, and swamps the figure. And
**`PeerLatency` must not replace the link row whole** — it fires per heartbeat, more
often than the byte window closes, so the obvious `insert` wipes the counters and
throughput reads zero almost always. The two halves of a `PeerLink` are measured on
different schedules and each writes only its own.

One hole, named rather than hidden: the per-port commands an OpenHaunt node is given
travel over MQTT from the device manager, on its own schedule and inside nobody's
frame, so they are not in the output figure. Small by construction — a three-second
fade is one message to a node that can run it, not a hundred and twenty — and the
panel says so.

**And the figure found a bug on its first evening, which is the argument for having
it.** An sACN output configured for universe 1 alone reported 55 kB/s at 35 frames a
second, which is about two and a half universes' worth of packet per frame and not
one. `OutputConfig::universes` documents itself as "which universes to send" and
`carries()` is the predicate for it — and **only `OutputCoverage::of` ever calls it**.
The connectors render every universe in the patch and send every one that the dedup
has not settled, so an output restricted to one universe is transmitting all seven,
and the Outputs panel's coverage warnings describe a routing nobody implements. Left
unfixed here deliberately: this task is an instrument, and changing which universes
reach a wire is a change to what reaches lamps. It wants its own entry and its own
decision — whether `carries` should gate the send, or whether the field should stop
claiming to.

```
cargo test -p pult-backend --lib clients   # the map, the sweep, and who may write a row
cargo test -p pult-schema client           # a frame rate read off its window
cd frontend && npm test                    # the meter, the traces, and what counts as struggling
```

What is left of the entry's list is **sync backlog** and **broker stats**. Network
throughput is done and then some — four figures rather than the one the entry imagined
— and WebSocket client counts are the Browsers section, which counts them by listing
them. The two remaining are a figure each and neither was blocking anything: a backlog
is a depth on the sync channel and belongs with `station-clock-offset`, which is
already reading that link; broker stats belong to the OpenHaunt device manager, which
is also where the MQTT bytes this task could not count honestly would come from. Both
are better done there than bolted on here. What was blocking `performance-tests` was
the browser figure, and that exists now.

### 50. What actually leaves the console (done)

The third of the three panels task 48 opened, and the one that had to answer nothing
new: *Stations* is who is here, *System* is what it costs, and **On the wire** is
which bytes went. A DMX sheet per universe, and the messages a node was sent.

**Asked for, never published — and the entry already knew why.** A universe image is
512 bytes forty times a second. Task 49 settled the same question for browser stats:
continuous figures stay where they are and only the exception crosses. So a view of a
connector's traffic exists **while somebody is watching and not otherwise**.
`infra/connectors/viewers.rs` holds who is looking, `output.watch` and
`output.unwatch` are the RPCs, and a connector nobody is watching is never asked —
the manager's view arm sleeps for an hour rather than waking ten times a second to
find out that nobody is there. Nothing expires: the ask is recomputed from who is
here, so a tab that vanishes stops the drawing as surely as one that closes.

That left one question the entry raised and did not answer: snapshot on demand, or
diff at panel rate. **Both, in the simplest form each can take.** The rate is 100 ms,
because ten a second reads as live and forty does not read at all; and a drawn view
that has not changed is not sent, so a settled rig with the panel open costs nothing.
The comparison excludes the stamp, or every redraw would be news.

**The pluggable half.** A connector describes its
own traffic in **shapes rather than protocols**: `OutputPlugin::observe(focus)` answers
`Vec<OutputSection>`, each carrying a `SectionBody` — `Universes` or `Messages` today
— and `frontend/src/lib/components/wire/views.ts` is the one place a shape becomes a
component. So an output whose traffic carries universes gets the DMX sheet for
nothing; one that looks like neither adds a variant, a component and one line in that
table, and no panel changes. A shape this build has never heard of draws as itself
rather than vanishing, which is the rule the layout tree already follows for a panel
id it does not know and the geometry loader for a mesh it cannot read. And `observe`
defaults to `None`, so a connector that does not describe itself says so and the
panel prints that rather than an empty sheet.

`focus` is opaque the whole way through — it is named in the connector's own terms, a
universe number or a node's serial — because a field per protocol is exactly what a
seam meant to carry a protocol nobody has written yet cannot have. The one place a
universe is spelled as a focus string is `universeFocus`, beside the sheet that asks
for one.

**A peer's output is asked for down the link**, because only the station holding a
socket can say what went through it. `SyncMessage::OutputWatch` carries the whole ask
and empty is the withdrawal; `OutputTraffic` carries the answer back. Protocol version
6, and the same shape `LogRaise`/`LogLines` already had. A peer's ask lands in the
*same* `Viewers` table a browser's does, with the peer standing in for a session — so
a connector cannot tell a booth across the room from a tab on this machine, and there
is one unwind rather than two.

Three things worth holding on to.

**The DMX family pays nothing for being watched.** `UniverseCache::observe` reads the
images the dedup was already keeping, because skipping an unchanged universe needs
them anyway. Art-Net, sACN and the sACN a gateway is fed all answer through it, so a
sheet reads the same whichever carried the universe — and watching costs nothing on
the frame path, which is what makes it safe to offer on a rig that is already busy.

**A keep-alive is not movement.** The cache now records when a universe last *changed*
as well as when it was last *sent*, because the two differ by design: a settled
universe goes out every 800 ms and has not changed in an hour. A sheet that read the
send as movement would report every idle universe as busy, which is the failure mode
of every DMX tester that only has one timestamp.

**What is not free is a ring.** Discrete messages have to be kept to be shown, so
`OutputPlugin::watched` tells a connector whether anybody is reading at all. OpenHaunt
keeps its port commands only while somebody is, and throws away what it held when the
last viewer goes — it was a picture of what happened while somebody was looking, and
nobody was. Two rings then, the connector's and `frontend/src/lib/wire.ts`'s, and
**both count what they dropped**: the station's drains on every look, the browser's is
bounded by what a person can read, and neither loses a message silently, for the
reason the system log makes a gap in `seq` visible.

One thing this panel makes visible rather than fixes. Task 49 found that
`OutputConfig::universes` claims to be a filter and only `OutputCoverage::of` ever
calls `carries()` — the connectors render every universe in the patch and send every
one the dedup has not settled. **The sheet's universe chips are now where an operator
sees that**: an output restricted to universe 1 lists every universe in the show. Left
unfixed here for the reason task 49 left it: changing which universes reach a wire is a
change to what reaches lamps, and it wants its own decision. It has an entry now.

```
cargo test -p pult-backend --lib connectors   # the registry, the cache, and who is drawn for
cargo test -p pult-backend --test wire        # two stations, and a console watching the other's wire
cd frontend && npm test                       # what a browser makes of the batches
```

### 51. What it actually costs at five thousand, and a beam that reads as light (done)

Three items at once — performance-tests, rig-viewer-fidelity and engine-admission —
because the last two are answers to the first, and doing them apart is how each gets
decided on taste instead of on a number. One task rather than three, on the same
reasoning: they were one piece of work and the decisions cross between them.

**The instrument was wrong first, and that was the whole blocker.** `demo-measure.mjs`
slept a second, slept four more, read `stations` once and printed whatever window
happened to be sitting in the row. One sample of one two-second window, caught at an
arbitrary phase against the station's own reporting tick — and the window it printed
could still be half full of the cue-taking the script does to get the show moving.
Two runs at 505 fixtures came out fifty per cent apart, and that is why.

The fix is three things. Take several windows and report the **median with the spread
beside it**. Discard the first. And, before any of that, **wait for the cue-taking to
go quiet** rather than sleeping a guessed constant — quiet is the real end of the
burst on any rig size, where a constant is right only for the one it was measured on.
That last one mattered most: it took the frame spread from 92% to 8%, the CPU spread
from 2332% to 14%, and two consecutive runs from 50% apart to **3.4%** apart. The
instrument now prints how much it disagrees with itself, and refuses to be compared
when that is over a quarter.

One thing it found on the way, and it is worth writing down because it looked exactly
like a regression. The script reported **300 browser updates during a run**, against
the zero task 44 promised. Nothing was wrong with the console: taking a cue writes
`live_fades` per captured fixture, three hundred of them on a 505-fixture rig, and the
script zeroed its counter the instant the last `goNext` returned while those
broadcasts were still flushing. A second station connected to the same running show
saw zero. The measurement was counting its own setup.

**What five thousand fixtures actually cost.** In release, `--size 5000`:

| | 505 fixtures | 5000 fixtures |
|---|---|---|
| frame | 0.89 ms (±8%) | **4.77 ms** (±8%) |
| evaluating | 0.71 ms (81%) | 4.50 ms (**94%**) |
| assembling | 0.00 ms | 0.00 ms |
| socket | 0.17 ms (19%) | 0.27 ms (6%) |
| rate | 36 Hz | 29 Hz |
| browser updates in 10 s | 0 | 0 |

**The entry's own prediction was wrong in the way that mattered.** It said "per-universe
assembly and 59 socket writes are the part of the frame that does not shrink", and
expected the station to be fine and the browser not to be. Assembly and the socket are
**6% of the frame** at five thousand. The third figure — assembly against the socket
write, added by hand exactly as the entry suggested — is what shows that, and it turned
the whole prediction over. Evaluating is 94%, and it is *sublinear*: ten times the rig
for six and a half times the cost.

So the two questions this was measured for are answered, and neither the way it was
guessed:

- **Rayon over fixtures inside a connector's frame** is the only lever with anything
  under it. Everything else is noise beside 94%. But at 19% of a 25 ms budget it is
  not urgent, which is the more useful half of the answer.
- **Instancing the fixture bodies** does not have to be decided yet, so it is not.
  The viewer went imperative, which buys the option without presuming it.

The one figure that is not comfortable is the **rate**: 29 Hz where DMX wants 40, on a
frame that is using a fifth of its budget. That is not frame cost, and it was written
up as `connector-frame-rate` — the item this measurement most argued for, and task 56.

**Threlte is gone, and the two live defects went with it.** Three files, 844 lines,
and the hard part was already imperative: gizmo picking and dragging were hand-written
raycasting before this. What the declarative layer actually supplied was a canvas, a
render loop, hover events and an HTML overlay. What it also supplied was both defects
this entry carried — `<T.ConeGeometry args={...}>` had reactive `args`, so a geometry
was rebuilt per fixture per frame; and a `<T.SpotLight>` inside `{#if level > 0.01}`
changed the scene's light count as a fade crossed 1%, recompiling every material in
the scene on the most ordinary thing a console does. **Both are gone by construction
rather than by fix**, which is what the entry predicted for the first and not the
second.

`camera-controls` stays as a direct dependency: only the Threlte wrapper went, and
every `controls.setLookAt` survived unchanged.

**The beam is not geometry.** One instanced open-ended cylinder for the whole rig, and
the cone is vertex displacement — the far ring scaled by `tan(angle)` times the throw
in the vertex shader, so a zoom costs one float in a buffer. Brightness is the tube's
own surface normal against the view, raised to a power that falls with how end-on the
beam is seen — so side-on the edges fade to nothing and down the barrel the whole disc
lights up, one term for both — times an attenuation in metres that is steeper for a
wider beam, all additive blending in one fragment shader with no post-processing chain.
Colour is scaled in HSV, **value only**, so a dim beam keeps its hue rather than
crushing towards grey. Haze is turbulence in world space with time as the third axis,
floored at the beam's own intensity; value noise rather than simplex, because four
octaves of it is a dozen lines somebody can check.

The first shader landed here got the silhouette wrong and it was visible at once: it
took the term from the beam's *axis* against the eye, which is the same for every pixel
across the beam and so cannot make the edge differ from the middle, and drew flat,
hard-edged, faceted cones. It also wrote the strength into alpha, which additive
blending multiplies the colour by, so everything was squared into a ghost; started
every beam from a point rather than the lens; and multiplied by smooth noise, which
took light away in blobs. The fix is the paragraph above, checked against a
standalone page rendering five beams from four camera positions rather than against
the demo, whose one lamp from front of house was too little to judge a shader by.

**Haze is show data**, seeded from a station preference the way `home_fade_ms` is: how
hazy the room is is a fact about the room rather than about the screen looking at it.
It reaches no lamp. Note the cost, which is the `home_fade_ms` cost and deliberate:
SQLite cannot add a NOT NULL column without a default, so `add_missing_columns` adds it
nullable and the required-column check then refuses an older showfile, plainly. No
`SCHEMA_GENERATION` bump — a showfile is not a migration target.

**Strobe cost almost nothing, because the entry was out of date.** `ParameterKind::Shutter`
and `ParameterKind::Strobe` already existed, and `attributes.rs` already mapped
`Shutter1`, `Shutter1Strobe`, `StrobeFrequency` and `StrobeRate` in both directions with
tests. All that was missing was the drawing. And the drawing is the *only* place it
could go: a strobe channel carries a **rate**, the console sends the byte and the
fixture does the flashing, so `pult-render` has nothing to work out and needs no corpus
case. The square wave is in `beam.ts` because it is a fact about the picture.

**The disk is off the actor, and the batch has no constant in it.** `persist`,
`oplog::append` and `order::save` were awaited inside the actor's command arm against a
pool of one connection, so one operator's edit waited behind another's fsync. A queue
where each reply waits for its *own* fsync would serialise them just as thoroughly, so
the writer commits a **group**: while a commit is in flight everything that arrives
queues up, and when it lands they all go into the next one. That is the whole rule —
no window in milliseconds and no batch size, because a constant would have to be right
for somebody else's disk. On a fast disk with one operator it degenerates to one write
per commit and adds no latency.

A command still replies only when its write is durable: the actor hands its receipts to
a task that answers the caller when they land. The non-goal held — no new durability
guarantee, and none taken away.

**Moving the write off the actor's thread was not enough, and the first attempt proved
it.** While the actor still `await`ed each write before reading its next command, the
writer never held more than one submission and had nothing to group, so a
five-thousand-row import was five thousand commits. Coalescing inside the batch did
nothing at all until the actor stopped waiting, which is the useful lesson: the disk
being on another task and the disk being out of the critical path are different things.

**The oplog is the exception and stays awaited.** Entity state is read from memory, so a
create that has not reached the disk is still visible to the next `Get` — which is what
makes deferring it safe. The oplog is read *back from the file*: undo is a query over it,
the History panel reads it, and a peer catching up is served `oplog::since` from SQLite.
Deferring it raced a user's own Ctrl-Z against their own write, and **seven tests said so
immediately**. Worth knowing for showfile-management, which proposed keeping the show in
memory until an explicit save: that proposal has the same problem and a larger version of
it, since undo, history and catch-up all read persisted state today. **Task 52 took the
proposal off the table for exactly this reason** — Save is a checkpoint, the crash
journal keeps writing, and a version is a copy of a file that is always current.

**And the write path had a quadratic in it that none of this was about.** `persist_order`
rewrote the *whole* collection order after every create — a DELETE and N INSERTs, N times,
which is about 12.5 million inserts to patch 5000 fixtures. Seeding that rig took over two
minutes; with `order::append` doing the one row a create actually needs it takes 78
seconds, and 2000 fixtures went from 21.9 s to 6.3 s. The comment on `order::save` said
creates are human-paced, which is true of an operator and false of an MVR import.

**And the larger quadratic was not the disk at all.** With the order writes fixed, seeding
5000 fixtures still took 78 seconds where everything on the persistence path accounted for
three. The cost was `broadcast_after_set`: a create has to broadcast the *collection*,
because a subscriber watching `fixtures` is watching the collection and a pattern matched
against `fixtures/__create` reaches nobody — so every created fixture deep-cloned every
fixture already in the show, as JSON.

Coalescing it is what fixed it, and the first attempt at that is worth recording because it
barely worked. "Flush when the queue is empty" sounds sufficient and is not: a client with
sixty-four writes in flight empties the queue between almost every one of them, so it still
flushed per row and bought a factor of two. A **ceiling in time** is what was wanted —
`COLLECTION_FLUSH_EVERY`, fifty milliseconds — which is twenty a second, faster than anybody
reads a list filling up, and a bound that holds however the queue happens to behave. It is a
ceiling on a burst rather than a delay on a write: an idle console has not flushed for far
longer than that, so a single create still goes out at once.

| | before | after |
|---|---|---|
| 2000 fixtures | 21.9 s | **1.01 s** |
| 5000 fixtures | over 120 s | **3.40 s** |
| 5000 with 300 cues | over 120 s | **6.19 s** |

One trap, and it hangs rather than slows. **An owed broadcast has to be able to wake the
engine loop.** The last write of a burst marks the collection, the flush is not yet due,
and the actor blocks on whatever the show wants next — on a settled station, never. So the
sleep is shortened to what is left of the interval and the wake branch flushes. Two tests
caught it, one of them by never finishing.

Twenty-fold, and linear again. The lesson worth keeping is the order the two were found in:
the disk was the obvious suspect and was real but minor, and the expensive thing was a
broadcast nobody was reading. **Measuring which is which is why the instrument came first.**

Two things it needed. A **second pool** to the same file, because the showfile is WAL
and a peer's catch-up read must not queue behind a commit or land inside one; a show
in memory shares the one pool instead, since every `sqlite::memory:` connection is a
different database. And `order::save` stays **outside** the batch — it opens its own
transaction and SQLite has no nested `BEGIN` — which is free, because an order changes
when something is created or moved and never when a value does.

**Per-source admission is a queue per class, in front of the engine rather than inside
it.** Plugins, browsers and catching-up peers shared one 256-deep channel with no
priority, so the way to make an operator's fader stop responding was for somebody's
plugin to be busy. Now each of Operator, Station, Peer and Plugin has its own bounded
queue and a router forwards in weighted turns. The engine still reads one channel and
still knows nothing about where a command came from, so `EngineCommand` is unchanged
and none of the 44 `.0` call sites had to learn a new shape.

The weights are **turns, not priorities**. Strict priority starves, and a peer replaying
twenty minutes of oplog would never finish while anybody was programming. And a full
queue makes its own senders wait rather than dropping — `OutputHandle::push` drops
because a skipped frame is redrawn a fortieth of a second later, and a skipped write is
gone.

**Three counts became gates, and milliseconds did not.** Task 43 explains why a timing
threshold on a shared runner flaps, and a flapping gate gets disabled. These do not
flap: a running show pushes **zero** fixture updates at a browser; a drag of sixty
frames is **one** row in the history; and a settled rig reports **no** universe as
having changed. A fourth candidate, engine messages per cue take, was left out because
its honest bound varies with sequence and capture count, and a gate whose threshold is
arguable gets loosened until it means nothing.

The third one caught something on the way, and the diagnosis is the useful part. It
failed at 9 changed universes — and reported **the same 9** when the observation window
was tripled from four seconds to twelve. A count that does not grow with the window is
one event, not a leak: the rig coming up. So the first second of views is discarded, for
exactly the reason the measurement discards its first window, and the dedup was never
wrong.

**A regression that was not one, and what it found.** With all of the above in place the
full suite came back 1121 passed and 7 failed, against 1123 and none on `main`, every
failure in `tests/roster.rs` and every one a station that "never accepted a connection
in 30s". Three structural suspects were lined up — the coalesced collection broadcast,
the admission router as an extra hop, the deferred write receipts — and all three were
wrong. Roster alone passed, which was worse than useless: it passed alone on the branch
and failed alone on the branch, one run in three, and only a full-suite run against a
full-suite run said anything at all.

What the failing stations were doing was `Disks::new_with_refreshed_list()`, on the
runtime thread, in `StationReporter::new`, for **six and a half seconds** — and on `main`
for a third of one. Same code, same `sysinfo`. The difference was the *directory the
binary sat in*: on macOS the first enumeration of the volumes goes through a
`dispatch_once` in `FSMountGetVolumeUUID` that reads the executable's own directory as
a bundle, `_CFBundleReadDirectory` over every file in it, and the repository's
`target/debug/deps` held 645,585 files and 181 GB where the `main` worktree's held
4,739. `main`'s own roster binary copied into that directory took the same six seconds;
the branch's copied into an empty one took a quarter of one. The branch, built into a
clean target directory, passed the whole suite: **1128 and none**.

Two things are worth keeping from it. The comparison was not like for like — a worktree
is a fresh target directory, and a fresh target directory was the variable — so the rule
is the same target directory on both sides, or the same disk under both binaries. And a
synchronous constructor that asks the operating system questions was a defect on `main`
too, one that a fast machine with a tidy build directory never showed: a station that
blocks its own runtime for the length of a sensor read spends a test's connect budget
before the test begins, and a budget that is barely enough on this machine is none at all
on a slower one. The probe is a thread now, and the reporter reads its latest answer off
a `watch`; the first row waits for the first reading, and no row after it waits for
anything. No test's budget was raised.

**What is left, deliberately.** 3D placement went to `scene-editing`, where it belongs:
moving things is a new capability rather than fidelity, and it changes the show rather
than the picture. Instancing the fixture bodies waits for a reason to exist. And
`--measure-browser` is its own mode rather than part of `--measure`, because a page
drawing five thousand fixtures competes for exactly the CPU that run holds still — the
two sets of figures must not be read side by side, and it says so where they are printed.

```
scripts/demo.sh --measure --release --size 5000    # what a frame costs, with its spread
scripts/demo.sh --size 5000 --cues 60 --slice 0.02 # one axis at a time
scripts/demo.sh --measure-browser --release --size 5000   # the page's own figures
cargo test -p pult-backend --test counts           # the three that are gates
cd frontend && npm test                            # the evaluator corpus and the helpers
```

### 52. A show is a folder, Save is a version, and the console opens onto a welcome screen

`showfile-management` and `showfile-assets-folder` together, because the second was
never a separate question — the entry said so — and both fall out of the same four
decisions, taken with the user on 2026-09-03.

**1. A showfile is a folder bundle, `Name.pult/`.** `bundle.toml`, `show.db`,
`assets/<sha256>` and `versions/<id>.db`. The assets left the database because a
version is a copy of `show.db`, and a copy carrying a 256 MB fixture archive would
cost that per save; as files, fifty versions of a show hold one copy of each mesh.
A `.pultz` — the folder zipped — is the travelling form, because a folder does not go
in an email and on some platforms is not one thing at all.

**2. A version is a replicated row, and the snapshot is each station's own file.**
This is the answer to the question the old entry called the hard part: *is a
checkpoint session-wide agreed or per-station?* It is **both, and they are different
objects**. The row is PERSISTED, so every station knows the version exists, who took
it and when, and undoing the save undoes it everywhere — which is what Ctrl-Z after an
accidental Save should do. The snapshot cannot replicate: it is a copy of *this*
station's `show.db` at that instant, and a station that joined afterwards never held
that state. So each station copies its own when a `versions` row lands, and publishes
the LOCAL `versions_here` — which is the only way a panel can honestly say "not on
this station" about a peer's version.

The entry also guessed wrong about the mechanism. **Revert is not an oplog rewind.**
The log is pruned on task 37's retention, so yesterday is not reachable through it and
never will be; a version has to be a whole-file copy. The row keeps its `clock` all
the same, because a future *diff between two versions* has to anchor on something, and
a timestamp across machines with unsynchronised clocks is not it.

**3. Four demo shows, generated in Rust at open time.** Haunt, Theatre, Club and
Festival. The demo was a Node script driving a running station over the WebSocket,
which is right for the *measurement* rigs and cannot be a button: somebody opening
this console for the first time has no Node, no repository and no terminal.

**4. With no show argument the console starts with no show open.** Which turned out to
be a real state worth building rather than an absence: the engine, the sync layer and
the HTTP server all run, against a database that is never written anywhere, and the
asset store is the one part with nowhere to put anything — so it is the one part that
says no. The welcome screen is served over the same socket the show would be.

**Opening a show is this station stopping and another one starting in its place.** A
station is built around one showfile from `start` down — the pools, the engine's
state, the asset store, the plugins the roster asked for — so `Console` is the process
around it: it keeps the configuration, pins the port the OS gave out so a `--port 0`
console does not move every time somebody opens a show, and starts the next station.
The `show.*` calls are RPCs rather than commands, because which showfile a console has
open is nobody's to undo and must not be told to a peer; they answer `{ok: true}` and
*then* the station stops, which the client sees as the disconnect it already handles.

**The identity moved to the machine.** It was already meant not to travel with a show,
and a folder is far easier to drag onto a stick than a file was. A station is now told
where its id is — `Config::identity`, `PULT_IDENTITY`, or the config directory — so a
copied bundle no longer clones the console that made it.

### The traps

**Three things held a resource past the station that owned it, and all three looked
like a console that never came back.** `JoinHandle::abort` lands at the task's next
suspension point, so the HTTP listener was still bound when its replacement tried to
bind it — shutdown now *awaits* what it aborted. The sync accept loop was a spawned
task holding its own listener, so it is selected beside the event loop and dies with
it. And `axum::serve` hands each connection to a task of its own, which is not a child
of the future that accepted it: aborting the server left every open WebSocket talking
happily to a station that had stopped, with the page still saying "Connected" and
subscribed to an engine that no longer existed. A station now tells its sockets it is
going.

**A snapshot has to contain its own row.** The copy waits on a `WriteJob::Barrier`
rather than on the version's own receipt — the writer's queue is ordered, so a barrier
submitted after the upsert lands after it. Getting that backwards makes every restore
quietly forget the point it restored to. Shutdown waits for the checkpointer *after*
the engine, too, since the engine holds the only handle and a `VACUUM INTO` aborted
mid-copy leaves half a snapshot and a connection the pool close is about to wait on.

**Restore always leaves an orphan, by construction.** The "Before restoring…" version
is taken *after* the database being put back was written, so its row is not in that
database. `versions::reconcile` reads the row back out of the snapshot's own `versions`
table and re-creates it — otherwise the safety net an operator reaches for when the
restore was a mistake is a file with nothing naming it.

**Seeding a demo inside `start` made the console unreachable.** The listener is bound
before the demo runs, so the port *accepted* and then answered nothing for as long as
two hundred writes took — the worst of the three states a console can be in. Demos are
seeded on a task beside the server.

**A version's name must not carry a time.** The station would have to format it in UTC,
and it would sit in the list beside its own row rendered in the reader's local time,
disagreeing with itself by an hour or nine. `Version::named()` answers whether an
operator gave it one; the browser decides what an unnamed one is shown as.

**A copy is a new show to the network.** Two bundles carrying one `show.id` find each
other over mDNS, decide they are one show and merge — so an operator who copied a show
to try something out would watch it land back on the original. Save-as writes a new id.

**And a demo that wrote its own rotations aimed three rigs at the back wall.** A
fixture's own axis is −Y, so zero rotation *is* hanging, and `{90, 0, 0}` written
meaning "hanging" is a quarter turn away from it. `Transform::facing` already existed
and does the decomposition properly; the demos now say which way a light points as a
direction and never write an angle. Found by the user looking at the result, which is
the only way it could have been.

### What came with it

**A stock catalogue.** A `SceneObject` with no mesh draws an empty group, so a truss a
console made for itself was invisible and its lights hung in the air. `pult-schema`'s
`catalogue` names a handful of standard pieces — F34 in three lengths and a corner,
stage decks, wall panels and flats — with their dimensions; `pult-codegen` emits the
table to TypeScript so there is one of it; and `frontend/src/lib/stock.ts` draws them
procedurally, one merged geometry per id however many are in the rig. The MVR importer
deliberately never guesses one: a drawing's object says what it is with its mesh, and
picking an `f34-2m` because the name said "truss" would put a measurement into
somebody's rig that nobody measured.

```
cargo test -p pult-backend --test shows      # opening, saving, restoring, travelling
cargo test -p pult-backend --lib demo        # every demo hangs together, and points down
cargo test -p pult-schema --lib catalogue    # and the pieces are the sizes they are named for
cd frontend && npm test                      # the reload rule, and what a card says
cargo run -p pult-backend                    # → the welcome screen
cargo run -p pult-backend -- --demo festival --show /tmp/F.pult
```

### 53. Three rigs looked at, a viewer that draws when there is something to draw, and a switch that says so

The user opened the four demos from task 52 and looked at them, which is the only
review a demo can have. Four things came back, and a fifth was found on the way.

**1. The Theatre's booms were horizontal.** `truss_run` runs along X and the booms
were two more of it, with their lanterns stacked in the air beside them. A boom is a
run of truss stood on its end, so `kit::boom` is the same run with its handle turned
a quarter about Z — and a lantern on it has to be written *in the boom's frame*,
since a fixture's position is relative to what it hangs off, rotation included.
`kit::on(parent, world_offset, world_direction)` does that inversion once, so no
show file hand-inverts a rotation: the demos still say where a light is and which
way it points in world terms. The demo test now composes through `world_transform`
before asking whether a light points down, which is what it should have been asking
all along, and checks that nothing hangs *inside* the bar it is clamped to.

**2. The Club's washes floated beside the truss.** Hung 600 mm off the bar in Z. Now
a mover and a wash by turns along it, all at `HUNG_BELOW` — 350 mm under the centre
line, which is the chord square plus a clamp, and the one figure every demo uses.

**3. The Festival was two hundred of one thing, all on at once, in cues that meant
nothing.** It is now five kinds — profiles out front, spots and washes by turns over
the stage, a row of blinders on the downstage truss looking at the crowd, beams and
LED strobes along the back wall, a floor package standing on the deck, and a wash
tower each side — in a layer per system, and **seven playbacks** each with a short
stack of looks that ends in *Out*. Five come up in a look and two (the blinders, the
floor) sit at nothing until Go, because a rig where everything is asserted at once
is a rig where nothing reads. `the_club_and_the_festival_come_up_running` had
asserted that *every* sequence was running; it now asserts that something is moving,
and a second test that something is waiting.

**4. The GPU was pinned at 100% on the Festival and at 40% on a dark Theatre, on an
M4 Max.** Three things, each of which was true on its own:

- The view drew at the display's rate, which on a ProMotion display is a hundred and
  twenty a second, whether or not anything had changed. It now draws **at most sixty
  a second and only when there is something new to see**: the camera moved, the rig
  changed, or the picture animates on its own clock (a lit beam with haze in it, a
  strobe). "Changed" is worked out by comparing this frame's attributes against the
  last frame's rather than by watching a store, because the output store ticks every
  frame whether or not a value moved. A settled dark rig now costs **nothing**, and
  the panel prints *idle* rather than a rate for frames it did not draw.
- Every fixture was an instance of the beam mesh, lit or not, and an unlit cone
  whose fragments all `discard` is still a cone that is rasterised. Only lit beams
  get an instance now, packed from the front.
- The canvas was rendered at the display's full device pixel ratio. That is now a
  view setting, defaulting to 1.5.

And the one that was *not* true, found by measuring. The beam fragment shader — four
octaves of value noise per fragment, over a hundred and forty overlapping additive
cones — was the obvious suspect, and it is not the cost. Measured on this machine
with `EXT_disjoint_timer_query_webgl2` on a 960-px-wide panel: the beams are 5 ms of
a 5.4 ms frame; drawing them at 1×, 1.5× and 2× gives 4.9, 5.4 and 6.5 ms; four
octaves of haze against none is the same number; 1,536 triangles per cone against 64
is the same number; but 10, 40 and 140 lit beams give 1.1, 3.1 and 5.7 ms, and
single-sided unblended cones 2.8 ms. So the cost is neither per pixel nor per
triangle nor per instruction: it is **per blended layer stacked on the same tile**,
which is what a tile-based GPU serialises. That finding is a candidate,
`beam-overdraw`, below — the lever is how many cones cover the same pixels, and
neither resolution nor shader work moves it. What did come out free was
`HEIGHT_SEGMENTS = 1`: a cone is straight, so its rings were drawing nothing.

**And the figure the panel showed was the wrong one to feel lag by.** "8.7 ms" was
the gap between animation frames, which a rAF loop keeps up whatever the GPU is
doing; the picture can be several frames behind the pointer while every CPU number
looks fine. The panel now prints both: the page's own work per drawn frame, and how
long the GPU took over it where the browser will say.

**5. Work light.** A `View` sheet on the rig panel: how brightly this screen draws
what no fixture is lighting, and how many pixels it renders. This screen's and nobody
else's — the haze is the show's, because how hazy the room is is a fact about the
room, and a work light is not — so it sits in `localStorage` beside the layout, in
`frontend/src/lib/stores/view.ts`, and every rig panel on the screen reads the one
store.

**6. Opening or closing a show showed "the console stopped answering", twice.**
Opening a show is the station stopping and another starting in its place, and a page
that treated that as a lost console drew three screens in a row for one act: the
stopped-answering cover after its 600 ms grace, a reload, then "connecting to the
console". Now it is one screen. The menu and the welcome screen **begin the switch
before they ask** — `beginSwitch("opening Festival")` — so the cover is up before
the socket goes, and the switch is kept in `sessionStorage` so the reloaded page
comes up already saying it. It ends when the station answers `/api/config` and this
tab is staying put, which is the hook `watchStation` now offers. And the tablet at
the back of the room, on somebody else's socket, is *told*: the station's stop signal
carries **why** (`ShowSwitch::describe` — "opening Festival.pult", "closing the
show", "restoring a version") and the socket's send task writes it into a close frame
with code **4001**, so a page that did not press the button draws the same screen.
Any other close code is still a lost console and draws as one.

### The traps

**The send task holds the sink, so only it can say goodbye.** The socket loop and
the send task both watch the stop signal; the loop drops the last sender into the
send task's queue at the same instant the signal fires, and a `select!` that took the
closed-queue branch first hung up with no farewell. The two-page test caught it on
the first run — one direction got the reason and the other did not — so the reason
is checked on *both* branches.

**A hidden tab has no animation frames.** Half a morning of "the readout says idle
while the picture animates" was Chrome pausing `requestAnimationFrame` in a tab
whose window was occluded — the browser extension used to drive the page could
screenshot it but not unhide it. Every figure above was taken in a headed Chrome
launched by Playwright, where the window is on screen; nothing measured through the
extension's tab is worth anything, and neither is a `--measure-browser` run in
headless Chromium, whose GPU is SwiftShader.

**A single-precision record compared against a double is never equal.** The
last-frame record is a `Float32Array` and the value is a `number`; compared raw, a
rig that has not moved "changed" every frame and the Theatre drew forever. Through
`Math.fround`.

```
cargo test -p pult-backend --lib demo      # the booms, the clamp distance, what is running and what waits
cargo test -p pult-backend --test shows    # a switch still opens, closes, restores
cd frontend && npm test                    # the close code, the patience, what the screen says
cargo run -p pult-backend -- --demo festival --show /tmp/F.pult   # and look at it
```

### 54. A second look, and a cue is the stack up to it

The user looked again at task 53's work. Four corrections, one of which was a
playback bug that had nothing to do with the demos, and two questions answered below
as a candidate rather than as code.

**1. Haze density meant nothing at 0.** The number mixed the turbulence folds into a
beam that was drawn in full regardless, so a clear room still showed every beam and
the scale read as arbitrary. It now means what it says: the haze is the only reason a
beam can be seen in the air, so density is first how much of the beam shows — none at
0, all of it at 1 — and then how much of the folds. The default is 1, because a rig
view that draws no beams is the more misleading picture of the two, and a designer
lighting a clear room turns it down.

**2. The cyc batten's cells hung in the air past its ends.** Every system was spread
over ten metres whichever bar it hung on, and the batten is nine. A bar carries its
length now and a system is spread over all but the last half metre of it.

**3. The work light slider took the camera home.** The effect that builds the scene
read the view store for the pixel ratio, so a change to *any* view setting rebuilt the
renderer, the camera and the controls. Read untracked now; a separate effect keeps
both settings applied. And the range was wrong: it is 0–100% now, blackout to house
lights up, with 40% the view as it was first drawn.

**4. Going back from cue 5 to cue 1 in the Theatre left the side booms on.**
`start_cue` applied a cue's own captures and nothing else, so a parameter a later cue
had brought in stayed where that cue put it — playback had no notion that a cue is
the *stack up to it*. `Playback::take_cue` works that out from the show rather than
from memory: the latest capture of every key over the cues up to and including this
one. A capture tracked in from an earlier cue is left alone if that cue's fade or
effect is already what is driving the parameter — which is every forward Go, so
nothing that used to happen changed — and otherwise started with the taken cue's
times. A key only later cues capture goes home over the cue's down time, unless
another sequence that is on could drive it, the same exception a release makes.
Jumping *forward* over cues applies what they set on the way, which is also what a
tracking console does and also was not happening.

The choice worth stating: this is **tracking** playback, not cue-only. A cue that
names four channels is the look of every cue before it with those four changed. The
Theatre demo's blackout still has to zero everything explicitly, because tracking
never releases a key on its own; only jumping to a point in the stack before the key
was ever captured does.

### The traps

**The demo tests said the Theatre hung together, and it did — in every cue's own
captures.** No test drove a stack backwards. `a_cue_is_the_stack_up_to_it` now does,
in both directions.

```
cargo test -p pult-backend --lib playback   # the stack, both ways, and what another sequence keeps
cargo test -p pult-backend --lib demo       # the batten holds its cells
```

### 55. Four ways to draw a rig

`render-modes`, built the day it was asked for. A mode is what a screen *draws*, so
it lives beside the work light in the per-screen view store and is a row of four
buttons on the rig panel's View sheet. Nothing is rebuilt on a switch: `dress` puts
materials and visibility flags on things the scene already has, and the change costs
one frame.

- **Wireframe** — where is everything. Every truss, deck and imported mesh in one
  wire material *per panel* (the stock materials are shared by every panel on the
  page and are not edited in place), the bodies as wire in their own tint, and a
  line per fixture from the lens to where the beam lands, in its colour, never
  fading to nothing so an unlit head still says where it points.
- **Cones** — where is everything pointing. The same instanced cone under a
  six-line fragment shader: flat, alpha-blended, no haze, no attenuation, nothing
  added. A hundred cones crossing stay a hundred cones.
- **Real** — what is in the air. The beam shader, unchanged.
- **Photoreal** — what a camera would see. The scene into a half-float target with
  four samples of multisampling, bloom above white, and ACES over the *sum*, which
  is the one thing that stops crossing beams going to flat white: a blue and an
  amber added in floating point and rolled off stay a colour. This is the
  post-processing chain task 51 stayed away from, here for the one thing only it
  can do.

Measured on the Festival at 1.5×, GPU per frame: wireframe 1.1 ms, cones 3.4,
real 5.2, photoreal 8.3. The cones figure confirms `beam-overdraw`: the cheapest
possible fragment shader over the same cones is still two thirds of the beam shader's
cost, so the cost is the layers and not the arithmetic.

### The traps

**A `$state` proxy swallows a plain field.** The scene object was `$state`, and
writing `scene.mode = …` through the proxy stored the value in the proxy's own map,
where the render loop — holding the object itself — never saw it. The lights and the
pixel ratio had worked through the same proxy only because three.js instances are
class objects and escape proxying. `$state.raw` now, with the reason beside it.

**A linear frame is not a screen.** Everything written as a screen value on the
plain path — the clear colour, the grid's grey, the beams' strength — came out
brighter through the output pass, which encodes linear light to sRGB on the way
out; the first photoreal frame had a grey sky, which looked like too much bloom and
was not. The clear colour is converted, the grid takes a `uLinear` uniform, and the
beam material has a `uGain` the photoreal path sets to a half.

**Bloom at a wide radius is fog on the lens.** 0.45 strength at 0.3 radius lifted
the whole frame from its widest mip. 0.22 at 0.1, threshold 1.3: a halo round a lamp
and nothing across the sky.

```
cd frontend && npm test        # the mode survives storage and refuses one it does not know
```

### 56. Where the missing ten hertz went

`connector-frame-rate`. Task 51 measured the Art-Net connector drawing at **29 Hz**
where `Frames::DMX` asks for 40, on a frame costing 4.77 ms of a 25 ms budget, and
said that nothing yet named what was holding the rate down. Nothing in the entry's
list of suspects turned out to be it. The instrument that answered it was a probe on
the output loop that timed each arm of the select and the lateness of each wake, which
took ten minutes to write and is the only reason this was not a search.

Two causes, roughly two thirds and one third, and neither is the frame.

**The station's own health report made the engine re-push the entire rig.**
`state_version` was one counter over the whole show, bumped by every write and read by
`push_output` to decide whether the connectors needed telling. But a station writes its
own `stations` row every two seconds and its output status every second — figures about
a processor, which cannot reach a lamp — so an idle console handed its connectors an
identical patch one to two times a second. At 5000 fixtures that push costs **116 ms**
inside the output loop, rebuilding `Patch` for a rig nothing had changed, and no frame
can be drawn while it does. That is a sixth of every second.

Now there is a version **per collection**, and each consumer names the collections it
reads: `OUTPUT_COLLECTIONS` is the three `push_output` hands over, `PLAYBACK_COLLECTIONS`
is what `playback_pass` reads. `version_of` sums them, so both call sites stay a single
`u64` comparison. The list is beside the read it belongs to, and the failure mode of
forgetting to add a collection is a rig that stops updating — which is why the second
test below exists and why the fallback is *everything*: a write nobody can attribute to
a collection (a registered command, a snapshot, a showfile loaded) counts as all of them
having moved, which is what the single counter always assumed.

**And lateness was compounding into the rate.** `schedule` measured the next deadline
from `Instant::now()` at the moment the loop woke, which is the deadline *plus* however
late the wake was. Nothing wakes on time — 2.4 ms of timer granularity and scheduler
latency was the steady figure here — so every frame carried the sum of every lateness
before it, and a 25 ms period was really a 27.4 ms one. Measured from the deadline that
fired, the same 2.4 ms is jitter about a fixed rate.

**29 → 40 Hz**, exactly what `Frames::DMX` asks for, at 5000 fixtures and at 500. The
frame itself did not change and was never the problem: it is still about 4.5 ms, still
94% evaluating, still under a fifth of budget. Which also leaves `parallel-render`
exactly where task 51 left it — there is now *more* headroom, not less.

### The traps

**Chaining deadlines carries the old period with it.** The first version was
`(due + period).max(from)`, which is right until a connector changes gait: a settled DMX
line waits 800 ms between keep-alives, so the deadline chained off one is up to 800 ms
away and the first frame of a cue would arrive after the light had got where it was
going. The rule is a clamp, not a floor — never earlier than now, never more than one
period from now — which fixes the gait change and keeps the short-of-frame case, where
the chained deadline is already in the past and drawing again at once is the honest
answer.

**A version counter narrowed until it says nothing is far worse than one that says
everything.** The first is a rig that silently stops updating; the second is only slow.
So the gate is a pair, and the second half asserts that an operator taking a fader still
reaches the wire.

**The frame cost never showed any of this**, and could not have. `began.elapsed()` wraps
`plugin.send`, which is the right thing for it to measure and is why 4.77 ms was honest
all along. Everything found here was in the *gaps between* frames — a push that blocks
the loop, a deadline that drifts — and nothing that measures the work inside a frame can
see a frame that was never asked for. The Hz column was the only thing that could, and
it had been printing the answer since task 51 with nobody able to say what it meant.

```
cargo test -p pult-backend --lib engine::tests::pushing_the_rig   # what makes the engine push
cargo test -p pult-backend --lib connectors::tests               # and when the next frame goes
scripts/demo.sh --measure --release --size 5000                  # 40 Hz
```

## What is next

This document is the whole of the planning, again. The numbered tasks above are
finished work with the decisions and the traps recorded; the entries below are
what has not been started, each one carrying the questions it has to answer
before it can be built. That is the part worth keeping: an entry records what was
asked and what is true of the code today, so the questions do not get
re-discovered from scratch every time somebody picks the item up. When one is
built it becomes the next numbered task and leaves this list.

Verified against the code on 2026-09-04 unless an entry says otherwise.

### The order

Sections below are themes, not sequence. This list is the sequence, and it is the
one thing here meant to be rearranged. `→` names what has to exist first.

Reordered on 2026-09-02, into four phases with a reason each, rather than into a
ranking. **Get a real rig in, be able to see what the console is doing, measure
it, then fix what the measurement found.**

**A real rig first.** Everything downstream reads better against one. The viewer
gets a beam angle it cannot otherwise have and a plan worth drawing; the
measurement gets a rig somebody actually hung rather than a generated one; and
the 540°/270° constants task 14 complains about get real numbers. This is also
the answer to a question asked and got wrong earlier the same day: a
`default_beam_angle` on `FixtureType` does **not** free the viewer from
gdtf-import, because fixture types are built by `fixture_type_from` off a node's
port description and by the demo seed and by nothing else, and there is no
fixture type editor in the frontend, so the field would be written by nobody.

**That phase is done**, as tasks 45 and 47. A GDTF import brings the beam angle, the
travel and the geometry tree, and `stage.ts` reads the type's own range where it
has one, so the constants are the fallback rather than the answer. An MVR import
brings the rig around it: positions as transforms, trusses, layers, and meshes the
browser draws. Everything below is now measured against, or drawn from, a rig
somebody actually hung — which was the whole reason for putting this first.

**Then the console can be seen at all.** Three panels that shared one open
question — where a per-station diagnostic lives, and whether it reaches a peer.
**That question is now answered**, by task 48 and once for all three: a
diagnostic is not `ShowState`, it rides the existing `Update` broadcast on a path
of its own without going through the engine, and it reaches a peer at a *separate,
quieter threshold* that a watching console can raise as far as — and no further
than — what that peer keeps for itself. The two panels below inherit that shape
rather than re-deciding it, and the browser-reports-on-itself path the stats panel
needs already works: `log.report` is it.

**Both of those panels are now built**, as tasks 49 and 50. The System panel reads
`frame_costs` and puts the browsers beside them, and it answered the open question
the entry carried — a browser's *continuous* figures stay LOCAL to the station
serving the page, and only the exception crosses, as the `warn` line task 48's
path already carries everywhere. The wire viewer inherited both shapes without
re-deciding either, and added the one thing it had to settle for itself: a
connector describes its own traffic in *shapes*, so a new output that carries
universes gets the sheet for nothing and one that looks like nothing here adds a
component and one line in a table. **So the block task 48 opened is closed.**

**Then measure, then act on what it found** — and **both of those are now done**,
together, as task 51. Measuring first turned out to matter more than the plan
allowed for: the instrument itself was untrustworthy, two runs at 505 fixtures
disagreeing by 50%, and nothing measured with it could have been read. Fixing that
is most of why the task exists.

What it found overturned the prediction this section was ordered around. The worry
was that assembly and 59 socket writes would be the part of the frame that does not
shrink; at 5000 fixtures they are **6% of it**, and evaluating is 94%. So the
parallel-render question got its answer (rayon is the only lever) *and* its
priority (nothing is short of frame, so not yet), the viewer went imperative without
having to decide instancing at all, and the item that came out of it is one nobody
had written down: the connector draws at 29 Hz where DMX wants 40, on a frame using
a fifth of its budget.

**That last one is now task 56**, and its answer was in neither half of the frame. The
rate was held down by the station's own health report making the engine re-push the
whole rig every second — 116 ms in the output loop at 5000 fixtures, for figures that
cannot reach a lamp — and by scheduling each deadline from the moment the loop woke
rather than from the deadline it woke to, so 2.4 ms of ordinary scheduler latency
compounded into the period. 40 Hz now. Worth holding on to: **the frame cost could not
have found either**, because both live in the gaps between frames, and the Hz column
had been printing the answer since task 51 with nobody able to say what it meant.

Items 1, 3 and 4 were built together as task 51, and showfile-management and
showfile-assets-folder together as task 52. All five have left this list. What they
answered changes what is worth doing next, so the top of it is now:

1. **universe-routing** — `OutputConfig::universes` says it is a filter and no
   connector reads it. A decision rather than a feature: either `carries` gates
   the send, or the field stops claiming to. → none, and the wire viewer is now
   where an operator sees it
2. **parallel-render** — rayon over fixtures inside a connector's frame. Task 51
   measured evaluating at **94%** of an output frame at 5000 fixtures, which is
   the answer the question was waiting for, and `pult-render` is pure and takes
   no locks. What the same measurement also says is that it is **not urgent**: the
   frame is at 19% of budget, and task 56 gave it *more* headroom rather than
   less. → none, and it should not be done until something is actually short of
   frame
3. **typed-plugin-sdk** — codegen into `plugins/sdk` from the same inventory the
   frontend proxy comes from; the wire stays generic. → none
4. **camera-home-presets** — front, plan, section, three-quarter, and
   focus-on-selection. The smallest of everything here and the one an operator
   reaches for most often. Added 2026-09-03. → none: task 51's viewer owns its
   own camera already
5. **scene-editing** — and specifically a picker for task 52's stock catalogue
   first, which is smaller than a gizmo and is what a console that has never
   imported an MVR needs in order to have a room at all. → none
6. **paperwork-export** — patch lists, cue sheets, rider paperwork. A read-only
   plugin over introspection, which is what introspection is for. → none, and
   much better now that gdtf-import has landed and put a real patch in the show
7. **3d-programmer-remainder** — blind, highlight, fan, and modifiers that are
   themselves dynamic. → none: the viewer landed as task 51
8. **voice-input** — speech to the command line, grammar first and NL on parse
   failure. → none
9. **nl-show-context** — what relative syntax cannot reach, and whether it is
   worth the permission it costs. → voice-input, which is what shows which
   utterances actually arrive
10. **control-transports** — MIDI and OSC as ports, in and out, with nothing
   above them decided. Was open-control-interfaces until 2026-09-02, when the
   three things people send over those ports turned out to want separate
   entries. → none
11. **timecode-workflow** — waveform and beat-grid timecode, timed playback,
   audio import. The biggest item here and the one the spec is most opinionated
   about. → none technically
12. **llm-cost-overview** — token and cost accounting out of the NL plugin.
   → none
13. **openhaunt-as-plugin** — output connectors as WASM, if a connector's own
   frame rate survives the boundary. → the benchmarks from tasks 43 and 44 and
   from task 51, which measured a connector's frame at 4.77 ms for 5000 fixtures —
   the number a WASM boundary now has to be compared against, and task 56, which
   says the boundary has to survive being asked 40 times a second and not 29
14. **video-mapping-ndi** — NDI output. Scope carefully, it hides a media server.
   → openhaunt-as-plugin, as the first proof the plugin API carries heavy output
15. **plugin-language-hosts** — TS plugins, via a host plugin or as components.
   → a real TS plugin wanting to exist
16. **show-control** — MSC in and out, and MIDI and OSC as plain triggers. A
   stage manager's Go arriving at the lights, and this console sending its own
   to sound and video. → control-transports
17. **surface-layer** — a bound physical thing, which is what the transports are
   not: one event type under every surface, plus the two questions (where a
   headless surface's selection lives, where a fader's gesture begins and ends)
   that decide whether any of the three below is a week or a month.
   → control-transports for the MIDI half, nothing for the USB half
18. **midi-surfaces** — documented, and the hardware costs fifty pounds, so this
   is what proves the layer before anybody spends a weekend on USB captures.
   → surface-layer
19. **makepro-x** — MakePro X hardware. Blocked on naming what it speaks before
   it can be estimated at all. → surface-layer
20. **ma3-command-wing** — a grandMA3 command wing over USB, protocol
   undocumented and to be read off the device. → surface-layer, and
   midi-surfaces for the binding model
21. **showfile-migrations** — so a show made in the beta still opens after it.
   Added 2026-09-03, and the trigger is the beta rather than anything in the
   code: until somebody is carrying real work in a showfile, refusing one from
   another generation by name is the better trade. → the first beta
22. **plugins-that-travel** — the gaps in a mechanism that mostly exists. Added
   2026-09-03. → none, and the sharpest question in it is whether an imported
   `.pultz` should ask before running the plugins it carries

Items 16 to 20 were added on 2026-09-02 and sit at the end rather than being
placed, because three of them are blocked on hardware being in the room and not
on anything in this repository. Any of those can move up the day the hardware is
on the desk. **show-control is the exception and the one with a case for moving
up now**: it needs a MIDI port and nothing else, and a stage manager pressing Go
is a more common way for this console to be driven than any surface in the list.

**render-modes** was added on 2026-09-04 out of the user's second look at task 53
and built the same day as task 55. What photoreal still wants — a floor lit by every
beam, bodies lit by their own beam, haze that thins with height — is listed under
`photoreal-remainder` below rather than left in the task.

**beam-overdraw** was added on 2026-09-04 out of task 53's measurement and is not
placed either: it is the answer to "why is the GPU busy" with the levers named, and
it becomes worth doing the day a rig view is short of frame on a machine somebody is
actually running a show from. Task 53 already took the free wins — sixty a second at
most, nothing drawn when nothing changed, only lit beams drawn — which is what turned
a pinned GPU into an idle one on a dark stage.

Items 21 and 22 were added on 2026-09-03 and sit at the end for a different reason:
neither is blocked on anything, and both are blocked on *time*. A migration path is
worth nothing until there is a showfile worth migrating, and the gaps in how plugins
travel are gaps rather than absences.

The `<T.SpotLight>` recompiling every material as a fade crossed 1% used to be
called out here as the one thing not belonging to any phase. It is gone, and not by
being fixed: task 51 removed the declarative layer that made it possible, so there is
no light count for a fade to change. The reactive-`args` geometry rebuild beside it
went the same way, which is what that entry predicted for one of them and not the
other.

### Plugins

#### typed-plugin-sdk

Introspection is the right wire and a poor thing to program against. A plugin
learns the schema from `introspection::entities()` as JSON and navigates it by
hand, with no types, no compile-time field names, and stringly-typed paths
(`&["cues", id, "fade_time"]`). Every plugin author pays for that.

The fix is codegen into the **SDK**, not the WIT. `pult-codegen` already
generates `frontend/src/lib/ws/data.ts` from the `EntityMeta` inventory, giving
the frontend `data.sequences[5].cues[3].fadeTime.set(4)` while the WebSocket wire
stays generic path-plus-JSON. `plugins/sdk` can have that same split, from the
same inventory and the same tool: `sdk::data::cues().nth(3).fade_time().set(4.0)`
over an unchanged `data.set(path, json)`.

- Codegen'ing the WIT itself is ruled out, and task 35 records why. A
  component's imports are stamped with the package version, and a record type's
  fields are part of every signature using it, so `Cue` gaining a field would be
  a breaking ABI change. The schema changes daily and a show now carries its
  plugins between machines, so a bundle built against schema-of-Tuesday would
  refuse to load on a station from schema-of-Wednesday. Today no plugin notices
  the schema growing, and that property is worth keeping.
- Introspection stays. It answers the runtime question, which is what *this*
  station has including collections the SDK never heard of. A command-line plugin
  building its grammar and a sync plugin walking unknown tables both need that.
  Typed codegen is for what is known at build time.
- How does the SDK version relate to the station's? A plugin built against a
  newer SDK writing a field an older station lacks gets a per-path runtime error,
  the same graceful failure the frontend already has. Worth confirming the
  message is good.
- Where does the generated code live, checked in beside `sdk/src/lib.rs` or
  emitted into `OUT_DIR` by a build script? Checked in matches how the frontend
  does it and keeps the plugins workspace buildable without the console's.

#### openhaunt-as-plugin

Should the OpenHaunt output path, and Art-Net and sACN with it, be WASM plugins
rather than built-in connectors? The spec calls output a plugin layer already;
today `OutputPlugin` is a Rust trait inside the backend.

- Measure the 40 Hz output path through a WASM boundary before deciding. Tasks
  43 and 44 have the numbers to compare against, and they are not the numbers
  this question was first asked against: a connector's cost is now its own frame,
  2.86 ms for 2005 fixtures, not a share of a tick.
- Discovery over mDNS and the embedded MQTT broker are harder to host in a guest
  than frame emission is.
- Middle ground: keep the connectors native and expose the same registration to
  plugins, so a *new* protocol can be a plugin without moving the built-ins.

#### plugin-language-hosts

A plugin that hosts other plugins in another language, say a TypeScript host
embedding a JS runtime so plugins can be written in TS on top of it.

- Does the host API, meaning permissions, introspection and surfaces, pass
  through cleanly, or does the host become a second plugin API that drifts?
- Alternative: componentize JS directly with jco or StarlingMonkey, so a TS
  plugin is a component and no host plugin exists.
- Defer until a real TS plugin wants to exist.

### Natural language and voice

#### voice-input

Voice as an input path to the command line. The question that shapes it: an
utterance may already be valid command-line syntax, and with the NL plugin
installed it would go to the LLM anyway, costing money and latency for nothing.
Without the NL plugin it should parse directly.

- Route: try the grammar first and fall back to NL only on parse failure?
- Where speech-to-text runs, in the browser through the Web Speech API, on the
  station, or in a plugin.
- Push-to-talk against a wake word, and confirmation before destructive commands.

#### nl-show-context

"A bit darker" needs the current value and the NL plugin has none.
`plugins/natural-language-control/pult-plugin.toml` grants no data access
(`commands = false`) by design; everything goes through the command line.

Task 39 answered most of this. `at +10` and `at -10` are command-line syntax, so
"a bit darker" is an utterance the plugin can answer with no show data and no new
permission, and the one grammar and one audit trail survive. What is left is the
part relative syntax cannot reach, "make it look like the second verse", which
needs the show, and whether that is worth the safety story it costs. The
alternative is still read access with state in the prompt, which weakens the
story and grows the prompt with the rig.

#### llm-cost-overview

Token and cost accounting for the NL plugin, visible over the REST API.

- Where it is measured. The plugin sees the usage fields on each response; the
  host sees only bytes. So the plugin reports, into LOCAL state, and a
  `GET /api/...` beside `/api/config` and a panel read the same numbers.
- Per session, per show, or per station? And cost tables per provider and model
  live where, kept up to date by whom?

### Programming model

#### fade-curves

Asked by the user on 2026-09-04: a cue's fade needs a timing curve, and most of all
for position — a head that eases into a mark reads as a move, and one that runs
linear and stops dead reads as a mistake.

What is true today: `ParameterCapture::easing` exists with `Step`, `Linear`,
`EaseIn`, `EaseOut` and `EaseInOut`; the evaluator honours it, so it reaches the
lamps and the screen alike; the programmer's store menu sets it per store and
defaults to `Linear`; the cue editor does not show it; and every demo writes
`Linear`. So the mechanism is there and nothing puts a curve into a cue except by
hand at store time.

- **Where the default lives.** A cue-level curve that captures inherit, the way a
  cue's fade times work — and a per-*kind* default under that, because the answer
  for intensity (linear, which is what dimmers have always done) is not the answer
  for pan and tilt (ease in and out). Probably `Show`-level defaults per kind, seeded
  from a preference, the way `home_fade_ms` is.
- **What shape.** The five named curves may be enough; if not, one number — the
  strength of an S-curve — beats a bezier editor nobody will use during a show.
- **Split with the split fade.** A cue already fades up and down at different times;
  a curve per direction is the same question, and probably the same answer: one
  curve, both ways, unless somebody asks.
- **The cue editor** has to show it, per cue and per capture, beside the times.
- **The demos** should use it: the Club's `Centre` and the Festival's `Fan` are
  position cues and the first place a linear move looks wrong.

#### 3d-programmer-remainder

What is left of the spec's §Programming once the rig view (task 13), programming
in it (task 14) and effects over a selection (task 25) are done: **blind**,
**highlight** and **fan**, and modifiers that are themselves dynamic, meaning an
effect whose rate is an effect.

- Blind wants a second programmer buffer that does not reach the output. Is that
  a second `programmer_values` collection or a flag on the existing one? It is
  SYNCED either way, so two operators can be blind separately or not at all,
  and which of those is right is the decision.
- Highlight is a temporary output override for the selection, the same shape as
  home (`__home`) pointed the other way. Reuse that machinery or not?
- Fan needs an order over the selection, and `SelectionQuery` now carries one
  from task 38. Does fan reuse it, and what does fanning an unordered selection
  mean?
- A dynamic modifier is a graph rather than a value, and nothing in the schema is
  recursive yet.

### Visualisation

#### rig-viewer-remainder

What task 51's viewer rewrite deliberately did not do. The beam, the haze, the strobe,
the infinite grid, the gizmo `depthTest` and the camera-transition cancel all landed;
these did not.

- **Instancing the fixture bodies.** The beams are one `InstancedMesh`; the bodies are
  individual meshes reused between frames. Task 51 measured evaluating at 94% of the
  *station's* frame and left the browser's own body-drawing cost unasked, because
  going imperative bought the option without needing to spend it. **Task 53 asked
  it**, on the 176-fixture Festival: the bodies, the trusses, the grid, the deck and
  the pool light together are under half a millisecond of GPU per frame and about a
  millisecond of CPU. Not worth instancing at this size; ask again at 5000, in a
  headed browser (see the task's traps).
- **Picking, if the bodies are instanced.** The raycast is against per-fixture objects
  today. Instanced picking means either an id buffer or a manual intersection, and it
  is the reason instancing is not free.
- **A `SpotLight` per fixture.** There is one, following the brightest, because a scene
  with thousands of real lights does not render. Whether the floor pools should instead
  be part of the beam shader is open.
- **Placement.** Moving a fixture or a truss in 3D is `scene-editing`, not this: it
  changes the show rather than the picture, so it needs gestures, snap and multi-select.

#### photoreal-remainder

What task 55's photoreal mode does not yet do, in the order it would pay:

- **A floor lit by every beam**, not by one spotlight following the brightest. Either
  a dozen real `SpotLight`s reused for the brightest dozen — never mounted or
  unmounted, task 51's rule — or a projected disc per beam drawn into the floor as a
  second additive layer, which scales the way the beams do. The second is the one
  that survives a five-thousand-fixture rig.
- **Bodies and trusses lit by the beams near them.** A head's lens already glows
  with its colour; the truss above it catching light does not.
- **Haze that thins with height and with distance** from the machine. A per-show
  gradient, cheap once the density is a real quantity — which task 54 made it.
- **Exposure** as a view setting beside the work light, once there is a reason to
  look at a frame darker or brighter than ACES at one.
- Shadows are not on this list. Nothing in a rig view casts one anybody looks at.

#### beam-overdraw

What task 53's measurement found and did not fix. The beams cost about 5 ms of GPU
per frame on the Festival, and that figure is **not per pixel, per triangle or per
instruction** — rendering at 1× and 2× differs by a quarter, the haze octaves and the
cone's triangle count make no difference at all — but it scales with the number of
lit beams and halves when they are drawn single-sided and unblended. That is a
tile-based GPU serialising the blended layers stacked on one tile: the cost is the
*depth* of overlapping cones over the same pixels, and the levers are only the ones
that reduce it.

- **Frustum-cull per instance.** Every lit beam is drawn wherever the camera looks. A
  cone entirely off screen costs nothing, but one that is *behind* the camera and
  reaches past it does not, and neither does the 40 m run a shallow beam is drawn to.
- **Shorten what is drawn.** `drawnLength` runs a cone on until its whole end ring is
  under the deck, which for a beam aimed at the crowd is up to 40 m of tube covering
  the whole screen. A cap per beam angle, or a fade that ends earlier for a wide one,
  is fewer layers on every pixel.
- **Front faces only, if the back wall can be faked.** Single-sided halved the cost
  in the measurement, and the back wall of the tube is what makes the core bright.
  Whether a term in the shader can stand in for it is a picture question.
- **Not resolution, and not the noise.** Both were the obvious suspects and both were
  measured out. The `View` sheet's resolution setting is still worth having for a
  full-screen Retina display, where the tile count does finally exceed the cores.
- Measure in a *headed* browser only: a hidden tab has no frames, and headless
  Chromium's GPU is software.

#### camera-home-presets

Where the rig view looks from. Today the camera starts wherever `Rig3D.svelte` puts it
and an operator flies back by hand every time they open the panel — on a tablet, with
`camera-controls`, that is a slow way to answer "what does the front look like".

- **The obvious four are free and cost no schema at all**: front of house, plan,
  section, and a three-quarter. Buttons that animate the existing controls to a
  computed position, framing the rig's own bounding box so they work on a five-fixture
  demo and a two-hundred-head festival alike.
- **A *saved* preset is the question.** Whose is it? A camera position is one operator
  looking at one screen, which argues for `localStorage` beside the layout. But "the
  designer's view" is a thing a team shares, which argues for show data beside
  `layouts` — and layouts are already the precedent for exactly that split: the
  arrangement is the show's, which one this browser is looking at is not.
- **Two `rig` tiles can be open at once**, each with its own renderer and controls, so
  a preset is applied to *a* view rather than to the panel. Whatever holds them cannot
  assume one camera.
- Focus-on-selection is the same machinery and probably the more used of the two: frame
  what is selected rather than the whole rig.
- Not a `stage_plans` question. That is the flat view, and it has its own framing.

### Showfiles

#### showfile-migrations

Showfiles are **not** migrated, and `SCHEMA_GENERATION` refuses one from another
generation by name rather than panicking somewhere deep in a generated
`from_columns`. That is the right trade while the console is in development and
nobody is carrying a season's work in one. It stops being the right trade the day
somebody does — so this is the entry for that day, and the trigger is the first beta.

- **What the refusal already gets right, and must keep.** Two things fail differently.
  A *shape* change inside a JSON column is invisible to the columns and silently reads
  as `None`, which only a stamp catches. A non-`Option` field added later leaves NULL
  in every existing row and panics on open, which the file says itself and which
  `a_required_column_nothing_filled_in` names. A migration path has to answer both, not
  replace the check with hope.
- **`add_missing_columns` is already half of it** — adding a field is free today. What
  it cannot do is *fill one in*, and that is precisely what a migration is: a default
  per added field, and a rewrite per changed shape.
- **The version to migrate from is already written down.** `PRAGMA user_version` is the
  stamp, so the shape is a chain of `2 → 3 → 4` steps rather than a matrix.
- **A version snapshot is a showfile too**, and there may be fifty of them in a bundle.
  Migrating on open means migrating one file; restoring an old snapshot means migrating
  another, later, on a copy that is about to become the show. Decide whether a restore
  migrates or refuses.
- **Test it against real files, not synthetic ones.** The corpus that would make this
  trustworthy is a `.pultz` per generation, checked in, opened by CI. Start collecting
  them *before* the beta rather than reconstructing them afterwards.
- What it is not: a promise that every future shape is reachable. A generation that
  cannot be migrated should still refuse by name.

#### plugins-that-travel

A show already carries its plugins — `plugin_packages` is a PERSISTED roster naming
each bundle by sha256, the bytes live in the asset store, and a station that lacks one
fetches it from a peer and verifies it (task 41). Task 52's `.pultz` export carries
`assets/`, so the bytes go with the show now as well. So the *mechanism* exists, and
what is left is the gaps in it — which is why this is an entry rather than a feature.

- **A `--plugins` directory beats the show and is not in the roster.** That is the dev
  loop working as intended, and it means a plugin somebody is developing does not
  travel with the showfile they are developing it against. Whether "install what I am
  running into this show" should be a button is the question.
- **An export does not know which assets the roster needs.** It copies the whole
  `assets/` directory, which is correct and indiscriminate; a show whose plugin was
  removed still carries its bundle. Related to whether anything ever garbage-collects
  the store, which nothing does.
- **Opening a showfile runs its plugins**, bounded by the sandbox and the manifest
  permissions and nothing else. That is already a deliberate decision, and it gets
  sharper the moment showfiles are things people email each other. Whether an imported
  `.pultz` should ask before running what it carries is the real question here.
- **Station-scoped plugin data deliberately does not travel** — it is beside
  `preferences.toml`, and credentials live there. A plugin that arrives on a new
  machine comes up unconfigured, which is right and should be *said* somewhere.
- The `api` floor already handles the version half: a bundle records what it was built
  against, and `scripts/check-api-compat.sh` proves an older one still runs.

### Interop

#### gdtf-share-panel-polish

Task 45 landed GDTF in and out, the Share behind a station credential, and a modes
table in the Fixture Types panel. What it left is the part that needs somebody to use
it against real files for a week.

- The Share's list has no manufacturer facet in the UI, only a text match across both
  names. The backend already filters by manufacturer; the panel does not offer it.
- A gobo wheel's slot names come across and its **images do not**: the archive carries
  them and nothing extracts them into the asset store, because there is nothing drawing
  a gobo yet. Task 47 built that pipeline — the asset store, `named_assets`, and a
  browser that loads what is in it — so this is now a matter of extracting the images
  on import and drawing them.
- Fine channels are folded into their coarse one by `MainAttribute`, which is right for
  every file that sets it. A file that does not is read as two parameters, one of which
  does almost nothing. Worth a heuristic on the attribute name once there is a corpus
  file that needs it.
- `ChannelFunction`'s `ModeMaster` is modelled and not acted on. A shutter's strobe rate
  is a parameter of its own here rather than a range that only exists while the shutter
  channel is in its strobe band, which is a simplification an operator will eventually
  find.

#### scene-editing

The plan and rig views become an editor, in the spirit of Vectorworks: move and rotate
trusses, fixtures and objects, parent a fixture to a truss, show, hide and lock layers,
duplicate, snap to a grid, and place primitives and symbols. Task 47 built the
entities to edit and the views that draw them, so this is now unblocked.

- The gizmo pattern in `Rig3D.svelte` already does pan and tilt handles; move and rotate
  are the same shape with a different write.
- A drag has to be one Ctrl-Z, which the gesture machinery already does.
- Articulated fixture bodies are the visible payoff and are nearly free now:
  `FixtureType::geometry` carries the parts, which turn, and the beam angle, so
  `Rig3D.svelte` can lose its 0.12 constant and its single box per fixture.
- **A picker for the stock catalogue is the smallest useful start.** Task 52 added
  `SceneObject::catalogue` and the pieces to draw — F34 in three lengths and a corner,
  decks, wall panels and flats — and the demos build rigs out of them. What is missing
  is any way for a *person* to: there is no scene-object editor at all, so objects
  arrive by MVR import and nothing else. A list of pieces and a click to place one is
  a smaller thing than a gizmo and unblocks a console that has never imported anything.
- A truss *run* is the unit an operator thinks in, not a section: task 52's demos build
  one as a `Group` with sections parented to it, which is the shape a picker should
  make too.

#### mvr-xchange

The other half of MVR: a protocol for two consoles or a console and a previz to share a
scene as it changes, rather than a file somebody exports. Out of scope while there is no
scene to share.

- It is mDNS discovery and a WebSocket, which this codebase has both of.
- The open question is whether a shared scene is a *session* in this console's sense or
  something beside one. Two stations of one show already agree about the rig; an
  xchange peer is somebody else's software that agrees about part of it.

#### paperwork-export

Patch lists, cue sheets, rider paperwork.

- A read-only report is an ideal plugin, since introspection already exposes all
  the data. The open part is print CSS against generating a PDF.

#### open-control-interfaces

Dissolved on 2026-09-02 into *External control* at the end of this document, and
kept here as a heading so a reader looking for it lands somewhere. It had OSC,
MIDI and control surfaces in one entry, on the assumption that they were one
piece of work. They are three: a transport, show control over it, and a bound
piece of hardware that is a different thing again. Its two questions survive, the
address-mapping one under show-control and the connector-or-plugin one under
control-transports.

### Outputs

#### universe-routing

**`OutputConfig::universes` documents itself as "which universes to send", and no
connector has ever read it.** `carries()` is the predicate for the field and
**only `OutputCoverage::of` calls it**: the connectors render every universe in the
patch and put on the wire every one the dedup has not settled. So an output
restricted to universe 1 transmits all seven, and the Outputs panel's coverage
warnings describe a routing nobody implements.

Found by task 49's throughput figure — an sACN output configured for one universe
reporting two and a half universes' worth of packet per frame — and left there
deliberately, because changing which universes reach a wire is a change to what
reaches lamps and wants its own decision rather than a drive-by fix inside an
instrument.

- **The decision is which way to make it honest**, and they are not the same size.
  Gating the send in `render` (or in each connector's loop over its universes)
  makes the field mean what it says, and makes a two-output split — this Art-Net
  node carries 1–4, that one carries 5–8 — work, which is an ordinary way to
  build a rig and currently does not work. Deleting the field is a schema change
  and admits that every output carries everything.
- **`OutputCoverage` already believes the first answer**, which is the argument for
  it: the gap warnings, the "Add sACN output for universe 1" button and the panel's
  whole model of coverage are written against a filter that exists. Deleting the
  field means rewriting all of that to say something weaker.
- **The trap is the dedup cache.** `UniverseCache` is per connector and keyed by
  universe number, so filtering at the send is safe; filtering earlier, in `render`,
  would change what the *evaluation* costs per output and make two outputs on one
  station render the patch twice. Task 43's split says which half that lands in.
- **Task 50 made it visible**, which is what an instrument is for: the wire viewer's
  universe chips list what a connector is actually carrying, so an output restricted
  to one universe now shows every universe in the show to anybody who looks.

### Observability

#### station-clock-offset

**Two stations do not agree on what time it is, and every fade is anchored in an
absolute millisecond.** Found while building task 48, whose merged log needed to
interleave two stations' lines and could only do it to within their skew — but the
log is the harmless version of this. The load-bearing one is that
`live_fades`, `live_effects` and a cue's `went_at` are all anchored in *unix*
milliseconds, and `Sequence::off`, the browser and every connector evaluate them
against their own `now_ms()`.

`types/sequence.rs` says "two stations still agree because they agree on the anchors
they replicate, not on their clocks". That sentence is only true if the clocks agree:
`now_ms()` is the wall clock read once at first use plus elapsed, so **station B
evaluating station A's fade runs it out by exactly B's skew from A**. Silently, and
each individual value looks plausible — which is the same failure
`frontend/src/lib/ws/clock.ts` exists to prevent in the browser, between stations,
unaddressed. A show LAN with no route to the internet is normal for an isolated
Art-Net network, and nothing is disciplining those clocks at all.

**The open question is PTP against the estimator already in the building**, and it
was left open deliberately on 2026-09-02 rather than guessed at.

- **The RTT estimator.** `infra/sync/peer.rs`'s heartbeats already measure the round
  trip to each peer (`Outstanding::answered`), which is exactly the input
  `clock.ts` uses. A per-peer offset out of that is single-digit milliseconds on a
  LAN, needs no daemon, no privilege and no per-platform story, and would reuse an
  estimator this repo has already written once and holds to a corpus.
- **PTP (IEEE 1588).** Tens of microseconds in software, sub-microsecond with
  hardware timestamping. The complication is that **a console cannot steer the OS
  clock without privilege**, so what it would actually do with PTP's answer is hold
  an offset and apply it in software — the same *shape* as the estimator, at much
  higher cost. `ptp4l` is Linux-and-root; macOS has no general daemon (its PTP lives
  inside the AVB audio stack); Windows client support is thin.
- **So the question is what needs the precision**, and the honest answer may be
  "nothing here". An output frame is 25 ms and a fade is seconds. Where PTP earns its
  keep is sub-millisecond determinism against *other departments* — SMPTE, audio,
  video frame alignment — which is timecode-workflow, and that entry already names
  the OpenHaunt clock topic as its prior art. If this is decided for the estimator,
  say so there too, because that is the item that will want to reopen it.
- **What it costs to be wrong is asymmetric.** The estimator is a week and can be
  replaced; PTP is a dependency and a deployment story. Doing the estimator first
  does not foreclose PTP, and it makes the size of the problem visible: publish the
  measured offset per peer in the `stations` row beside `cpu_percent` and
  `frame_costs`, and a rig will say how bad its own skew actually is.
- **And the same rule as `clock.ts`: say nothing until you have one.** A station that
  has not yet estimated an offset must not apply a plausible wrong number; it should
  be visibly without one, the way `consoleNow()` answers `null` and panels show a gap.
- Task 48's merged log is **already correct whatever this decides** — a line carries
  its own station's `seq` and clock, deduping is exact, and only the cross-station
  interleave is approximate. It gets better for free the day an offset exists.

### Performance

#### parallel-render

Rayon over fixtures inside a connector's frame. Answered "no" twice before, and both
answers were against a world that no longer exists: task 29 refused it at 0.07 ms of a
35 ms tick, and there is no tick.

Task 51 is the measurement it was waiting for. Evaluating is **94% of an output frame**
at 5000 fixtures — 4.50 ms of 4.77 — and it is the only part of the frame with anything
under it, since assembly and the socket together are 6%. `pult-render` is pure, takes no
locks and touches no OS, so it is embarrassingly parallel across fixtures, and a
connector's thread is already off the engine, so this costs nothing architecturally.

**And the same measurement says not yet.** 4.77 ms is 19% of the budget. Nothing is
short of frame, and a rig that is not short of frame does not need its evaluator
parallelised — it needed whatever was holding the rate down, which task 56 found and
fixed without touching the frame at all. So the headroom is larger now, not smaller:
the same 4.5 ms frame, asked for 40 times a second instead of 29.

- Worth doing the day something is actually short of frame, and worth *not* doing
  before then.
- Evaluating came out **sublinear** in the rig between 505 and 5000 fixtures — ten
  times the rig for six and a half times the cost — so the extrapolation past 5000 is
  not the obvious one either. Measure again before assuming.

#### Partitioning computation across stations

The one part of the old multithreading item that nothing has answered.
Parallelising the render was answered with **no** (there was nothing there to
parallelise, and task 44 removed the field the other proposal wanted cheaper
writes for), but splitting the work of a show between consoles is task 10's
question and also the redundancy one. Worth asking again only when there is a
workload a single station cannot carry, and the numbers for that will be
different from any measured so far.

### Media and time

#### timecode-workflow

The big one, and the spec is opinionated about it: waveform and beat-grid
timecode, plus "timecode without timecode" meaning timed cue playback, with audio
import and playback. `FollowMode::Timecode` has existed unimplemented since task
3, deliberately waiting for this design rather than getting a stopgap. It should
subsume that follow mode rather than sit beside it.

- Audio import lands in the asset store. Playback happens on which station, and
  what do the others chase? The OpenHaunt clock topic and `went_at` anchoring are
  the prior art for shared time.
- Beat grids relate to speed masters; tap tempo is a degenerate beat grid.
- Is external SMPTE or MTC in scope, or explicitly out?

#### video-mapping-ndi

NDI output for video mapping. Almost certainly a plugin, which would be the first
real test of whether the plugin API can carry a heavy output, or else a sibling
connector.

- Frames come from where, a pixel-mapped fixture array rendered by the engine, or
  media playback? Scope this carefully. It hides a media server.

### External control

Everything that drives this console from outside a browser, and everything it
drives from outside itself. Added on 2026-09-02 out of a question about one
piece of hardware, and restructured the same day after the first draft got the
layering wrong: it filed MIDI and OSC under control surfaces, when a surface is
only one of the things people send over those ports.

Three layers, and the entries below are in that order. A **transport** is a
port: bytes and messages in and out with nothing above them decided. **Show
control** is a message meaning an act, in either direction, with no hardware
identity behind it. A **surface** is a bound physical object with state,
feedback and a learn mode, and it is the highest of the three. A MIDI controller
is a surface reached over a transport; an MSC Go from a stage manager's desk
arrives over the same transport and is not a surface at all.

#### control-transports

MIDI and OSC as ports. Nothing above them decided here on purpose.

- **A port belongs to a station, what arrives over it belongs to the show.** A
  MIDI interface is plugged into one machine, so the port list is LOCAL, in the
  shape `infra/devices/mod.rs` already uses: an actor owning a piece of LOCAL
  state and pushing it to the engine whenever it changes. What arrives is a Go,
  and a Go is SYNCED. That split is the same one a fixture connector makes, and
  it is the only part of this that is already answered.
- **Which station listens, when a session has several?** MIDI decides itself,
  since whoever has the cable has it. OSC does not: every station could bind
  8000, and then two consoles take the same Go from one packet. Worth deciding
  here rather than discovering it in a tech.
- **Outbound is half of this and the half that gets forgotten.** A transport
  that only receives makes show-control's second half impossible to write later
  without reopening this item.
- `midir` for MIDI ports, `rosc` for OSC. Neither is a large dependency.
- **Native or plugin?** Inherited from open-control-interfaces, and this is the
  cheapest place in the repository to answer it: a UDP socket and a string is
  close to free either way, so measuring a plugin on the input path here costs
  almost nothing and tells openhaunt-as-plugin something it wants to know.

#### show-control

Triggering this console from other equipment, and triggering other equipment
from it. MSC, plus plain MIDI notes and control changes and OSC addresses used
as triggers. Both directions.

This is the item with a real theatre behind it rather than a desk toy: the SM
presses Go on a prompt desk, or QLab does, and the lights take it.

**Inbound MSC is a SysEx frame this schema cannot currently express.** The frame
is `F0 7F <device id> 02 <command format> <command> <data> F7`, command format
`0x01` for lighting, and the cue argument is ASCII with `0x00` between cue
number, cue list and cue path. Check the byte-level detail against MMA RP-002
before implementing rather than against this paragraph. Two problems fall
straight out of it.

- **A cue has a number and a sequence does not.** `Cue::number` is a fractional
  f64 (`types/cue.rs:52`), which is exactly the shape MSC addresses. `Sequence`
  has `id` and `name` and nothing an SM would write down as a list number. So
  either sequences gain a number, which is a schema change with an ordering
  question behind it, or the binding table maps a string to a sequence uuid by
  hand. The second changes no data model for the sake of one protocol, and is
  the coward's answer that is probably right.
- **And `go_to_cue` takes a uuid.** Its signature is `{ cueId: string, at?:
  number }` (`types/sequence.rs:66`), so no inbound MSC GO can be spelled with
  the commands that exist today. Either a command that takes a number, or the
  lookup happens in whatever receives the message. Where that lookup lives is
  the same question as where a group name gets resolved, and should get the same
  answer.
- **GO, STOP, RESUME, TIMED_GO, ALL_OFF.** GO is `go_next` and `go_to_cue`, and
  ALL_OFF is `Sequence::off` across every sequence, which task 43 built along
  with the release fade and `Show::home_fade_ms`, so that one arrives free. STOP
  and RESUME have no equivalent and cannot get one cheaply: a fade here is a
  function of time rather than a thing being stepped, so pausing one means
  re-anchoring it, and the whole model says nothing keeps what a parameter is
  doing. That is a reason to answer "out of scope" out loud rather than leave it
  looking like an oversight.
- **An external trigger has no operator, and the oplog already has a view on
  that.** `Operation::is_undoable` requires `user_id.is_some()`
  (`events/operation.rs:196`), so an unattributed write is not undoable and not
  in the History panel, which is the same mechanism plugin stores use. A Go from
  the SM's desk probably wants to be exactly that: it happened, the log of the
  show records it, and no operator can Ctrl-Z the stage manager. Decide it
  deliberately, because minting a user for the prompt desk is also defensible
  and much harder to reverse later.
- **Outbound has nowhere in the show to hang.** Nothing says "when this cue
  goes, send this". `FollowMode` is the nearest thing and it is about this
  console's own next cue. So it is either a field on the cue, which is simple
  and puts a MIDI string in the middle of a lighting cue, or a table of rules
  keyed by event, which is one more place to look when a trigger does not fire.
  The rule table, on balance: a rig that talks to sound, video and a fog machine
  wants all of that visible together, and half of those rules will not be about
  cues at all.
- **All-call is device id `0x7F` and answering it is a choice.** A console that
  responds to every device id will take another department's Go. That is a
  station preference, not a constant.
- **What does an OSC address map onto?** Inherited from open-control-interfaces
  and still open: the path API directly (`/pult/sequences/3/go`), or the command
  line, which already has one grammar and one audit trail. It should get the
  same answer as surface-layer's question about a key press.

#### surface-layer

One event type under every control surface, and the reason the three devices
below are entries rather than one item each carrying the same questions.

**A surface is input, so `OutputPlugin` is the wrong trait.** That one is patch
to wire at a frame rate (`infra/connectors/mod.rs:97`) and has nothing a key
press fits into. Note what this layer is *not*: it sits above control-transports
and above USB, and it exists for the things a raw message does not have, which
are identity, binding, feedback and a learn mode. An MSC Go needs none of those,
which is why show-control is a separate item and not a client of this one.

- **One event type, decided before any device is decoded.** Key down and up,
  fader moved to an absolute position, encoder turned by a delta, wheel spun.
  Every decoder produces it and every binding consumes it. Skip it and the wing
  becomes a special case, and MIDI re-argues all of it afterwards.
- **Where does a key press go?** The command line is already a grammar and
  already an audit trail, and an MA-style keypad spells command-line syntax with
  its legends. Routing keys through `exec` gets one implementation and one
  history. Faders and encoders cannot go that way at a few hundred events a
  second.
- **An encoder is a `__by`, not a value.** The delta verb exists
  (`engine/mod.rs:1480`), resolves at the station above the oplog, and an
  encoder has no absolute position to send in the first place. A fader is the
  one control that genuinely wants an absolute write.
- **The hard one: a headless surface has no selection.** `selection_of(ctx)`
  (`plugins/command-line/src/lib.rs:676`) reads the selection out of the
  caller's context, and the browser fills that from a Svelte store, because a
  selection is one operator's. A wing on a desk has no store and no browser.
  Either the station starts keeping a selection per surface, which makes a
  surface a kind of operator, or a surface binds to a browser session and is
  dead without one. This decision is most of the work in the whole section, and
  every device below waits on it.
- **And no gesture boundaries.** `frontend/src/lib/stores/gesture.ts` is blunt
  about it: only the client knows where an act begins and ends, because the
  backend sees a stream of writes and no guessing at the gaps would tell a drag
  from two quick edits. A physical fader has no pointer-down to begin one. So a
  surface has to invent the boundaries from when motion stops, with a tail like
  the store's `TAIL_MS`, or every fader move costs a few hundred presses of
  Ctrl-Z to take back.
- **Learn mode splits across two lifecycles.** The binding is the show's and
  PERSISTED, so it travels in the showfile and a spare console inherits it.
  Which surface is plugged into which station is LOCAL and must not travel, or
  opening the show on a laptop claims a wing that is in another building.

#### midi-surfaces

MIDI control surfaces, on top of control-transports and surface-layer. First of
the three devices deliberately, and not because it is the most wanted: it is
documented, the hardware is cheap, and there is nothing to reverse engineer, so
it is the honest test of whether surface-layer's event type and binding model
survive contact with hardware. Finding that out on a fifty-pound controller
beats finding it out halfway through a USB capture.

- Notes and control changes map onto the event type with no decoding worth the
  name, so this entry is almost entirely surface-layer's questions with a real
  device attached.
- **A CC is 7 bits, which is 128 steps across a fader's travel.** Whether that
  is enough for an intensity fader is a question to settle with a fader in a
  hand, not by arithmetic. 14-bit CC exists in the spec and few surfaces send
  it, so assume 7 and see.
- Feedback (LED rings, motor faders) is the same problem as wing feedback and
  far cheaper to get wrong. Worth doing here first for that reason alone.
- **MTC belongs to timecode-workflow and MSC belongs to show-control.** Neither
  is this item, and all three want to open a MIDI port, which is what
  control-transports is for.

#### makepro-x

MakePro X hardware.

**Written on 2026-09-02 from a request rather than from a device, and the first
question is unanswered: what does it speak?** USB HID, MIDI, a serial protocol,
Art-Net, or something of its own. Nothing below is worth much until the model is
named and the interface known, and this entry wants rewriting rather than
building from as it stands.

- If it turns out to be HID or MIDI, this is a mapping file over midi-surfaces
  and not an item at all, which would be the good outcome.
- If it has a protocol of its own, it is the second consumer of surface-layer
  and therefore the one that says whether the layer generalised or whether it
  was quietly shaped around whatever got decoded first.
- Feedback, as with the other two: which half is in scope.

#### ma3-command-wing

A grandMA3 command wing, over USB.

- **The protocol is not documented and MA will not publish it**, so this is read
  off the device or not at all. Reverse engineering for interoperability is on
  firm ground (EU Software Directive art. 6, DMCA 1201(f)). The complication is
  that an MA wing is also the onPC licence dongle, so some of what crosses the
  bus is likely a challenge and response with nothing in it worth having, and
  that part is not something to replicate.
- **The cheap question first, and it is still unanswered: what does it enumerate
  as?** `system_profiler SPUSBDataType` with one plugged in. Checked on
  2026-09-02 with none attached, so the answer is not in this document. HID with
  a readable report descriptor is a weekend with `hidapi` and a loop watching
  which bit flips. Vendor-specific bulk or interrupt means capture instead
  (USBPcap on Windows against onPC, or `ifconfig XHC20 up` and Wireshark on
  macOS) and pressing one key a hundred times to find it in the diff. **Nobody
  should put a number on this item before that answer exists**, and the two
  answers are a week apart.
- **Keys, faders and encoders are the achievable half.** The LEDs and the
  encoder displays are output reports with their own framing, and they are a
  separate project rather than a later afternoon of the same one. Decide which
  half is in scope before starting, or the item never finishes.
- The keypad is the strongest argument in the section for routing keys through
  the command line, since the wing's key legends already are the grammar's
  words.
