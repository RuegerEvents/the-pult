//! The TCP mode's own framing.
//!
//! ```text
//! uint32 MVR_PACKAGE_HEADER    778682
//! uint32 MVR_PACKAGE_VERSION   1
//! uint32 MVR_PACKAGE_NUMBER    zero based
//! uint32 MVR_PACKAGE_COUNT     how many packages this message is
//! uint32 MVR_PACKAGE_TYPE      0 JSON UTF-8, 1 MVR file
//! uint64 MVR_PAYLOAD_LENGTH
//! char[] MVR_PAYLOAD_BUFFER
//! ```
//!
//! All multi-byte fields big-endian.
//!
//! One thing to know before changing this: **the specification states the field order
//! twice and the two disagree.** Table 66 lists count before number; the byte layout
//! under it lists number before count. The layout is the normative one and is what is
//! implemented here — and, usefully, the disagreement is invisible in the ordinary
//! case, since a message sent as one package has number 0 and count 1, and those two
//! words are the same bytes either way round. It only shows up in a chunked message,
//! which is exactly where a reader would find it hardest to see.
//!
//! Messages are *written* as a single package. Nothing in the specification asks for
//! chunking — the length is 64 bits and one package can carry any file — and a
//! receiver assembling one package is doing the same work as a receiver assembling
//! forty. Chunked messages are still *read*, because other implementations may send
//! them, and [`encode_in_chunks`] exists so that path is tested rather than assumed.

use thiserror::Error;

/// The magic at the head of every package.
pub const PACKAGE_HEADER: u32 = 778682;
/// The only package format version that exists.
pub const PACKAGE_VERSION: u32 = 1;

/// Bytes before the payload: five 32-bit words and a 64-bit length.
const HEADER_BYTES: usize = 4 * 5 + 8;

/// What a payload is, which is the whole of what the framing says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `MVR_PACKAGE_TYPE` 0 — a JSON message.
    Json,
    /// `MVR_PACKAGE_TYPE` 1 — an MVR archive.
    File,
}

impl Kind {
    fn code(self) -> u32 {
        match self {
            Kind::Json => 0,
            Kind::File => 1,
        }
    }

    fn from_code(code: u32) -> Option<Kind> {
        match code {
            0 => Some(Kind::Json),
            1 => Some(Kind::File),
            _ => None,
        }
    }
}

/// One complete message, reassembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    Json(Vec<u8>),
    File(Vec<u8>),
}

impl Payload {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Payload::Json(b) | Payload::File(b) => b,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("not an MVR-xchange package: header was {0}, expected {PACKAGE_HEADER}")]
    BadHeader(u32),
    #[error("package format version {0} is not supported")]
    BadVersion(u32),
    #[error("package type {0} is not a payload kind this build knows")]
    BadKind(u32),
    #[error("package {number} of {count} is not a package anyone can send")]
    BadPackaging { number: u32, count: u32 },
    #[error("a package of kind {got:?} arrived in the middle of a {expected:?} message")]
    KindChanged { expected: Kind, got: Kind },
    #[error("payload of {declared} bytes is over this station's limit of {limit}")]
    TooLarge { declared: u64, limit: u64 },
}

/// Frame one message as a single package.
pub fn encode(kind: Kind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    write_package(&mut out, kind, 0, 1, payload);
    out
}

/// Frame one message as several packages, for testing the reassembly other
/// implementations may need.
///
/// `chunk` of zero, or a payload that fits, gives exactly what [`encode`] gives.
pub fn encode_in_chunks(kind: Kind, payload: &[u8], chunk: usize) -> Vec<u8> {
    if chunk == 0 || payload.len() <= chunk {
        return encode(kind, payload);
    }
    let parts: Vec<&[u8]> = payload.chunks(chunk).collect();
    let count = parts.len() as u32;
    let mut out = Vec::with_capacity(payload.len() + parts.len() * HEADER_BYTES);
    for (number, part) in parts.into_iter().enumerate() {
        write_package(&mut out, kind, number as u32, count, part);
    }
    out
}

