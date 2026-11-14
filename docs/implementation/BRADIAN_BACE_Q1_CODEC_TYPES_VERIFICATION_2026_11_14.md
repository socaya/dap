# BACE Component Q.1 — codec types + RAW codec verification

14 November 2026. Baseline: `dc187a1`.
First implementation slice against accepted ADRs 0060–0063.

## Executed result

**867 Rust tests passed, 0 failed, 0 ignored.** Clippy clean on
`bradian-relational --all-targets`. No persistent format change
reaches disk; this component is a standalone type model + the RAW
codec only.

| Executed check | Result |
|---|---|
| `cargo check -p bradian-relational` | Passed |
| `cargo test --workspace` | 867 passed, 0 failed |
| `cargo test -p bradian-relational --lib bace` | 4 passed |
| `cargo clippy -p bradian-relational --all-targets -- -D warnings` | Passed |

## What landed

New module `crates/bradian-relational/src/bace/` with one submodule
`codec` for this component:

- `CodecId(u16)` — opaque identifier per ADR 0061 §3.1.
- `Determinism { Deterministic, HardwareDependent }` — per ADR 0061 §3.3.
- `CapabilityFlags { critical, experimental, preserves_null_distinct, allows_outer_compression }` + `default_inner()` / `default_outer()` constructors.
- `CodecCapability` — immutable descriptor published by every codec.
- `EncodeError { InputTooLarge, OutputTooLarge, UnknownCodec }`.
- `DecodeError { UnknownCodec, UnsupportedFormatVersion, DecodedLengthExceeded, DecodedLengthMismatch, EncodedLengthMismatch }`.
- `Codec` trait: `capability() -> &CodecCapability`, `encode(input, output)`, `decode(input, decoded_length, output)`. Documented contract: output must be bounded DURING decoding, not merely after (ADR 0062 §4 step 13).
- `codec::raw::RAW_ID = CodecId(48)` — the first allocated BACE codec ID. Outer-compression range (48–63), no legacy constraint.
- `codec::raw::RawCodec` — deterministic identity codec. Output bytes equal input bytes; declared `Determinism::Deterministic`; marked `critical: true` so a registry missing RAW fails closed.

Four unit tests inline in `bace::codec::tests`:

1. `raw_codec_identifies_itself_as_id_48_deterministic_critical` —
   capability descriptor is the declared ID, version, determinism
   and critical flag.
2. `raw_codec_round_trip_preserves_every_byte_exactly` — identity
   round-trip for empty, one-byte, three-byte, 4096-byte full and
   single-zero inputs.
3. `raw_decoder_rejects_declared_length_mismatch` — a decoder
   given input shorter than `decoded_length` returns
   `DecodedLengthMismatch`.
4. `raw_decoder_rejects_input_exceeding_declared_decoded_length` —
   a decoder given input longer than `decoded_length` returns
   `DecodedLengthExceeded` (BACE-FMT-003 pre-allocation bound).

## What this component explicitly does NOT do

- No envelope byte layout yet — Component Q.2.
- No codec registry yet — Component Q.3.
- No policy service yet — Component Q.4.
- No carrier integration (BRPG v2, row_store, `.derc`, BRWL, backup).
- No ID assignment in the 16–47 inner-encoding range — P1 still
  deferred.
- No `ZSTD` or `LZ4` codec — only RAW.
- No golden-file corpus yet (envelope is Q.2's responsibility).

## Compliance check against the accepted ADRs

- **ADR 0061 §3.1** codec ID namespace partition: RAW at ID 48 is
  inside the 48–63 outer range. No legacy IDs 0–5 are reused. ✓
- **ADR 0061 §3.3** `CodecCapability` field list: `id`,
  `format_version`, `max_input_bytes`, `max_output_bytes`,
  `determinism`, `flags` all present. `supported_types`,
  `parameters`, `decode_cpu_ns_per_kib_estimate` and
  `simd_availability` are NOT YET present; they are added when the
  registry (Q.3) needs them. This is a scoped shortfall, flagged
  here.
- **ADR 0061 §5** resolved decisions: RAW is `Deterministic` per
  item 4 (D1); flags.experimental is false (D2 default); flags.critical
  is true because RAW is the fallback and a registry missing it is
  release-blocking.
- **ADR 0062 §4** bounded-decompression contract (step 13): the
  `Codec::decode` doc-comment states the obligation; the RAW
  implementation returns `DecodedLengthExceeded` before extending
  the output buffer past the declared length.

## Next

Component Q.2 — envelope v1 byte layout.

See [BACE_ADR_REVIEW_PACKAGE](BACE_ADR_REVIEW_PACKAGE.md),
[ADR 0061](../adr/0061-bace-codec-registry-capability-negotiation.md),
[ADR 0062](../adr/0062-bace-compressed-envelope-v1.md).
