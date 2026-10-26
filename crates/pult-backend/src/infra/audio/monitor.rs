//! A timeline's sound, rendered for a page that is listening while somebody programs.
//!
//! **Rendered from the show, not tapped from the device.** Where a timeline is is
//! arithmetic over its SYNCED anchor, and every station holds every asset (see
//! `infra::assets::mirror`), so the sound at any show millisecond can be worked out
//! anywhere — and is, here, a lead ahead of the show clock, by the same mixer the
//! callback runs (`pult_audio::Mixer`) with every clip routed to a stereo pair. Three
//! things follow, and each is why it is done this way rather than with a ring out of
//! the audio callback:
//!
//! - **The callback is not touched.** It never locks, allocates or frees, and a
//!   monitor that read out of it would be the first thing in it that could.
//! - **A page listens to its own station.** Any station renders the same sound, so the
//!   tablet's own socket carries it and nothing has to reach across to whichever
//!   station holds the loudspeakers — the hop the wing makes and the monitor need not.
//! - **It hears every clip**, a node's loudspeaker's included, which a tap on this
//!   station's device would not.
//!
//! Each chunk says the show millisecond its first frame belongs to, and the page plays
//! it then by its own estimate of the station clock (`ws/clock.ts`), so what it hears
//! is where the playhead it draws is. Nothing a page does reaches the show.
//!
//! **Asked for, never published.** With nobody listening nothing is rendered; a page
//! that goes takes its receiver with it, and the next chunk sent to it finds that out.
//! Nothing expires.

use pult_audio::monitor::{self, Chunk, CHANNELS, SAMPLE_RATE};
use pult_schema::types::timeline::Timeline;
use tokio::sync::mpsc;
use tracing::debug;
use uuid::Uuid;

use super::{deck_end, folded, placement, AudioManager};

/// Chunks held for a page before the next one is dropped rather than queued: a second
/// of sound. A page that slow is a page that would play them late anyway.
pub const QUEUE: usize = (1_000 / monitor::CHUNK_MS) as usize;

pub struct Listener {
    /// One timeline, or every running one.
    timeline: Option<Uuid>,
    chunks: mpsc::Sender<Vec<u8>>,
    /// The show millisecond the next chunk starts at.
    next: Option<u64>,
    /// Files this page's monitor has asked for that are not decoded here. Each is
    /// tried once, or a file no station can supply would be fetched twenty times a
    /// second for as long as somebody listened.
    tried: std::collections::HashSet<String>,
}

impl Listener {
    pub fn new(timeline: Option<Uuid>, chunks: mpsc::Sender<Vec<u8>>) -> Self {
        Listener { timeline, chunks, next: None, tried: Default::default() }
    }
}

impl AudioManager {
    /// Render and send what each listening page is owed, up to a lead ahead of `now`.
    pub(super) async fn feed_monitors(&mut self, now: u64) {
        if self.monitors.is_empty() {
            return;
        }
        let mut listeners = std::mem::take(&mut self.monitors);
        listeners.retain(|l| !l.chunks.is_closed());
        for listener in &mut listeners {
            let (due, next) = monitor::due(listener.next, now);
            listener.next = Some(next);
            for at in due {
                let Some(samples) = self.monitor_chunk(listener.timeline, &mut listener.tried, at).await else { continue };
                let bytes = monitor::encode(&Chunk { at_ms: at as f64, channels: CHANNELS, sample_rate: SAMPLE_RATE, samples });
                if listener.chunks.try_send(bytes).is_err() {
                    debug!("[audio] a listening page is behind; a monitor chunk was dropped");
                }
            }
        }
        // A page that asked while this ran is in `self.monitors` now; keep both.
        listeners.append(&mut self.monitors);
        self.monitors = listeners;
    }

    /// One chunk of sound at show millisecond `at`, or `None` when nothing it covers is
    /// running — a page is sent silence by being sent nothing.
    async fn monitor_chunk(&mut self, only: Option<Uuid>, tried: &mut std::collections::HashSet<String>, at: u64) -> Option<Vec<f32>> {
        let timelines: Vec<Timeline> = self
            .timelines
            .iter()
            .filter(|t| t.running && only.is_none_or(|id| id == t.id) && !t.clips.is_empty())
            .cloned()
            .collect();
        if timelines.is_empty() {
            return None;
        }
        let mut mixer = pult_audio::Mixer::new(CHANNELS, SAMPLE_RATE, timelines.len());
        for timeline in &timelines {
            let mut voices = Vec::new();
            for clip in &timeline.clips {
                let decoded = match self.cache.get(&clip.asset) {
                    Some(decoded) => decoded,
                    None if tried.insert(clip.asset.clone()) => {
                        let Ok(decoded) = self.decoded(&clip.asset).await else { continue };
                        decoded
                    }
                    None => continue,
                };
                let Ok(routes) = pult_audio::mix::route(decoded.channels as usize, &[1, 2], CHANNELS) else { continue };
                voices.push(pult_audio::Voice {
                    source: pult_audio::Source {
                        samples: decoded.samples.clone(),
                        channels: decoded.channels as usize,
                        sample_rate: decoded.sample_rate,
                    },
                    placement: placement(clip),
                    routes,
                });
            }
            // A count-in still to come: the deck waits in silence until the anchor.
            let (cursor, wait_ms) = match timeline.anchor_ms.checked_sub(at) {
                Some(wait) if wait > 0 => (folded(timeline, timeline.position_at_anchor_ms), wait),
                _ => (folded(timeline, timeline.position_at(at)), 0),
            };
            let mut deck = pult_audio::Deck::new(timeline.id.as_u128(), cursor as f64, voices);
            deck.wait_ms = wait_ms as f64;
            deck.rate = timeline.rate.max(0.0) as f64;
            deck.end = deck_end(timeline);
            let _ = mixer.put(Box::new(deck), false);
        }
        let mut samples = vec![0.0; monitor::chunk_frames() * CHANNELS];
        mixer.render(&mut samples);
        Some(samples)
    }
}
