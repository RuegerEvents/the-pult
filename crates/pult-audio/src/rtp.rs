//! Sound to a node, as it is being played.
//!
//! RTP with an L16 payload (RFC 3551): sixteen-bit signed samples, **big-endian**,
//! interleaved if stereo. The node does nothing with an RTP timestamp except map it to
//! show time through the session it was given (`ts` is heard at `at`), so the numbers
//! here only have to be consistent with themselves: a sequence number that counts
//! packets and a timestamp that counts frames, both wrapping.

/// What the payload type field says. Dynamic, because L16 at anything but 44.1 kHz has
/// no static number, and the node does not read it anyway: the session says what the
/// bytes are.
pub const PAYLOAD_TYPE: u8 = 96;

/// The fixed header's length. No CSRCs and no extension.
pub const HEADER_BYTES: usize = 12;

/// Numbers one stream's packets.
#[derive(Debug, Clone)]
pub struct Packetiser {
    pub ssrc: u32,
    pub sequence: u16,
    /// The RTP timestamp of the next frame to be sent.
    pub timestamp: u32,
}

impl Packetiser {
    pub fn new(ssrc: u32, first_timestamp: u32) -> Packetiser {
        Packetiser { ssrc, sequence: 0, timestamp: first_timestamp }
    }

    /// One packet of interleaved f32 at `channels`, and the counters moved on by it.
    pub fn packet(&mut self, samples: &[f32], channels: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_BYTES + samples.len() * 2);
        out.push(0x80); // version 2, no padding, no extension, no CSRCs
        out.push(PAYLOAD_TYPE & 0x7f);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.ssrc.to_be_bytes());
        for sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&value.to_be_bytes());
        }
        let frames = samples.len() / channels.max(1);
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(frames as u32);
        out
    }
}

/// How many frames go in a packet: as many as keep the payload under 1200 bytes, which
/// leaves room for the IP and UDP headers inside any MTU a venue will have, wifi
/// included. 5 ms of 48 kHz stereo, 10 ms of mono.
pub fn frames_per_packet(sample_rate: u32, channels: usize) -> usize {
    let ten_ms = (sample_rate as usize / 100).max(1);
    let fits = 1_200 / (2 * channels.max(1));
    if ten_ms <= fits { ten_ms } else { (ten_ms / 2).min(fits).max(1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packet_carries_the_header_and_big_endian_samples() {
        let mut rtp = Packetiser::new(0x1234_5678, 1_000);
        let packet = rtp.packet(&[0.5, -0.5, 1.0, -1.0], 2);
        assert_eq!(packet.len(), HEADER_BYTES + 8);
        assert_eq!(packet[0], 0x80);
        assert_eq!(packet[1], PAYLOAD_TYPE);
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 0);
        assert_eq!(u32::from_be_bytes(packet[4..8].try_into().unwrap()), 1_000);
        assert_eq!(u32::from_be_bytes(packet[8..12].try_into().unwrap()), 0x1234_5678);
        assert_eq!(i16::from_be_bytes([packet[12], packet[13]]), 16_384);
        assert_eq!(i16::from_be_bytes([packet[14], packet[15]]), -16_384);
        assert_eq!(i16::from_be_bytes([packet[16], packet[17]]), 32_767);
        // Clamped at -1, so the most negative sample is -32767 rather than wrapping.
        assert_eq!(i16::from_be_bytes([packet[18], packet[19]]), -32_767);
    }

    /// The timestamp counts frames, not samples, and both counters wrap.
    #[test]
    fn the_counters_count_frames_and_wrap() {
        let mut rtp = Packetiser::new(1, u32::MAX - 1);
        rtp.sequence = u16::MAX;
        rtp.packet(&[0.0; 4], 2);
        assert_eq!(rtp.sequence, 0);
        assert_eq!(rtp.timestamp, 0, "two frames on from u32::MAX - 1");
    }

    #[test]
    fn a_packet_fits_any_mtu() {
        assert_eq!(frames_per_packet(48_000, 1), 480);
        assert_eq!(frames_per_packet(48_000, 2), 240);
        for (rate, channels) in [(48_000, 1), (48_000, 2), (44_100, 2), (96_000, 2)] {
            assert!(frames_per_packet(rate, channels) * 2 * channels <= 1_200);
        }
    }
}
