# Backlog of candidate changes

Each entry is a candidate for `/opsx:propose <name>`. The entry records what
exists today (verified against the code, 2026-08-31) and the questions a
proposal has to answer — the point of moving this out of a todo list is that
the open questions travel with the item instead of being re-discovered.

Sections are themes, not order. The order is the list below, and it is the one
thing here that is meant to be rearranged: an entry's own text records what was
asked and what is true, the order records what we intend to do next. `→` names
what has to exist first.

## Order

1. ~~**demo-shows**~~ — done, see `changes/archive/`. The numbers item 2 is judged
   by now exist, and `scripts/demo.sh --measure` reproduces them.
2. ~~**values-as-functions**~~ — done, see `changes/archive/`. A live value stopped
   being state: 35.2 ms of tick became 2.86 ms of output frame, and a cue on a
   2000-fixture rig now puts nothing at all on a connected console's socket. It also
   left the browser running the evaluator in wasm, which is what items 5 and 8 below
   now build on.
3. **typed-plugin-sdk** — codegen into `plugins/sdk` from the same inventory the
   frontend proxy comes from; the wire stays generic. → none
4. **gdtf-import** — fixture definitions from a file; the physical data it brings is
   where a beam angle and real pan/tilt ranges come from, which is why the two items
   after it wait on it. → none
5. **mvr-import** — fixtures, positions and geometry into `StagePlan` and the asset
   store. → gdtf-import, for the definitions MVR references
6. **rig-viewer-fidelity** — beams that read as light, and the one live defect left
   (a `ConeGeometry` rebuilt per fixture per frame). `values-as-functions` unblocked
   it and left it a per-frame reading of the rig to draw from. → gdtf-import, for the
   beam angle it has nowhere else to get; and better after mvr-import, which is what
   puts a rig in there worth drawing
7. **paperwork-export** — patch lists, cue sheets, rider paperwork; a read-only
   plugin over introspection, which is what introspection is for. → none
8. **outputs-viewer** — what actually leaves the console, per universe and per node.
   → none
9. **system-stats-panel** — throughput, sync backlog, per-connector frame cost, client
   counts, and what the *browser* costs itself. Unblocked: there is now a browser load
   to report, because the browser is what evaluates. → none
10. **peer-address-selection** — two stations on one laptop discover each other and
    never sync, because the first address mDNS offers is a scopeless link-local IPv6.
    Small, and it is the demo command most likely to be run. → none
11. **tick-isolation** — on hold, and due a re-scope: `values-as-functions` answered
    most of what it was for. What survives is disk off the write path, per-source
    admission and the single engine queue. → nothing, but re-read it before proposing
12. **showfile-management** — versioning, save-as, autosave, backup. → none
13. **showfile-assets-folder** — a folder with an assets directory, or one file.
    → decided with showfile-management, not separately
14. **3d-programmer-remainder** — blind, highlight, fan, and modifiers that are
    themselves dynamic. → rig-viewer-fidelity, for anything that happens in the 3D view
15. **voice-input** — speech to the command line, grammar first and NL on parse
    failure. → none
16. **nl-show-context** — what relative syntax cannot reach, and whether it is worth
    the permission it costs. → voice-input, which is what shows which utterances
    actually arrive
17. **open-control-interfaces** — OSC, MIDI, control surfaces. → none
18. **timecode-workflow** — waveform and beat-grid timecode, timed playback, audio
    import. The biggest item here and the one the spec is most opinionated about.
    → none technically
19. **llm-cost-overview** — token and cost accounting out of the NL plugin. → none
20. **openhaunt-as-plugin** — output connectors as WASM, if a connector's own frame
    rate survives the boundary. → the benchmarks from demo-shows and
    values-as-functions, which are what decides it
21. **video-mapping-ndi** — NDI output; scope carefully, it hides a media server.
    → openhaunt-as-plugin, as the first proof the plugin API carries heavy output
22. **plugin-language-hosts** — TS plugins, via a host plugin or as components.
    → a real TS plugin wanting to exist

## Plugins (builds on the WIP WASM runtime)

**All four are proposed.** `plugin-installation`, `plugin-showfile-binding` and
`plugin-sync` became one change, `openspec/changes/plugin-distribution/`, because
they share the authority decision; `plugin-datastores` is its own change. The
entries below are kept as the record of what was asked; the proposals are where
the answers are.

### plugin-installation — see `changes/plugin-distribution/`
How a plugin gets onto a console that is not a dev checkout. Today plugins are
loaded from a directory passed as `--plugins`; they are not release artifacts
(ROADMAP task 8 left this open).
- Install from a file? A URL? A registry? Drag-drop into the web UI?
- Who verifies what a plugin is allowed to do — the permission grant UX at
  install time vs. first run.
- Are plugins per-station or per-show (see plugin-showfile-binding)?

