//! Recordings, decoded once and shared.
//!
//! A track is an asset: immutable bytes under a sha, which is what makes caching it
//! trivial and correct — a sha that is here is the same recording it was the first
//! time, for ever. What is *not* trivial is where the cache lives, and this is why it
//! is its own thing rather than a field on the output manager.
//!
//! Two places read a track and they are on opposite sides of the console. The output
//! manager reads one every frame, to put what it is asserting on a wire. The **engine**
//! reads the same one at exactly one moment: when a timeline stops, the keys that
//! recording was holding have to go home from where the recording had got to, which
//! means sampling it at the stop position. Two caches would decode a forty-minute take
//! twice for a show that plays it once, and would let the two disagree about what a
//! stopping track was last showing — which is the one moment they must not.
//!
//! Nothing is ever evicted. A show carries as many recordings as an operator made, and
//! the alternative — a size bound — is a rig where the first bar of a song is
//! occasionally silent while a file is read again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pult_render::Track;

use super::assets::AssetStore;

/// The recordings this station has decoded, by sha.
#[derive(Clone)]
pub struct TrackCache {
    assets: AssetStore,
    loaded: Arc<Mutex<HashMap<String, Option<Arc<Track>>>>>,
}

impl TrackCache {
    pub fn new(assets: AssetStore) -> Self {
        TrackCache { assets, loaded: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// One recording, decoded.
    ///
    /// A sha that is not here, or whose bytes do not parse, is remembered as **`None`
    /// rather than as a miss**, so a track a peer has not sent yet is not re-read from
    /// the disk forty times a second for the whole of the show it is missing from. It
    /// comes back when [`TrackCache::forget`] is called, which is what a fetch from a
    /// peer does when it lands.
    pub async fn get(&self, sha: &str) -> Option<Arc<Track>> {
        if let Some(known) = self.peek(sha) {
            return known;
        }
        let decoded = match self.assets.get(sha).await {
            Ok(Some(asset)) => match pult_render::decode(&asset.bytes) {
                Ok(track) => Some(Arc::new(track)),
                Err(e) => {
                    tracing::warn!("[tracks] {sha} is not a recording this build can read: {e}");
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("[tracks] {sha} could not be read: {e}");
                None
            }
        };
        if let Ok(mut loaded) = self.loaded.lock() {
            loaded.insert(sha.to_string(), decoded.clone());
        }
        decoded
    }

    /// What is already decoded, without touching the disk. `None` means "not looked
    /// at yet"; `Some(None)` means "looked at, and not here".
    pub fn peek(&self, sha: &str) -> Option<Option<Arc<Track>>> {
        self.loaded.lock().ok()?.get(sha).cloned()
    }

    /// Forget one sha, so the next read goes back to the store.
    #[allow(dead_code, reason = "for the peer fetch, which does not carry tracks yet")]
    pub fn forget(&self, sha: &str) {
        if let Ok(mut loaded) = self.loaded.lock() {
            loaded.remove(sha);
        }
    }
}
