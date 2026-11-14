# BACE Component Q.2 — envelope v1 byte layout verification

14 November 2026. Baseline: `b6a7ee2`.
Second BACE implementation step against accepted ADR 0062.

## Executed result

**884 Rust tests passed, 0 failed, 0 ignored.** Clippy clean on
`bradian-relational --all-targets`. The envelope byte codec is now
implemented but **nothing in production writes a BACE envelope to
disk yet.** Envelope bytes appear only in in-memory buffers
constructed by callers and in test fixtures.

| Executed check | Result |
|---|---|
| `cargo check -p bradian-relational` | Passed |
| `cargo test --workspace` | 884 passed, 0 failed |
| `cargo test -p bradian-relational --lib bace::envelope` | 17 passed |
| `cargo clippy -p bradian-relational --all-targets -- -D warnings` | Passed |

## What landed

New submodule `crates/bradian-relational/src/bace/envelope.rs`:

- Constants: `ENVELOPE_MAGIC = *b"BACE"`, `ENVELOPE_VERSION = 1`,
  `ENVELOPE_HEADER_SIZE = 32`, `ENVELOPE_TRAILER_SIZE = 4`,
  `ENVELOPE_MIN_SIZE = 36`, `VENDOR_FLAG_THRESHOLD = 256`.
- `EnvelopeFlags { payload_is_raw, critical, encryption_ordered,
  has_sidecar_dictionary, vendor_bits: u16 }` with `to_bits()` and
  `from_bits(bits, codec_id)` that enforce the strictness and
  vendor-bit policies from ADR 0062 §9 items 4 (X1) and 5 (A2).
- `EnvelopeHeader { codec_id, codec_format_version, flags,
  encoded_length, decoded_length }`.
- `Envelope<'a> { header, payload: &'a [u8] }` — borrows payload
  bytes from the input buffer.
- `EnvelopeCaps { max_decoded_length, max_encoded_length }` with
  an `unbounded()` constructor for tests.
- `EnvelopeError` — 11 fail-closed variants named after ADR 0062
  §4.
- `write_envelope(output, header, payload)` — writes a well-formed
  envelope to a `Vec<u8>`. Rejects if `header.encoded_length`
  disagrees with `payload.len()`.
- `read_envelope(input, caps)` — executes ADR 0062 §4 steps 1–5
  and 8–12. Steps 6–7 (codec resolution) and 13–14 (bounded decode)
  are the codec/registry layer's responsibility.

## 17 unit tests

Positive:
- `empty_payload_envelope_is_exactly_36_bytes_and_roundtrips`
- `byte_layout_matches_adr_0062_verbatim` — golden vector asserts
  every header byte position for codec 48, version 1, flags 0,
  payload `"test"`.
- `multi_size_payload_round_trip_preserves_every_byte` across 9
  sizes (1, 2, 3, 15, 16, 17, 1024, 65535, 1_000_000).
- `vendor_flag_bits_round_trip_when_codec_id_meets_threshold`
  (codec_id = 300, vendor_bits = 0xDEAD).

Fail-closed (one per ADR 0062 §4 or §9 invariant):
- `unknown_magic_fails_closed`
- `unsupported_envelope_version_fails_closed` (header CRC rebuilt
  to isolate the version check from CRC)
- `nonzero_reserved_a_fails_closed`
- `nonzero_reserved_b_fails_closed`
- `reserved_mid_flag_bits_fail_closed` (bit 4 of flags)
- `vendor_flag_bits_fail_closed_when_codec_id_below_threshold`
  (bit 16 of flags, codec_id = 48)
- `header_checksum_mismatch_fails_closed`
- `payload_checksum_mismatch_fails_closed`
- `declared_decoded_length_exceeding_cap_fails_before_allocation`
  (BACE-FMT-003 pre-allocation obligation)
- `declared_encoded_length_exceeding_cap_fails_before_payload_read`
- `input_shorter_than_declared_body_fails_closed`
- `write_rejects_payload_len_disagreement` (write path guard)
- `input_too_small_for_header_fails_closed`

## Compliance check against ADR 0062

- §3.1 overall structure — 32-byte header + payload + 4-byte trailer. ✓
- §3.2 header field table — every offset and size matches. ✓ (proven byte-by-byte by `byte_layout_matches_adr_0062_verbatim`).
- §3.3 flags — four named bits, 16-bit vendor region, strict
  reserved-must-be-zero in `[4..=15]`, vendor bits gated by
  `codec_id >= 256`. ✓
- §3.5 trailer — 4-byte payload CRC32 LE. ✓
- §3.7 endianness — little-endian throughout. ✓
- §4 read protocol — steps 1–5 and 8–12 implemented in the
  documented order; test matrix covers every step's failure mode. ✓
- §5 write protocol — staged into a `Vec<u8>`; actual durable
  publication is a carrier concern (not yet integrated).
- §9 resolved decisions — A1 (reserved `"BACA"` magic, never
  emitted by this module), A2 (vendor bits gated), P4/P5 (`u32`
  lengths), X1 (strict reserved zero), X2 (empty payload
  permitted — proven), X3 (ASCII magic acts as endianness marker).

## What this component explicitly does NOT do

- No codec resolution (ADR 0062 §4 steps 6–7). The caller passes
  the parsed `CodecId` to Component Q.3's registry.
- No bounded decode (ADR 0062 §4 steps 13–14). The caller passes
  the parsed `payload` slice and `decoded_length` to the resolved
  codec's `decode`.
- No carrier integration. BRPG v2 (ADR 0063) will embed envelopes
  in page payloads; that is a later slice.
- No golden corpus yet beyond the single `"test"` payload. The
  corpus grows as Components Q.3 and beyond add codecs.

## Next

Component Q.3 — codec registry with `BuiltInRegistry` registering
RAW, and the negotiation algorithm from ADR 0061 §3.5.

See [BACE_ADR_REVIEW_PACKAGE](BACE_ADR_REVIEW_PACKAGE.md),
[ADR 0062](../adr/0062-bace-compressed-envelope-v1.md).