fn write_package(out: &mut Vec<u8>, kind: Kind, number: u32, count: u32, payload: &[u8]) {
    out.extend_from_slice(&PACKAGE_HEADER.to_be_bytes());
    out.extend_from_slice(&PACKAGE_VERSION.to_be_bytes());
    out.extend_from_slice(&number.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&kind.code().to_be_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    out.extend_from_slice(payload);
}

/// A TCP stream turned back into messages.
///
/// Fed whatever a read returned, and asked for whatever that completed. A stream has
/// no message boundaries in it, so both halves of that — a package split across two
/// reads, and four packages arriving in one — are ordinary rather than edge cases.
///
/// The **limit is enforced against the declared length, before a byte of payload is
/// kept.** That is the point of having it: a cap applied after reading is a cap that
/// has already cost what it was meant to prevent. A message over the limit is a hard
/// error rather than a skip, because the only way to find the end of a payload this
/// decoder is refusing to buffer is to buffer it.
pub struct Decoder {
    buf: Vec<u8>,
    /// Packages of the message being assembled, and what it is.
    parts: Vec<u8>,
    assembling: Option<(Kind, u32, u32)>,
    limit: u64,
}

impl Decoder {
    /// A decoder that will assemble a message of up to `limit` bytes per package.
    pub fn new(limit: u64) -> Self {
        Decoder { buf: Vec::new(), parts: Vec::new(), assembling: None, limit }
    }

    /// Add what a read returned.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete message, if the bytes so far make one.
    ///
    /// `Ok(None)` means "not yet"; an error means the stream is not this protocol and
    /// the connection carrying it is not recoverable.
    pub fn next(&mut self) -> Result<Option<Payload>, FrameError> {
        loop {
            if self.buf.len() < HEADER_BYTES {
                return Ok(None);
            }
            let header = u32::from_be_bytes(self.buf[0..4].try_into().unwrap());
            if header != PACKAGE_HEADER {
                return Err(FrameError::BadHeader(header));
            }
            let version = u32::from_be_bytes(self.buf[4..8].try_into().unwrap());
            if version != PACKAGE_VERSION {
                return Err(FrameError::BadVersion(version));
            }
            let number = u32::from_be_bytes(self.buf[8..12].try_into().unwrap());
            let count = u32::from_be_bytes(self.buf[12..16].try_into().unwrap());
            let kind_code = u32::from_be_bytes(self.buf[16..20].try_into().unwrap());
            let length = u64::from_be_bytes(self.buf[20..28].try_into().unwrap());

            if count == 0 || number >= count {
                return Err(FrameError::BadPackaging { number, count });
            }
            let Some(kind) = Kind::from_code(kind_code) else {
                return Err(FrameError::BadKind(kind_code));
            };
            if length > self.limit {
                return Err(FrameError::TooLarge { declared: length, limit: self.limit });
            }

            let total = HEADER_BYTES + length as usize;
            if self.buf.len() < total {
                return Ok(None);
            }

            let payload = self.buf[HEADER_BYTES..total].to_vec();
            self.buf.drain(..total);

            match self.assembling {
                Some((expected, _, _)) if expected != kind => {
                    return Err(FrameError::KindChanged { expected, got: kind });
                }
                _ => {}
            }

            if count == 1 {
                self.parts.clear();
                self.assembling = None;
                return Ok(Some(finish(kind, payload)));
            }

            self.parts.extend_from_slice(&payload);
            self.assembling = Some((kind, number, count));
            if number + 1 == count {
                let whole = std::mem::take(&mut self.parts);
                self.assembling = None;
                return Ok(Some(finish(kind, whole)));
            }
            // More packages of this message to come; keep reading whatever else
            // arrived in the same buffer.
        }
    }
}

fn finish(kind: Kind, bytes: Vec<u8>) -> Payload {
    match kind {
        Kind::Json => Payload::Json(bytes),
        Kind::File => Payload::File(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: u64 = 1024 * 1024;

    fn decode_all(bytes: &[u8]) -> Result<Vec<Payload>, FrameError> {
        let mut decoder = Decoder::new(LIMIT);
        decoder.feed(bytes);
        let mut out = Vec::new();
        while let Some(payload) = decoder.next()? {
            out.push(payload);
        }
        Ok(out)
    }

    #[test]
    fn a_package_is_twenty_eight_bytes_and_its_payload() {
        let framed = encode(Kind::Json, b"{}");
        assert_eq!(framed.len(), 28 + 2);
        assert_eq!(&framed[0..4], &778682u32.to_be_bytes());
        assert_eq!(&framed[20..28], &2u64.to_be_bytes());
    }

    #[test]
    fn one_package_reads_back() {
        let framed = encode(Kind::Json, b"{\"Type\":\"MVR_LEAVE\"}");
        assert_eq!(
            decode_all(&framed).unwrap(),
            vec![Payload::Json(b"{\"Type\":\"MVR_LEAVE\"}".to_vec())]
        );
    }

    #[test]
    fn a_file_is_a_kind_of_its_own() {
        let framed = encode(Kind::File, &[0x50, 0x4b, 0x03, 0x04]);
        assert_eq!(decode_all(&framed).unwrap(), vec![Payload::File(vec![0x50, 0x4b, 0x03, 0x04])]);
    }

    #[test]
    fn several_packages_become_one_message() {
        let payload: Vec<u8> = (0..500u32).map(|n| n as u8).collect();
        let framed = encode_in_chunks(Kind::File, &payload, 64);
        assert!(framed.len() > payload.len() + 28, "should have been chunked");
        assert_eq!(decode_all(&framed).unwrap(), vec![Payload::File(payload)]);
    }

    #[test]
    fn a_package_split_across_reads_waits_for_the_rest() {
        let framed = encode(Kind::Json, b"{\"Type\":\"MVR_LEAVE\"}");
        let mut decoder = Decoder::new(LIMIT);
        for byte in &framed[..framed.len() - 1] {
            decoder.feed(&[*byte]);
            assert_eq!(decoder.next().unwrap(), None);
        }
        decoder.feed(&framed[framed.len() - 1..]);
        assert!(decoder.next().unwrap().is_some());
    }

    #[test]
    fn two_messages_in_one_read_are_two_messages() {
        let mut both = encode(Kind::Json, b"{\"a\":1}");
        both.extend(encode(Kind::Json, b"{\"b\":2}"));
        assert_eq!(decode_all(&both).unwrap().len(), 2);
    }

    #[test]
    fn a_payload_over_the_limit_is_refused_before_it_is_read() {
        // Only the header is fed: the refusal must not need the payload to arrive.
        let framed = encode(Kind::File, &[0u8; 64]);
        let mut decoder = Decoder::new(8);
        decoder.feed(&framed[..28]);
        assert_eq!(decoder.next(), Err(FrameError::TooLarge { declared: 64, limit: 8 }));
    }

    #[test]
    fn something_that_is_not_this_protocol_says_so() {
        let mut decoder = Decoder::new(LIMIT);
        decoder.feed(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n padding padding");
        assert!(matches!(decoder.next(), Err(FrameError::BadHeader(_))));
    }

    #[test]
    fn a_package_numbered_past_its_own_count_is_refused() {
        let mut framed = encode(Kind::Json, b"{}");
        framed[8..12].copy_from_slice(&3u32.to_be_bytes());
        assert_eq!(
            decode_all(&framed),
            Err(FrameError::BadPackaging { number: 3, count: 1 })
        );
    }
}
