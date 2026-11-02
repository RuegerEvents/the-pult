//! A strip in `effect` mode, drawn: OpenHaunt doc 14's ten pixel effects.
//!
//! The node draws these itself — the console only tells it the nine parameters — so
//! without this a strip running `comet` is a single lamp on screen, which is the one
//! thing it is not. This is the console's copy of the node's `oh_pixfx.c`, written
//! operation for operation in the same order and in `f32`, so the two round alike.
//!
//! **Two languages, so a corpus between them.** The node firmware generates
//! `testdata/pixel-effects.json` (its `docs/14-pixel-effects.json`, copied here) from
//! the C, checks the C against it exactly, and `tests/pixfx_corpus.rs` checks this
//! against the same file to within one byte — `sinf` and `cosf` are the platform's on
//! both sides and are allowed to differ in their last bit, which can move a byte by one
//! and never by more. An effect changed in the firmware regenerates the corpus there,
//! and this test then says what to follow.
//!
//! The console draws the *ideal* phase, `time × speed` on the show clock, which is what
//! every node converges to (doc 14, "Two nodes agree"); a node's own gentle pull towards
//! it is not modelled, so for the few seconds after a Speed change the two can show the
//! pattern a little apart.

/// The effects a node draws, in the order and spelling of doc 14.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelEffect {
    Solid,
    Gradient,
    Chase,
    Comet,
    Wave,
    Breathe,
    Fire,
    Candle,
    Sparkle,
    Lightning,
}

impl PixelEffect {
    pub const ALL: [PixelEffect; 10] = [
        PixelEffect::Solid,
        PixelEffect::Gradient,
        PixelEffect::Chase,
        PixelEffect::Comet,
        PixelEffect::Wave,
        PixelEffect::Breathe,
        PixelEffect::Fire,
        PixelEffect::Candle,
        PixelEffect::Sparkle,
        PixelEffect::Lightning,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PixelEffect::Solid => "solid",
            PixelEffect::Gradient => "gradient",
            PixelEffect::Chase => "chase",
            PixelEffect::Comet => "comet",
            PixelEffect::Wave => "wave",
            PixelEffect::Breathe => "breathe",
            PixelEffect::Fire => "fire",
            PixelEffect::Candle => "candle",
            PixelEffect::Sparkle => "sparkle",
            PixelEffect::Lightning => "lightning",
        }
    }

    /// A name a node does not know draws as `solid`, the way the node draws it.
    pub fn parse(name: &str) -> PixelEffect {
        PixelEffect::ALL.into_iter().find(|e| e.name() == name).unwrap_or(PixelEffect::Solid)
    }
}

pub const SPEED_MAX: f32 = 10.0;
pub const SEGMENTS_MAX: i32 = 16;

/// The nine parameters, in the node's own units: colors as bytes, Speed in cycles a
/// second, Segments a whole number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelParams {
    pub effect: PixelEffect,
    /// Level 1.
    pub color: [u8; 3],
    /// Level 0.
    pub color2: [u8; 3],
    pub brightness: f32,
    pub speed: f32,
    pub size: f32,
    pub reverse: bool,
    pub mirror: bool,
    pub segments: i32,
}

impl Default for PixelParams {
    /// What every parameter is before the console says anything: the old strip.
    fn default() -> Self {
        PixelParams {
            effect: PixelEffect::Solid,
            color: [0; 3],
            color2: [0; 3],
            brightness: 0.0,
            speed: 1.0,
            size: 0.25,
            reverse: false,
            mirror: false,
            segments: 1,
        }
    }
}

impl PixelParams {
    fn clamped(mut self) -> Self {
        self.brightness = self.brightness.clamp(0.0, 1.0);
        self.speed = self.speed.clamp(0.0, SPEED_MAX);
        self.size = self.size.clamp(0.0, 1.0);
        self.segments = self.segments.clamp(1, SEGMENTS_MAX);
        self
    }
}

// ── Time ──────────────────────────────────────────────────────────────────────

const MICRO: i64 = 1_000_000;