### plugin-showfile-binding — see `changes/plugin-distribution/`
Which plugins a show *needs* vs. which a station *has*. A show authored with
the command-line plugin's panels in its layout degrades on a station without
it (panel ids already survive missing plugins).
- Distinguish setup-relevant vs. runtime-relevant plugins?
- Does the showfile record required plugins + versions? Warn or refuse on
  open when one is missing?
- Plugin *config* (e.g. NL provider/model) — showfile, station preferences,
  or the plugin's own directory as today?

### plugin-fetch-retry — done, see `changes/archive/2026-09-01-plugin-fetch-retry/`
Not proposed from here: it came out of roadmap task 40, where a flaky test turned out
to be the console reporting "no station has the bundle" when what had happened was
that it could not reach one. The change is the spec delta those commits should have
carried — three outcomes instead of two, a bounded retry for the unreachable case
only, and one `peer_addresses` that excludes the asking station.

**Since closed.** A station arriving re-drives a failed fetch: the manager watches
`stations` for a row appearing or an `http_addr` being published, compares the peer
*addresses* against what it last saw, and re-drives only where somewhere new turned
up. Which does not contradict "an answer is an answer" — that rule is about asking
the same stations twice inside one fetch, and a station that was not there has not
answered. No timer: a station that is not there will not be there in thirty seconds
either, and a session has as many re-drives in it as it has consoles arriving.

### plugin-sync — see `changes/plugin-distribution/`
Do plugins replicate between stations the way show data does? Today each
station loads its own directory; a session can have stations with different
plugin sets.
- Sync the wasm bytes (they are content-addressable like assets) or only
  agree on the manifest?
- A plugin holding an API key must NOT replicate its secrets.
- Interaction with hot reload: an edit on one station — does it propagate?

### plugin-datastores — done, see `changes/archive/2026-08-31-plugin-datastores/`
Shipped as roadmap task 35. Both scopes exist: `scope = "show"` is a PERSISTED
`plugin_data` entity, `scope = "station"` is SQLite beside the preferences.
Quotas are 1,000 keys and 1 MB, lowerable by the manifest; data outlives its
plugin and orphaned data is shown in the Plugins panel. The entity machinery was
reused rather than a per-plugin schema — values stay opaque JSON, which the
non-goals argue for.

Two things it left open, and both are closed. The oplog is now pruned —
`history-pruning`, roadmap task 37, which retains a plugin's writes by the same two
rules as anything else and knows nothing about plugins to do it. And there is now a
change notification: `store.subscribe(store)`, delivered through the existing
`lifecycle.on-update` as `[store, key]`, built on the engine's broadcast rather than
a hook in the store's write path — a hook would see only this station's guest
writing, where the broadcast also sees an undo and a peer's copy of the same plugin.
The contract went to `pult:plugin@1.1.0` for it, which is the first exercise of the
floor rule the 1.0 move was made for.

### typed-plugin-sdk
Introspection is the right *wire*, and a poor thing to program against. A plugin
learns the schema from `introspection::entities()` as JSON and navigates it by
hand, with no types, no compile-time field names, and stringly-typed paths
(`&["cues", id, "fade_time"]`). Every plugin author pays for that.

The fix is codegen — but into the **SDK**, not the WIT. `pult-codegen` already
generates `frontend/src/lib/ws/data.ts` from the `EntityMeta` inventory, giving
the frontend `data.sequences[5].cues[3].fadeTime.set(4)` while the WebSocket wire
stays generic path-plus-JSON. `plugins/sdk` can have exactly that split, from the
same inventory and the same tool: `sdk::data::cues().nth(3).fade_time().set(4.0)`
over an unchanged `data.set(path, json)`.

- **Codegen'ing the WIT itself is ruled out, and the reason is recorded** in
  roadmap task 35: a component's imports are stamped with the package version,
  and a record type's fields are part of every signature using it — so `Cue`
  gaining a field would be a breaking ABI change. The schema changes constantly,
  and a show now carries its plugins between machines, so a bundle built against
  schema-of-Tuesday would refuse to load on a station from schema-of-Wednesday.
  Today the schema grows daily and no plugin notices; that property is worth
  keeping.
- **Introspection stays.** It answers the runtime question — what does *this*
  station have, including collections this plugin's SDK never heard of — which a
  command-line plugin building its grammar and a sync plugin walking unknown
  tables both need. Typed codegen is for what is known at build time.
- How does the SDK version relate to the station's? A plugin built against a
  newer SDK writing a field an older station lacks gets a per-path runtime error,
  which is the same graceful failure the frontend already has. Worth confirming
  the message is good.
- Where does the generated code live — checked in beside `sdk/src/lib.rs`, or
  generated into `OUT_DIR` by a build script? Checked in matches how the frontend
  does it and keeps the plugins workspace buildable without the console's.

