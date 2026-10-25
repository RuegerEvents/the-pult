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
use pult_schema::types::timeline::TimelineTrack;
use uuid::Uuid;

use super::assets::AssetStore;

/// The recordings this station has decoded, by sha, and what each timeline's rows of
/// them compose to.
#[derive(Clone)]
pub struct TrackCache {
    assets: AssetStore,
    loaded: Arc<Mutex<HashMap<String, Option<Arc<Track>>>>>,
    /// Per timeline, the rows it was composed from and what they came to. One entry
    /// per timeline rather than per set of rows, because dragging a crop rewrites the
    /// rows every frame and every one of those would otherwise be kept.
    composed: Arc<Mutex<HashMap<Uuid, (String, Arc<Track>)>>>,
}

impl TrackCache {
    pub fn new(assets: AssetStore) -> Self {
        TrackCache {
            assets,
            loaded: Arc::new(Mutex::new(HashMap::new())),
            composed: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// What a timeline's enabled rows play, composed (`pult_render::compose`), and
    /// whether every recording they point at was here. An incomplete composition is
    /// what is here, played rather than nothing, and not kept: the next look tries the
    /// missing ones again.
    pub async fn compose(&self, timeline_id: Uuid, rows: &[TimelineTrack]) -> (Arc<Track>, bool) {
        let enabled: Vec<&TimelineTrack> = rows.iter().filter(|row| row.enabled).collect();
        let signature = serde_json::to_string(&enabled).unwrap_or_default();
        if let Some(composed) = self.cached(timeline_id, &signature) {
            return (composed, true);
        }
        let mut decoded = Vec::with_capacity(enabled.len());
        let mut complete = true;
        for row in &enabled {
            match self.get(&row.asset).await {
                Some(track) => decoded.push((*row, track)),
                None => complete = false,
            }
        }
        let composed = Arc::new(compose(&decoded));
        if complete {
            self.keep(timeline_id, signature, composed.clone());
        }
        (composed, complete)
    }

    /// [`TrackCache::compose`] from what is already decoded, for the evaluations that
    /// happen at one moment and must not wait on a disk.
    pub fn compose_now(&self, timeline_id: Uuid, rows: &[TimelineTrack]) -> Arc<Track> {
        let enabled: Vec<&TimelineTrack> = rows.iter().filter(|row| row.enabled).collect();
        let signature = serde_json::to_string(&enabled).unwrap_or_default();
        if let Some(composed) = self.cached(timeline_id, &signature) {
            return composed;
        }
        let decoded: Vec<(&TimelineTrack, Arc<Track>)> = enabled
            .iter()
            .filter_map(|row| Some((*row, self.peek(&row.asset)??)))
            .collect();
        Arc::new(compose(&decoded))
    }

    fn cached(&self, timeline_id: Uuid, signature: &str) -> Option<Arc<Track>> {
        let composed = self.composed.lock().ok()?;
        composed.get(&timeline_id).filter(|(was, _)| was == signature).map(|(_, track)| track.clone())
    }

    fn keep(&self, timeline_id: Uuid, signature: String, track: Arc<Track>) {
        if let Ok(mut composed) = self.composed.lock() {
            composed.insert(timeline_id, (signature, track));
        }
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
            // Not here *yet* is not remembered: a follower gets a take's file from the
            // leader seconds after the take exists, and remembering the miss meant it
            // never played it until it restarted (found at the gig). Only the output
            // manager's patch build and its once-a-second retry read through here.
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!("[tracks] {sha} could not be read: {e}");
                return None;
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

/// Rows over their recordings, composed, in list order.
fn compose(rows: &[(&TimelineTrack, Arc<Track>)]) -> Track {
    let parts: Vec<pult_render::Part<'_>> = rows.iter().map(|(row, track)| row.part(track)).collect();
    pult_render::compose(&parts)
}
