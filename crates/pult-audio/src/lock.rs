//! Chasing somebody else's clock without the show jumping.
//!
//! A console following LTC has two positions: where the timecode says the show is, and
//! where its own playhead has got to. They will not agree — a generator and a sound
//! card are two crystals, and neither of them is right — so the question is not whether
//! to correct but *how*, and the answer is the same shape the show clock's already is.
//!
//! **The same two thresholds `pult_schema::clock` uses**, and deliberately so rather
//! than by coincidence: under [`CLOSE_MS`] the difference is inaudible and invisible,
//! and is taken up by running slightly fast or slow until it is gone; over
//! [`STEP_MS`] nothing can hide it and pretending otherwise only makes the correction
//! longer, so the playhead is moved. A console that stepped on every small drift would
//! click on every step; one that only ever varispeeded would take a minute to catch a
//! locate.
//!
//! **±2% is the ceiling on the ratio**, which is about a third of a semitone. Past that
//! a resampled stem is audibly wrong — a piano is the instrument that gives it away —
//! and a station that has drifted far enough to need more than 2% has been asked for a
//! locate rather than a nudge. It is also what keeps the correction bounded in *time*:
//! at 2%, twenty milliseconds closes in one second.
//!
//! Pure, and it takes two numbers. What it does not take is a clock, which is what lets
//! a test drive a whole chase — lock, drift, locate, drift back — in microseconds.

/// Under this, run fast or slow. The show clock's own figure.
pub const CLOSE_MS: i64 = 20;
/// Over this, move the playhead. The show clock's own figure.
pub const STEP_MS: i64 = 1_000;
/// The most a chase may resample by. See the module header.
pub const MAX_RATIO: f32 = 0.02;

/// What to do about the difference between where timecode says we are and where we are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Chase {
    /// Near enough. Nothing at all — an anchor rewritten every frame is a show clock
    /// nothing downstream can trust.
    Hold,
    /// Play at this ratio of real time until the gap closes. `1.0` is never returned;
    /// that is [`Chase::Hold`].
    Varispeed(f32),
    /// Too far. Put the playhead here.
    Locate(u64),
}

/// What to do, given where the timecode is and where we have got to.
///
/// Both in milliseconds of show position, not of wall time. The ratio is chosen to
/// close the gap in about a second, capped at [`MAX_RATIO`]: a five-millisecond gap
/// closes at half a percent rather than being slammed shut at two, because the
/// smallest correction that works is the one nobody hears.
pub fn chase(timecode_ms: u64, playhead_ms: u64) -> Chase {
    let drift = timecode_ms as i64 - playhead_ms as i64;
    if drift.abs() > STEP_MS {
        return Chase::Locate(timecode_ms);
    }
    if drift.abs() <= CLOSE_MS {
        return Chase::Hold;
    }
    // Close it over a second. `drift` is at most `STEP_MS`, so this saturates at the
    // cap for anything over 20 ms — which is the whole band between the thresholds,
    // and is why the cap is what actually decides the speed in practice.
    let ratio = (drift as f32 / 1_000.0).clamp(-MAX_RATIO, MAX_RATIO);
    Chase::Varispeed(1.0 + ratio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_difference_is_left_alone() {
        assert_eq!(chase(10_000, 10_000), Chase::Hold);
        assert_eq!(chase(10_019, 10_000), Chase::Hold);
        assert_eq!(chase(9_981, 10_000), Chase::Hold);
    }

    #[test]
    fn a_middling_difference_is_taken_up_by_running_fast_or_slow() {
        // Behind the timecode: run fast.
        let Chase::Varispeed(fast) = chase(10_500, 10_000) else { panic!("should varispeed") };
        assert!(fast > 1.0 && fast <= 1.0 + MAX_RATIO);

        // Ahead of it: run slow.
        let Chase::Varispeed(slow) = chase(10_000, 10_500) else { panic!("should varispeed") };
        assert!(slow < 1.0 && slow >= 1.0 - MAX_RATIO);
    }

    /// The cap is the audible one, and it is what actually decides the speed for
    /// anything the varispeed band contains.
    #[test]
    fn the_ratio_never_leaves_two_percent() {
        for drift in [21i64, 100, 500, 999, -21, -100, -999] {
            let base = 60_000i64;
            match chase((base + drift) as u64, base as u64) {
                Chase::Varispeed(ratio) => {
                    assert!((ratio - 1.0).abs() <= MAX_RATIO + 1e-6, "{drift} gave {ratio}");
                    assert!((ratio - 1.0).abs() > 0.0);
                }
                other => panic!("{drift} gave {other:?}"),
            }
        }
    }

    #[test]
    fn a_large_difference_is_a_locate_to_where_the_timecode_says() {
        assert_eq!(chase(120_000, 10_000), Chase::Locate(120_000));
        assert_eq!(chase(0, 10_000), Chase::Locate(0), "and backwards, which is a rewind");
    }

    /// The boundary is exact rather than approximate, because the two thresholds are
    /// the show clock's own and a console that disagreed with itself about what
    /// "20 ms" means would correct one of them twice.
    #[test]
    fn the_thresholds_are_the_show_clocks_own() {
        assert_eq!(chase(20, 0), Chase::Hold, "20 ms is inside");
        assert!(matches!(chase(21, 0), Chase::Varispeed(_)));
        assert!(matches!(chase(1_000, 0), Chase::Varispeed(_)), "a second is still a chase");
        assert_eq!(chase(1_001, 0), Chase::Locate(1_001));
    }
}
