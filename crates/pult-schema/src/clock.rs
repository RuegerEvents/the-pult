//! The show clock: console milliseconds, and what this station adds to its own to
//! get them.
//!
//! Everything the console is doing is anchored in an absolute millisecond — a fade's
//! `t0`, an effect's anchor, a cue's `went_at` — and every consumer works out a number
//! by evaluating those against *now*. Which is fine on one station and wrong on two:
//! the anchors replicate exactly, the clocks they are read against do not, and a
//! station whose clock is a second behind runs every one of another station's fades a
//! second late. Silently, and each individual value looks plausible — the same failure
//! `frontend/src/lib/ws/clock.ts` exists to prevent between a browser and a station,
//! between stations, unaddressed until now.
//!
//! So there is one reference — the session leader — and a follower holds an offset to
//! it. [`now_ms`] is that offset applied; [`machine_now_ms`] is this machine's own
//! reading and is what an estimate is measured against. The estimating itself is the
//! sync layer's, because only it has a link to measure over; this module is what holds
//! the answer and the rules for applying it.
//!
//! **The rules are the interesting part**, and each of them is a defect that would
//! otherwise be reported as "the rig jumped".
//!
//! *Monotonic, always.* The base is a wall reading taken once plus `Instant::elapsed`,
//! because `SystemTime::now()` can step — NTP correcting a drift mid-show, a laptop
//! waking — and a clock that steps steps every running fade and effect in the rig with
//! it. Correcting towards a reference puts that hazard straight back unless the
//! correction is disciplined, which is what the bands below are.
//!
//! *Three bands.* Under [`APPLY_BELOW_MS`] a change is refresh noise and applies
//! directly. Over [`STEP_ABOVE_MS`] it is not drift but a different clock, and is
//! stepped. Between, it is worked off at [`SLEW_RATE`] — 200 ms in four seconds, with
//! the show running 5% fast or slow while it happens, which is under a lighting cue's
//! perceptual floor and is a great deal less visible than a step.
//!
//! *And never backwards.* A backward step falls before a landed fade's `t1`, and a
//! parameter that had arrived starts moving again — the one thing in the evaluator
//! that cannot happen from any other cause. So a large negative correction is slewed
//! however long it takes rather than stepped, and "a landed fade stays" holds without
//! an exception. Slewing is forward-only by construction: a 5% slow clock still
//! advances.
//!
//! One process-wide offset rather than one per station, because [`now_ms`] is a free
//! function with no station in scope and correcting it corrects everything at once —
//! playback, the connectors, the `at` a Go carries, a log line's `at_ms`, and the
//! answer a browser syncs against, which is how a page inherits show time with no
//! protocol change. The consequence to know is that two stations inside one process
//! share it: harmless, because they also share the machine clock and their real skew
//! is zero, and it is why a test that wants a skew to converge on skews what a station
//! *reports* rather than what it applies.

use std::sync::{OnceLock, RwLock};

/// Below this, a change is the estimate's own noise and is simply applied. Sitting at
/// 20 ms because that is comfortably above what the estimator's own jitter comes to on
/// a LAN and comfortably below an output frame.
pub const APPLY_BELOW_MS: f64 = 20.0;

/// Above this, a *forward* change is not drift but a different clock, and waiting for
/// a slew to walk a second would leave the station knowingly wrong for twenty.
pub const STEP_ABOVE_MS: f64 = 1000.0;

/// How fast the middle band is worked off, as a fraction of real time: 200 ms of skew
/// absorbed in four seconds.
pub const SLEW_RATE: f64 = 0.05;

