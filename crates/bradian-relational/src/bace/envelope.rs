//! BACE compressed envelope v1 byte layout (accepted ADR 0062).
//!
//! On-disk layout:
//!
//! ```text
//! ┌──────────────────────────────┬──────────────────────┬────────────────┐
//! │ FIXED HEADER  (32 bytes)     │ PAYLOAD (encoded_len)│ TRAILER (4 B)  │
//! │ incl. header CRC32 at 28..=31│   codec-specific     │  payload CRC32 │
//! └──────────────────────────────┴──────────────────────┴────────────────┘
//! ```
//!
//! All multi-byte integers are little-endian (matches `page.rs` and
//! `row_store.rs`).
//!
//! This module implements the byte codec and the fail-closed read
//! protocol steps 1–5 and 8–12 from ADR 0062 §4. Steps 6–7 (codec
//! resolution) and 13–14 (bounded decode) are the codec/registry
//! layer's responsibility (Component Q.3 and beyond).

use crc32fast::Hasher;
use thiserror::Error;

use super::codec::CodecId;

/// ASCII `"BACE"` as a little-endian byte sequence on disk. ADR 0062 §9
/// item 1 reserves `"BACA"` for the future authenticated variant; this
/// module never emits that magic.
pub const ENVELOPE_MAGIC: [u8; 4] = *b"BACE";

/// Current frozen envelope version. ADR 0062 §3.2.
pub const ENVELOPE_VERSION: u16 = 1;

/// Fixed header length per ADR 0062 §3.2.
pub const ENVELOPE_HEADER_SIZE: usize = 32;

/// Trailer CRC32 length per ADR 0062 §3.5.
pub const ENVELOPE_TRAILER_SIZE: usize = 4;

/// Minimum envelope size (empty payload is permitted per ADR 0062 §9
/// item 6 / review package X2).
pub const ENVELOPE_MIN_SIZE: usize = ENVELOPE_HEADER_SIZE + ENVELOPE_TRAILER_SIZE;

/// The threshold at which vendor flag bits are permitted per
/// ADR 0062 §9 item 5 / review package A2. Codec IDs at or above this
/// value may set bits `[16..=31]` of the `flags` field.
pub const VENDOR_FLAG_THRESHOLD: u16 = 256;

/// Flag bits per ADR 0062 §3.3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnvelopeFlags {
    /// Bit 0 — payload is uncompressed (BACE-SEL-002 avoid-expansion).
    /// The `codec_id` still names the codec that was considered.
    pub payload_is_raw: bool,
    /// Bit 1 — producer considered this codec critical. A reader that
    /// cannot resolve `codec_id` fails closed (ADR 0061 §3.6).
    pub critical: bool,
    /// Bit 2 — producer expects an outer authenticated-encryption
    /// wrapper. A standalone reader not inside an authenticated
    /// wrapper, under a trust policy that enforces this bit, MUST
    /// refuse (ADR 0062 §3.3, §7).
    pub encryption_ordered: bool,
    /// Bit 3 — codec references a dictionary stored outside this
    /// envelope. Dictionary scope is a future ADR; this flag MUST
    /// round-trip until that ADR lands.
    pub has_sidecar_dictionary: bool,
    /// Bits 16..=31 — vendor-reserved. Permitted only when
    /// `codec_id >= VENDOR_FLAG_THRESHOLD` (ADR 0062 §9 item 5).
    /// Preserved verbatim on round-trip for forward compatibility.
    pub vendor_bits: u16,
}

impl EnvelopeFlags {
    /// Encode the struct into the `u32 LE` on-disk flags field.
    pub fn to_bits(self) -> u32 {
        let mut bits = 0_u32;
        if self.payload_is_raw { bits |= 1 << 0; }
        if self.critical { bits |= 1 << 1; }
        if self.encryption_ordered { bits |= 1 << 2; }
        if self.has_sidecar_dictionary { bits |= 1 << 3; }
        bits |= (self.vendor_bits as u32) << 16;
        bits
    }

