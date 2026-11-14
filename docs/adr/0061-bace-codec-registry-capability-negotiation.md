# ADR 0061 — BACE codec registry and capability negotiation

**Status: ACCEPTED — 14 November 2026** for the architecture
(namespace partition, descriptor shape, negotiation algorithm,
fail-closed semantics).

**P1 CODEC ID ASSIGNMENTS IN RANGES 16–47 REMAIN DEFERRED.** The
review package explicitly flagged these as having no default and
requiring owner choice. The first implementation (Slice Q)
allocates only one codec ID — RAW at `48` in the outer-compression
range, which has no legacy constraint and no interference with
future inner-encoding assignments. Inner-encoding IDs stay
unassigned until owner signoff.

Owner review completed via `BACE_ADR_REVIEW_PACKAGE.md` §4 for
all other questions (recorded as resolved in §5 below).

## 1. Context

BACE SRS v1.0 §3.1 requires:

> **BACE-ARCH-002 — Codec registry.** Each codec MUST publish stable
> identifier, format version, supported types, parameters, min/max
> sizes, decoder bounds, determinism and SIMD availability. Unknown
> critical codecs MUST fail closed.

BACE §5 enumerates eleven codec families (BACE-CODEC-001..011). BACE
§6 adds the backward-compatibility constraint:

> Existing `.derc` v1 identifies codecs 0 Plain, 1 PlainUtf8, 2
> Dictionary and reserves 3 RLE, 4 Delta, 5 BitPacked. The current
> writer is limited to the first three; ZSTD level 3 wraps pages by
> default when enabled. New codecs MUST NOT repurpose reserved
> numeric meanings incompatibly.

BACE-NFR-PORT-001 adds the portability obligation:

> Compressed payload decode and visible SQL results MUST be
> equivalent across supported architecture/OS builds, including
> different CPU SIMD paths.

Today the only compression in-tree is a hardcoded `ZSTD-3` wrap on
`.derc` pages; no `Codec` trait, no capability descriptor, no
registry, no negotiation surface. ADR 0060 (policy service) depends
on being able to ask the registry "which codecs are available on
this build?".

## 2. Scope of this ADR

**In scope:** the codec identifier namespace, the `CodecCapability`
descriptor fields, the registry API shape, the negotiation
algorithm, the fail-closed contract for unknown codecs, and the
portability guarantee.

**Explicitly out of scope** (each needs its own ADR):

- Each codec's actual implementation and byte layout.
- The compressed-envelope wire format that carries the codec ID at
  rest or over the wire — ADR 0062.
- The BRPG v2 feature-flag page format — ADR 0063.
- The replication / federation negotiation wire protocol — Gate 4+
  workstream; this ADR specifies the data model, not the wire.
- The authenticated encryption format that wraps the envelope — the
  BRD-SEC-003 workstream.

## 3. Proposed design

### 3.1 Codec identifier namespace

Codec IDs are `u16`. The namespace is pre-partitioned so legacy
allocations stay stable and future allocations have room:

| Range | Allocation | Notes |
|---|---|---|
| **0–5** | **FROZEN — mirror `.derc` v1 legacy IDs** | `0 Plain`, `1 PlainUtf8`, `2 Dictionary`, `3 RLE` (reserved), `4 Delta` (reserved), `5 BitPacked` (reserved). Production invariant: an ID in this range MUST decode the same payload on BACE as on `.derc` v1. |
| 6–15 | RESERVED — legacy compatibility buffer | Unallocated. May be used only for backward-compatibility bridges, never for new BACE codec families. |
| 16–31 | Core inner encodings (BACE §5) | Constant, RLE (BACE native), Dictionary+packed, FOR, Delta, BitPacked, Boolean bitmap. |
| 32–47 | Specialized inner encodings | FSST or equivalent, Gorilla, Chimp, ALP. |
| 48–63 | Outer stream compression | LZ4, ZSTD, RAW. |
| 64–255 | RESERVED — future BACE | |
| 256–65_535 | RESERVED — vendor / experimental | Must set the `experimental: true` capability flag. Never written into a durable envelope without explicit operator opt-in. |

**Concrete number assignments inside 16–63 are deferred to this
ADR's acceptance review** (§5 Q1). Freezing a number is an
irrevocable commitment: once published, an ID MUST decode the same
payload on every future Bradian build forever.

### 3.2 Backward compatibility with `.derc` v1

- `.derc` segments written before BACE continue to use IDs 0, 1, 2
  (and MAY use the reserved 3, 4, 5). The BACE registry MUST
  resolve these IDs and SHOULD route them through the same decoder
  path used today, so no re-encoding is required at read time.
- BACE codec families that correspond semantically to legacy IDs
  (e.g. BACE RLE, BACE Dictionary) MUST be allocated **new** IDs
  in the 16–31 range. They MUST NOT reuse 0–5.
- The `.derc` → BACE transition for an individual segment is a
  rewrite, not an in-place reinterpretation.

### 3.3 `CodecCapability` descriptor

Every registered codec publishes an immutable capability descriptor.
The shape below is illustrative; the committed field set waits on
owner review.