/// This machine's own console clock, uncorrected.
///
/// A unix millisecond advanced monotonically from a wall reading taken once, rather
/// than read fresh from the system clock each time — see the module docs for why that
/// matters, and note that it is `std`'s `Instant` rather than tokio's on purpose:
/// tokio's is per-runtime, so one process running several runtimes — which is what a
/// test binary is — would read one base against several unrelated clocks. The
/// consequence to know is that pausing tokio's clock does not fast-forward the show.
///
/// The base is taken lazily on first use rather than at startup, so nothing has to
/// remember to initialise it, and a restart re-reads the wall clock as it always did.
///
/// This is what an offset is measured *against*, and it is the reason the arithmetic
/// does not feed back on itself: a sample compares the reference's corrected clock
/// with this station's raw one, so the answer is the whole offset every time rather
/// than a residual to be added to whatever is already applied.
pub fn machine_now_ms() -> u64 {
    struct Base {
        wall_ms: u64,
        at: std::time::Instant,
    }
    static BASE: OnceLock<Base> = OnceLock::new();

    let base = BASE.get_or_init(|| Base {
        wall_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        at: std::time::Instant::now(),
    });
    base.wall_ms.saturating_add(base.at.elapsed().as_millis() as u64)
}

/// Console unix ms: the show clock, which is what everything is anchored in.
pub fn now_ms() -> u64 {
    let raw = machine_now_ms();
    let offset = correction().read().map(|c| c.at(raw)).unwrap_or(0.0);
    if offset >= 0.0 {
        raw.saturating_add(offset as u64)
    } else {
        raw.saturating_sub(-offset as u64)
    }
}

/// What is being added to this machine's clock right now, in milliseconds.
///
/// Mid-slew this is not the same as what the last estimate asked for, which is the
/// point: a station converging says so, and [`converging`] is how a row tells the
/// difference between "I am 12 ms out" and "I am on my way to being 12 ms out".
pub fn offset_ms() -> f64 {
    let raw = machine_now_ms();
    correction().read().map(|c| c.at(raw)).unwrap_or(0.0)
}

/// True while the applied offset is still walking towards the last estimate.
pub fn converging() -> bool {
    correction().read().map(|c| c.applied_ms != c.target_ms).unwrap_or(false)
}

/// When the last estimate landed, as a machine millisecond, or `None` if none ever has.
///
/// `None` is the state that must not look like zero: a station that has never
/// estimated anything is applying zero *provisionally*, which is a different claim
/// from a station that has measured and found itself in step.
pub fn estimated_at_ms() -> Option<u64> {
    correction().read().ok().and_then(|c| c.estimated_at)
}

/// A new estimate of how far the reference's clock is ahead of this machine's.
///
/// Absolute rather than a residual — see [`machine_now_ms`] — so calling this twice
/// with the same answer asks for nothing the second time.
pub fn correct_towards(measured_ms: f64) {
    let raw = machine_now_ms();
    let Ok(mut c) = correction().write() else { return };
    *c = c.corrected_towards(measured_ms, raw);
}

/// Stop correcting, and keep what is applied.
///
/// What a station does on being promoted to leader: it is the reference now, and the
/// offset it had estimated to the leader that just died becomes a standing bias, so
/// the show clock is *continuous* across a failover and nothing in flight moves. The
/// consequence, worth saying out loud, is that after a failover show time is no
/// machine's wall clock — it is a timeline the session carries, which is what it had
/// to become for a fade not to lurch at the exact moment a console failed.
pub fn hold_as_reference() {
    let raw = machine_now_ms();
    let Ok(mut c) = correction().write() else { return };
    c.applied_ms = c.at(raw);
    c.target_ms = c.applied_ms;
    c.since_raw = raw;
}

/// Forget everything, for a test that wants a station's clock as it starts.
#[doc(hidden)]
pub fn reset_for_test() {
    if let Ok(mut c) = correction().write() {
        *c = Correction::default();
    }
}

/// What this station is adding to its own clock, and where it is heading.
#[derive(Debug, Clone, Copy, Default)]
struct Correction {
    applied_ms: f64,
    target_ms: f64,
    /// The machine millisecond at which `applied_ms` was last exactly true.
    since_raw: u64,
    /// When an estimate last landed. `None` means none ever has.
    estimated_at: Option<u64>,
}

