//! A node's loudspeaker, as far as a console can tell it apart from a real one.
//!
//! The audio extension (`node/docs/13-audio.md`): a clip store addressed by sha256 over
//! HTTP, commands under `openhaunt/<sn>/audio/` over MQTT, an RTP port opened by a
//! `stream start`, and a status message once a second while anything is playing. It
//! makes no sound. What it does is keep what it was sent where a test, and the window
//! onto a simulated node, can read it — every command as it arrived, every clip by the
//! sha its bytes really hash to, and how many stream packets came in.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tracing::{info, warn};

/// What a node with a loudspeaker says about it, when its config does not say more.
/// A V0.2's two outputs.
pub fn default_description() -> Value {
    json!({
        "rate": 48000,
        "voices": 4,
        "outputs": [
            { "id": "line", "name": "Line out", "kind": "line", "channels": 2 },
            { "id": "speaker", "name": "Speaker", "kind": "amp", "channels": 1 },
        ],
        "clips": { "container": "wav", "formats": ["pcm16", "ima-adpcm"],
                   "bytes_total": 10_264_576u64, "bytes_free": 10_264_576u64 },
        "stream": { "payload": "L16", "max_buffer_ms": 1000 },
    })
}

#[derive(Default)]
pub struct Audio {
    clips: Mutex<BTreeMap<String, Vec<u8>>>,
    voices: Mutex<BTreeMap<String, Value>>,
    stream: Mutex<Option<(u16, tokio::task::JoinHandle<()>)>>,
    packets: Arc<std::sync::atomic::AtomicU64>,
    commands: Option<watch::Sender<Vec<(String, Value)>>>,
}

impl Audio {
    pub fn new(commands: watch::Sender<Vec<(String, Value)>>) -> Audio {
        Audio { commands: Some(commands), ..Audio::default() }
    }

    /// The `audio` object of `/api/v1/info`, with the store's budget as it stands.
    pub fn describe(&self, described: &Value) -> Value {
        let mut audio = described.clone();
        let total = audio["clips"]["bytes_total"].as_u64().unwrap_or(10_264_576);
        let used: u64 = self.clips.lock().unwrap().values().map(|c| c.len() as u64).sum();
        audio["clips"]["bytes_free"] = json!(total.saturating_sub(used));
        audio
    }

    pub fn list(&self, described: &Value) -> Value {
        let budget = self.describe(described);
        let clips: Vec<Value> = self
            .clips
            .lock()
            .unwrap()
            .iter()
            .map(|(sha, bytes)| json!({ "sha": sha, "bytes": bytes.len() }))
            .collect();
        json!({
            "clips": clips,
            "bytes_total": budget["clips"]["bytes_total"],
            "bytes_free": budget["clips"]["bytes_free"],
        })
    }

    /// A `PUT`, answered as a node answers it: 201 stored, 200 already there, 400 for
    /// bytes that do not hash to the name or are not a RIFF WAVE, 413 if it does not fit.
    pub fn put(&self, described: &Value, sha: &str, bytes: Vec<u8>) -> u16 {
        if self.clips.lock().unwrap().contains_key(sha) {
            return 200;
        }
        let free = self.describe(described)["clips"]["bytes_free"].as_u64().unwrap_or(0);
        if bytes.len() as u64 > free {
            return 413;
        }
        if hex::encode(Sha256::digest(&bytes)) != sha || bytes.len() < 12 || &bytes[0..4] != b"RIFF" {
            warn!("[sim] a clip did not hash to {sha} or is not a WAV");
            return 400;
        }
        info!("[sim] stored clip {sha:.12} ({} bytes)", bytes.len());
        self.clips.lock().unwrap().insert(sha.to_string(), bytes);
        201
    }

    pub fn delete(&self, sha: &str) -> u16 {
        let playing = self.voices.lock().unwrap().values().any(|v| v["clip"] == json!(sha));
        if playing {
            return 409;
        }
        match self.clips.lock().unwrap().remove(sha) {
            Some(_) => 204,
            None => 404,
        }
    }

    pub fn clip_count(&self) -> usize {
        self.clips.lock().unwrap().len()
    }

    pub fn stream_packets(&self) -> u64 {
        self.packets.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// One command off `openhaunt/<sn>/audio/<verb>`.
    pub async fn command(&self, verb: &str, body: Value) {
        info!("[sim] audio/{verb} {body}");
        if let Some(commands) = &self.commands {
            commands.send_modify(|log| log.push((verb.to_string(), body.clone())));
        }
        match verb {
            "play" => {
                let Some(voice) = body["voice"].as_str() else { return };
                let known = body["clip"].as_str().is_some_and(|c| self.clips.lock().unwrap().contains_key(c));
                if !known {
                    warn!("[sim] play names a clip this node has not got: {}", body["clip"]);
                    return;
                }
                self.voices.lock().unwrap().insert(voice.to_string(), body);
            }
            "stop" => match body["voice"].as_str() {
                Some(voice) => {
                    self.voices.lock().unwrap().remove(voice);
                }
                None => {
                    self.voices.lock().unwrap().clear();
                    self.end_stream();
                }
            },
            "stream" => match body["op"].as_str() {
                Some("start") => {
                    let port = body["port"].as_u64().unwrap_or(5004) as u16;
                    let open = self.stream.lock().unwrap().as_ref().map(|(p, _)| *p);
                    if open == Some(port) {
                        return; // a re-anchor: the same port, the same socket
                    }
                    self.end_stream();
                    match tokio::net::UdpSocket::bind(("0.0.0.0", port)).await {
                        Ok(socket) => {
                            let packets = self.packets.clone();
                            let task = tokio::spawn(async move {
                                let mut buffer = [0u8; 2048];
                                while socket.recv_from(&mut buffer).await.is_ok() {
                                    packets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            });
                            *self.stream.lock().unwrap() = Some((port, task));
                        }
                        Err(e) => warn!("[sim] the stream port {port} could not be opened: {e}"),
                    }
                }
                _ => self.end_stream(),
            },
            _ => {}
        }
    }

    fn end_stream(&self) {
        if let Some((_, task)) = self.stream.lock().unwrap().take() {
            task.abort();
        }
    }

    /// What `audio/status` says. None while nothing is playing, which is when a node
    /// stops publishing it.
    pub fn status(&self) -> Option<Value> {
        let voices: Vec<Value> = self
            .voices
            .lock()
            .unwrap()
            .values()
            .map(|v| json!({ "voice": v["voice"], "clip": v["clip"], "pos_ms": 0, "loop": v["loop"], "gain": v["gain"] }))
            .collect();
        let stream = self.stream.lock().unwrap().as_ref().map(|(port, _)| *port);
        if voices.is_empty() && stream.is_none() {
            return None;
        }
        Some(json!({
            "voices": voices,
            "stream": { "active": stream.is_some(), "port": stream.unwrap_or(0), "depth_ms": 150,
                        "late": 0, "missing": 0, "resyncs": 0 },
            "underruns": 0,
            "master": 1.0,
            "locked": true,
        }))
    }
}