/// Console time × speed in millionths of a cycle, exactly: speed is taken to the
/// milli-hertz so the product is an integer two machines agree on.
fn micro_cycles(speed: f32, now_ms: i64) -> i64 {
    let mhz = (speed * 1000.0).round() as i64;
    now_ms * mhz
}

fn floor_div(a: i64, b: i64) -> i64 {
    a.div_euclid(b)
}

fn frac_of(units: i64) -> f32 {
    (units - floor_div(units, MICRO) * MICRO) as f32 / MICRO as f32
}

/// Where `time × speed` is, 0..1.
pub fn phase_at(speed: f32, now_ms: i64) -> f32 {
    frac_of(micro_cycles(speed, now_ms))
}

fn wrap(x: f32) -> f32 {
    let x = x - x.floor();
    if x >= 1.0 {
        0.0
    } else {
        x
    }
}

// ── Randomness that is the same on every node ─────────────────────────────────

/// splitmix64's finalizer.
fn mix(x: u64) -> u32 {
    let mut x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x as u32
}

fn hash2(a: i64, b: i64) -> u32 {
    mix((a as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ mix(b as u64) as u64)
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise on an integer lattice in space (`x`) and time (`j + tf`).
fn noise2(x: f32, j: i64, tf: f32, salt: u32) -> f32 {
    let xi = x.floor() as i64;
    let xf = x - x.floor();
    let (sx, st) = (smooth(xf), smooth(tf));
    let salt = salt as i64;
    let a = unit(hash2(xi * 31 + salt, j));
    let b = unit(hash2((xi + 1) * 31 + salt, j));
    let c = unit(hash2(xi * 31 + salt, j + 1));
    let d = unit(hash2((xi + 1) * 31 + salt, j + 1));
    let lo = a + (b - a) * sx;
    let hi = c + (d - c) * sx;
    lo + (hi - lo) * st
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn blend(lo: [u8; 3], hi: [u8; 3], level: f32, k: f32, out: &mut [u8]) {
    let level = level.clamp(0.0, 1.0);
    for c in 0..3 {
        let v = lo[c] as f32 + (hi[c] as f32 - lo[c] as f32) * level;
        out[c] = (v * k + 0.5).clamp(0.0, 255.0) as u8;
    }
}

/// Fire's palette: black, then Color at half heat, then Color 2 at full.
fn heat_color(p: &PixelParams, heat: f32, k: f32, out: &mut [u8]) {
    let heat = heat.clamp(0.0, 1.0);
    if heat < 0.5 {
        blend([0; 3], p.color, heat * 2.0, k, out);
    } else {
        blend(p.color, p.color2, (heat - 0.5) * 2.0, k, out);
    }
}

/// Whether a lightning strike is lit `tw_ms` into window `b`.
fn strike_lit(b: i64, window_ms: f32, tw_ms: f32, probability: f32) -> bool {
    if unit(hash2(b, 101)) >= probability {
        return false;
    }
    let mut t = unit(hash2(b, 102)) * 0.6 * window_ms;
    let flashes = 2 + (hash2(b, 103) % 2) as i64;
    for f in 0..flashes {
        let on = 40.0 + unit(hash2(b, 110 + f)) * 40.0;
        if tw_ms >= t && tw_ms < t + on {
            return true;
        }
        t += on + 60.0 + unit(hash2(b, 120 + f)) * 60.0;
    }
    false
}

/// One frame: three bytes a pixel, R G B, with brightness applied and no gamma —
/// exactly what the node puts on the strip. `phase` is normally [`phase_at`].
pub fn render(params: &PixelParams, phase: f32, now_ms: i64, count: u16) -> Vec<u8> {
    let p = params.clamped();
    let k = p.brightness;
    const TWO_PI: f32 = 6.283_185_307_18;
    let mut rgb = vec![0u8; 3 * count as usize];

    let units = micro_cycles(p.speed, now_ms);
    let j = floor_div(units, MICRO);
    let tf = frac_of(units);
    let breathe = 0.5 - 0.5 * (TWO_PI * phase).cos();
    let lightning = p.effect == PixelEffect::Lightning
        && p.speed > 0.0
        && strike_lit(j, 1000.0 / p.speed, tf * 1000.0 / p.speed, p.size);

    for i in 0..count {
        let u = (i as f32 + 0.5) * p.segments as f32 / count as f32;
        let seg = u as i32;
        let mut x = u - seg as f32;
        if p.mirror {
            x = 1.0 - (2.0 * x - 1.0).abs();
        }
        if p.reverse {
            x = 1.0 - x;
        }
        let out = &mut rgb[3 * i as usize..3 * i as usize + 3];

        let level = match p.effect {
            PixelEffect::Solid => 1.0,
            PixelEffect::Gradient => 1.0 - wrap(x - phase),
            PixelEffect::Chase => {
                if wrap(x - phase) < p.size {
                    1.0
                } else {
                    0.0
                }
            }
            PixelEffect::Comet => {
                let d = wrap(phase - x);
                let tail = if p.size > 0.001 { p.size } else { 0.001 };
                if d < tail {
                    (1.0 - d / tail) * (1.0 - d / tail)
                } else {
                    0.0
                }
            }
            PixelEffect::Wave => {
                let waves = 1.0 / if p.size > 0.02 { p.size } else { 0.02 };
                0.5 + 0.5 * (TWO_PI * (x * waves - phase)).sin()
            }
            PixelEffect::Breathe => breathe,
            PixelEffect::Fire => {
                let cells = 1.0 / if p.size > 0.05 { p.size } else { 0.05 };
                let u2 = units * 2;
                let mut heat = noise2(x * cells, j, tf, 7) * 0.75
                    + noise2(x * cells * 2.0, floor_div(u2, MICRO), frac_of(u2), 13) * 0.35;
                heat *= 1.0 - 0.6 * x;
                heat_color(&p, heat, k, out);
                continue;
            }
            PixelEffect::Candle => {
                let (u3, u7) = (units * 3, units * 7);
                let n = noise2(seg as f32 * 3.7, floor_div(u3, MICRO), frac_of(u3), 29) * 0.7
                    + noise2(seg as f32 * 3.7, floor_div(u7, MICRO), frac_of(u7), 31) * 0.3;
                blend([0; 3], p.color, 0.55 + 0.45 * n, k, out);
                continue;
            }
            PixelEffect::Sparkle => {
                // Each pixel has its own offset into the flash period, so they do not
                // all flash on the same frame.
                let mine = units + (hash2(i as i64, 1) % MICRO as u32) as i64;
                let b = floor_div(mine, MICRO);
                let f = frac_of(mine);
                let lit = p.speed > 0.0 && unit(hash2(i as i64, b)) < p.size;
                if lit {
                    (1.0 - f) * (1.0 - f)
                } else {
                    0.0
                }
            }
            PixelEffect::Lightning => {
                if lightning {
                    1.0
                } else {
                    0.0
                }
            }
        };
        blend(p.color2, p.color, level, k, out);
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_is_the_old_strip() {
        let p = PixelParams { color: [200, 100, 50], brightness: 0.5, ..Default::default() };
        let rgb = render(&p, 0.3, 1_760_000_000_000, 4);
        assert_eq!(&rgb[..3], &[100, 50, 25]);
        assert!(rgb.chunks(3).all(|px| px == [100, 50, 25]));
    }

    #[test]
    fn a_name_the_node_does_not_know_is_solid() {
        assert_eq!(PixelEffect::parse("comet"), PixelEffect::Comet);
        assert_eq!(PixelEffect::parse("plasma"), PixelEffect::Solid);
    }

    /// The phase is exact in the console clock's own range: a trillion milliseconds
    /// in, a cycle a second, and a quarter of a second past the cycle is a quarter.
    #[test]
    fn the_phase_is_time_times_speed_without_losing_the_fraction() {
        assert!((phase_at(1.0, 1_760_000_000_250) - 0.25).abs() < 1e-6);
        assert_eq!(phase_at(0.0, 1_760_000_000_250), 0.0);
    }
}
