//! A take recorded from the programmer: the station's half of `model::takes`.
//!
//! **The leader records.** The programmer is SYNCED, so every station sees every
//! write; one has to be the one that stores the take or it is stored once per station.
//! The leader already fires the events and owns the show clock, and when leadership
//! moves mid-pass the old leader stores what it has and the new one starts a pass of
//! its own from there.
//!
//! **A change is timed when the leader's engine applied it**, not when the operator
//! moved the fader. A write from a tablet is a network hop late, a few milliseconds on
//! a wired show network and tens on a busy wifi, which is small beside the reaction
//! time of a hand following a voice and is what a track's `offset_ms` nudge is for.
//! Stamping each write with the page's own `at` would have meant every writer of the
//! programmer — faders, the values panel, the command line, a wing, a plugin —
//! carrying one, for a correction smaller than the thing being recorded.
//!
//! **A jump closes the pass.** A loop wrapping or somebody locating is the playhead
//! moving further than `model::takes::jumped` allows, told from progress exactly the way the timelines
//! pass tells one: where the old transport says the playhead is now, against where the
//! new one says. The pass so far is stored and the next one starts at once, still
//! armed, carrying on from what the programmer held — so a scare looping round is a
//! rehearsal in which every lap that was touched replaces the last. The programmer is
//! released only at the final stop: releasing on a wrap would take the fader out of the
//! operator's hand mid-rehearsal.

use std::collections::BTreeSet;

use pult_schema::{
    lifecycle::Lifecycle,
    path::PathSegment,
    types::{
        programmer::{programmer_entry_id, ProgrammerValue},
        timeline::{RecordSource, TakeStatus, TakeStatuses, Timeline, TimelineTrack},
    },
};
use tracing::{info, warn};
use uuid::Uuid;

use super::InputManager;
use crate::model::takes::{jumped, merge, Pass};

/// A programmer take in progress.
pub(super) struct ProgrammerTake {
    timeline_id: Uuid,
    release: bool,
    pass: Pass,
    /// Every key a stored pass of this take recorded, so the final stop can give them
    /// all back — a looped rehearsal stores a pass per lap.
    recorded: BTreeSet<(Uuid, String)>,
    /// How many passes have been stored.
    passes: u32,
}

impl InputManager {
    /// Start, or finish, a programmer take because the timelines said so.
    pub(super) async fn reconcile_take(&mut self) {
        let wanted: Option<(Uuid, bool)> = self.timelines.iter().find_map(|timeline| {
            let RecordSource::Programmer { release } = timeline.recording? else { return None };
            (timeline.running && self.leading).then_some((timeline.id, release))
        });
        let current = self.take.as_ref().map(|take| take.timeline_id);
        match (current, wanted) {
            (Some(current), Some((wanted, _))) if current == wanted => {}
            (_, Some((timeline_id, release))) => {
                self.finish_take().await;
                if self.assets.is_none() {
                    warn!("[take] cannot record: this console has no show open to put a take in");
                    return;
                }
                info!("[take] recording the programmer into timeline {timeline_id}");
                self.take = Some(ProgrammerTake {
                    timeline_id,
                    release,
                    pass: Pass::new(),
                    recorded: BTreeSet::new(),
                    passes: 0,
                });
                self.publish_takes().await;
            }
            (Some(_), None) => self.finish_take().await,
            (None, None) => {}
        }
    }

    /// A snapshot of the programmer, at the moment the engine applied it.
    pub(super) fn observe_programmer(&mut self, values: &[ProgrammerValue], levels: &[(Uuid, f32)], at_ms: u64) {
        let Some(take) = &mut self.take else { return };
        let Some(timeline) =
            self.timelines.iter().find(|t| t.id == take.timeline_id && t.running)
        else {
            return;
        };
        take.pass.observe_with_levels(timeline.position_at(at_ms), values, levels);
    }

    /// Close the pass if the playhead jumped. Called with the new timelines before
    /// they replace the old ones.
    pub(super) async fn take_jumped(&mut self, incoming: &[Timeline]) {
        let Some(take) = &self.take else { return };
        let id = take.timeline_id;
        let (Some(before), Some(after)) = (
            self.timelines.iter().find(|t| t.id == id && t.running),
            incoming.iter().find(|t| t.id == id && t.running),
        ) else {
            return;
        };
        let now = pult_schema::types::sequence::now_ms();
        let (was, is) = (before.position_at(now), after.position_at(now));
        if !jumped(was, is) {
            return;
        }
        let Some(take) = &mut self.take else { return };
        let next = take.pass.continuing();
        let pass = std::mem::replace(&mut take.pass, next);
        if let Some(touched) = self.store_pass(id, pass, was).await {
            if let Some(take) = &mut self.take {
                take.passes += 1;
                take.recorded.extend(touched);
            }
        }
    }

    /// Store the last pass, give back what the take recorded, and disarm.
    async fn finish_take(&mut self) {
        let Some(mut take) = self.take.take() else { return };
        // Where the playhead is now — the stop position, for a timeline that stopped.
        let out = self
            .timelines
            .iter()
            .find(|t| t.id == take.timeline_id)
            .map_or(0, |t| t.position_at(pult_schema::types::sequence::now_ms()));
        let pass = std::mem::take(&mut take.pass);
        if let Some(touched) = self.store_pass(take.timeline_id, pass, out).await {
            take.passes += 1;
            take.recorded.extend(touched);
        }
        if take.release {
            self.release(&take.recorded).await;
        }
        info!("[take] finished: {} pass(es) stored into timeline {}", take.passes, take.timeline_id);
        self.disarm(take.timeline_id).await;
        self.publish_takes().await;
    }

