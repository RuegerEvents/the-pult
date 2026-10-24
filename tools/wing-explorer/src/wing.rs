//! The explorer's half of the USB wing: what its page can ask of the frame.
//!
//! The handle, the boot and the loop are `pult_wing::usb`'s, shared with the console.

pub use pult_wing::usb::*;

/// What the world asks of the wing.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Command {
    SetLed { channel: usize, level: u8 },
    SetRgb { r: usize, g: usize, b: usize, color: [u8; 3] },
    AllLeds { level: u8 },
    SetFader { index: usize, value: u16 },
    ReleaseFader { index: usize },
}

/// Apply a command to the frame the sender thread is repeating.
pub fn apply(out: &Shared, cmd: &Command) {
    let mut o = out.lock().unwrap();
    match *cmd {
        Command::SetLed { channel, level } => o.set_led(channel, level),
        Command::SetRgb { r, g, b, color } => o.set_rgb(r, g, b, color),
        Command::AllLeds { level } => o.all_leds(level),
        Command::SetFader { index, value } => o.set_fader(index, value),
        Command::ReleaseFader { index } => o.release_fader(index),
    }
}
