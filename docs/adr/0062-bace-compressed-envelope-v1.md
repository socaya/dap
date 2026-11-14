# ADR 0062 — BACE compressed envelope v1

**Status: ACCEPTED — 14 November 2026.**

Owner review completed via `BACE_ADR_REVIEW_PACKAGE.md` §4 with the
proposed defaults recorded as resolved decisions (see §9 below).
Every byte position below is now a forever invariant of the Bradian
on-disk format.

**Host-local trust only.** The authenticated-envelope variant
(magic `"BACA"`, reserved) is a separate future ADR tied to the
BRD-SEC-003 workstream. v1 (CRC-only) must not be used for off-host
transport or untrusted payloads without an outer authenticated
wrapper.

## 1. Context

BACE SRS v1.0 §6 requires:

> **BACE-FMT-001 — Strict envelope.** Every compressed durable
> object MUST carry validated version, codec ID, flags, encoded
> length, decoded length, bounds and integrity material; dictionary
> references must resolve within authorized immutable scope.
>
> **BACE-FMT-002 — Atomic publication.** Compression write paths
> MUST use durable staging, integrity verification, atomic
> manifest/generation publication and snapshot-safe cleanup.
>
> **BACE-FMT-003 — Bounded decompression.** Readers MUST reject
> declared decoded sizes above configured page/frame maximum
> **before** allocating and enforce bounded output during decoding,
> not merely after decode completion.
>
> **BACE-FMT-004 — Rollback and rollback-safe upgrades.** Upgrades
> MUST preserve legacy readers for existing files or require a
> formally verified, backed-up migration with explicit downgrade
> restriction.
>
> **BACE-FMT-005 — Checksums versus authenticity.** CRC checks
> detect accidental corruption only; encrypted or untrusted
> distributed/backup payloads MUST use authenticated integrity.

BACE §3.1 adds:

> **BACE-ARCH-005 — Versioned compatibility.** On-disk/wire
> compression metadata MUST be self-describing enough for bounded
> decoding and independent of host endian/layout.
>
> **BACE-ARCH-006 — Encryption ordering.** Compression MUST precede
> authenticated encryption of the payload where permitted.

Today Bradian has three distinct byte layouts: BRPG pages (`page.rs`,
4 KiB with 32-byte header + CRC), BRWL WAL frames (12-byte header +
body + CRC), and checkpoint manifests (40-byte fixed record). None
is a general-purpose compressed envelope. The BACE envelope
specified here is a **fourth, independent byte layout** that these
subsystems will embed as payload once BACE lands.

## 2. Scope of this ADR

**In scope:** the on-disk byte layout of the BACE v1 envelope,
including every field position, size, meaning, integrity material,
fail-closed semantics, and the bounded-decompression contract.

**Explicitly out of scope** (each needs its own ADR):

- **Authenticated envelope for untrusted payloads** (BACE-FMT-005).
  v1 specified here is CRC-only. The authenticated variant is a
  separate envelope, tied to the BRD-SEC-003 encryption workstream.
- **How the envelope is embedded in BRPG pages** (ADR 0063 — BRPG v2
  feature-flag page). The envelope is a self-contained byte sequence
  that pages, WAL frames, backup archives and column segments will
  carry inside their own existing envelopes.