### plugin-language-hosts
A plugin that hosts other plugins in another language — e.g. a TypeScript
host plugin embedding a JS runtime, so plugins can be written in TS on top
of it.
- Does the host API (permissions, introspection, surfaces) pass through
  cleanly, or does the host become a second, drifting plugin API?
- Alternative: componentize JS directly (jco/StarlingMonkey) so a TS plugin
  is just a component and no host plugin exists.
- Defer until a real TS plugin wants to exist?

### openhaunt-as-plugin
Should the OpenHaunt output path (and Art-Net/sACN) be WASM plugins instead
of built-in connectors? The spec calls output "a plugin layer" already;
today `OutputPlugin` is a Rust trait inside the backend.
- The 40 Hz output path through a WASM boundary — measure before deciding
  (task 29 has the tick-cost numbers to compare against).
- Discovery (mDNS) and the embedded MQTT broker are harder to host in a
  guest than frame emission is.
- Middle ground: keep connectors native, expose the same registration to
  plugins so a *new* protocol can be a plugin without moving the built-ins.

## Natural language and voice

### voice-input
Voice as an input path to the command line. Open question that shapes it:
an utterance may already be valid command-line syntax — with the NL plugin
it could go to the LLM anyway (waste, latency), without the NL plugin it
should parse directly.
- Route: try the grammar first, fall back to NL only on parse failure?
- Where speech-to-text runs: browser (Web Speech API), station, or plugin.
- Push-to-talk vs. wake word; confirmation before destructive commands.

### nl-show-context
"A bit darker" needs the current value; the NL plugin has none. Verified:
`plugins/natural-language-control/pult-plugin.toml` grants no data access
(`commands = false`), by design — everything goes through the command line.
- Options: (a) give the plugin read access and put state in the prompt,
  (b) add relative syntax to the command line (see relative-values) so the
  model never needs the value, (c) both.
- (b) keeps the "one grammar, one audit trail" property; (a) weakens the
  safety story and grows the prompt with the rig.
- **(b) is done**, as of `relative-values`: `at +10` and `at -10` are command-line
  syntax, so "a bit darker" is an utterance the plugin can answer with no show data
  and no new permission. What is left of this entry is the part relative syntax
  cannot reach — "make it look like the second verse", which needs the show — and
  whether that is worth the safety story it costs.

### llm-cost-overview
Token/cost accounting for the NL plugin, visible over the REST API.
- Where measured: the plugin sees the responses (usage fields); host sees
  only bytes. Plugin reports → LOCAL state? Then a `GET /api/...` beside
  `/api/config` and a panel reading the same numbers.
- Per session, per show, per station? Cost tables per provider/model live
  where, and who updates them?

## Programming model

### 3d-programmer-remainder
What is left of the spec's §Programming once the rig view (task 13), programming in
it (task 14) and effects over a selection (task 25) are done: **blind**, **highlight**
and **fan**, and modifiers that are themselves dynamic — an effect whose rate is an
effect. Predates the move to OpenSpec; folded here from `docs/ROADMAP.md`.
- Blind wants a second programmer buffer that does not reach the output. Is that a
  second `programmer_values` collection, or a flag on the existing one? It is SYNCED
  either way, so two operators can be blind separately or not at all — which?
- Highlight is a temporary output override for the selection, which is the same
  shape as home (`__home`) pointed the other way. Reuse that machinery or not?
- Fan needs an order over the selection, and `SelectionQuery` now carries one
  (`fixture-groups`). Does fan reuse it, and what does fanning an unordered
  selection mean?
- A dynamic modifier is a graph rather than a value; nothing in the schema is
  recursive yet.

### parameter-defaults — done, see `changes/archive/2026-09-01-parameter-defaults/`
Shipped as roadmap task 41. A parameter has a **home value** — the fixture's own
`home_values` override, else its type's `default_value` — resolved once in
`pult-schema`, with no TypeScript twin because nothing here is on the per-frame path
that made `fixture-groups` pay for two evaluators.

The answers to the questions below. "Nothing is controlling this" is decided by
**what the show says**, not by the absence of a key: `live_values` keeps its keys and
the home value is written into them, because panels read that map and an absent key
reads as unknown rather than as at-default. The override is per **fixture** and
PERSISTED — a type is derived from the device and rebuilt whenever it describes
itself again, so an override there would not survive. It **snaps** by default, over
`Show::home_fade_ms` when a show asks for a fade; show data rather than a station
preference, because `history_depth` had already written the argument — two stations
answering differently about one show is a disagreement, not a preference — and the
station preference decides what a new show starts with.

And **no, it did not need a tracking model.** Release is defined at two boundaries:
taking a sequence off, and the programmer letting go. What a sequence releases is
read from its cues rather than remembered, which is coarser than tracking, needs none
of it, and is the same answer on a station that joined at the interval as on one that
ran the act.

