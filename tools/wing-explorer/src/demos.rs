//! Demo programs — small things that make the desk do something visible.
//!
//! Each is a closure over the shared output frame, ticked on its own interval. They
//! exist to prove the output half end to end: an LED map read out of MA's table, a
//! frame the wing accepts, and something a person in the room can see.

use crate::wing::{Shared, SharedInput};
use pult_wing::map::WingMap;
use pult_wing::output::LED_BYTES;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Demo {
    /// Not a program: the absence of one. Nobody paints, so whatever is in the frame
    /// stays there, which is what manual control needs. It exists because `Off` is
    /// *not* that — `Off` paints black thirty times a second and fights a slider into
    /// a flicker. Selected automatically the moment anything is set by hand.
    Manual,
    /// Everything off.
    Off,
    /// Everything on, dim — which is what grandMA3 itself rests at.
    Rest,
    /// A single lit LED walking the frame, so every channel can be seen to be alive
    /// and the table's offsets can be checked against the panel by eye.
    Chase,
    /// Breathe the whole desk up and down.
    Breathe,
    /// Every RGB LED through the hues, each one offset from the last.
    Rainbow,
    /// Light only the keys MA's table gives a name to; the rest stay dark. Which is a
    /// picture of the control map itself.
    Named,
    /// Light whatever is being held. The desk answering its own keys, which is also
    /// the quickest way to see that a key, its LED and MA's table all agree.
    Follow,
}

pub const ALL: &[Demo] = &[
    Demo::Manual,
    Demo::Off,
    Demo::Rest,
    Demo::Chase,
    Demo::Breathe,
    Demo::Rainbow,
    Demo::Named,
    Demo::Follow,
];

impl Demo {
    pub fn slug(self) -> &'static str {
        match self {
            Demo::Manual => "none",
            Demo::Off => "off",
            Demo::Rest => "rest",
            Demo::Chase => "chase",
            Demo::Breathe => "breathe",
            Demo::Rainbow => "rainbow",
            Demo::Named => "named",
            Demo::Follow => "follow",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Demo::Manual => "no program painting - the sliders and the panel own the LEDs",
            Demo::Off => "everything dark",
            Demo::Rest => "everything dim, the way grandMA3 leaves it",
            Demo::Chase => "one lit channel walking the frame",
            Demo::Breathe => "the whole desk up and down",
            Demo::Rainbow => "every RGB led through the hues",
            Demo::Named => "only the keys MA's table names",
            Demo::Follow => "lights whatever key is being held",
        }
    }
}

/// Runs whichever demo is current, and can be told to change.
pub struct Runner {
    out: Shared,
    map: Arc<WingMap>,
    input: SharedInput,
    /// Which hardkey lights which LED channel, worked out once.
    lit_by: Arc<Vec<(usize, u16)>>,
    current: Arc<std::sync::Mutex<Demo>>,
    stop: Arc<AtomicBool>,
}

/// Pair every LED with the hardkey it belongs to.
///
/// MA's two tables do not reference each other — an LED carries a `Code` or an
/// `ExecutorIndex`, and so does a hardkey — so they are joined on whichever the LED
/// has. An LED that matches nothing simply never lights, which is right: some of them
/// are not keys at all.
fn pair_leds_with_keys(map: &WingMap) -> Vec<(usize, u16)> {
    let mut out = Vec::new();
    for led in &map.leds {
        let key = map.assigned_hardkeys().find(|(_, c)| match (&led.code, &led.executor) {
            (Some(code), _) => c.code.as_deref() == Some(code.as_str()),
            (None, Some(ex)) => {
                c.code.as_deref() == Some("EXEC") && c.executor.as_deref() == Some(ex.as_str())
            }
            _ => false,
        });
        if let (Some(channel), Some((index, _))) = (led.r, key) {
            out.push((channel, index));
        }
    }
    out
}

