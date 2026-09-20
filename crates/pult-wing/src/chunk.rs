//! The grammar, and there is only one rule.
//!
//! ```text
//! chunk := u16 tag      little endian
//!          u16 len      little endian; bit 15 set means the body is more chunks
//!          len bytes body
//! ```
//!
//! A frame is one chunk whose tag is [`ROOT`] — `0x5342`, ASCII `BS`, which is why
//! every frame on the wire begins `42 53`. So a frame is exactly `4 + len` bytes and
//! nothing about it has to be guessed from the transfer size.
//!
//! What looked like a 24-byte header before the grammar was known is not one: it is the
//! root's four bytes, plus the eight-byte sender address of [`ADDRESS`], plus the next
//! chunk's own four.

use std::fmt;

/// `0x5342`, ASCII `BS`. The tag of the frame itself.
pub const ROOT: u16 = 0x5342;
/// The sender's `ID8`, eight bytes, first child of every frame.
pub const ADDRESS: u16 = 0x0024;

/// Set in the length field when the body is more chunks rather than bytes.
const SUBCHUNKS: u16 = 0x8000;
const LEN_MASK: u16 = 0x7fff;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChunkError {
    #[error("frame is {0} bytes, too short to hold a chunk header")]
    Short(usize),
    #[error("chunk 0x{tag:04x} says {len} bytes but only {have} are left")]
    Overrun { tag: u16, len: usize, have: usize },
    #[error("frame root is 0x{0:04x}, not 0x5342")]
    NotAFrame(u16),
    #[error("frame declares {declared} bytes, got {actual}")]
    Length { declared: usize, actual: usize },
    #[error("chunk 0x{0:04x} holds sub-chunks, not bytes")]
    NotBytes(u16),
    #[error("chunk 0x{0:04x} holds bytes, not sub-chunks")]
    NotChunks(u16),
    #[error("chunk 0x{tag:04x} is {len} bytes, expected {want}")]
    Size { tag: u16, len: usize, want: usize },
}

/// One chunk: a tag, and a body that is either bytes or more chunks.
#[derive(Clone, PartialEq, Eq)]
pub enum Chunk {
    Bytes { tag: u16, bytes: Vec<u8> },
    Group { tag: u16, children: Vec<Chunk> },
}

impl Chunk {
    pub fn bytes(tag: u16, bytes: impl Into<Vec<u8>>) -> Self {
        Chunk::Bytes { tag, bytes: bytes.into() }
    }

    pub fn group(tag: u16, children: impl Into<Vec<Chunk>>) -> Self {
        Chunk::Group { tag, children: children.into() }
    }

    /// A `u32` payload, which is what most leaf chunks in this protocol are.
    pub fn u32(tag: u16, v: u32) -> Self {
        Chunk::bytes(tag, v.to_le_bytes().to_vec())
    }

    pub fn tag(&self) -> u16 {
        match self {
            Chunk::Bytes { tag, .. } | Chunk::Group { tag, .. } => *tag,
        }
    }

    pub fn as_bytes(&self) -> Result<&[u8], ChunkError> {
        match self {
            Chunk::Bytes { bytes, .. } => Ok(bytes),
            Chunk::Group { tag, .. } => Err(ChunkError::NotBytes(*tag)),
        }
    }

    pub fn children(&self) -> Result<&[Chunk], ChunkError> {
        match self {
            Chunk::Group { children, .. } => Ok(children),
            Chunk::Bytes { tag, .. } => Err(ChunkError::NotChunks(*tag)),
        }
    }

    /// The first child with this tag, at this level only.
    pub fn child(&self, tag: u16) -> Option<&Chunk> {
        self.children().ok()?.iter().find(|c| c.tag() == tag)
    }

