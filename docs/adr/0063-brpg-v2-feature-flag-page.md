# ADR 0063 — BRPG v2 feature-flag page (design-only; **PROPOSED**)

**Status: PROPOSED — not accepted for implementation.**

Blocked on **ADR 0060** (central policy service), **ADR 0061**
(codec registry) and **ADR 0062** (compressed envelope v1)
acceptance.

This ADR freezes a **durable on-disk byte layout** for the row-page
extension that carries BACE envelopes. Per CLAUDE.md §4.9
("Spec-first binary formats") and the
[comprehensive implementation plan](../implementation/BRADIAN_COMPREHENSIVE_IMPLEMENTATION_PLAN.md)
§13, owner review is mandatory before implementation. Every byte
position below becomes a forever invariant if accepted.

## 1. Context

BACE SRS v1.0 §4.1 requires:

> **BACE-FR-ROW-002 — Page compatibility.** The BRPG envelope, page
> IDs, checksums, free-space handling and B+tree addresses MUST
> remain valid. A new page representation must use negotiated
> feature/version metadata and safe migration.

BACE §6 adds:

> The relational BRPG page and BRWL log are distinct formats and
> cannot be migrated implicitly as a side effect of `.derc`
> upgrades.

BACE-FMT-004 (§6) requires rollback-safe upgrades:

> Upgrades MUST preserve legacy readers for existing files or
> require a formally verified, backed-up migration with explicit
> downgrade restriction.

Current state. `crates/bradian-relational/src/page.rs` implements
**BRPG v1**:

- 4 096-byte fixed page size.
- 32-byte header with `magic "BRPG"` at `[0..=3]`, `format major
  version` at `[4..=5]` (currently `1`), `kind` at `[6]`,
  **`reserved flags (must be zero)` at `[7]`**, `page_id` at
  `[8..=15]`, `lsn` at `[16..=23]`, `payload_length` at `[24..=27]`,
  `CRC32` over the whole page at `[28..=31]`.
- A v1 reader **rejects any non-zero byte at `[7]`** with
  `PageError::UnsupportedFlags(_)`.

That rejection is why a new `major version = 2` in bytes `[4..=5]`
is the natural fail-closed entry point for a v2 extension: a v1
reader presented with a v2 page returns
`PageError::UnsupportedVersion(2)` and reads nothing. This is
exactly the compatibility behaviour BACE-FR-ROW-002 requires.

## 2. Scope of this ADR

**In scope:** the BRPG v2 byte layout, the feature-flag byte
semantics, the one initial v2 feature (`payload_is_bace_envelope`),
the read/write fail-closed invariants, the migration posture
between v1 and v2, and the relationship to ADR 0062 (envelope).

**Explicitly out of scope** (each needs its own ADR):

- **Overflow page chains** (BACE-FR-ROW-003 individually framed
  chunks for large varlen values). BRPG v2.0 does not yet define
  the chain structure; a future flag bit is reserved.
- **Sidecar dictionary references** (BACE-FR-COL-002). Reserved
  flag bit; semantics deferred.
- **Multi-envelope pages** (one BRPG page carrying several BACE
  envelopes). v2.0 carries exactly one envelope per page when the
  flag is set.
- **`.derc` column segments** — a different format with its own
  envelope story. Follow-up.
- **BRWL WAL frames** — same note.
- **Compaction / recompression lifecycle** (BACE-FR-BKP-003).
  Follow-up.
- **Authenticated page envelope** (BACE-FMT-005). Host-local trust
  only in v2.0; authenticated variant is a separate ADR tied to
  BRD-SEC-003.
- **Codec identifier numbers** (ADR 0061 §5 Q1).

## 3. Proposed byte layout

### 3.1 Overall page structure

BRPG v2 is the **same 4 096-byte fixed page size as BRPG v1**.
The header position layout is preserved **except** for the version
field (bumped) and the flags byte (now a bitfield instead of
must-be-zero).

```
┌────────────────────┬─────────────────────────────────────────┐
│  HEADER (32 bytes) │  PAYLOAD + zero-filled tail (4064 B)    │
└────────────────────┴─────────────────────────────────────────┘
```

Total page size: **4 096 bytes** (unchanged).
Maximum payload size: **4 064 bytes** (unchanged).

### 3.2 Header — 32 bytes

Positions marked **(= v1)** are byte-identical in meaning to BRPG v1.
Positions marked **(v2)** change meaning or value.

