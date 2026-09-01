# Backlog of candidate changes

Each entry is a candidate for `/opsx:propose <name>`. The entry records what
exists today (verified against the code, 2026-08-31) and the questions a
proposal has to answer — the point of moving this out of a todo list is that
the open questions travel with the item instead of being re-discovered.

Ordering within a section is rough priority; sections are themes, not order.

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

Left open, and the natural next question: nothing re-drives a failed fetch except a
roster change, so a station that gives up stays given up until somebody touches the
show.

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

Two things it left open. The oplog is now pruned — `history-pruning`, roadmap task
37, which retains a plugin's writes by the same two rules as anything else and
knows nothing about plugins to do it. Still open: there is no change notification,
so a plugin holding a value in memory learns about an undo on its next read.

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

Left open: no cue-out fades — `Cue::fade_out_ms` is still declared and still unread —
and no way to take a fixture's current output as its home value, which is how an
operator would actually set a house light's.

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
file, saved in place; the oplog grows forever (pruning is history-pruning).
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
sync backlog, tick cost, WS client counts, broker stats.
- Extend `Station` rows vs. a new LOCAL stats collection; sample rates.

## Performance

### multithreading
The engine is one actor; task 29 measured 2000 fixtures at ~137% of one core
with the tick itself the small half (apply/broadcast/output per moved
fixture is the cost). Named cheaper wins first: per-key writes instead of
cloning whole `live_values` maps.
- Parallelize the render (rayon over fixtures) vs. partition computation
  across stations (task 10's open question — which also answers redundancy).
- Do the cheap win, then measure again before adding threads.

### demo-shows
Small / big / huge seeded shows to find bottlenecks (task 29's numbers came
from ad-hoc rigs). Extend `scripts/demo-seed.mjs` with size presets; huge =
thousands of fixtures, hundreds of cues, effects running, several plans.
- Doubles as regression material: record tick cost per preset in CI?

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