The cost taken knowingly: **Go at the last cue now stays there** instead of wrapping
to no active cue, because "off" has to be a state playback can tell apart from "ran
out of cues" and a second field encoding the same thing would be worse.

**Both of the things it left open are closed.** `fade_out_ms` is read: a cue fades two
ways, the out time going down and the in time going up, on the cue and per capture
with the capture winning. Zero out means "this cue does not split its fade" rather
than "snap", so nothing an existing show does changed. Only values with an order can
be going down — a colour has three and no agreed ranking, a relay none — and those
take the in time rather than have the console guess. What a cue *out* time is not:
the outgoing cue's parameters leaving, which is tracking and still out of scope.

And `["fixtures", "__set_home"]` takes a fixture's current output as its home value,
one parameter or all of them. A verb rather than a write to `home_values` for the
reason `__home` is one — a caller able to act should not have to be able to read the
rig — and one write of the whole map, so a fixture is one Ctrl-Z.

### relative-values — done, see `changes/archive/2026-08-31-relative-values/`
Shipped as roadmap task 39. A path verb, `__by`, beside `__create` and `__delete`,
so a relative write arrives over the existing `Set` message with no new protocol,
host function or permission.

The answers to the questions below: **the engine**, and specifically at the *front
door* — resolution runs at the top of the `Set` arm, before `previous` is read, so
the oplog, the broadcast and the sync layer all see an absolute and a peer receives
the number rather than the delta. Relative to task 14's stack read rather than
re-implemented: the programmer's value where it holds the key, `live_values` where
it does not, the type's `default_value` where nothing ever drove it. Undo came free
for exactly the reason guessed.

The cost taken knowingly: the engine names one collection.
`["programmer_values", "__by"]` exists because the ordinary case is *not already
holding the key*, and a test nudging a `speed_masters` field keeps "a new collection
needs no edit here" honest.

Left open: no fan, no multiplying sibling ("half as bright"), and no relative value
*stored in a cue* — that last is tracking, which is its own design.

### fixture-groups — done, see `changes/archive/2026-08-31-fixture-groups/`
Shipped as roadmap task 38. A PERSISTED `groups` row is a name and a
`SelectionQuery`; the query types moved to `pult-schema` and the frontend re-exports
them under the names its panels already used.

The answers to the questions below: the query, never the id list — a group is the
question, and resolving it reads the rig as it is now. Command-line addressing did
**not** come free: `fixture` is a keyword in `parse.rs` and `group` had to become one
too, though generic entity addressing did give `rename group 1 "Movers"`.

Two things it turned up. `Order::Manual` kept the dragged order in a *browser store*,
which a station resolving the group has never seen — so the order moved into the
query, and the evaluator distinguishes "no hand order" from "an empty one". And an
RPC's prefix is a reserved word in the command line: `group.resolve` silently ate the
`group` keyword, so it is `selection.resolve`.

Two evaluators is the standing cost — Rust for the station and plugins, TypeScript
for a cone being dragged at frame rate — paid by `testdata/selection-queries.json`,
which both suites read. A new term or order needs a case there.

Left open: no `Term::InGroup`, so a group cannot appear inside another group's query.
Recall-then-refine covers the workflow; the term needs cycle detection and an answer
for deleting a referenced group.

## Visualisation

### rig-viewer-fidelity
The 3D rig viewer draws a beam as a `ConeGeometry` wearing a flat additive material
(`frontend/src/lib/components/stage/Rig3D.svelte`). That is enough to say where a
light is pointing and not enough to look like light. Prior art read in full on
2026-09-01: ASLS Studio's visualizer (`src/plugins/visualizer/`, about 2.7k lines),
which is this problem solved a level up. It is GPL-3.0 and we are MIT, so what
travels is the technique, not the code.

What they do, in the order it matters to us.

- The beam is not geometry. One 100 m open-ended cylinder, instanced, and the beam
  angle is vertex displacement: the far ring is scaled by `tan(angle)` in the vertex
  shader. Zoom costs a float in an attribute and nothing is ever rebuilt.
- Brightness depends on where you stand. Four terms multiply together: how side-on
  the beam is seen, how near the camera is to looking down the barrel, an
  inverse-square-ish falloff along its length, and a power term on the silhouette so
  a cylinder stops reading as a tube.
- Haze is four octaves of 3D simplex noise sampled in world space with *time as the
  third axis*, so it drifts. Density and turbulence are the two knobs they expose.
- The beam smoothsteps out over the last centimetre above the deck instead of
  clipping through it.
- Colour is scaled in HSV, value only, so a dim beam keeps its hue rather than
  crushing towards grey the way scaling RGB does.