- **How the envelope is embedded in `.derc` segments.** Follow-up.
- **How the envelope is embedded in BRWL WAL frames.** Follow-up.
- **Dictionary-reference resolution semantics.** The envelope carries
  codec parameters but not the dictionary bytes themselves. Dictionary
  storage, lifecycle, authorized scope (BACE-FMT-001 "within authorized
  immutable scope") is a separate ADR tied to the catalog.
- **Replication / federation wire framing.** The envelope is the
  compressed payload; wire protocol wraps it (Gate 4+).
- **Codec identifier numbers in the 16–63 range** (ADR 0061 §5 Q1).
- **Encryption ordering details** (BACE-ARCH-006, the BRD-SEC-003
  workstream).

## 3. Proposed byte layout

### 3.1 Overall structure

```
┌──────────────────────────────┬──────────────────────┬────────────────┐
│ FIXED HEADER  (32 bytes)     │ PAYLOAD (encoded_len)│ TRAILER (4 B)  │
│ incl. header CRC32           │  codec-specific bytes│  payload CRC32 │
└──────────────────────────────┴──────────────────────┴────────────────┘
```

Total overhead: **36 bytes per frame** (32-byte header + 4-byte
trailer CRC32). All multi-byte integers are **little-endian**
(matches `page.rs`, `row_store.rs`, `checkpoint_manifest.rs`).

### 3.2 Header — 32 bytes

| Offset | Size | Field | Meaning |
|---|---|---|---|
| `0..=3` | 4 | `magic` | ASCII `"BACE"` (`0x42 0x41 0x43 0x45`). Unknown magic → fail closed. |
| `4..=5` | 2 | `envelope_version` | `u16 LE`. Current frozen value: **`1`**. Unknown version → fail closed. |
| `6..=7` | 2 | `codec_id` | `u16 LE`. Must resolve in the current build's codec registry (ADR 0061). Unregistered ID with `critical` flag → fail closed. |
| `8..=9` | 2 | `codec_format_version` | `u16 LE`. Matched against the registered codec's declared format version. Mismatch → fail closed. |
| `10..=11` | 2 | `reserved_a` | `u16 LE`. Must be `0` in v1. Non-zero → fail closed. |
| `12..=15` | 4 | `flags` | `u32 LE`. See §3.3. |
| `16..=19` | 4 | `encoded_length` | `u32 LE`. Bytes of payload. Must equal observed payload length. Mismatch → fail closed. Max value **`u32::MAX`** (≈ 4 GiB). Pages / WAL / segment carriers may impose smaller caps. |
| `20..=23` | 4 | `decoded_length` | `u32 LE`. Declared output size. **BACE-FMT-003**: reader MUST compare to the caller-supplied cap **before** allocating the output buffer. |
| `24..=27` | 4 | `reserved_b` | `u32 LE`. Must be `0` in v1. Non-zero → fail closed. |
| `28..=31` | 4 | `header_crc32` | `u32 LE`. CRC32 (same polynomial as `crc32fast`, matching `page.rs` and `row_store.rs`) over bytes `0..=27` of the header. Mismatch → fail closed. |

### 3.3 `flags` bitfield (bit 0 = LSB)

| Bit | Name | Meaning |
|---|---|---|
| `0` | `payload_is_raw` | The payload is uncompressed (BACE-SEL-002 avoid-expansion path). The codec_id MUST still be registered; it names the codec that was *considered* but not applied. Decoders simply return the payload bytes unchanged after validating the trailer CRC. |
| `1` | `critical` | The producer considered this codec critical. If a reader cannot resolve `codec_id` in its registry, the operation fails closed. |
| `2` | `encryption_ordered` | Set by BACE-ARCH-006 pipelines: the producer expects an outer authenticated-encryption layer to wrap this envelope. A standalone reader that is not inside an authenticated-encryption wrapper and sees this bit set with the trust-boundary policy enforced MUST refuse. |
| `3` | `has_sidecar_dictionary` | Codec references a dictionary stored outside this envelope. Dictionary lookup rules are deferred to the dictionary-scope ADR. |
| `4..=15` | reserved | Must be `0` in v1. Non-zero → fail closed. |
| `16..=31` | vendor-reserved | Must be `0` in v1 unless `codec_id >= 256`. |

### 3.4 Payload — `encoded_length` bytes

The payload is interpreted by the codec identified by
`(codec_id, codec_format_version)`. Its internal structure is the
codec's concern.

### 3.5 Trailer — 4 bytes

| Offset | Size | Field | Meaning |
|---|---|---|---|
| `32 + encoded_length ..= 35 + encoded_length` | 4 | `payload_crc32` | `u32 LE`. CRC32 over the payload bytes. Mismatch → fail closed, decoded output discarded. |

### 3.6 Total frame size

```
total_frame_bytes = 32 + encoded_length + 4
                  = encoded_length + 36
```

An empty-payload envelope (`encoded_length = 0`) is permitted; its
total size is 36 bytes.

### 3.7 Endianness and portability

All multi-byte integers are little-endian. The on-disk layout does
not depend on host endianness or alignment (BACE-ARCH-005). The
envelope is read / written byte-by-byte with `read_exact` / `write_all`
on the storage-backend file descriptor, matching the convention in
`page.rs`.

## 4. Read protocol (fail-closed invariants)

A conformant reader MUST execute every check below in this order,
rejecting at the first failure. A reader that skips any check is
non-conformant.

1. Read exactly 32 bytes into a fixed-size header buffer.
2. `magic == b"BACE"`; else → `UnknownMagic`.
3. `envelope_version == 1`; else → `UnsupportedEnvelopeVersion`.
4. `reserved_a == 0` and `reserved_b == 0` and `flags[4..=15] == 0`;
   else → `UnknownEnvelopeFlag`.
5. `header_crc32` matches CRC32 over bytes `0..=27`; else →
   `HeaderChecksumMismatch`.
6. `codec_id` resolves in the local registry (ADR 0061); else →
   `UnknownCodec` if `flags.critical`, otherwise return the codec
   ID to the caller for a policy-mediated decision (ADR 0060).
7. `codec_format_version` matches the registered codec's declared
   format version; else → `UnsupportedCodecFormatVersion`.
8. `decoded_length <= caller_supplied_max` (BACE-FMT-003). The
   caller supplies this cap from the policy service
   (`compression.max_decoded_page_bytes`, ADR 0060). Reader MUST
   check **before** allocating the output buffer. Else →
   `DecodedLengthExceedsCap`.
9. `encoded_length <= caller_supplied_frame_cap`. Reader MUST check
   before allocating the payload buffer. Else → `EncodedLengthExceedsCap`.
10. Read exactly `encoded_length` bytes into the payload buffer.
11. Read exactly 4 bytes into a trailer buffer.
12. `payload_crc32` matches CRC32 over the payload; else →
    `PayloadChecksumMismatch`.
13. Decode the payload. **During decode, enforce the declared
    `decoded_length` bound.** A codec that would produce more bytes
    MUST error, not truncate silently (BACE-FMT-003 "enforce bounded
    output during decoding, not merely after").
14. `decoded_length` equals the actual decoded output length; else →
    `DecodedLengthMismatch`.

At no step is partial data returned to the caller. A failure at
any step discards every byte read so far.

## 5. Write protocol (fail-closed invariants)

1. Resolve the codec and its policy (ADRs 0060 + 0061).
2. Encode the payload. Record its length in `encoded_length`.
3. Compute `header_crc32` over bytes `0..=27` of the staged header.
4. Compute `payload_crc32` over the staged payload.
5. Write to a **staged location** (file-staging, memory-staging, or
   encryption-pipeline staging) — never directly into the authoritative
   generation / page / segment (BACE-FMT-002 atomic publication).
6. Publish via rename / manifest commit / page commit as the
   containing subsystem defines. The envelope itself specifies **no**
   publication mechanism; that is the subsystem's responsibility.

## 6. Rollback and upgrade semantics (BACE-FMT-004)

- A v1 reader seeing a future `envelope_version > 1` MUST fail
  closed with `UnsupportedEnvelopeVersion`. There is no attempt to
  interpret fields the reader does not know.
- A v2 or later envelope is a **separate ADR**. v2 MAY add fields;
  the required rollback-compatibility pattern is that v2 payloads
  are **not written** into containers that any live v1 reader might
  touch, until the container generation itself has been bumped.
- A migration from v1 → v2 is a copy-and-replace operation; there
  is no in-place reinterpretation.

## 7. Authenticity boundary (BACE-FMT-005)

The v1 envelope carries CRC32 only. CRC32 detects **accidental**
corruption; it is **not** authenticated integrity. Therefore:

- v1 is sufficient for host-local durable storage where the host
  itself is the trust boundary.
- v1 is **not sufficient** for off-host transport (replication,
  federation, remote backup) or for payloads authored by an
  untrusted producer.
- Off-host / untrusted payloads MUST be wrapped in a separate
  authenticated-encryption envelope that encloses the BACE v1
  envelope verbatim. That envelope's byte layout is deferred to the
  BRD-SEC-003 workstream.

The `flags.encryption_ordered` bit is set by producers that intend
such a wrapper. A standalone reader that is not inside an
authenticated-encryption wrapper and sees this bit, under a trust
policy that enforces it, MUST refuse.

## 8. Alternatives considered

**A. Variable-length header with tag-length-value encoding.** More
flexible for future extensions. Rejected: makes bounded decoding
harder; fixed 32 bytes is cheap and pre-allocation-friendly
(BACE-FMT-003).

**B. One CRC covering header + payload.** Simpler. Rejected: an
error in the header (notably in `decoded_length`) cannot be
detected before allocation, so bounded decompression becomes
unsafe. Separate header CRC lets the reader validate lengths first.

**C. 64-bit lengths.** Allows > 4 GiB frames. Rejected: BACE §4.2
requires per-page / per-row-group framing with fixed maximum
uncompressed size; no single BACE frame should exceed 4 GiB. A
future variant can add a `large_frame` flag if the need arises.

**D. Embed the codec parameter values in the header.** Simpler for
decoders. Rejected: codec parameter shape is codec-specific
(ADR 0061). Putting them in the envelope header forces the envelope
format to grow every time a codec parameter is added.

**E. Use an existing format (Parquet, Zarr, etc.) as the envelope.**
Rejected: these carry semantics (schemas, chunk indices) that
Bradian's row-store / WAL / backup do not need; the overhead is
unjustified; and BACE §3.1 requires a central Bradian-owned codec
registry which a foreign envelope cannot honor.

## 9. Design decisions (resolved)

Resolved on 14 November 2026 via `BACE_ADR_REVIEW_PACKAGE.md` §4.

1. **Separate magic for the authenticated variant.** Reserve
   `"BACA"` (ASCII `0x42 0x41 0x43 0x41`) for the future
   authenticated-envelope variant. Unauthenticated v1 uses `"BACE"`.
   Any ADR introducing the authenticated variant uses `"BACA"` and
   cannot repurpose a flag bit under the already-published `"BACE"`
   magic. (Review package A1.)
2. **`encoded_length` sizing.** `u32` (4 GiB cap). Carriers may
   impose smaller caps. (Review package P4.)
3. **`decoded_length` sizing.** `u32`. (Review package P5.)
4. **`reserved_a` / `reserved_b` must-be-zero.** Strict. Any
   non-zero reserved byte fails closed with
   `InvalidReservedByte`. (Review package X1.)
5. **Vendor bits in `flags[16..=31]`.** Permitted only when
   `codec_id >= 256`. Non-zero vendor bits with `codec_id < 256`
   fail closed with `UnknownEnvelopeFlag`. (Review package A2.)
6. **Empty payload.** Permitted. `encoded_length = 0` yields a
   36-byte envelope. (Review package X2.)
7. **Byte-order marker via ASCII magic.** Accepted. `"BACE"` as
   ASCII doubles as an endianness check: mis-interpreted as a
   different byte order it is not `0x42 0x41 0x43 0x45`. (Review
   package X3.)

## 10. Impact on invariants

Nothing in this ADR violates SRS v1.1 §20 invariants:

- **INV-001** commit correctness: envelope is a byte layout; it
  does not change commit semantics.
- **INV-002** one authoritative state: carriers (pages, WAL, segments)
  retain their authoritative role. The envelope is a payload
  format.
- **INV-003** no authorization gap: the envelope does not carry
  authorization context. A reader MUST still apply caller-supplied
  caps from the policy service (ADR 0060) before allocating. The
  `flags.encryption_ordered` bit signals the expectation of an
  outer authenticated wrapper; the trust-boundary policy enforces
  it.
- **INV-004** no unsafe automatic availability: N/A; no
  distributed decision.
- **INV-005** recoverable lifecycle: §6 above states the rollback
  policy. A future v2 ADR MUST include an explicit downgrade
  restriction and a copy-and-replace migration plan.

## 11. Dependencies this ADR would add

None in design. The `crc32fast` crate is already in CLAUDE.md §5.
If accepted and implemented, the first integration using an
authenticated variant will require crypto primitives — those come
in via the BRD-SEC-003 workstream and need their own ADR entry.

## 12. What this ADR explicitly does NOT do

- Freeze an authenticated / encrypted envelope.
- Specify dictionary-reference storage, scope or lifecycle.
- Specify how pages / WAL / segments carry the envelope. ADR 0063
  covers BRPG v2; `.derc` embedding and BRWL embedding are
  follow-ups.
- Specify replication / federation wire framing.
- Choose codec identifier numbers (ADR 0061 §5 Q1).
- Implement any codec.
- Modify `.derc`, BRPG, BRWL, or any existing on-disk format today.
- Introduce any `BACE*` admission or `ResourceError` variant.
- Modify any existing ADR.

## 13. Acceptance evidence (satisfied, with ongoing obligations)

1. **ADRs 0060 and 0061 accepted first** — **satisfied** (all four
   accepted in the same review cycle on 14 November 2026).
2. Owner sign-off on §9 decisions — **satisfied** via
   `BACE_ADR_REVIEW_PACKAGE.md`.
3. **Golden-corpus commitment — ongoing.** The first implementation
   slice (Slice Q) ships a golden-vector test for the envelope
   round-trip with the RAW codec; subsequent codecs add their own
   golden entries as they are implemented. Every target in CI runs
   the corpus byte-identically.
4. **Host-local-only consent required.** Production use of v1
   envelopes without an outer authenticated wrapper is restricted
   to host-local trust boundaries until the `"BACA"` authenticated-
   variant ADR is accepted. This is a deployment-posture decision
   that the policy service (ADR 0060) MUST surface to operators
   via an explicit configuration.

Implementation ADRs may now follow. The first codec implementation
(RAW) and ADR 0063 BRPG v2 are both unblocked.
