//! An MVR address is absolute; a console address is a universe and a channel.
//!
//! `<Address break="0">1025</Address>` is universe times 512 plus the channel, and
//! which universe that is depends on where the counting starts. **Universes are
//! numbered from one**, so 1025 is universe 3 channel 1 — that is what grandMA and
//! Vectorworks put in front of an operator, and what this console's patch panel
//! shows. The arithmetic lives here rather than at each call site because the
//! off-by-one is invisible in a patch that happens to start at universe 1 and wrong
//! for every fixture in a rig that does not.
//!
//! Breaks are numbered from zero in MVR and from one here, for the same reason: the
//! console says "break 1" to an operator looking at a fixture whose manual says the
//! same.

/// The size of a DMX universe, which is the only number in here that is not a
/// convention.
pub const UNIVERSE: u32 = 512;

/// Universe and channel, both numbered from one.
pub fn to_universe_and_channel(absolute: u32) -> (u16, u16) {
    // Absolute 0 is not a legal address; treat it as the first, since refusing a
    // fixture over it would lose the rest of what the file says about it.
    let absolute = absolute.max(1);
    let universe = (absolute - 1) / UNIVERSE + 1;
    let channel = (absolute - 1) % UNIVERSE + 1;
    (universe.min(u16::MAX as u32) as u16, channel as u16)
}

/// And back, for export.
pub fn to_absolute(universe: u16, channel: u16) -> u32 {
    (universe.max(1) as u32 - 1) * UNIVERSE + channel.max(1) as u32
}

/// MVR numbers breaks from zero and the console from one.
pub fn to_break(mvr_break: Option<u16>) -> u16 {
    mvr_break.unwrap_or(0).saturating_add(1)
}

/// And back.
pub fn from_break(console_break: u16) -> u16 {
    console_break.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_address_splits_the_way_an_operator_reads_it() {
        assert_eq!(to_universe_and_channel(1), (1, 1));
        assert_eq!(to_universe_and_channel(512), (1, 512));
        assert_eq!(to_universe_and_channel(513), (2, 1));
        // The one that would be silently wrong under the other convention: a real
        // file in the corpus patches an Astera here.
        assert_eq!(to_universe_and_channel(1025), (3, 1));
        assert_eq!(to_universe_and_channel(37), (1, 37));
    }

    #[test]
    fn and_goes_back_to_the_number_it_came_from() {
        for absolute in [1u32, 37, 512, 513, 1025, 1081, 65535] {
            let (universe, channel) = to_universe_and_channel(absolute);
            assert_eq!(to_absolute(universe, channel), absolute, "{absolute}");
        }
    }

    #[test]
    fn a_break_is_numbered_from_one_here_and_from_zero_there() {
        assert_eq!(to_break(Some(0)), 1);
        assert_eq!(to_break(Some(1)), 2);
        assert_eq!(to_break(None), 1, "an unnumbered break is the first");
        assert_eq!(from_break(1), 0);
    }
}