```rust
pub struct CodecCapability {
    pub id: CodecId,                       // u16
    pub format_version: u16,                // bumped on breaking changes
    pub supported_types: TypeMask,          // bitset of DataType variants
    pub parameters: ParamSchema,            // declared name/type/range per param
    pub min_input_bytes: u32,
    pub max_input_bytes: u32,
    pub max_output_bytes: u32,              // BACE-FMT-003 bounded decompression
    pub decode_cpu_ns_per_kib_estimate: u32,
    pub determinism: Determinism,           // Deterministic | HardwareDependent
    pub simd_availability: SimdCapabilities,
    pub flags: CapabilityFlags,             // experimental, critical, encryption-ordered
}

pub enum Determinism {
    /// Same input bytes always produce the same output bytes, on
    /// every supported architecture and SIMD path (BACE-NFR-PORT-001).
    Deterministic,
    /// Output bytes may differ across SIMD paths but visible SQL
    /// results do not. Allowed only for inner encodings whose
    /// cross-build equivalence is proven by golden corpora.
    HardwareDependent,
}

pub struct CapabilityFlags {
    pub critical: bool,       // if critical, missing support MUST fail closed
    pub experimental: bool,   // never selected without explicit opt-in
    pub preserves_null_distinct: bool,
    pub allows_outer_compression: bool,
}
```

### 3.4 Registry API (proposed)

```rust
pub trait CodecRegistry {
    /// Enumerate every codec available on this build. Immutable
    /// after registry construction; concurrent readers see the
    /// same set.
    fn capabilities(&self) -> &[CodecCapability];

    /// Resolve one codec ID. Returns `None` for unregistered IDs.
    /// Callers MUST treat `None` as fail-closed; there is no
    /// silent fallback.
    fn capability(&self, id: CodecId) -> Option<&CodecCapability>;

    /// Negotiate the intersection of two capability sets. Returns
    /// the agreed codec list and a list of any codec marked
    /// `critical` on one side but absent on the other — the latter
    /// is the fail-closed signal.
    fn negotiate(&self, remote: &[CodecCapability]) -> Negotiation;
}

pub struct Negotiation {
    pub agreed: Vec<CodecId>,
    pub missing_critical: Vec<CodecId>,  // if non-empty, abort the session
}
```

### 3.5 Negotiation algorithm

Given two capability sets `L` (local) and `R` (remote):

1. Compute `A = { id ∈ L.ids ∩ R.ids | L.format_version(id) ==
   R.format_version(id) }`. Format-version mismatch means the two
   sides do not agree on the byte layout; this is NOT silent
   downgrade territory.
2. For each `id` in `A`, parameters must intersect — a side that
   insists on a parameter value the other side refuses rejects the
   codec.
3. Compute `missing_critical = { id | (id ∈ L.critical ∧ id ∉ R.ids)
   ∨ (id ∈ R.critical ∧ id ∉ L.ids) }`.
4. If `missing_critical` is non-empty, **negotiation fails closed**.
   The session is aborted; there is no silent downgrade. Operators
   see an audit record naming the missing critical codecs.
5. Otherwise, the agreed set `A` is published to the policy service
   (ADR 0060) and becomes the `compression.allowed_codecs` bitset
   for the session.

### 3.6 Fail-closed invariants (BACE-ARCH-002)

1. **Unknown codec ID on decode.** `capability(unknown_id) → None`.
   Any decoder presented with an unknown ID MUST return
   `DecodeError::UnknownCodec { id }`. No partial decode, no
   best-effort passthrough.
2. **Unsupported format version.** Known ID, newer `format_version`
   on disk than on the current build → `DecodeError::UnsupportedFormatVersion
   { id, declared, max_supported }`.
3. **Out-of-range parameter.** Declared parameter outside the
   registered range → `DecodeError::InvalidParameter { id, param, value }`.
4. **Declared vs actual length mismatch.** Decoded output length
   does not match the declared length in the envelope (ADR 0062) →
   `DecodeError::DecodedLengthMismatch`.
5. **Determinism violation under test.** `Deterministic` codecs
   must produce bit-identical output across SIMD paths on the
   golden corpus; a divergence is a release-blocking defect.

### 3.7 Portability (BACE-NFR-PORT-001)

The registry MUST ship a **golden corpus**: for every codec marked
`Deterministic`, one or more `(input, format_version, expected_output)`
tuples that every build on every supported architecture must
reproduce bit-identically. The corpus lives next to the codec
implementation; CI runs it on every supported target.

`HardwareDependent` codecs are permitted but MUST ship paired
scalar and SIMD implementations with a differential test proving
visible SQL results are identical.

### 3.8 Registration

The build-time registry is static. There is **no runtime
registration** of new codecs — the capability set a process exposes
is a function of its build. This matches BACE-ARCH-005 (versioned
compatibility) and makes negotiation decidable: both peers
negotiate from their immutable build-time sets.

A future enterprise extension may allow signed codec plugins; that
is out of scope for this ADR.

## 4. Alternatives considered