- Base, yoke and head are three `InstancedMesh`es sharing one material, per-fixture
  state carried in `InstancedBufferAttribute`s, and the model articulates: the yoke
  swings on pan and the head nods on tilt.
- Selection is one per-instance float that an `onBeforeCompile` patch turns into
  emissive. No material swap and no extra draw call.

Two things about them are worth not copying. Their README credits the
`postprocessing` library and there is no `EffectComposer` anywhere in their `src/`;
every bit of glow is additive blending in one fragment shader, which is the cheaper
lesson. And their fixture bodies are pure black, so the render cannot tell you what
is hanging up there. Our emissive body tinted by its own output is the better call
and should survive whatever else changes.

One defect in ours turned up while comparing, and is still there.

- `<T.ConeGeometry args={[beam.length * 0.12, beam.length, ...]}>`. `args` is
  reactive, so Threlte rebuilds the geometry whenever the throw changes. Dragging a
  beam spot allocates a fresh cone per fixture per frame — and since
  `values-as-functions` the throw is re-evaluated every animation frame, so a fade now
  does it too.

A second one, the `<T.SpotLight>` inside `{#if beam.output.level > 0.01}`, was the
worse of the two: crossing that threshold changed the scene's light count, which
changed three.js's program cache key and recompiled every material mid-fade. It went
with the `values-as-functions` rewire and is no longer here.

Open questions.

- Beam angle has nowhere to come from. `FixtureType` carries no beam angle and
  `ParameterKind` has no `Zoom`, so everything is drawn at the hardcoded
  `length * 0.12` (a 6.8° half-angle) and a wash looks like a beam. That is a
  `pult-schema` change and it is the same one `gdtf-import` wants. Do they land
  together, or does a `default_beam_angle` on `FixtureType` come first?
- Where does haze live? A station preference seeded into the show the way
  `home_fade_ms` is, or a per-browser view setting? How hazy the room is is a fact
  about the room, which argues for the show, but two operators on two tablets may
  reasonably want different pictures.
- Instancing against the derived `beams` array. Every frame rebuilds a `Quaternion`,
  an `Euler` and a `Color` per fixture — *sixty* times a second now, not forty, since
  the viewer draws its own frames rather than waiting to be pushed values. Instanced
  attributes are the fix, and they sit badly with Threlte's declarative `#each` and
  with picking, which raycasts against per-fixture objects. Does the viewer drop to
  imperative three.js inside one Threlte component, and what happens to the gizmos if
  it does?
- **What is already done for you.** The evaluator is in the page: `stores/output.ts`
  registers what a panel is showing and evaluates all of it in one wasm crossing per
  frame (200 parameters in ~17 µs), and `Showing.at` is `null` while the browser
  cannot place itself on the station's clock. A beam that is drawn is a beam that was
  evaluated for the moment it is drawn at, which is what this item wanted.
- Their singletons do not survive the move. `SceneManager`, `Controls` and
  `AnimationManager` are module-level globals over shared mutable buffers, which is
  fine for one viewport and breaks in our tiled workspace, where two `rig` panels can
  be open at once. Anything we take has to be per-panel.
- Placement, as opposed to aiming. They have `TransformControls` with a 0.5 m
  translate snap, keyboard modes, and multi-select through a bounding-box group so a
  whole truss moves together. We have pan/tilt/spot gizmos for aiming a head and
  nothing for rigging one in 3D. Same change or its own?
- Strobe needs a `ParameterKind` before it can be rendered at all; theirs is a square
  wave against the animation clock driving the intensity attribute. Out of scope
  here, or the reason to do that schema work once?
- Three cheap wins need no design and could go in ahead of the rest: `depthTest:
  false` on our existing gizmo rings so they are never buried inside a fixture body,
  cancelling a `follow` camera transition on any pointer or wheel input, and an
  infinite grid shader (`fwidth` line antialiasing, two scales, distance fade) to
  replace the fixed `GridHelper`, which aliases badly past about 40 m and stops at
  the edge of the plan.

## Users and undo

### default-user — done, see `changes/archive/2026-08-31-default-user/`
Shipped as roadmap task 36. Every show gets one user, named "Operator", seeded by
the engine at the end of `load_from_showfile` so a headless station is covered.
The answers to the questions below: the backend creates it, not the first client;
one per *show* rather than per station, because `user.rs` argues identity is
chosen rather than taken from the machine; a browser with nothing stored adopts it
and says so, since two people sharing one undo history is a real cost and belongs
on screen; and *Sign out* stays but falls back to the default instead of to
nobody. Old showfiles gain one on open — their existing `user_id: None` rows stay
un-undoable, which is history rather than a defect.

The id is a fixed constant rather than a v5 over the show, because the frontend
has to work as the default *before* the `users` collection arrives or the window
reopens; `frontend/src/lib/users.ts` holds the same constant and a Rust test
asserts they agree. The trap was `create_entity`, which inserts with no existence
check — an unconditional seed would replicate "Operator" over a rename.

