# The beat detector's models

Two ONNX files, both MIT, both checked in exactly as they were published — byte for
byte, which is what makes the sha256 in `beats.rs` mean anything.

| File | Size | What it is |
| --- | --- | --- |
| `mel_spectrogram.onnx` | ~270 KB | The log-mel front end. An ONNX model rather than arithmetic in Rust, so the spectrogram this console computes is numerically the same one the network was trained against. |
| `beat_this_small.onnx` | ~10 MB | The small Beat This! beat and downbeat model. What a console has with no internet. |

The **full-accuracy model** (`beat_this.onnx`, ~83 MB) is deliberately *not* here. It is
more than the rest of the console's binary put together, and most rigs will never ask
for it; `timeline.model { full: true }` fetches it into the station's config directory
and verifies it against `pult_audio::beats::FULL_MODEL_SHA256`.

## Where they came from

Both files are the ONNX exports published by **beat-this-rs**
(<https://github.com/danigb/beat-this-rs>, MIT, © Daniel Gómez), converted from the
official checkpoints of **Beat This!** (<https://github.com/CPJKU/beat_this>, MIT,
© Institute of Computational Perception, JKU Linz) — Foscarin, Schlüter and Widmer,
*"Beat This! Accurate and Generalizable Beat Tracking"*, ISMIR 2024,
<https://arxiv.org/abs/2407.21658>.

`LICENSE-beat-this` beside this file is the MIT licence both projects are under. The
port of the inference path itself lives in `../src/beats.rs` and carries its attribution
in the module header.

## Why they are `.onnx` and not `.rten`

`rten` 0.26 loads ONNX directly (`Model::load_static_slice`), so there is nothing to
convert — which also means no Python toolchain anywhere in this repository's build, and
a file whose checksum can be compared against the one upstream publishes. A converted
`.rten` would be a fourth artefact nobody could check against anything.
