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
| Frontend | Working for show, session, sequences, cues, patch, the programmer, effects and speed masters. A tiled workspace of resizable panels replaced the sidebar and tabs; layouts are saved in the showfile. Panels that can change the show open read-only behind an Edit toggle and are sized for a finger. The typed proxy runs end to end. Vitest covers the pure helpers; components are untested. |
| Playback engine | Working. Fades, active-cue tracking, FollowAfter cues and effects at 40 Hz, anchored on the cue's `went_at` so two stations render the same instant rather than each measuring from its own arrival. |
| Output plugins | Working for Art-Net, sACN, and OpenHaunt nodes, several at once. Configured from the `outputs` collection and editable while the show is up, with per-output status in the UI. Flags only seed an empty showfile. |
| Stage view | Working. A ground plan is uploaded, calibrated against something of known length, and fixtures are dragged onto it — then the same rig in 3D from front of house, beams and all. |
| Flows | Working. The spec's node graph, evaluated as a graph: sources, conditions, boolean logic, delays and actions, with live state on every node. Replaced `triggers`. |
| Devices / events | Working. OpenHaunt nodes are discovered over mDNS and adopted as fixtures; their inputs land in `live_values`; flows turn those into cues. A port that says it can trace a shape is handed one descriptor instead of forty messages a second. Tested end to end against `tools/openhaunt-node-sim` and, since task 22, against real firmware on an ESP32. |
| WASM plugins | Working. wasmtime component runtime with a WIT contract, permissions, hot reload, plugin-to-plugin calls and runtime introspection of the schema registries. Two reference plugins in `plugins/`: a command line (grammar built from introspection, console panel with completion and spans) and natural-language control (an LLM over the plugin's own gated HTTP, executing through the command line). Plugin UI is built-in surfaces or plugin-shipped web components. `docs/PLUGINS.md` is the author guide. |
| 3D programmer | Working in outline. A shared programmer buffer beats playback, and pan and tilt are puppeteered by grabbing a ring, an arc, or the beam spot on the floor — in the rig and on the plan. Effects are in, and a selection is a question about the rig rather than a list. |
| Selection | Working as a query over the rig: by type, name, sphere, box or the spec's radial cone, built up by adding, narrowing and removing, and ordered along an axis or outwards from a point. Re-evaluated as the rig changes, so a fixture patched under a live selection joins it. Queries cannot be saved as groups yet. |
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

- Pan is taken as 540° about the way a fixture hangs, because `FixtureType` carries
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

The first change planned in OpenSpec rather than here:
`openspec/changes/plugin-distribution/`. A plugin was a directory on one
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

Left open: nothing vacuums the showfile, so SQLite keeps the freed pages and the
file stays at its high-water mark — a `showfile-management` question, and not what
this was paying down. **A station running a build without this change can
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

## Further out

Planning has moved to OpenSpec: candidate changes and their open questions live in [`openspec/BACKLOG.md`](../openspec/BACKLOG.md), and become changes under `openspec/changes/` via `/opsx:propose`. This document remains the record of finished work. The items below predate that move and are folded into the backlog.

Everything below is in the spec and has no schema and no code yet. Listed so the near-term work does not paint itself into a corner.

**3D programmer.** The rig *view* is task 13 and programming in it is task 14 — the camera frames a picked fixture, pan and tilt are grabbed by ring and arc, and the quicksheet opens at the light. Effects over a selection are task 25. What that leaves of the spec's §Programming is blind, highlight and fan, and modifiers that are themselves dynamic — an effect whose rate is an effect.

**Waveform timecode and "timecode without timecode".** Beat grids, markers, live audio analysis for band sync. Should subsume the `Timecode` follow mode rather than sit beside it.

**Open control interfaces.** OSC, MIDI, and control surfaces alongside the existing WebSocket API.
