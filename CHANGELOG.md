# Changelog

Notable changes, newest first, in the spirit of [Keep a Changelog][kac], with
[semantic][semver] versions. The release workflow extracts the section whose
heading starts with the version being tagged — plainly, `## 0.0.1`, not the
bracketed form — so every release needs one and it has to be spelled that way.

[kac]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html

## Unreleased

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