Left open: a station seeding offline can race a rename on another station, and
the sync layer breaks that tie rather than intent. Self-healing, since the ids
match.

### history-pruning — done, see `changes/archive/2026-08-31-history-pruning/`
Shipped as roadmap task 37. Two retentions: authored rows bounded by the show's
`history_depth`, unattributed ones by a station preference defaulting to an hour —
counted differently on purpose, since `history_depth` counts changes because an
operator does, and an absence is a duration.

The answers to the questions below: pruning runs on open and every thousand
appends, spawned off the actor's loop so a `DELETE` cannot stall the tick; it
keeps the by-people distinction, because that is what `recent_by_people`'s filter
exists for; and the floor **cuts freely** rather than waiting on peer
acknowledgement — a station that went home for the weekend would otherwise pin the
log, which is the growth being fixed. A peer behind the floor gets a snapshot,
which is the path every joining station already takes.

The floor is a seq **per node**, not a timestamp, because catch-up compares a
peer's vector-clock entry for the node that wrote each row. It is written before
the rows go: over-reporting costs a snapshot, under-reporting loses a peer's
writes silently. Pruning is local and never replicated.

`since` was left alone — its missing `WHERE` is a per-request vector-clock
predicate, so bounding the table is the fix for it. Measured: 25,160 rows cut to
500 in 36 ms.

Left open: nothing vacuums the file, so it stays at its high-water mark (see
`showfile-management`). A station on an older build can short-change a peer from a
pruned showfile, so a session should not mix builds across this.

## Showfiles

### showfile-management
Versioning, backup, automated backup to an external drive. Today: one SQLite
file, written on every PERSISTED write, with **no explicit save at all** — there
is no `save` RPC and nothing defers a write.
- **Save should mean checkpoint, not flush.** The want is committed intent — try
  something in rehearsal and discard it, name a version, get back to the show as it
  was at the end of yesterday. The want is *not* deferred durability: a show that
  loses an evening's programming because nobody pressed Save is the worst failure
  this console has, and it happens exactly where people forget — a long tech, late,
  everyone tired. So keep writing continuously as the crash journal and let Save mark
  a point, rather than making the write wait for a keypress.
- There is no performance case for deferring either, and there will be even less
  once `values-as-functions` takes the tick off the write path: operator edits
  happen at human rate.
- **Revert-to-last-save wants the oplog, not a second history.** The log is already
  per-node sequenced and already bounded by `history-pruning`, so a checkpoint is a
  marked seq and reverting is a rewind — the same machinery undo uses.
- The hard part, and the reason this cannot be a small change: **the show is
  replicated live.** If one console defers or reverts while another saves, what got
  saved? A checkpoint is either session-wide agreed or explicitly per-station, and
  that decision drives everything else here.
- Save-as / snapshots / autosave cadence; what a "version" is when the show
  is also replicated live to peers.
- Backup target configuration is a station preference (task 33's
  preferences.toml is the home).
- Restore UX: open a backup read-only vs. roll the working file back.
- Oplog pruning on save/backup: see history-pruning, which is that question on
  its own; showfile-management only has to say whether a backup is also a prune
  point.

### showfile-assets-folder
Assets are a blob table inside the SQLite file (task 13), addressed by
sha256. Question: a showfile *folder* with an assets directory instead, and
zip on export — plus dedup across versions.
- Content addressing already gives dedup; versioned backups of a folder
  share unchanged assets naturally (hardlinks / store-once).
- A single file is robust against half-copies; a folder is friendlier to
  rsync and inspection. Export-as-zip can exist either way.
- Decide together with showfile-management, not separately.

## Interop

### mvr-import
MVR (My Virtual Rig) import — native or plugin. Task 13 noted `StagePlan` +
the asset store are what an import needs and nothing is in its way.
- Brings fixtures, positions, and 3D geometry; maps to `Fixture::position`
  and plans. GDTF references inside MVR need gdtf-import first or stubs.

### gdtf-import
GDTF fixture definitions — native or plugin. `FixtureType` is derived data
today (OpenHaunt nodes describe themselves); GDTF is the same idea as a
file: the description becomes a fixture type.
- Real pan/tilt ranges fix the 540°/270° constants task 14 complains about.
- Channel-mode selection, wheels, and physical data: how much of GDTF maps
  onto `ParameterDefinition` before it needs to grow.

### paperwork-export
Patch lists, cue sheets, rider paperwork — native or plugin.
- A read-only report is an ideal plugin (introspection already exposes all
  the data); print CSS vs. PDF generation.

### open-control-interfaces
OSC, MIDI and control surfaces alongside the existing WebSocket API. Predates the
move to OpenSpec; folded here from `docs/ROADMAP.md`.
- Native connector or plugin? Argues with openhaunt-as-plugin: a control surface is
  input rather than a 40 Hz output path, so the latency case against a WASM boundary
  is much weaker here.
