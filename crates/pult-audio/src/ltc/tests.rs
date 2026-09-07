//! What LTC has to survive, and the arithmetic nobody can check by eye.
//!
//! Two halves. `testdata/ltc.json` is a table of positions and what they are on the
//! clock — the drop-frame rows are the reason it exists, since 29.97df's frame number
//! is not the count of frames elapsed and no amount of staring at the code says
//! whether the correction is applied in the right direction. The rest is the encoder
//! feeding the decoder, at every rate, at four sample rates, upside down and quietly.

use super::*;

#[derive(serde::Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    rate: String,
    hours: u8,
    minutes: u8,
    seconds: u8,
    frames: u8,
    ms: u64,
}

fn rate_named(name: &str) -> LtcRate {
    match name {
        "F24" => LtcRate::F24,
        "F25" => LtcRate::F25,
        "F30" => LtcRate::F30,
        "F2997" => LtcRate::F2997,
        "F2997Df" => LtcRate::F2997Df,
        other => panic!("no such rate: {other}"),
    }
}

#[test]
fn the_corpus_says_where_every_position_is_on_the_clock() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/ltc.json");
    let raw = std::fs::read_to_string(path).expect("testdata/ltc.json");
    let corpus: Corpus = serde_json::from_str(&raw).expect("ltc.json is not readable");
    assert!(corpus.cases.len() > 20, "a corpus this small is not covering the drops");

    for case in corpus.cases {
        let rate = rate_named(&case.rate);
        let got = to_ms(case.hours, case.minutes, case.seconds, case.frames, rate);
        assert_eq!(
            got, case.ms,
            "{} {:02}:{:02}:{:02}:{:02}",
            case.rate, case.hours, case.minutes, case.seconds, case.frames
        );
    }
}

/// An hour of drop-frame is an hour, to within a frame. This is the whole of what the
/// mechanism is for, and it is one assertion.
#[test]
fn an_hour_of_drop_frame_is_an_hour_and_an_hour_of_non_drop_is_not() {
    let dropping = to_ms(1, 0, 0, 0, LtcRate::F2997Df);
    assert!(dropping.abs_diff(3_600_000) < 34, "{dropping} should be an hour");

    let counting = to_ms(1, 0, 0, 0, LtcRate::F2997);
    assert_eq!(counting, 3_603_600, "non-drop counts 30 a second and runs 3.6 s slow an hour");
}

/// Frame numbers `:00` and `:01` are never sent at the top of a minute, so the frame
/// before `:02` is the one before it in time as well.
#[test]
fn the_dropped_numbers_leave_no_gap_on_the_clock() {
    let last = to_ms(0, 0, 59, 29, LtcRate::F2997Df);
    let next = to_ms(0, 1, 0, 2, LtcRate::F2997Df);
    assert_eq!(next - last, 33, "one frame apart, over the drop");

    // And the tenth minute, where nothing is dropped, is also one frame.
    let last = to_ms(0, 9, 59, 29, LtcRate::F2997Df);
    let next = to_ms(0, 10, 0, 0, LtcRate::F2997Df);
    assert_eq!(next - last, 33);
}

fn decode_all(samples: &[f32], sample_rate: u32) -> Vec<LtcFrame> {
    let mut decoder = Decoder::new(sample_rate);
    decoder.push(samples)
}

/// The round trip, at every rate and four sample rates. A generator at 96 kHz and one
/// at 44.1 differ by a factor of two in every interval the decoder measures, which is
/// the whole of what "survives any sample rate" means.
#[test]
fn a_generated_stream_decodes_back_to_the_frames_it_was_made_from() {
    for rate in [LtcRate::F24, LtcRate::F25, LtcRate::F30, LtcRate::F2997, LtcRate::F2997Df] {
        for sample_rate in [44_100u32, 48_000, 88_200, 96_000] {
            let samples = encode(1, 2, 3, 4, rate, 12, sample_rate);
            let frames = decode_all(&samples, sample_rate);
            assert!(
                frames.len() >= 9,
                "{rate:?} at {sample_rate}: only {} frames out of 12",
                frames.len()
            );
            // The first whole frame the decoder reports: the one before it is only
            // partly in the buffer, because a stream starts mid-bit as far as a
            // decoder that has just been switched on is concerned.
            let first = frames[0];
            assert_eq!((first.hours, first.minutes), (1, 2), "{rate:?} at {sample_rate}");
            assert_eq!(first.drop_frame, rate.drops_frames());

            // And they run on, one frame number at a time.
            for pair in frames.windows(2) {
                let expected =
                    advance(pair[0].hours, pair[0].minutes, pair[0].seconds, pair[0].frames, rate);
                assert_eq!(
                    (pair[1].hours, pair[1].minutes, pair[1].seconds, pair[1].frames),
                    expected,
                    "{rate:?} at {sample_rate}: frames are not consecutive"
                );
            }
        }
    }
}