    /// Decode the `u32 LE` flags field. Fails if any reserved bit
    /// (`[4..=15]`) is set, or if vendor bits (`[16..=31]`) are set
    /// but `codec_id < VENDOR_FLAG_THRESHOLD`.
    pub fn from_bits(bits: u32, codec_id: CodecId) -> Result<Self, EnvelopeError> {
        // Bits 4..=15 are reserved and must be zero (ADR 0062 §9 item 4 X1).
        let reserved_mid = (bits >> 4) & 0x0000_0FFF;
        if reserved_mid != 0 {
            return Err(EnvelopeError::UnknownEnvelopeFlag { bits });
        }
        let vendor = ((bits >> 16) & 0x0000_FFFF) as u16;
        // Vendor bits only permitted when codec_id >= VENDOR_FLAG_THRESHOLD.
        if vendor != 0 && codec_id.0 < VENDOR_FLAG_THRESHOLD {
            return Err(EnvelopeError::UnknownEnvelopeFlag { bits });
        }
        Ok(Self {
            payload_is_raw: (bits & (1 << 0)) != 0,
            critical: (bits & (1 << 1)) != 0,
            encryption_ordered: (bits & (1 << 2)) != 0,
            has_sidecar_dictionary: (bits & (1 << 3)) != 0,
            vendor_bits: vendor,
        })
    }
}

/// Everything in the envelope header except the integrity-material
/// field. Callers construct this to write an envelope; readers
/// receive it from a successful `read_envelope`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvelopeHeader {
    pub codec_id: CodecId,
    pub codec_format_version: u16,
    pub flags: EnvelopeFlags,
    pub encoded_length: u32,
    pub decoded_length: u32,
}

/// A successfully-parsed envelope view. `payload` borrows from the
/// original input buffer; the caller passes it to the resolved codec's
/// `decode` method.
#[derive(Debug, PartialEq, Eq)]
pub struct Envelope<'a> {
    pub header: EnvelopeHeader,
    pub payload: &'a [u8],
}

/// Caller-supplied admission caps consulted on the pre-allocation
/// bounds check. ADR 0062 §4 step 8/9 require these to be checked
/// BEFORE the reader allocates payload or output buffers. In
/// production callers derive these from the policy service
/// (`compression.max_decoded_page_bytes` / `compression.max_encoded_
/// page_bytes`, ADR 0060 §3.2).
#[derive(Clone, Copy, Debug)]
pub struct EnvelopeCaps {
    pub max_decoded_length: u32,
    pub max_encoded_length: u32,
}

impl EnvelopeCaps {
    /// A permissive cap set suitable for tests and administrative
    /// paths that want to defer admission to the codec layer. Not for
    /// production carrier reads.
    pub const fn unbounded() -> Self {
        Self {
            max_decoded_length: u32::MAX,
            max_encoded_length: u32::MAX,
        }
    }
}

/// Envelope-level fail-closed variants. ADR 0062 §4.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnvelopeError {
    #[error("input is {bytes} bytes; minimum envelope size is {min}")]
    InputTooSmall { bytes: usize, min: usize },
    #[error("envelope magic does not match {:?}", ENVELOPE_MAGIC)]
    UnknownMagic,
    #[error("envelope version {declared} is not supported; this build accepts {}", ENVELOPE_VERSION)]
    UnsupportedEnvelopeVersion { declared: u16 },
    #[error("envelope reserved byte region is nonzero")]
    InvalidReservedByte,
    #[error("envelope flags {bits:#010x} contain unknown or forbidden bits")]
    UnknownEnvelopeFlag { bits: u32 },
    #[error("envelope header CRC32 does not match")]
    HeaderChecksumMismatch,
    #[error("declared encoded_length {declared} exceeds caller cap {cap}")]
    EncodedLengthExceedsCap { declared: u32, cap: u32 },
    #[error("declared decoded_length {declared} exceeds caller cap {cap}")]
    DecodedLengthExceedsCap { declared: u32, cap: u32 },
    #[error("envelope claims encoded_length {declared} but observed body is {actual} bytes")]
    EncodedLengthMismatch { declared: u32, actual: usize },
    #[error("envelope payload CRC32 does not match")]
    PayloadChecksumMismatch,
    #[error("envelope would produce {bytes} bytes but max_encoded_length cap is {cap}")]
    OutputExceedsCap { bytes: usize, cap: u32 },
    #[error("payload length {bytes} exceeds u32::MAX")]
    PayloadTooLarge { bytes: usize },
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut h = Hasher::new();
    h.update(bytes);
    h.finalize()
}