| Offset | Size | Field | Meaning |
|---|---|---|---|
| `0..=3` | 4 | `magic` (= v1) | ASCII `"BRPG"`. Unknown magic → fail closed. |
| `4..=5` | 2 | `format_version` (v2) | `u16 LE`. Current v2 value: **`2`**. A v1 reader sees `2` and returns `UnsupportedVersion(2)` — exactly the v1 fail-closed path. |
| `6` | 1 | `kind` (= v1) | `u8`. `PageKind::{Data, Index, FreeMap, Metadata}`. Unknown kind → fail closed. |
| `7` | 1 | `feature_flags` (v2) | `u8` bitfield, see §3.3. In v1 this byte was must-be-zero; v2 reinterprets it. |
| `8..=15` | 8 | `page_id` (= v1) | `u64 LE`. |
| `16..=23` | 8 | `lsn` (= v1) | `u64 LE`. |
| `24..=27` | 4 | `payload_length` (= v1) | `u32 LE`. Bytes of payload immediately following the header. Must equal the observed payload. |
| `28..=31` | 4 | `page_crc32` (= v1) | `u32 LE`. CRC32 over the whole 4 096-byte page with this field zeroed. Identical algorithm and computation to BRPG v1. |

**Zero-padded tail invariant preserved.** Bytes after `payload_length`
through the end of the 4 096-byte page MUST be zero. A non-zero
trailing byte → fail closed with `PageError::NonZeroPadding`
(same semantics as v1).

### 3.3 `feature_flags` byte semantics (v2)

Bit 0 is the LSB.

| Bit | Name | Meaning |
|---|---|---|
| `0` | `payload_is_bace_envelope` | Payload bytes are exactly one BACE v1 envelope (ADR 0062). `payload_length` equals the envelope's total size (`36 + envelope.encoded_length`). |
| `1` | reserved — `has_overflow_chain` | Reserved for BACE-FR-ROW-003. Must be `0` in v2.0. |
| `2` | reserved — `has_sidecar_dictionary` | Reserved for BACE-FR-COL-002. Must be `0` in v2.0. |
| `3..=7` | reserved | Must be `0` in v2.0. Non-zero → fail closed. |

A v2.0 reader seeing any reserved bit set → fail closed with
`UnsupportedFlags(flags_byte)`. This preserves the room for v2.1
to add new flags without breaking v2.0 forward-compatibility — a
v2.1 writer SHOULD NOT emit v2.1 flag bits into a store whose
generation still admits v2.0 readers.

### 3.4 Payload interpretation

| `feature_flags` | Interpretation of payload[0..payload_length] |
|---|---|
| `0b0000_0000` | Plain bytes (BRPG v1 semantics). A v2 page with all flags cleared is wire-compatible with v1 **except** for the version field — it must still be read by a v2-aware reader because of `format_version=2`. |
| `0b0000_0001` (`payload_is_bace_envelope`) | Exactly one BACE v1 envelope (ADR 0062): 32-byte envelope header + `encoded_length` bytes + 4-byte envelope-trailer CRC32. Envelope magic MUST be `"BACE"`; else fail closed. Envelope `encoded_length + 36 == payload_length`; else fail closed. |

**No other combinations are defined in v2.0.** Any flag bit beyond
bit 0 set → fail closed.

### 3.5 Example — a BRPG v2 Data page carrying a BACE envelope

```
bytes 0..=3    :  "BRPG"
bytes 4..=5    :  02 00                    // version = 2
byte  6        :  01                       // PageKind::Data
byte  7        :  01                       // feature_flags = payload_is_bace_envelope
bytes 8..=15   :  <page_id u64 LE>
bytes 16..=23  :  <lsn u64 LE>
bytes 24..=27  :  <payload_length u32 LE>  // = 36 + envelope.encoded_length
bytes 28..=31  :  <page_crc32 u32 LE>      // over the whole page, this field zeroed
bytes 32..=63  :  BACE envelope header     // "BACE" + envelope_version=1 + ...
bytes 64..     :  BACE encoded payload bytes
bytes ..=32+payload_length-5 : (end of envelope payload)
bytes 32+payload_length-4..=32+payload_length-1 : envelope trailer CRC32
bytes 32+payload_length..    :  zero padding
```

## 4. Read protocol (fail-closed invariants)

A v2 reader executes every check below in this order:

1. Page buffer is exactly 4 096 bytes; else → `InvalidLength`.
2. `magic == "BRPG"`; else → `InvalidMagic`.
3. `format_version == 2`; else → `UnsupportedVersion`. (A reader
   that also supports v1 may branch to the v1 decode path here.
   Each path is independent; they do not share state.)
4. `kind` resolves to a known `PageKind`; else → `UnknownKind`.
5. `feature_flags` has no reserved bits set (bits `1..=7` all
   zero in v2.0); else → `UnsupportedFlags`.
6. `payload_length <= MAX_PAYLOAD_SIZE` (4 064); else →
   `InvalidPayloadLength`.