impl Correction {
    /// This correction, told how far ahead the reference actually is.
    ///
    /// Pure, and the whole of the band rule, so the corpus exercises the rule rather
    /// than a copy of it and no test needs a clock.
    fn corrected_towards(self, measured_ms: f64, raw: u64) -> Self {
        // Re-base first: whatever a slew in progress has reached is where this
        // correction starts from, or the walk would restart from where the last one
        // began and the clock would jump back by however far it had got.
        let reached = self.at(raw);
        let delta = measured_ms - reached;

        // Noise, or a clock so different that walking to it is not a correction. Both
        // land at once; the second is a forward step, which is a jump to where the
        // show already is rather than a rewrite of what has happened. Everything else
        // — including a backward change of any size — is walked.
        let lands_at_once = delta.abs() < APPLY_BELOW_MS || delta >= STEP_ABOVE_MS;

        Self {
            applied_ms: if lands_at_once { measured_ms } else { reached },
            target_ms: measured_ms,
            since_raw: raw,
            estimated_at: Some(raw),
        }
    }

    /// The offset in force at a machine millisecond, walking a slew forward.
    ///
    /// A pure function of the correction and the time, so reading the clock takes a
    /// read lock and never a write one, and so the bands are testable without a clock
    /// at all.
    fn at(&self, raw: u64) -> f64 {
        let delta = self.target_ms - self.applied_ms;
        if delta == 0.0 {
            return self.applied_ms;
        }
        let elapsed = raw.saturating_sub(self.since_raw) as f64;
        let moved = elapsed * SLEW_RATE;
        if moved >= delta.abs() {
            self.target_ms
        } else {
            self.applied_ms + moved * delta.signum()
        }
    }
}

fn correction() -> &'static RwLock<Correction> {
    static CORRECTION: RwLock<Correction> = RwLock::new(Correction {
        applied_ms: 0.0,
        target_ms: 0.0,
        since_raw: 0,
        estimated_at: None,
    });
    &CORRECTION
}

/// One estimate of a reference clock, from the samples a link has taken.
///
/// The arithmetic is `ws/clock.ts`'s, deliberately and to the millisecond: stamp the
/// question, halve the round trip, and keep the sample whose round trip was *shortest*
/// rather than averaging. A short round trip is the one least likely to have queued in
/// either direction, so its midpoint is the one closest to the truth, and a mean is
/// dragged by exactly the samples worth least.
///
/// Held to that browser implementation by `testdata/clock-offset.json`, which both
/// this crate's tests and `frontend/src/lib/ws/clock.test.ts` read — the rule this
/// repo applies to every other piece of arithmetic written twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// How far ahead of this machine's clock the reference is, in milliseconds.
    pub offset_ms: f64,
    /// The round trip it came from, which is roughly its uncertainty.
    pub rtt_ms: f64,
}

impl Sample {
    /// One sample from an answer: the stamp that went out, what the far end said its
    /// show clock was, and when the answer came back — all machine milliseconds here.
    ///
    /// `offset = station + rtt/2 - received`: the far end's reading, moved forward by
    /// half the round trip to guess where it has got to by the time the answer
    /// arrived, then measured against this machine's own clock.
    pub fn taken(sent_at: u64, station_ms: u64, received_at: u64) -> Self {
        let rtt_ms = received_at.saturating_sub(sent_at) as f64;
        Self {
            offset_ms: station_ms as f64 + rtt_ms / 2.0 - received_at as f64,
            rtt_ms,
        }
    }
}

/// The samples of one estimate in progress, and the best of them.
#[derive(Debug, Clone, Default)]
pub struct Estimate {
    samples: Vec<Sample>,
    /// How many have been taken since this burst began. Counted rather than read off
    /// `samples`, whose length is 1 both at the start of a burst and at the end of
    /// one — and "ask again at once" and "wait thirty seconds" are the two things
    /// that must not be confused.
    taken: usize,
}