**A. One flat `CodecId = u32` space with no partition.** Simpler.
Rejected: makes legacy `.derc` compatibility constraint harder to
state and leaves no obvious room for vendor extensions.

**B. Tagged union of codec enums instead of integer IDs.** More
type-safe in Rust. Rejected: the on-disk encoding still needs an
integer, and the registry must be dynamic enough to evolve without
breaking older readers' type declarations.

**C. Dynamic runtime registration.** Would allow operator-loaded
codec plugins. Rejected: adds a signature / sandbox threat surface
(BACE-SEC-002) and makes capability negotiation undecidable without
a global signed capability list. Deferred.

**D. Let each subsystem own its own codec IDs.** Current state.
Rejected per BACE-ARCH-002 "stable identifier"; cross-subsystem
replication cannot negotiate consistently.

## 5. Design decisions (resolved)

Resolved on 14 November 2026 via `BACE_ADR_REVIEW_PACKAGE.md` §4.

1. **Concrete number assignments inside 16–47 REMAIN DEFERRED.**
   The review package §4.2 P1 had no default. The first
   implementation allocates ID `48` for RAW only (outer-compression
   range, no legacy constraint, no interference with future
   inner-encoding assignments). Inner-encoding IDs 16–47 stay
   unassigned until owner signoff. Any ADR that proposes a specific
   inner-encoding ID must be reviewed separately.
2. **Format-version strategy.** Monotone `u16` per codec, bumped
   only on breaking changes. No major/minor split. (Review package
   P2.)
3. **Initial codec set.** `RAW` (ID 48, deterministic by definition)
   ships first. `ZSTD` follows in a later slice after its
   determinism corpus is prepared. Inner encodings defer until P1
   is decided. (Review package P3.)
4. **Determinism bar.** `RAW` is `Deterministic`. `ZSTD` will be
   `HardwareDependent` for the first implementation until a pinned
   library version + SIMD path produces a reproducible golden
   corpus. `LZ4` is TBD per the empirical corpus. (Review package
   D1.)
5. **Experimental flag semantics.** Writes only. Reads of
   previously-written experimental payloads are always honored.
   (Review package D2.)
6. **Vendor / third-party codec IDs (256–65_535).** Open to
   third-party BACE implementations; third-party codecs must set
   the `experimental` flag by default. (Review package E1.)
7. **Plugin future.** Post-1.0 with a dedicated signed-plugin ADR.
   Not in the Gate 0–9 roadmap. (Review package E2.)

## 6. Impact on invariants

Nothing in this ADR violates SRS v1.1 §20 invariants:

- **INV-001** commit correctness: codec selection changes encoded
  bytes but not commit semantics.
- **INV-002** one authoritative state: capability negotiation does
  not touch row state.
- **INV-003** no authorization gap: capability negotiation does
  **not** carry authorization context; it is a format-level
  protocol only. Combining negotiated capabilities with tenant
  authorization (BACE-SEC-002: cross-tenant dictionary reuse MUST
  NOT leak) is the policy service's responsibility (ADR 0060).
- **INV-004** no unsafe automatic availability: fail-closed on
  unknown critical codecs is the registry's enforcement.
- **INV-005** recoverable lifecycle: format-version bumps need a
  documented rollback path (BACE-FMT-004); this ADR requires one
  but does not specify it.

## 7. Dependencies this ADR would add

None in design. If accepted and implemented, the first codec
registration using the actual `zstd` and `lz4_flex` crates is
covered by the existing CLAUDE.md §5 allowlist. FSST / Gorilla /
Chimp / ALP implementations are **not in the allowlist today**;
each would need an ADR entry added to §5 before its codec can be
registered.

## 8. What this ADR explicitly does NOT do

- Freeze codec ID numbers inside the 16–63 range. Only range
  partitioning is proposed here; final number assignment waits on
  owner review (§5 Q1).
- Specify the compressed-envelope byte layout. ADR 0062.
- Specify the BRPG v2 feature-flag page. ADR 0063.
- Specify a replication / federation wire protocol for exchanging
  capability sets.
- Implement any codec.
- Modify `.derc` or any existing on-disk format today.
- Introduce any `BACE*` admission or `ResourceError` variant.
- Modify any existing ADR.

## 9. Acceptance evidence (satisfied, with one deferral)

1. ADR 0060 accepted first — **satisfied** (both accepted in the
   same commit on 14 November 2026).
2. Owner sign-off on §5 decisions — **satisfied** via
   `BACE_ADR_REVIEW_PACKAGE.md`. **P1 (concrete inner-encoding
   codec IDs in range 16–47) remains deferred;** the first
   implementation uses only RAW at ID 48.
3. Threat-model review confirming capability negotiation does not
   leak tenant information (BACE-SEC-002) — deferred to the
   BRD-SEC-003 workstream; not required for RAW-only Slice Q.
4. Determinism policy confirmed — §5 item 4.
5. Plugin-future decision recorded — §5 item 7.

Implementation ADRs may now follow for RAW; any inner-encoding
codec (ID 16–47) requires P1 signoff before its implementation ADR
can be written.