- What does an OSC address map onto — the path API directly (`/pult/sequences/3/go`),
  or the command line, which already has one grammar and one audit trail?
- MIDI needs a device on a particular station, so a surface is LOCAL to whoever it is
  plugged into while the thing it drives is SYNCED. Same shape as a fixture connector.
- Learn mode (press a fader, bind it) is the UX that makes it usable, and it is a
  write to the show — which collection?

## Observability

### outputs-viewer
A live view of what leaves the console: DMX sheet per universe, OpenHaunt
messages per node. The dedup caches in `connectors::dmx` already hold the
current universe images; OH sends are discrete messages worth a ring buffer.
- LOCAL state on the owning station; the viewer subscribes cross-station
  (latency numbers set precedent: a link property is published by whoever
  measured it).
- 40 Hz × 512 bytes should not hit the WebSocket unthrottled — snapshot on
  demand or diffed at panel rate.

### system-stats-panel
Stations panel (task 10) has cpu/mem/uptime. Missing: network throughput,
sync backlog, WS client counts, broker stats.
- **Frame cost is done and the shape question is answered.** `values-as-functions` put
  it on the `Station` row as `Vec<FrameCost>` — one entry per connector, each with the
  mean, the worst, the evaluating half of each, and the frame count for the window —
  on the grounds that a station is already the sole authority on its own numbers there.
  So: extend the row, not a new LOCAL collection, unless something arrives that a row
  genuinely cannot hold (a ring buffer of recent frames would be that).
- What is left here is the panel: nothing in the frontend reads `frame_costs` yet.
  Absent has to render as absent — a settled connector is not an instant one — and a
  station with two connectors shows two rows, not an average.
- Sample rates for the rest; `REPORT_INTERVAL` is two seconds and everything on the
  row shares it.
- **The browser's load belongs here too, not just the backend's.** Since
  `values-as-functions`, a console *is* a browser evaluating a rig at frame rate in
  wasm, and that is a real cost on a real machine — a tablet at the back of the
  room can be the thing that is struggling while every station is comfortable. So the
  panel shows both.
  - What a browser can honestly report about itself: frame rate and dropped frames
    (`requestAnimationFrame` deltas), time spent in the evaluator per frame, how many
    parameters it is evaluating, `performance.memory` where the browser offers it, and
    its measured clock offset from the station — which is the one number that says
    whether what it is showing can be trusted at all.
  - Where it lives: a browser is not a station and must not appear in `stations`.
    A LOCAL collection keyed by WebSocket session is the obvious shape, published by
    the client and owned by the station it is connected to — which also makes it
    disappear correctly when the tab closes.
  - Open: does a client's report replicate to peers, so any console can see that the
    tablet is struggling, or is it LOCAL to the station serving it? Seeing it from
    anywhere is the useful version and costs a row per client per session.

### peer-address-selection
`infra/session/mod.rs:291` takes the *first* address mDNS advertises for a discovered
session and connects to that. On a machine whose stack lists a link-local IPv6 first,
that is `[fe80::…]` with no scope id, and the connect fails with "No route to host" —
two stations discover each other, join the session, and never sync. Seen on macOS
while verifying `values-as-functions` with `scripts/demo.sh --two`, which is the
command most likely to hit it.

- Try each advertised address rather than the first, in an order that prefers what is
  likely to work: routable IPv4, routable IPv6, then link-local with a scope id
  attached. mdns-sd reports the interface, so a scope can be attached rather than
  guessed.
- Or filter at discovery and never record an address that cannot be dialled — which is
  simpler and loses the machine whose *only* address is link-local.
- Either way the failure should be visible: a session joined and not syncing is worse
  than one that would not join, and today the only trace is a `WARN` in the log.
- Not observed in the field, because a real rig is on a real network. Observed every
  time on one laptop, which is where the demo runs.

## Performance

### values-as-functions — done, see `changes/archive/2026-09-02-values-as-functions/`
Shipped as roadmap task 44. A live value stopped being state: what is *driving* a
parameter is the state, and every consumer evaluates a number for the moment it needs
one. The engine's 25 ms timer went with it.

**What it came to**, `--release`, `--size huge`, 2005 fixtures: 35.2 ms per tick at
40 Hz became **2.86 ms per output frame** at 34 Hz — eleven percent of the frame budget
against a hundred and forty percent of the tick budget — and a connected browser was
sent **nothing at all** about the rig across four seconds of a running show.

