# Changelog

Notable changes, newest first, in the spirit of [Keep a Changelog][kac], with
[semantic][semver] versions. The release workflow extracts the section whose
heading starts with the version being tagged — plainly, `## 0.0.1`, not the
bracketed form — so every release needs one and it has to be spelled that way.

[kac]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html

## Unreleased

### Added

- **Effects.** A shape or a list of keyframes, applied across a selection and
  spread as a chase, from the centre out, in wings, in groups or at random. A new
  Effects panel builds one against a live waveform with a dot per fixture; the
  Programmer shows an amber chip for a parameter under an effect rather than a
  number, because the value beneath is only where it falls back to. Effects are
  held in the programmer or stored into a cue, and every station renders the same
  one from replicated state, so two consoles chase in step.
- **Speed masters.** A tempo several effects follow, tapped along with the band.
  Halve or double it, run or stop it, watch a beat dot. A tap writes the tempo and
  its anchor together, which is what makes a tempo change a step every station
  lands on rather than a drift each one accumulates.
- **A node is told the shape, not the samples.** An OpenHaunt port that says in
  `/info` which shapes it can trace is handed one descriptor and then left alone.
  On real firmware a half-hertz sine is one MQTT message and then twelve seconds
  of silence, where forty a second used to go out; a three second fade is one
  timed `set` rather than a hundred and twenty samples. The console publishes a
  retained `openhaunt/clock` so every node times its cycles against the same
  answer. A port that advertises nothing is driven exactly as before.
- **Cue timing has somewhere to be typed.** Fade in and out, follow mode, and
  per-capture fade, delay and curve — all honoured by the playback engine since it
  landed, and none of them reachable. A running cue now shows a strip of the fades
  and effects it is actually producing, which during a three second fade is not
  what the cue list says. Cues can be inserted between two others and dragged into
  a different order.
- **Panels that can change the show open read-only.** An Edit toggle in the tile
  chrome unlocks one and closing it locks it again, because a console is a tablet
  on a truss as often as it is a desk. Locked controls are absent rather than
  greyed out. Controls across the programmer are sized for a finger.
- **Fixture types can be edited properly.** Rename one, set each parameter's
  default value with the right control for its kind, and pick `Raw` or `Named`, so
  a light nobody has written a profile for can be patched without editing JSON.
- **Device detail, several stage plans, and positions by typing.** A device row
  opens to show its address, firmware, module and a port table saying what each
  port can trace for itself. A show can hold more than one plan and switch between
  them, with the 3D rig following, and a plan can be turned to match the room.
  Positions can be typed as x, trim and z with a resting direction. Flows can be
  renamed.
- **A Go says when it happened.** `goNext` and `goToCue` carry the time, so every
  station anchors a cue's fades and effects at the same millisecond instead of at
  whenever each of them processed the command.
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

### Removed

- **Three fields nothing ever read.** `Show.is_running`, `Show.active_sequence`
  and `Fixture.active_preset` are gone, along with the Show panel's
  Running/Stopped button, which toggled a flag no code consulted. All three were
  SYNCED, so they had no SQL column and their removal needs no migration; a
  showfile or a peer that still names one loads fine.

### Fixed

- **A tick was quadratic in the size of the rig.** `ShowView` scanned the fixture
  list for every lookup rather than indexing it, so the cost of a tick grew with
  the square of the rig. It went unnoticed while a settled show stopped ticking
  altogether; an effect never lets it settle. A thousand fixtures under one effect
  went from 29% of the tick budget to 16%.

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
