# Changelog

Notable changes, newest first, in the spirit of [Keep a Changelog][kac], with
[semantic][semver] versions. The release workflow extracts the section whose
heading starts with the version being tagged — plainly, `## 0.0.1`, not the
bracketed form — so every release needs one and it has to be spelled that way.

[kac]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html

## Unreleased

### Added

- **Panels that can change the show open read-only.** An Edit toggle in the
  tile chrome unlocks one; closing it locks it again. Patch is the first: in
  view mode it is text cells and the row selectors, and the inputs, delete
  buttons and *+ Fixture* are not there at all rather than greyed out. Controls
  across the programmer are sized for a finger.
- **Fixture types can be edited properly.** Rename a type, set each
  parameter's default value with the right control for its kind, and pick
  `Raw` or `Named` — so a light nobody has written a profile for can be
  patched without editing JSON.
- **Effects have a shape in the schema.** `EffectSpec` describes one periodic
  instruction — a curve that is either a shape scaled between two values or a
  list of keyframes carrying their own, plus rate, width, direction, per-fixture
  phase and spread — and can be held in the programmer or stored in a cue
  capture. A `SpeedMaster` collection carries a tempo several effects can
  follow. Nothing renders them yet.
- **Effects run.** The engine renders a shape or a step list into a fixture
  parameter on every tick, from the cue's anchor or the programmer's own, at a
  rate given in Hz or borrowed from a speed master. A programmer effect beats a
  cue effect, and grabbing a fader takes that light out of the chase. Nothing
  leaves the console differently yet.
- **Nodes are told the shape, not the samples.** An OpenHaunt port that says
  in `/info` which shapes it can trace is handed one description and then left
  alone, instead of a value forty times a second; a port that advertises
  `transitions` gets a three second fade as one timed `set` rather than a
  hundred and twenty samples. The console publishes a retained
  `openhaunt/clock` once a second so a node can place the start of a cycle. A
  port that advertises nothing behaves exactly as before.
- **The simulated node traces shapes for itself.** `openhaunt-node-sim`
  advertises what each port can do, renders a shape or a timed fade at 40 Hz
  without being sent anything, and tracks the console's clock from
  `openhaunt/clock`. Its window shows a badge beside a port that is tracing, and
  its config editor has per-port capability toggles. The curve arithmetic is
  written from the protocol documents rather than shared with the console, and
  both test suites assert the same numbers.
- **A Go says when it happened.** `Sequence.went_at`, with `goNext` and
  `goToCue` taking an optional `at`, so every station anchors a cue's fades and
  effects at the same millisecond instead of at whenever each of them processed
  the command.

### Removed

- **Three fields nothing ever read.** `Show.is_running`, `Show.active_sequence`
  and `Fixture.active_preset` are gone, along with the Show panel's
  Running/Stopped button, which toggled a flag no code consulted. All three were
  SYNCED, so they had no SQL column and their removal needs no migration; a
  showfile or a peer that still names one loads fine.

### Added

- **The console says when a fixture has no way out.** A new LOCAL
  `output_coverage` path lists the fixtures no enabled output reaches — a DMX
  fixture on a universe nothing carries, or an adopted node with no OpenHaunt
  output — and the Outputs and Devices panels show each gap with a button that
  adds exactly the output it names. Deleting the OpenHaunt output no longer
  leaves adopted nodes silently undriven.
- **Selecting without the plan.** The Patch panel has a selector on every row
  and the Devices panel a *Select* button on every adopted node — click for
  one, shift-click to add — so a fixture can be programmed before it has been
  placed anywhere. Chips in the plan's *Not placed* tray can be dragged onto
  the plan to place them where they land.

### Fixed

- **Unpatching the last fixture reaches the output plugins.** The engine sent
  them nothing for an empty show, so whatever they remembered about the last
  fixture — including that nothing reached it — outlived it. One empty patch now
  follows the last fixture out.
- **Adopted OpenHaunt nodes are actually driven.** The plugin that sends a
  node's ports only runs where an `outputs` row of kind OpenHaunt says so, and
  nothing created one: values moved in the console and never left it. Starting
  to drive nodes now adds that output for the station, once, unless one already
  covers it.
- **A node that reboots gets its values again.** The OpenHaunt output remembered
  what it last sent and would not repeat it, while the node had come back at its
  defaults. A node seen going offline and back is sent every port afresh.

## 0.1.0

### Changed

- **OpenHaunt nodes describe their own ports.** `GET /api/v1/info` now carries a
  `ports` list in E1.73 UDR's vocabulary, and the console builds a fixture type
  from it at adoption. The module table is gone: a node newer than the console, or
  anybody else's module, adopts on its own say-so, and a node that describes
  nothing is refused rather than guessed at. A port whose `class` this console has
  no word for becomes a named parameter.
- **Output payloads follow the port's data type**, not the module. A number port
  takes `{ "value": … }`; `{ "brightness": … }` is retired.
- **`openhaunt-sim` is now `openhaunt-node-sim`**, and `openhaunt-sim-gui` is
  `openhaunt-node-sim-gui`.

### Added

- **A simulated node is a config file.** `openhaunt-node-sim` takes `--config`, and
  its window edits the running node: identity, module descriptor, and every port —
  access, data type, unit, range and class. Applying stops the node and starts a
  new one in its place without the window closing, so a module nobody has built is
  something to try rather than something to write. Presets for the catalogue,
  worked examples in `tools/openhaunt-node-sim/configs/`, and `--write-config` to
  get a file to start editing.

## 0.0.1

The first release, and a deliberately small number: this one is here to prove the
build and the release path rather than to be depended on.

A distributed lighting console — a show engine that several stations share, a
tiled web workspace to run it from, and the output and device layers that reach a
rig.

### Added

- **The show engine.** Cues, sequences, playback with fades and follow-ons, and a
  programmer buffer that outranks playback until it is cleared or stored.
- **Peer sync.** Stations find each other over mDNS, converge from an oplog,
  elect a leader, and survive losing one.
- **Output.** Art-Net, sACN and OpenHaunt nodes, several at once, configured from
  the show rather than from the command line.
- **Devices and flows.** OpenHaunt I/O nodes are discovered, adopted as fixtures,
  and wired to cues through a node graph that shows its own state.
- **The stage.** A calibrated ground plan and the same rig in 3D, with pan and
  tilt puppeteered by grabbing a ring, an arc, or the beam spot on the floor.
- **A tiled workspace.** Panels in a tree of splits and tab groups, with layouts
  saved into the show.
- **Desktop apps.** `pult-gui` runs a console and its server in one window;
  `openhaunt-node-sim-gui` is a window onto a simulated node.
- **One artifact per product.** The frontend is built into the server binary, so
  a station serves its own console — and any tablet on the network gets the same
  one.