**The evaluator question was answered WASM**, and the reasoning is worth keeping: the
surface (easings, curves, step lists, spread, phase, direction, width, master rates,
priority, home fallback, split fades) is an order of magnitude larger than
`SelectionQuery`, and a drift between twins shows up as the screen disagreeing with the
lamps. `crates/pult-render` is linked natively and compiled to
`wasm32-unknown-unknown` by `crates/pult-render-wasm`; `testdata/driven-values.json`
holds the two *compilations* together the way `selection-queries.json` holds the two
implementations of a query together.

**What it leaves for others.**
- `rig-viewer-fidelity` — unblocked and served: the 3D view already evaluates every
  beam it draws, per frame, in wasm, so the per-frame beam evaluation that item wanted
  exists. The `SpotLight` defect went with the rewire; the `ConeGeometry` rebuild did
  not, and is still there to fix.
- `system-stats-panel` — unblocked and enlarged. There is now a browser load worth
  reporting, and the clock offset is the one number that says whether what a browser is
  showing can be trusted at all.
- `tick-isolation` — most of what it was for is answered. What survives is disk off the
  write path and the single engine queue; re-scope before proposing.
- **Still open, untouched:** partitioning computation across stations. Task 10's
  question, and the numbers for it are different now that a station's cost is its
  output frames rather than its tick.

### multithreading — mostly answered, see `values-as-functions`
The record of what was asked. The engine is one actor; task 29 measured 2000
fixtures at ~137% of one core and named cheaper wins first: per-key writes
instead of cloning whole `live_values` maps.
- "Parallelise the render (rayon over fixtures)" is **answered: no.** The render
  is 0.07 ms of a 35 ms tick. There is nothing there to parallelise.
- "Per-key writes instead of cloning whole `live_values` maps" is **answered: there
  are no such writes.** `values-as-functions` removed the field rather than making its
  writes cheaper. See roadmap task 44.
- "Do the cheap win, then measure again before adding threads" was right, and the
  measuring is what moved the target — see roadmap tasks 43 and 44.
- **Still open, and untouched by any of this:** partitioning computation across
  stations, which is task 10's question and also the redundancy one. Worth asking
  again only when there is a workload that a single station cannot carry, and the
  numbers for that will be different once values are not state.

### demo-shows — done, see `changes/archive/2026-09-01-demo-shows/`
Shipped as roadmap task 43. `scripts/demo.sh --size small|big|huge`, with `small`
the hand-made show and the default, and `--measure` printing what a tick cost on
this machine.

The answers to the questions below. The presets are **additive** on the hand-made
show rather than replacing it, and a cue captures a **slice** of the rig rather than
all of it — 300 cues times 2000 fixtures is 600,000 captures, which measures JSON
rather than lighting. Seeding stayed **on the WebSocket API**, pipelined through a
bounded window of 64 rather than one awaited round trip at a time: a 2000-fixture
seed is the largest exercise of the write path in the repo and worth more than the
43 s it costs in release. Bounded rather than unbounded because the engine's command
channel is 256 deep, and the backpressure would otherwise arrive as a spurious
timeout.

And **no, not in CI.** A threshold needs a number that holds still; two identical
runs of `huge` varied by more than a percentage point of CPU and fifteen milliseconds
of tick. A gate that flaps gets disabled, which is worse than no gate. Revisit once
`multithreading` has moved the numbers.

What it turned up, which is the reason it was worth doing first: a station now
publishes what its own tick costs, as **two** figures — the whole tick and the
`Playback::tick` part — and at 2000 fixtures playback is **one percent** of the tick.
Task 29 put that split at roughly one in three. A single figure would have credited
all of it to playback and sent `multithreading` to the wrong half.

**And the third counter got added afterwards, for one run, because the answer changes
what `multithreading` is.** Of a 35.2 ms tick at 2005 fixtures: reading the show
**33.8 ms (93%)**, computing 0.07 ms, applying 2.2 ms. `playback_tick` calls
`read_collection` six times and each one clones a collection out of `ShowState` as
`serde_json::Value` and deserialises it whole. The engine re-deserialises the show
forty times a second, and that — not applying, and not concurrency — is the tick.

## Media and time

### video-mapping-ndi
NDI output for video mapping. Almost certainly a plugin (first real test of
whether the plugin API can carry a heavy output), or a sibling connector.
- Frames come from where — a pixel-mapped fixture array rendered by the
  engine, or media playback? Scope carefully; this hides a media server.

### timecode-workflow
The big one, and the spec is opinionated: waveform/beat-grid timecode, plus
"timecode without timecode" (timed cue playback), audio import and playback.
`FollowMode::Timecode` has existed unimplemented since task 3, deliberately
waiting for this design.
- Audio import lands in the asset store; playback on which station, and what
  do the others chase? (The OpenHaunt clock topic and `went_at` anchoring
  are prior art for shared time.)
- Beat grids relate to SpeedMasters (tap tempo is a degenerate beat grid).
- External SMPTE/MTC in scope or explicitly out?