/// Write an envelope for `payload`. Caller sets every header field
/// including the correct `encoded_length` and `decoded_length` — the
/// envelope does not infer them.
///
/// Returns the total number of bytes appended to `output`.
pub fn write_envelope(
    output: &mut Vec<u8>,
    header: &EnvelopeHeader,
    payload: &[u8],
) -> Result<usize, EnvelopeError> {
    if payload.len() > u32::MAX as usize {
        return Err(EnvelopeError::PayloadTooLarge { bytes: payload.len() });
    }
    let encoded_len_u32 = u32::try_from(payload.len())
        .map_err(|_| EnvelopeError::PayloadTooLarge { bytes: payload.len() })?;
    if header.encoded_length != encoded_len_u32 {
        return Err(EnvelopeError::EncodedLengthMismatch {
            declared: header.encoded_length,
            actual: payload.len(),
        });
    }

    let mut header_bytes = [0_u8; ENVELOPE_HEADER_SIZE];
    header_bytes[0..4].copy_from_slice(&ENVELOPE_MAGIC);
    header_bytes[4..6].copy_from_slice(&ENVELOPE_VERSION.to_le_bytes());
    header_bytes[6..8].copy_from_slice(&header.codec_id.0.to_le_bytes());
    header_bytes[8..10].copy_from_slice(&header.codec_format_version.to_le_bytes());
    // bytes 10..=11 reserved_a = 0 (already)
    header_bytes[12..16].copy_from_slice(&header.flags.to_bits().to_le_bytes());
    header_bytes[16..20].copy_from_slice(&header.encoded_length.to_le_bytes());
    header_bytes[20..24].copy_from_slice(&header.decoded_length.to_le_bytes());
    // bytes 24..=27 reserved_b = 0 (already)
    let header_crc = crc32(&header_bytes[0..28]);
    header_bytes[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let payload_crc = crc32(payload);

    let written = ENVELOPE_HEADER_SIZE + payload.len() + ENVELOPE_TRAILER_SIZE;
    output.reserve(written);
    output.extend_from_slice(&header_bytes);
    output.extend_from_slice(payload);
    output.extend_from_slice(&payload_crc.to_le_bytes());
    Ok(written)
}

/// Read an envelope from `input` applying the ADR 0062 §4 fail-closed
/// steps 1–5 and 8–12. Codec resolution (steps 6–7) and bounded decode
/// (steps 13–14) are the codec/registry layer's responsibility.
///
/// On success, the returned `Envelope::payload` borrows from `input`.
/// On error, `input` is untouched.
pub fn read_envelope<'a>(input: &'a [u8], caps: EnvelopeCaps) -> Result<Envelope<'a>, EnvelopeError> {
    // Step 1: must have at least a header.
    if input.len() < ENVELOPE_HEADER_SIZE {
        return Err(EnvelopeError::InputTooSmall {
            bytes: input.len(),
            min: ENVELOPE_HEADER_SIZE,
        });
    }
    let header_bytes = &input[..ENVELOPE_HEADER_SIZE];

    // Step 2: magic.
    if header_bytes[0..4] != ENVELOPE_MAGIC {
        return Err(EnvelopeError::UnknownMagic);
    }

    // Step 3: envelope version.
    let envelope_version = u16::from_le_bytes(header_bytes[4..6].try_into().unwrap());
    if envelope_version != ENVELOPE_VERSION {
        return Err(EnvelopeError::UnsupportedEnvelopeVersion { declared: envelope_version });
    }

    let codec_id = CodecId(u16::from_le_bytes(header_bytes[6..8].try_into().unwrap()));
    let codec_format_version = u16::from_le_bytes(header_bytes[8..10].try_into().unwrap());
    let reserved_a = u16::from_le_bytes(header_bytes[10..12].try_into().unwrap());
    let flags_bits = u32::from_le_bytes(header_bytes[12..16].try_into().unwrap());
    let encoded_length = u32::from_le_bytes(header_bytes[16..20].try_into().unwrap());
    let decoded_length = u32::from_le_bytes(header_bytes[20..24].try_into().unwrap());
    let reserved_b = u32::from_le_bytes(header_bytes[24..28].try_into().unwrap());
    let header_crc_declared = u32::from_le_bytes(header_bytes[28..32].try_into().unwrap());

    // Step 4: reserved-must-be-zero (strict per ADR 0062 §9 item 4 X1).
    if reserved_a != 0 || reserved_b != 0 {
        return Err(EnvelopeError::InvalidReservedByte);
    }
    let flags = EnvelopeFlags::from_bits(flags_bits, codec_id)?;

    // Step 5: header CRC32 over bytes 0..=27.
    if crc32(&header_bytes[0..28]) != header_crc_declared {
        return Err(EnvelopeError::HeaderChecksumMismatch);
    }

    // Step 8: decoded_length cap check BEFORE allocation (BACE-FMT-003).
    if decoded_length > caps.max_decoded_length {
        return Err(EnvelopeError::DecodedLengthExceedsCap {
            declared: decoded_length,
            cap: caps.max_decoded_length,
        });
    }

    // Step 9: encoded_length cap check BEFORE reading payload.
    if encoded_length > caps.max_encoded_length {
        return Err(EnvelopeError::EncodedLengthExceedsCap {
            declared: encoded_length,
            cap: caps.max_encoded_length,
        });
    }

    // Step 10: observed payload length matches declared.
    let expected_total = ENVELOPE_HEADER_SIZE
        .checked_add(encoded_length as usize)
        .and_then(|n| n.checked_add(ENVELOPE_TRAILER_SIZE))
        .ok_or(EnvelopeError::PayloadTooLarge { bytes: usize::MAX })?;
    if input.len() < expected_total {
        return Err(EnvelopeError::EncodedLengthMismatch {
            declared: encoded_length,
            actual: input.len().saturating_sub(ENVELOPE_HEADER_SIZE + ENVELOPE_TRAILER_SIZE),
        });
    }

    let payload_start = ENVELOPE_HEADER_SIZE;
    let payload_end = payload_start + encoded_length as usize;
    let payload = &input[payload_start..payload_end];

    // Steps 11–12: trailer CRC32 over the payload.
    let trailer_bytes: [u8; ENVELOPE_TRAILER_SIZE] = input[payload_end..payload_end + ENVELOPE_TRAILER_SIZE]
        .try_into().unwrap();
    let payload_crc_declared = u32::from_le_bytes(trailer_bytes);
    if crc32(payload) != payload_crc_declared {
        return Err(EnvelopeError::PayloadChecksumMismatch);
    }

    Ok(Envelope {
        header: EnvelopeHeader {
            codec_id,
            codec_format_version,
            flags,
            encoded_length,
            decoded_length,
        },
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(codec_id: u16, payload_len: u32) -> EnvelopeHeader {
        EnvelopeHeader {
            codec_id: CodecId(codec_id),
            codec_format_version: 1,
            flags: EnvelopeFlags::default(),
            encoded_length: payload_len,
            decoded_length: payload_len,
        }
    }

    #[test]
    fn empty_payload_envelope_is_exactly_36_bytes_and_roundtrips() {
        let mut buf = Vec::new();
        let written = write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        assert_eq!(written, ENVELOPE_MIN_SIZE);
        assert_eq!(buf.len(), ENVELOPE_MIN_SIZE);
        let env = read_envelope(&buf, EnvelopeCaps::unbounded()).unwrap();
        assert_eq!(env.header.codec_id, CodecId(48));
        assert_eq!(env.header.encoded_length, 0);
        assert!(env.payload.is_empty());
    }

    #[test]
    fn byte_layout_matches_adr_0062_verbatim() {
        // Golden vector: codec_id=48, codec_format_version=1, flags=0,
        // encoded_length=4, decoded_length=4, payload=b"test".
        let payload = b"test";
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 4), payload).unwrap();
        // Header offsets:
        assert_eq!(&buf[0..4], b"BACE");                 // magic
        assert_eq!(&buf[4..6], &[0x01, 0x00]);           // envelope_version = 1
        assert_eq!(&buf[6..8], &[0x30, 0x00]);           // codec_id = 48 (0x30)
        assert_eq!(&buf[8..10], &[0x01, 0x00]);          // codec_format_version = 1
        assert_eq!(&buf[10..12], &[0x00, 0x00]);         // reserved_a = 0
        assert_eq!(&buf[12..16], &[0x00, 0x00, 0x00, 0x00]); // flags = 0
        assert_eq!(&buf[16..20], &[0x04, 0x00, 0x00, 0x00]); // encoded_length = 4
        assert_eq!(&buf[20..24], &[0x04, 0x00, 0x00, 0x00]); // decoded_length = 4
        assert_eq!(&buf[24..28], &[0x00, 0x00, 0x00, 0x00]); // reserved_b = 0
        // bytes 28..32 = header CRC32
        assert_eq!(&buf[32..36], payload);               // payload = "test"
        assert_eq!(buf.len(), ENVELOPE_MIN_SIZE + 4);
    }

    #[test]
    fn multi_size_payload_round_trip_preserves_every_byte() {
        for payload_len in [1, 2, 3, 15, 16, 17, 1024, 65535, 1_000_000] {
            let payload: Vec<u8> = (0..payload_len).map(|i| (i % 251) as u8).collect();
            let mut buf = Vec::new();
            let written = write_envelope(&mut buf, &header(48, payload_len as u32), &payload).unwrap();
            assert_eq!(written, ENVELOPE_MIN_SIZE + payload_len);
            let env = read_envelope(&buf, EnvelopeCaps::unbounded()).unwrap();
            assert_eq!(env.payload, payload.as_slice());
        }
    }

    // ---------- fail-closed cases ----------

    #[test]
    fn unknown_magic_fails_closed() {
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        buf[0] ^= 0xff;
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::UnknownMagic)));
    }

    #[test]
    fn unsupported_envelope_version_fails_closed() {
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        // Set envelope_version to 2 and recompute the header CRC to isolate
        // the version check from the CRC check.
        buf[4..6].copy_from_slice(&2_u16.to_le_bytes());
        let crc = crc32(&buf[0..28]);
        buf[28..32].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::UnsupportedEnvelopeVersion { declared: 2 })));
    }

    #[test]
    fn nonzero_reserved_a_fails_closed() {
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        buf[10] = 0x01;
        let crc = crc32(&buf[0..28]);
        buf[28..32].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::InvalidReservedByte)));
    }

    #[test]
    fn nonzero_reserved_b_fails_closed() {
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        buf[24] = 0x01;
        let crc = crc32(&buf[0..28]);
        buf[28..32].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::InvalidReservedByte)));
    }

    #[test]
    fn reserved_mid_flag_bits_fail_closed() {
        // Set bit 4 of the flags word (first reserved bit in [4..=15]).
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        buf[12..16].copy_from_slice(&(1_u32 << 4).to_le_bytes());
        let crc = crc32(&buf[0..28]);
        buf[28..32].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::UnknownEnvelopeFlag { .. })));
    }

    #[test]
    fn vendor_flag_bits_fail_closed_when_codec_id_below_threshold() {
        // Set bit 16 (first vendor bit) with codec_id = 48 (< 256).
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        buf[12..16].copy_from_slice(&(1_u32 << 16).to_le_bytes());
        let crc = crc32(&buf[0..28]);
        buf[28..32].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::UnknownEnvelopeFlag { .. })));
    }

    #[test]
    fn vendor_flag_bits_round_trip_when_codec_id_meets_threshold() {
        let mut vendor_header = header(300, 0); // codec_id >= 256
        vendor_header.flags.vendor_bits = 0xDEAD;
        let mut buf = Vec::new();
        write_envelope(&mut buf, &vendor_header, &[]).unwrap();
        let env = read_envelope(&buf, EnvelopeCaps::unbounded()).unwrap();
        assert_eq!(env.header.flags.vendor_bits, 0xDEAD);
    }

    #[test]
    fn header_checksum_mismatch_fails_closed() {
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 0), &[]).unwrap();
        buf[28] ^= 0xff;
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::HeaderChecksumMismatch)));
    }

    #[test]
    fn payload_checksum_mismatch_fails_closed() {
        let payload = b"abc";
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 3), payload).unwrap();
        let last = buf.len() - 1;
        buf[last] ^= 0xff;
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::PayloadChecksumMismatch)));
    }

    #[test]
    fn declared_decoded_length_exceeding_cap_fails_before_allocation() {
        let mut buf = Vec::new();
        let mut h = header(48, 4);
        h.decoded_length = 1024;
        write_envelope(&mut buf, &h, b"test").unwrap();
        let caps = EnvelopeCaps { max_decoded_length: 512, max_encoded_length: u32::MAX };
        assert!(matches!(read_envelope(&buf, caps),
            Err(EnvelopeError::DecodedLengthExceedsCap { declared: 1024, cap: 512 })));
    }

    #[test]
    fn declared_encoded_length_exceeding_cap_fails_before_payload_read() {
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 4), b"test").unwrap();
        let caps = EnvelopeCaps { max_decoded_length: u32::MAX, max_encoded_length: 2 };
        assert!(matches!(read_envelope(&buf, caps),
            Err(EnvelopeError::EncodedLengthExceedsCap { declared: 4, cap: 2 })));
    }

    #[test]
    fn input_shorter_than_declared_body_fails_closed() {
        let mut buf = Vec::new();
        write_envelope(&mut buf, &header(48, 4), b"test").unwrap();
        buf.truncate(ENVELOPE_HEADER_SIZE + 2); // drop two payload bytes and the trailer
        assert!(matches!(read_envelope(&buf, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::EncodedLengthMismatch { declared: 4, .. })));
    }

    #[test]
    fn write_rejects_payload_len_disagreement() {
        let mut buf = Vec::new();
        let mut h = header(48, 3);
        h.encoded_length = 5; // disagrees with slice length
        let err = write_envelope(&mut buf, &h, b"abc").unwrap_err();
        assert!(matches!(err, EnvelopeError::EncodedLengthMismatch { declared: 5, actual: 3 }));
        assert!(buf.is_empty(), "write must not append anything on refusal");
    }

    #[test]
    fn input_too_small_for_header_fails_closed() {
        let small = [0_u8; 5];
        assert!(matches!(read_envelope(&small, EnvelopeCaps::unbounded()),
            Err(EnvelopeError::InputTooSmall { .. })));
    }
}