7. `page_crc32` matches CRC32 over the whole page with this field
   zeroed; else → `ChecksumMismatch`.
8. Bytes after the payload through the end of the page are all
   zero; else → `NonZeroPadding`.
9. If `feature_flags.payload_is_bace_envelope`:
    - Decode the payload as a BACE v1 envelope per ADR 0062 §4.
      Every BACE fail-closed step applies.
    - Verify `envelope.encoded_length + 36 == payload_length`;
      else → `BaceLengthMismatch`.
10. Otherwise, treat the payload as plain bytes (v1 semantics).

If any step fails, the page is rejected and no partial state
reaches the caller.

## 5. Write protocol (fail-closed invariants)

1. Resolve the page's policy from ADR 0060 to decide whether to
   emit a BACE envelope.
2. If emitting a BACE envelope, encode the payload per ADR 0062.
3. Set `format_version = 2`.
4. Set `feature_flags.payload_is_bace_envelope` iff the payload
   is a BACE envelope.
5. Compute `page_crc32` with the CRC field zeroed.
6. Write to staged storage.
7. Publish via the storage subsystem's existing manifest /
   generation commit (BACE-FMT-002). The BRPG format itself
   specifies no publication mechanism.

## 6. Mixed-generation coexistence (BACE-FR-ROW-002)

- A single row-store directory MAY contain BRPG v1 pages (legacy)
  and BRPG v2 pages (new generation) simultaneously.
- The **generation / checkpoint boundary** decides which format is
  authoritative for a given logical page. A reader always knows
  which generation it is reading from the checkpoint manifest
  (ADR 0014) and applies the appropriate decoder.
- A v2 page NEVER replaces a v1 page in place. v2 pages appear
  only in a new generation; the old generation retains its v1
  pages until GC'd per the existing checkpoint-publication
  protocol.
- An operator rollback to a prior v1 generation uses the existing
  generation-revert mechanism (ADR 0014). No in-place downgrade
  of a v2 page to a v1 page is defined.

### 6.1 Upgrade sequence

1. Operator sets a policy via ADR 0060 that admits BACE encoding
   on a table.
2. The next checkpoint for that table publishes v2 pages for
   modified rows; untouched rows remain in v1 pages in the prior
   generation.
3. Background recompression (BACE-FR-BKP-003 — deferred ADR) MAY
   rewrite v1 pages into v2 pages over time. Until that lands,
   coexistence is the default.

### 6.2 Downgrade restriction

Once a v2 page has been written and published to a generation, a
downgrade of that generation to a v1-only build requires a
formally verified backup-and-restore cycle (BACE-FMT-004). No
runtime downgrade path is defined.

## 7. Impact on existing modules

If this ADR is accepted and implemented, the following modules
require sibling implementation ADRs:

- `page.rs` — add a v2 decoder branch. v1 decode path untouched.
- `row_store.rs` — accept v2 pages in replay and checkpoint
  paths. Recovery must be able to read both generations.
- `btree.rs` — immutable B+tree pages may be v2 if the index
  policy admits BACE encoding; the B+tree key format inside a v2
  index page is a separate ADR (relates to BACE-FR-IDX-001).
- `buffer_pool.rs` — the LRU cache keys do not change; cached
  pages may be either v1 or v2.
- `index_recovery.rs` — generation-scoped as today; must branch
  on the page version it sees.
- `checkpoint_manifest.rs` — the manifest record contains the
  generation LSN; existing format unchanged. A future subfield
  recording which page formats appear in the generation is
  deferred.
- `backup.rs` / `database_backup.rs` / `history_backup.rs` — must
  preserve the page format verbatim in backups (no implicit
  conversion).

**None of these ADRs are drafted yet.** They become possible only
after 0060 + 0061 + 0062 + 0063 are all accepted.

## 8. Alternatives considered

**A. Enlarge the page to 8 KiB or 16 KiB for v2.** Rejected: page
size is a load-bearing invariant through `page.rs`, `row_store.rs`,
`btree.rs` and `buffer_pool.rs`. A larger page size is a separate
design project with BRPG v3 considerations.

**B. Keep `format_version = 1` and repurpose the flags byte.**
Rejected: silently flipping flag semantics at `format_version = 1`
would make an old v1 reader accept a v2 page as "no flags set"
when in fact the flag bits encode critical information. The
version bump is the explicit fail-closed signal.

**C. Use a second magic (e.g. `"BRPG"` for v1, `"BR2P"` for v2).**
Rejected: the existing `format_version` field is already in the
header and already fail-closes on unknown values. A magic change
is unnecessary and would complicate backup / restore tooling.