    /// A child's `u32`, which is how nearly every scalar in this protocol is carried.
    pub fn child_u32(&self, tag: u16) -> Option<u32> {
        let b = self.child(tag)?.as_bytes().ok()?;
        (b.len() >= 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A child's NUL-terminated ASCII, as the device type and file names are carried.
    pub fn child_str(&self, tag: u16) -> Option<String> {
        let b = self.child(tag)?.as_bytes().ok()?;
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        Some(String::from_utf8_lossy(&b[..end]).into_owned())
    }

    /// A child's bytes, insisting on a length. Blocks have fixed sizes and a block of
    /// the wrong size is a decode error rather than something to pad or truncate.
    pub fn child_block(&self, tag: u16, want: usize) -> Result<&[u8], ChunkError> {
        let c = self.child(tag).ok_or(ChunkError::Size { tag, len: 0, want })?;
        let b = c.as_bytes()?;
        if b.len() != want {
            return Err(ChunkError::Size { tag, len: b.len(), want });
        }
        Ok(b)
    }

    fn encoded_len(&self) -> usize {
        4 + match self {
            Chunk::Bytes { bytes, .. } => bytes.len(),
            Chunk::Group { children, .. } => children.iter().map(Chunk::encoded_len).sum(),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        let body = self.encoded_len() - 4;
        // An *empty* container is written without the bit, because that is what
        // grandMA3 writes: its capabilities request is `25 00 00 00`, not
        // `25 00 00 80`. The wing stalls its IN pipe on the latter, which is a
        // remarkably quiet way to be told the difference matters.
        let flag = match self {
            Chunk::Group { children, .. } if !children.is_empty() => SUBCHUNKS,
            _ => 0,
        };
        out.extend_from_slice(&self.tag().to_le_bytes());
        out.extend_from_slice(&((body as u16 & LEN_MASK) | flag).to_le_bytes());
        match self {
            Chunk::Bytes { bytes, .. } => out.extend_from_slice(bytes),
            Chunk::Group { children, .. } => children.iter().for_each(|c| c.write(out)),
        }
    }

    /// The bytes of one frame, ready for the OUT pipe.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
        self.write(&mut out);
        out
    }

    /// One frame, as read from the IN pipe.
    ///
    /// The declared length is checked against what arrived rather than trusted: a bulk
    /// read that returned short is a truncated frame, and decoding it would invent a
    /// block that was never sent.
    pub fn decode(buf: &[u8]) -> Result<Chunk, ChunkError> {
        let (c, used) = parse(buf)?;
        if c.tag() != ROOT {
            return Err(ChunkError::NotAFrame(c.tag()));
        }
        if used != buf.len() {
            return Err(ChunkError::Length { declared: used, actual: buf.len() });
        }
        Ok(c)
    }
}

fn parse(buf: &[u8]) -> Result<(Chunk, usize), ChunkError> {
    if buf.len() < 4 {
        return Err(ChunkError::Short(buf.len()));
    }
    let tag = u16::from_le_bytes([buf[0], buf[1]]);
    let raw = u16::from_le_bytes([buf[2], buf[3]]);
    let len = (raw & LEN_MASK) as usize;
    let have = buf.len() - 4;
    if len > have {
        return Err(ChunkError::Overrun { tag, len, have });
    }
    let body = &buf[4..4 + len];
    let chunk = if raw & SUBCHUNKS != 0 {
        let mut children = Vec::new();
        let mut off = 0;
        while off < body.len() {
            let (c, used) = parse(&body[off..])?;
            children.push(c);
            off += used;
        }
        Chunk::Group { tag, children }
    } else {
        Chunk::Bytes { tag, bytes: body.to_vec() }
    };
    Ok((chunk, 4 + len))
}

impl fmt::Debug for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Chunk::Bytes { tag, bytes } => write!(f, "0x{tag:04x}[{}]", bytes.len()),
            Chunk::Group { tag, children } => {
                write!(f, "0x{tag:04x}{children:?}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heartbeat, byte for byte off the wire.
    const HEARTBEAT: &[u8] = &[
        0x42, 0x53, 0x18, 0x80, // root, container, 24 bytes
        0x24, 0x00, 0x08, 0x00, // address, 8 bytes
        0, 0, 0, 0, 0, 0, 0, 0, // the wing sends zeros
        0x23, 0x00, 0x08, 0x80, // heartbeat, container, 8 bytes
        0x01, 0x00, 0x04, 0x00, // one u32
        0x01, 0x00, 0x00, 0x00, // device state 1
    ];

    #[test]
    fn a_frame_is_four_bytes_plus_its_length() {
        let c = Chunk::decode(HEARTBEAT).unwrap();
        assert_eq!(c.tag(), ROOT);
        assert_eq!(c.children().unwrap().len(), 2);
        assert_eq!(c.child(ADDRESS).unwrap().as_bytes().unwrap(), &[0u8; 8]);
        assert_eq!(c.child(0x0023).unwrap().child_u32(0x0001), Some(1));
    }

    #[test]
    fn round_trips_byte_for_byte() {
        let c = Chunk::decode(HEARTBEAT).unwrap();
        assert_eq!(c.encode(), HEARTBEAT);
    }

    /// A short read is a truncated frame. Decoding it anyway would hand the caller a
    /// block the wing never finished sending, which is worse than an error.
    /// grandMA3's capabilities request, byte for byte. An empty container carries no
    /// sub-chunk bit and the wing insists on it.
    #[test]
    fn an_empty_container_carries_no_subchunk_bit() {
        let c = Chunk::group(0x0025, vec![]);
        assert_eq!(c.encode(), vec![0x25, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn refuses_a_truncated_frame() {
        let short = &HEARTBEAT[..HEARTBEAT.len() - 2];
        assert!(matches!(Chunk::decode(short), Err(ChunkError::Overrun { .. })));
    }

    #[test]
    fn refuses_something_that_is_not_a_frame() {
        let mut bad = HEARTBEAT.to_vec();
        bad[0] = 0x41;
        assert!(matches!(Chunk::decode(&bad), Err(ChunkError::NotAFrame(_))));
    }

    /// Trailing bytes mean the read held more than one frame, or held rubbish. Either
    /// way the caller wanted one frame and did not get exactly one.
    #[test]
    fn refuses_trailing_bytes() {
        let mut extra = HEARTBEAT.to_vec();
        extra.push(0);
        assert!(matches!(Chunk::decode(&extra), Err(ChunkError::Length { .. })));
    }
}