/// How many samples make an estimate. Five, as in the browser.
pub const SAMPLES: usize = 5;

impl Estimate {
    /// Add one, and answer the best there is — which is published immediately rather
    /// than after all five, because a rough offset now beats an uncorrected station
    /// for the next four round trips, and the samples after it can only sharpen it.
    pub fn add(&mut self, sample: Sample) -> Sample {
        self.samples.push(sample);
        self.taken += 1;
        let best = self.best();
        if self.taken >= SAMPLES {
            // Keep the best as the floor for the next round: an estimate that has
            // settled should not be talked out of it by one slow refresh.
            self.samples = vec![best];
            self.taken = 0;
        }
        best
    }

    /// True once this burst has taken its full complement, so the link can stop
    /// asking back-to-back and wait for the refresh.
    pub fn is_complete(&self) -> bool {
        self.taken == 0 && !self.samples.is_empty()
    }

    fn best(&self) -> Sample {
        self.samples
            .iter()
            .copied()
            .reduce(|a, b| if b.rtt_ms < a.rtt_ms { b } else { a })
            .unwrap_or(Sample { offset_ms: 0.0, rtt_ms: 0.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corpus both implementations are held to. Cases are samples in, an offset
    /// out; `frontend/src/lib/ws/clock.test.ts` drives that estimator with the same file.
    #[test]
    fn the_corpus_agrees_with_this_estimator() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/clock-offset.json");
        let raw = std::fs::read_to_string(path).expect("testdata/clock-offset.json");
        let corpus: serde_json::Value = serde_json::from_str(&raw).expect("valid json");

        for case in corpus["samples"].as_array().expect("samples") {
            let name = case["name"].as_str().unwrap_or("");
            let mut estimate = Estimate::default();
            let mut best = Sample { offset_ms: 0.0, rtt_ms: 0.0 };
            for s in case["answers"].as_array().expect("answers") {
                best = estimate.add(Sample::taken(
                    s["sentAt"].as_u64().expect("sentAt"),
                    s["stationMs"].as_u64().expect("stationMs"),
                    s["receivedAt"].as_u64().expect("receivedAt"),
                ));
            }
            let want_offset = case["offsetMs"].as_f64().expect("offsetMs");
            let want_rtt = case["rttMs"].as_f64().expect("rttMs");
            assert!(
                (best.offset_ms - want_offset).abs() < 1e-6,
                "{name}: offset {}, wanted {want_offset}",
                best.offset_ms
            );
            assert!(
                (best.rtt_ms - want_rtt).abs() < 1e-6,
                "{name}: rtt {}, wanted {want_rtt}",
                best.rtt_ms
            );
        }
    }

    /// The bands, which are the station's alone: a browser draws a picture and a
    /// station drives a rig, so only one of them has to care what a correction does to
    /// a fade that is running while it lands.
    #[test]
    fn the_corpus_agrees_about_the_bands() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/clock-offset.json");
        let raw = std::fs::read_to_string(path).expect("testdata/clock-offset.json");
        let corpus: serde_json::Value = serde_json::from_str(&raw).expect("valid json");

        for case in corpus["bands"].as_array().expect("bands") {
            let name = case["name"].as_str().unwrap_or("");
            let started = Correction {
                applied_ms: case["appliedMs"].as_f64().expect("appliedMs"),
                target_ms: case["appliedMs"].as_f64().expect("appliedMs"),
                since_raw: 1_000_000,
                estimated_at: None,
            };
            let measured = case["measuredMs"].as_f64().expect("measuredMs");
            let c = started.corrected_towards(measured, started.since_raw);

            for step in case["after"].as_array().expect("after") {
                let elapsed = step["elapsedMs"].as_u64().expect("elapsedMs");
                let want = step["offsetMs"].as_f64().expect("offsetMs");
                let got = c.at(c.since_raw + elapsed);
                assert!(
                    (got - want).abs() < 1e-6,
                    "{name} after {elapsed} ms: offset {got}, wanted {want}"
                );
            }
        }
    }

    #[test]
    fn a_correction_within_the_noise_band_applies_at_once() {
        let c = Correction::default().corrected_towards(12.0, 500);
        assert_eq!(c.applied_ms, 12.0, "no walk to do");
        assert_eq!(c.at(500), 12.0);
    }

    /// The middle band, which is where almost every real correction lands.
    #[test]
    fn a_correction_in_the_middle_band_is_walked() {
        let c = Correction::default().corrected_towards(400.0, 0);
        assert_eq!(c.applied_ms, 0.0, "nothing applied at the moment it lands");
        assert_eq!(c.at(0), 0.0);
        assert_eq!(c.at(4_000), 200.0, "5% of four seconds");
        assert_eq!(c.at(8_000), 400.0, "and it arrives");
        assert_eq!(c.at(80_000), 400.0, "and stays");
    }

    /// The rule that keeps a landed fade landed. A backward correction of any size is
    /// walked, never stepped, so `now_ms` never answers a smaller number than it
    /// answered a moment ago.
    #[test]
    fn a_backward_correction_is_never_stepped() {
        let c = Correction::default().corrected_towards(-5_000.0, 0);
        assert_eq!(c.applied_ms, 0.0, "not stepped, however far back it is");
        assert_eq!(c.at(0), 0.0, "nothing has happened yet");
        assert_eq!(c.at(1_000), -50.0, "5% of a second");
        assert_eq!(c.at(100_000), -5_000.0, "and it arrives eventually");
        // Monotonic: raw + offset never decreases, because the offset falls at a
        // twentieth of the rate the clock rises.
        let a = 1_000.0 + c.at(1_000);
        let b = 2_000.0 + c.at(2_000);
        assert!(b > a, "the show clock still moved forwards");
    }

    #[test]
    fn a_large_forward_correction_is_stepped() {
        let c = Correction::default().corrected_towards(5_000.0, 0);
        assert_eq!(c.applied_ms, 5_000.0, "a jump to where the show already is");
        assert_eq!(c.at(0), 5_000.0, "with nothing left to walk");
    }

    /// A slew that is re-estimated mid-walk carries on from where it had got to.
    /// Restarting it from where the last one began is a clock that jumps backwards,
    /// which is the one thing this module is written to prevent.
    #[test]
    fn a_re_estimate_mid_slew_starts_from_where_the_walk_reached() {
        let c = Correction::default().corrected_towards(400.0, 0);
        assert_eq!(c.at(2_000), 100.0, "5% of two seconds");

        let rebased = c.corrected_towards(420.0, 2_000);
        assert_eq!(rebased.at(2_000), 100.0, "no step at the moment it lands");
        assert_eq!(rebased.at(8_400), 420.0, "and it walks on from there");
    }

    /// The best of a handful, not an average, and published on the first.
    #[test]
    fn the_shortest_round_trip_wins() {
        let mut estimate = Estimate::default();
        assert_eq!(estimate.add(Sample { offset_ms: 100.0, rtt_ms: 40.0 }).offset_ms, 100.0);
        assert_eq!(estimate.add(Sample { offset_ms: 90.0, rtt_ms: 4.0 }).offset_ms, 90.0);
        assert_eq!(
            estimate.add(Sample { offset_ms: 200.0, rtt_ms: 400.0 }).offset_ms,
            90.0,
            "a slow sample does not drag the answer"
        );
    }

    #[test]
    fn a_full_estimate_keeps_its_best_and_is_complete() {
        let mut estimate = Estimate::default();
        for i in 0..SAMPLES {
            assert!(!estimate.is_complete() || i == 0);
            estimate.add(Sample { offset_ms: 10.0, rtt_ms: 10.0 + i as f64 });
        }
        assert!(estimate.is_complete(), "five taken, and the best of them kept");
    }
}