    /// Join one pass to the takes on the timeline, in one write of its `tracks`.
    ///
    /// Answers the keys it recorded, or `None` where there was nothing to store.
    async fn store_pass(
        &self,
        timeline_id: Uuid,
        pass: Pass,
        punch_out_ms: u64,
    ) -> Option<Vec<(Uuid, String)>> {
        if pass.is_empty() {
            return None;
        }
        let assets = self.assets.clone()?;
        let pass = pass.into_track();
        let touched: Vec<(Uuid, String)> =
            pass.keys.iter().map(|k| (k.fixture_id, k.key.clone())).collect();

        // Read the row back rather than remembering it, for the reason an input's take
        // does: somebody may have added or disabled a take while this one rolled.
        let path = vec![
            PathSegment::Key("timelines".into()),
            PathSegment::Id(timeline_id),
            PathSegment::Key("tracks".into()),
        ];
        let mut tracks: Vec<TimelineTrack> = self
            .engine
            .get(path.clone())
            .await
            .ok()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let mut existing = Vec::new();
        for track in tracks.iter().filter(|t| t.enabled) {
            match assets.get(&track.asset).await {
                Ok(Some(asset)) => match pult_render::decode(&asset.bytes) {
                    Ok(decoded) => existing.push((track.id, decoded)),
                    Err(e) => warn!("[take] {} is not a take this build can read: {e}", track.name),
                },
                // A take whose bytes a peer has not sent yet cannot be spliced into;
                // the pass goes beside it rather than over it.
                _ => {}
            }
        }

        let merged = merge(pass, punch_out_ms, &existing);
        for (track_id, track) in merged.replaced {
            let bytes = pult_render::encode(&track);
            match assets.put(crate::infra::assets::TRACK_MIME, &bytes).await {
                Ok(sha) => {
                    if let Some(row) = tracks.iter_mut().find(|t| t.id == track_id) {
                        row.asset = sha;
                    }
                }
                Err(e) => warn!("[take] a pass could not be stored: {e}"),
            }
        }
        if let Some(track) = merged.fresh {
            let bytes = pult_render::encode(&track);
            match assets.put(crate::infra::assets::TRACK_MIME, &bytes).await {
                Ok(sha) => tracks.push(TimelineTrack {
                    id: Uuid::new_v4(),
                    name: format!("Take {}", tracks.len() + 1),
                    asset: sha,
                    input_id: None,
                    offset_ms: 0,
                    enabled: true,
                }),
                Err(e) => warn!("[take] a pass could not be stored: {e}"),
            }
        }
        match serde_json::to_value(&tracks) {
            Ok(value) => {
                if let Err(e) = self.engine.set(path, Lifecycle::Persisted, value).await {
                    warn!("[take] a pass was stored but could not be added to the timeline: {e}");
                }
            }
            Err(e) => warn!("[take] {e}"),
        }
        info!("[take] pass stored: {} key(s) into timeline {timeline_id}", touched.len());
        Some(touched)
    }

    /// Give the keys a take recorded back to it.
    ///
    /// The way Clear does, entry by entry, and a parked entry stays parked: parking is
    /// exactly the ask that a value survive being given back.
    async fn release(&self, recorded: &BTreeSet<(Uuid, String)>) {
        let held: Vec<ProgrammerValue> = self
            .engine
            .get(vec![PathSegment::Key("programmer_values".into())])
            .await
            .ok()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        for (fixture_id, key) in recorded {
            let Ok(id) = programmer_entry_id(&fixture_id.to_string(), key).parse::<Uuid>() else {
                continue;
            };
            if held.iter().any(|entry| entry.id == id && !entry.locked) {
                let path = vec![
                    PathSegment::Key("programmer_values".into()),
                    PathSegment::Id(id),
                    PathSegment::Key("__delete".into()),
                ];
                let _ = self.engine.set(path, Lifecycle::Synced, serde_json::Value::Null).await;
            }
        }
    }

    /// What is being recorded here, for the timeline panel: the LOCAL `takes` path.
    pub(super) async fn publish_takes(&mut self) {
        let mut takes = TakeStatuses::new();
        if let Some(take) = &self.take {
            takes.insert(
                take.timeline_id.to_string(),
                TakeStatus {
                    source: RecordSource::Programmer { release: take.release },
                    keys: take.pass.keys() as u32,
                    points: take.pass.len() as u32,
                    passes: take.passes,
                },
            );
        }
        if let Some(recording) = &self.recording {
            takes.insert(
                recording.timeline_id.to_string(),
                TakeStatus {
                    source: RecordSource::Input(recording.input_id),
                    keys: recording.points.len() as u32,
                    points: recording.points.values().map(Vec::len).sum::<usize>() as u32,
                    passes: 0,
                },
            );
        }
        if let Ok(json) = serde_json::to_value(&takes) {
            let path = vec![PathSegment::Key("takes".into())];
            let _ = self.engine.set(path, Lifecycle::Local, json).await;
        }
    }
}