impl Runner {
    pub fn start(out: Shared, map: Arc<WingMap>, input: SharedInput) -> Runner {
        let lit_by = Arc::new(pair_leds_with_keys(&map));
        tracing::info!(pairs = lit_by.len(), "leds paired with keys");
        let r = Runner {
            out,
            map,
            input,
            lit_by,
            current: Arc::new(std::sync::Mutex::new(Demo::Rest)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (out, map, input, lit_by, current, stop) = (
            r.out.clone(),
            r.map.clone(),
            r.input.clone(),
            r.lit_by.clone(),
            r.current.clone(),
            r.stop.clone(),
        );
        std::thread::Builder::new()
            .name("wing-demo".into())
            .spawn(move || {
                let mut tick: u64 = 0;
                while !stop.load(Ordering::Relaxed) {
                    let demo = *current.lock().unwrap();
                    paint(&out, &map, &input, &lit_by, demo, tick);
                    tick = tick.wrapping_add(1);
                    std::thread::sleep(Duration::from_millis(30));
                }
            })
            .expect("demo thread");
        r
    }

    pub fn set(&self, demo: Demo) {
        *self.current.lock().unwrap() = demo;
    }

    pub fn current(&self) -> Demo {
        *self.current.lock().unwrap()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn paint(
    out: &Shared,
    map: &WingMap,
    input: &SharedInput,
    lit_by: &[(usize, u16)],
    demo: Demo,
    tick: u64,
) {
    if demo == Demo::Manual {
        return;
    }
    // Read the desk before taking the output lock, never both at once.
    let held: Vec<(usize, bool)> = if demo == Demo::Follow {
        let state = input.lock().unwrap();
        lit_by.iter().map(|&(ch, key)| (ch, state.key(key))).collect()
    } else {
        Vec::new()
    };
    let mut o = out.lock().unwrap();
    match demo {
        Demo::Manual => {}
        Demo::Off => o.all_leds(0),
        Demo::Rest => o.all_leds(0x1e),
        Demo::Chase => {
            o.all_leds(0);
            o.set_led((tick as usize) % LED_BYTES, 0xff);
        }
        Demo::Breathe => {
            // A triangle rather than a sine: no float, and it reads the same.
            let phase = (tick % 120) as i32;
            let level = if phase < 60 { phase } else { 120 - phase };
            o.all_leds((level * 255 / 60) as u8);
        }
        Demo::Rainbow => {
            o.all_leds(0);
            let rgb: Vec<_> = map.leds.iter().filter(|l| l.is_rgb()).collect();
            for (i, led) in rgb.iter().enumerate() {
                let hue = ((tick * 3 + (i as u64 * 256 / rgb.len().max(1) as u64)) % 256) as u8;
                let (r, g, b) = wheel(hue);
                if let (Some(rc), Some(gc), Some(bc)) = (led.r, led.g, led.b) {
                    o.set_rgb(rc, gc, bc, [r, g, b]);
                }
            }
        }
        Demo::Follow => {
            o.all_leds(0);
            for &(channel, down) in &held {
                o.set_led(channel, if down { 0xff } else { 0x0a });
            }
        }
        Demo::Named => {
            o.all_leds(0);
            for led in &map.leds {
                if led.code.is_some() {
                    if let Some(c) = led.r {
                        o.set_led(c, 0xc0);
                    }
                }
            }
        }
    }
}

/// Hue to RGB, in integers. Good enough for a light; nothing here is colour science.
fn wheel(h: u8) -> (u8, u8, u8) {
    match h {
        0..=84 => (255 - h * 3, h * 3, 0),
        85..=169 => {
            let h = h - 85;
            (0, 255 - h * 3, h * 3)
        }
        _ => {
            let h = h - 170;
            (h * 3, 0, 255 - h * 3)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_stays_in_range_and_wraps() {
        for h in 0..=255u8 {
            let (r, g, b) = wheel(h);
            // Every hue lights something; a black spot in a rainbow reads as a fault.
            assert!(r as u16 + g as u16 + b as u16 > 200, "hue {h} is nearly black");
        }
    }

    #[test]
    fn every_demo_has_a_slug_and_a_description() {
        for d in ALL {
            assert!(!d.slug().is_empty());
            assert!(!d.describe().is_empty());
        }
    }
}