/// Biphase mark carries no polarity, which is what makes a cable wired backwards a
/// fault nobody can hear and nothing here can see.
#[test]
fn an_inverted_signal_decodes_identically() {
    let samples = encode(10, 20, 30, 5, LtcRate::F30, 8, 48_000);
    let inverted: Vec<f32> = samples.iter().map(|s| -s).collect();
    let straight = decode_all(&samples, 48_000);
    let flipped = decode_all(&inverted, 48_000);
    assert_eq!(straight.len(), flipped.len());
    for (a, b) in straight.iter().zip(flipped.iter()) {
        assert_eq!(
            (a.hours, a.minutes, a.seconds, a.frames),
            (b.hours, b.minutes, b.seconds, b.frames)
        );
    }
}

/// A quiet feed and a hot one decode alike, and a DC offset from a cheap interface
/// does not stop it: the crossings are found against a running mean.
#[test]
fn level_and_offset_do_not_matter() {
    let loud = encode(0, 1, 2, 3, LtcRate::F25, 8, 48_000);
    for (gain, offset) in [(0.02f32, 0.0f32), (1.0, 0.0), (0.05, 0.4), (0.3, -0.25)] {
        let fed: Vec<f32> = loud.iter().map(|s| s * gain + offset).collect();
        let frames = decode_all(&fed, 48_000);
        assert!(frames.len() >= 5, "gain {gain} offset {offset}: {} frames", frames.len());
        assert_eq!((frames[0].minutes, frames[0].seconds), (1, 2));
    }
}

/// Timecode arrives in whatever chunks a sound card hands over, and a frame that
/// straddles two of them is still a frame.
#[test]
fn the_decoder_does_not_care_how_the_samples_are_chopped_up() {
    let samples = encode(4, 5, 6, 7, LtcRate::F30, 10, 48_000);
    let whole = decode_all(&samples, 48_000);

    let mut decoder = Decoder::new(48_000);
    let mut chopped = Vec::new();
    // 137 is deliberately not a factor of anything: no chunk lines up with a bit, a
    // frame, or a buffer size any real device would use.
    for chunk in samples.chunks(137) {
        chopped.extend(decoder.push(chunk));
    }
    assert_eq!(whole.len(), chopped.len());
    for (a, b) in whole.iter().zip(chopped.iter()) {
        assert_eq!(
            (a.hours, a.minutes, a.seconds, a.frames),
            (b.hours, b.minutes, b.seconds, b.frames)
        );
    }
}

/// Silence is not timecode, and neither is noise that happens to cross zero a lot.
#[test]
fn nothing_is_decoded_out_of_what_is_not_timecode() {
    assert!(decode_all(&vec![0.0f32; 48_000], 48_000).is_empty());

    let mut state = 12_345u32;
    let noise: Vec<f32> = (0..48_000)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as f32 / 32_768.0 - 1.0
        })
        .collect();
    assert!(decode_all(&noise, 48_000).is_empty(), "noise decoded as a position");
}

/// Lock is a question about how long it has been since a frame, and the caller says
/// how patient it is — four frames at 30 fps is not four frames at 24.
#[test]
fn lock_is_waiting_then_locked_then_lost() {
    let mut decoder = Decoder::new(48_000);
    assert_eq!(decoder.lock(200), Lock::Waiting);

    decoder.push(&encode(0, 0, 10, 0, LtcRate::F30, 4, 48_000));
    assert_eq!(decoder.lock(200), Lock::Locked);

    // Half a second of silence: the frames have stopped.
    decoder.push(&vec![0.0f32; 24_000]);
    assert_eq!(decoder.lock(200), Lock::Lost);
}

/// The sync word is the only pattern that cannot occur in the data, which is what lets
/// a decoder find a frame boundary without being told where one is.
#[test]
fn the_sync_word_is_where_the_frame_ends() {
    let bits = frame_bits(12, 34, 56, 7, false);
    assert_eq!((bits >> 64) as u16, SYNC);
    assert_eq!(bcd(bits, 0, 4), 7, "frame units");
    assert_eq!(bcd(bits, 48, 4), 2, "hour units");
    assert_eq!(bcd(bits, 56, 2), 1, "hour tens");
    assert_eq!(bits >> 10 & 1, 0, "not drop-frame");
    assert_eq!(frame_bits(0, 0, 0, 0, true) >> 10 & 1, 1, "drop-frame");
}