**D. Embed the BACE envelope's magic `"BACE"` at page offset 32
and treat the page as a thin header around the envelope.** This is
effectively what `feature_flags.payload_is_bace_envelope` enables.
Rejected as a separate design because without the flag bit a v2
reader cannot distinguish "BACE envelope" from "plain bytes that
happen to start with those 4 bytes".

**E. Allow multiple BACE envelopes per page.** Would improve
space usage for small rows. Rejected for v2.0: adds complexity
to recovery and is not required by BACE-FR-ROW-001 for the first
integration. A future v2.1 flag can add it.

## 9. Open design questions (owner review)

1. **Initial coverage.** Does v2.0 target only `PageKind::Data`,
   or also `Index` and `FreeMap`? The draft permits any kind; is
   that the owner's intent?
2. **Reserved-bit strictness.** v2.0 fails closed on any reserved
   bit. Keep strict, or let v2.0 readers ignore bits they do not
   understand (making forward-compat v2.1 easier)?
3. **Mixed-generation lifetime.** How long are v1 pages allowed
   to coexist with v2 pages in a single directory? Forever, until
   next checkpoint, or until operator opt-in migration?
4. **Background recompression trigger.** BACE-FR-BKP-003 covers
   recompression lifecycle. Is BRPG v1 → v2 rewriting a mandatory
   part of that ADR, or an operator-driven opt-in?
5. **B+tree v2.** Index pages (`PageKind::Index`) under BACE
   encoding need their own key-encoding ADR. Do we draft that
   immediately after 0063, or defer?
6. **`.derc` coexistence.** `.derc` columnar segments are a
   separate format (BACE §4.2). Should a single table's data pages
   (BRPG v2) and its columnar projection (`.derc`) negotiate
   policy together, or independently?
7. **Downgrade mechanism.** §6.2 requires a formally verified
   backup-and-restore cycle. Is that acceptable, or does owner
   want an explicit page-by-page downgrade decoder?

## 10. Impact on invariants

Nothing in this ADR violates SRS v1.1 §20 invariants:

- **INV-001** commit correctness: BRPG v2 pages reach disk through
  the same checkpoint and WAL paths as v1. No commit semantic
  changes.
- **INV-002** one authoritative state: the authoritative row
  state remains `RowStore`'s in-memory decoded BTreeMap plus its
  WAL. Pages are the physical encoding, not the authoritative
  state.
- **INV-003** no authorization gap: the page decoder does not
  carry authorization. Readers MUST apply caller-supplied caps
  from the policy service (ADR 0060) before allocating the
  envelope's declared decoded length.
- **INV-004** no unsafe automatic availability: N/A; no
  distributed decision.
- **INV-005** recoverable lifecycle: §6 above states the
  mixed-generation lifecycle and downgrade restriction. The v2
  upgrade is a copy-and-replace migration under the existing
  checkpoint publication protocol (ADR 0014), which already has
  a documented state machine.

## 11. Dependencies this ADR would add

None in design. Implementation would use the existing `crc32fast`
crate for the page CRC (same as v1) and whichever codec crates
the ADR 0061 codec registry registers for the BACE envelope.

## 12. What this ADR explicitly does NOT do

- Freeze an authenticated / encrypted page variant.
- Define overflow-chain or sidecar-dictionary semantics.
- Define multi-envelope pages.
- Specify B+tree v2 key encoding.
- Specify `.derc` segment migration.
- Specify BRWL WAL frame BACE embedding.
- Choose codec identifier numbers (ADR 0061 §5 Q1).
- Implement any decoder or writer.
- Modify any existing on-disk format today.
- Introduce any `BACE*` admission or `ResourceError` variant.
- Modify any existing ADR.

## 13. Acceptance evidence required before marking Accepted

1. **ADRs 0060, 0061, 0062 accepted first** (dependencies).
2. Owner sign-off on §9 open design questions, especially Q1
   (initial coverage), Q3 (coexistence lifetime) and Q5 (B+tree
   v2 draft ordering).
3. Golden-corpus commitment: once accepted, a static corpus of
   BRPG v2 pages (both plain-payload and BACE-envelope-payload
   variants, across every PageKind) added to the test suite and
   verified byte-identical on every supported target on every
   build.
4. Mixed-generation regression plan: every existing row-store /
   btree / backup / recovery test must pass with both v1-only
   and mixed v1+v2 directories.
5. Explicit statement that no production write will emit v2
   pages until implementation ADRs for the page, row-store,
   B+tree and backup modules are also accepted.

Only then can this ADR move from **PROPOSED** to **ACCEPTED**, and
only then can `page.rs` grow a v2 decode branch and `row_store.rs`
grow a mixed-generation reader.
