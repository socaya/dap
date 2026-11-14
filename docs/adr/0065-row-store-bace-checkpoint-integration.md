# ADR 0065 — `row_store` BACE checkpoint integration (implementation; **PROPOSED**)

**Status: PROPOSED — not accepted for implementation.**

Blocked on **ADR 0064** (accepted 14 November 2026) which provides
the `Page::new_v2` / `PageFlags` Rust API, plus the four accepted
BACE prerequisite ADRs (0060–0063). This ADR is the first
carrier-integration ADR that **produces real BRPG v2 bytes on disk
in production**: an opt-in `RowStore` entry point that BACE-encodes
checkpoint pages and decodes them transparently on recovery.

Owner review required before any code lands, per CLAUDE.md §4.9 and
ADR 0064 §13 item 4 (which held v2 writes behind this ADR).

## 1. Context

- ADR 0063 §6 established the mixed-generation policy: v1 and v2
  pages may coexist in a `RowStore` directory; the checkpoint
  manifest decides which generation is authoritative; v2 pages
  never replace v1 pages in place.
- ADR 0064 shipped the `page.rs` v2 decoder/writer. `Page::new_v2`
  can produce v2 bytes; `Page::decode` auto-selects. But **no
  caller invokes `new_v2` today** — every production write path
  continues to call `Page::new(..)` which emits v1.
- Slice Q shipped `bace::{codec, envelope, registry, policy}` as
  a standalone module. `RawCodec` at ID 48 is the only codec; it
  is `Determinism::Deterministic` and `critical: true`.
- The `PolicyService` trait (ADR 0060) answers:
  "what compression profile, what allowed codec set, what
  per-page max decoded bytes?"
- BACE-FR-ROW-001 (OLTP hot row encoding) and BACE-FR-ROW-002
  (page compatibility) target this carrier.

## 2. Scope of this ADR

**In scope:**

- A new opt-in `RowStore` entry point that accepts a
  `PolicyService` and a `CodecRegistry`.
- Checkpoint writer branches: when the resolved
  `compression.profile` is not `Raw`, encode each data-kind page's
  payload through a codec, wrap in a BACE envelope (ADR 0062),
  emit as a BRPG v2 page (ADR 0064).
- Recovery reader branches: when a decoded page carries
  `flags.payload_is_bace_envelope`, unwrap the envelope and decode
  through the registered codec; otherwise treat the payload as
  plain bytes (existing v1 path).
- Mixed-generation tolerance: a directory may contain any mix of
  v1 checkpoints and v2 checkpoints; recovery picks the manifest-
  pointed generation and decodes it according to its page
  version.
- Fail-closed behavior: a resolved codec missing from the registry,
  a corrupt envelope, or a payload size exceeding the caller's
  cap all abort the checkpoint or the open, never produce partial
  state.

**Explicitly out of scope** (each needs its own ADR):

- **WAL frame BACE encoding.** `BRWL` frames keep their existing
  format; the WAL is untouched. Future ADR 0072.
- **B+tree index page v2.** `btree.rs` continues to emit v1 index
  pages. Future ADR 0069.
- **Backup / restore BACE.** `backup.rs`, `database_backup.rs`
  and `history_backup.rs` continue to copy pages verbatim (v1 or
  v2 is already preserved per the existing byte-for-byte copy
  contract, but the backup pipeline itself gains no new
  compression yet). Future ADR 0067.
- **Any codec beyond RAW.** Only RAW (ID 48) is registered today.
  Future ADR 0066 adds ZSTD.
- **BACE-SEL-002 avoid-expansion selection logic.** With RAW as
  the only codec, encoded size equals decoded size, so avoid-
  expansion is moot. When ZSTD lands, a selection-logic ADR adds
  the per-page cost comparison.
- **SQL `ALTER TABLE ... SET COMPRESSION`.** Deferred per
  ADR 0060 §5 item 6.
- **Catalog persistence of policy tree.** `PolicyService` is
  constructed in-memory by the caller; levels 2–5 persistence is
  a separate ADR per ADR 0060 §5 item 3.
- **`.derc` column segment BACE.** Future follow-up.

## 3. Proposed design

### 3.1 New entry point

```rust
impl RowStore {
    /// Open a RowStore under a BACE policy. Recovery remains
    /// backward-compatible: a directory written by the pre-BACE
    /// RowStore::open path (v1 pages only) opens unchanged. New
    /// checkpoints written through this entry point MAY emit v2
    /// pages depending on the resolved policy.
    pub fn open_with_bace<P, R>(
        root: impl AsRef<Path>,
        policies: StorageLimits,
        policy_service: Arc<dyn PolicyService + Send + Sync>,
        registry: Arc<R>,
    ) -> StoreResult<Self>
    where
        R: CodecRegistry + Send + Sync + 'static;
}
```

Shape rationale:

- `Arc<dyn PolicyService + Send + Sync>` because the service is
  consulted on every checkpoint and recovery, potentially from
  background threads (admin paths).
- `Arc<R>` generic on the registry: lets tests inject alternate
  registries; the shipped production call uses
  `Arc::new(BuiltInRegistry::new())`.
- The existing `RowStore::open` and `open_with_policies` entry
  points stay. They construct a store with **no BACE policy
  attached** — every checkpoint continues to emit v1 pages and
  every recovery continues to treat all payloads as plain bytes.
  This preserves wire compatibility with pre-ADR-0064 stores.

### 3.2 Checkpoint writer branch

The current `RowStore::checkpoint()` builds data pages from an
ordered row iterator, writes them into `snapshot-<lsn>.tmp`, and
renames to `snapshot-<lsn>.brp` after sync.

Under BACE, the writer gains an inner step per data page:

```text
for each page_batch:
    1. serialize rows into `plain: Vec<u8>` as today
    2. if self.bace is None OR resolved.profile == Raw:
           emit Page::new(..., plain)        // v1, no envelope
       else:
           codec_id = pick_codec(&resolved.allowed_codecs)
           codec    = registry.codec(codec_id)
                         .ok_or(StoreError::BaceCodecUnavailable)?
           let mut encoded = Vec::with_capacity(plain.len());
           codec.encode(&plain, &mut encoded)?;
           let envelope = write_envelope(&mut env_buf,
               &EnvelopeHeader {
                   codec_id,
                   codec_format_version: codec.capability().format_version,
                   flags: EnvelopeFlags {
                       payload_is_raw: codec_id == RAW_ID,
                       critical: codec.capability().flags.critical,
                       ..EnvelopeFlags::default()
                   },
                   encoded_length: encoded.len() as u32,
                   decoded_length: plain.len() as u32,
               }, &encoded)?;
           emit Page::new_v2(..., env_buf,
               PageFlags::empty().with_bace_envelope(true))
```

`pick_codec` is a trivial selector in this ADR: it returns the
first codec from `resolved.allowed_codecs` whose capability is in
the registry. With only RAW registered today, this is deterministic
— `pick_codec` returns `RAW_ID` or the checkpoint fails. A richer
selection algorithm (BACE §5.1 adaptive cost model) is a later
ADR.

### 3.3 Checkpoint failure semantics

If any step inside the BACE path fails (codec not registered,
envelope write fails, policy service emits a `CapabilityUnavailable`
veto that empties the allow-set), the whole checkpoint fails
closed with a new `StoreError::BaceCodecUnavailable { codec_id }`
or `StoreError::BaceEnvelope(EnvelopeError)` variant. The
`snapshot-<lsn>.tmp` is discarded; the manifest is never updated;
the WAL is never truncated.

Checkpoints are infrequent and atomic; a partial checkpoint is
worse than a failed checkpoint. Operators observe the failure via
`tracing` (ADR 0060 §5 item 7), fix the policy, and retry.

### 3.4 Recovery reader branch

On open, the existing recovery path reads `snapshot-<lsn>.brp`
page by page. The decoder (`Page::decode`) already auto-selects
v1 vs v2 per ADR 0064. The new step is: after `Page::decode`, if
`page.flags.payload_is_bace_envelope`:

```text
let caps = EnvelopeCaps {
    max_decoded_length: policy.resolve_u32(CompressionMaxDecodedPageBytes),
    max_encoded_length: PAGE_SIZE as u32,  // can never exceed a page
};
let env = read_envelope(&page.payload, caps)?;
let codec = registry.codec(env.header.codec_id)
    .ok_or(StoreError::BaceCodecUnavailable { codec_id: env.header.codec_id })?;
if codec.capability().format_version != env.header.codec_format_version {
    return Err(StoreError::BaceCodecFormatVersionMismatch { ... });
}
let mut decoded = Vec::with_capacity(env.header.decoded_length as usize);
codec.decode(env.payload, env.header.decoded_length, &mut decoded)?;
// decoded now contains the same bytes the v1 path would have seen
```

`EnvelopeCaps.max_encoded_length` is always `PAGE_SIZE` because
the envelope is embedded in a 4 KiB page; a declared envelope
longer than a page is corruption. `max_decoded_length` comes from
the policy service — this is the ADR 0062 §3.3 pre-allocation
cap.

A store opened via the **old** `RowStore::open` on a directory
containing v2 pages fails closed: the store has no registry and no
policy, so the recovery branch returns
`StoreError::BaceOpenWithoutBaceWiring`. This forces operators to
use `open_with_bace` consistently once a directory contains any
v2 pages.

### 3.5 Mixed-generation reading

The `checkpoint.current` manifest points at exactly one generation
at a time (ADR 0014). Recovery reads only that generation. Older
generations remain on disk until GC. Under BACE:

- Opening a directory whose manifest points at a v1-only
  generation: works via `open` or `open_with_bace`. The latter
  reads all v1 payloads as plain bytes (no envelope check).
- Opening a directory whose manifest points at a generation
  containing any v2 pages: requires `open_with_bace`. The old
  `open` returns `BaceOpenWithoutBaceWiring` after the first v2
  page is observed.
- Checkpointing under `open_with_bace` writes a new generation per
  the current policy. If the policy resolves to `Raw`, that
  generation's pages are v1-shape; otherwise v2.

A directory's lifecycle may alternate between v1-only and
v2-containing generations freely. The GC policy (unchanged from
today) only deletes snapshots the manifest no longer references.

### 3.6 Admission interactions

The new call paths observe existing admission already:

- `RowStore::open_with_bace` returns `RowStoreLimits`-admitted,
  `IndexLimits`-admitted, `IndexBuildLimits`-admitted, and
  `DirectoryLimits`-admitted (same as `open_with_policies` today).
- The envelope's `EnvelopeCaps.max_decoded_length` comes from the
  policy service; `max_encoded_length` is the page size. The
  pre-allocation cap in `read_envelope` prevents an oversized
  envelope from allocating a payload buffer larger than a page.
- No new `ResourceError` variant is introduced by this ADR.
  Capability-unavailability and codec-format-version-mismatch
  are `StoreError` variants because they pertain to persistent
  format recovery, not runtime admission.

### 3.7 New `StoreError` variants

```rust
pub enum StoreError {
    // existing variants unchanged
    ...
    BaceCodecUnavailable { codec_id: CodecId },
    BaceCodecFormatVersionMismatch { codec_id: CodecId, declared: u16, build_supports: u16 },
    BaceEnvelope(EnvelopeError),
    BaceDecode(DecodeError),
    /// Directory contains v2 pages but the caller opened the store
    /// via `open`/`open_with_policies` (no BACE wiring).
    BaceOpenWithoutBaceWiring { page_id: u64 },
}
```

`EnvelopeError` and `DecodeError` are the existing error types
from `bace::envelope` and `bace::codec`.

## 4. Compliance obligations (implementation test checklist)

An implementation of this ADR MUST demonstrate every item below
through a passing test:

1. Every existing `row_store_durability.rs` and `row_store_limits.rs`
   test continues to pass unchanged (the `open` and
   `open_with_policies` entry points are unaffected).
2. **v1 bit-compatibility.** A store opened via the old `open`
   path, checkpointed, and reopened via the old `open` path
   produces byte-identical `snapshot-<lsn>.brp` contents to the
   pre-ADR-0065 baseline. Golden vector in the test suite.
3. **BACE-opt-in with Raw profile emits v1 pages.** A store opened
   via `open_with_bace` with a policy resolving
   `compression.profile = Raw` emits v1 pages on checkpoint. The
   checkpoint file is byte-identical to the v1 baseline for the
   same row set.
4. **BACE-opt-in with non-Raw profile emits v2 pages.** A store
   opened via `open_with_bace` with a policy resolving
   `compression.profile = Balanced` and `allowed_codecs = [RAW_ID]`
   emits v2 pages whose payload is a valid BACE envelope wrapping
   the same plain bytes that the v1 path would have written.
5. **Recovery round-trip.** A store checkpointed under the Balanced
   profile and reopened via `open_with_bace` yields the exact
   same row set (same keys, same values, same count) as before
   the checkpoint.
6. **Mixed-generation recovery.** A directory with a v1 snapshot
   (gen N) followed by a v2 snapshot (gen N+1 under Balanced
   profile) opens correctly; the manifest points at the v2
   generation; all rows present after reopen.
7. **Cap enforcement.** A policy with
   `compression.max_decoded_page_bytes = 1024` applied to a
   recovery that encounters a page with `decoded_length = 2048`
   fails closed with `BaceEnvelope(EnvelopeError::DecodedLengthExceedsCap { .. })`.
8. **Old `open` on a v2-containing directory fails closed.**
   `RowStore::open(root)` on a directory whose manifest-pointed
   generation contains v2 pages returns
   `BaceOpenWithoutBaceWiring { page_id: _ }` — recovery does not
   read past the offending page, the WAL is not mutated, and the
   store is not published.
9. **Capability veto.** A policy whose
   `compression.allowed_codecs` contains only an unregistered ID
   fails checkpoint with `BaceCodecUnavailable { codec_id: _ }`.
   No snapshot file is published and the WAL is unchanged.
10. **Format version mismatch on recovery.** A hand-crafted v2
    page declaring `codec_format_version = 99` for RAW fails
    closed with `BaceCodecFormatVersionMismatch` on recovery.
11. **Checkpoint failure under BACE leaves prior state intact.**
    A checkpoint that fails inside the BACE path (e.g. the policy
    service is swapped mid-call to one with an unregistered
    allow-set) leaves the previous generation authoritative; the
    WAL is unchanged; the `.tmp` is cleaned.
12. **`open_with_bace` with the Raw profile and the default
    registry opens a legacy v1 directory without any error path
    firing.** Backward compatibility with today's stores.

## 5. Alternatives considered

**A. Always emit v2 pages (even under Raw profile) with
`payload_is_raw=true`.** Uniform read-side treatment. Rejected:
every v2 page costs an extra 36 envelope bytes. Under Raw, that
is pure overhead with zero compression benefit. The "Raw ≡ v1"
rule keeps the opt-out path cost-free.

**B. Thread the `PolicyService` through every `put`/`delete`
call.** Would allow per-operation policy. Rejected: BACE policy
is a checkpoint-granularity decision. Per-op policy complicates
the mutation mutex path and adds no value until per-page
selection (BACE-SEL-004) arrives.

**C. Embed BACE wiring on the existing `open_with_policies`
entry point via an optional field in `StorageLimits`.** Would
avoid a new public method. Rejected: `StorageLimits` is an
accounting type; adding behavioral wiring to it muddles the
abstraction. A separate entry point makes the BACE opt-in
explicit at every call site.

**D. Store the resolved policy snapshot on the `RowStore` at
`open` time instead of consulting the `PolicyService` live.**
Simpler, faster. Rejected: operators need to be able to flip
policy between checkpoints (e.g. switch a legacy store to
compressed storage). A live-consulted service supports that; a
frozen snapshot does not.

**E. Fail the checkpoint silently (continue with v1) when the
BACE path errors.** Would increase availability. Rejected: a
silent fallback hides policy-service bugs and makes operators
unaware that a checkpoint ran without the compression they
expected. Explicit failure + retry is the correct contract.

## 6. Open design questions (owner review)

1. **New entry point name.** `open_with_bace` is the draft
   default. Alternatives: `open_with_compression(...)`,
   `open_compressed(...)`, replace `open_with_policies` with a
   variant that takes an `Option<BaceWiring>`. Confirm.
2. **`BaceOpenWithoutBaceWiring` behaviour.** Draft fails closed
   on the first v2 page observed. Alternative: fail closed at
   `open()` before any page is read, by inspecting the generation's
   first page or a manifest field that notes whether v2 pages
   are present. The first-page approach is simpler but delays
   the error; a manifest field requires an ADR 0014 extension.
3. **Checkpoint failure policy.** Draft fails the whole
   checkpoint on any BACE error. Alternative: emit as v1 and
   log a warning. Confirm the strict path; it matches the
   project's existing fail-closed culture.
4. **`StoreError` variant naming.** Draft uses `BaceCodec*` /
   `BaceEnvelope` / `BaceDecode` for clear provenance. Alternative:
   collapse into one `BaceError(Box<dyn Error>)`. The draft
   preserves match-able specificity.
5. **Policy service lifecycle.** Draft takes
   `Arc<dyn PolicyService + Send + Sync>` and consults it on
   every checkpoint/recovery. Alternative: take a `&PolicyService`
   borrow at `open` and clone an in-memory snapshot. Live
   consulting matches ADR 0060 §3.5 atomicity better; confirm.
6. **GC of a v2 generation.** No change from existing GC: snapshots
   the manifest no longer references are eligible for deletion.
   Confirm that compressed snapshots follow the same rule.
7. **What page kinds carry envelopes?** Draft: `PageKind::Data`
   only. `PageKind::Metadata` (checkpoint-metadata page zero) and
   `PageKind::Index` stay v1. Future ADR 0069 handles Index.

## 7. Impact on invariants

- **INV-001 commit correctness:** no change. BACE operates on
  checkpoint pages; WAL + commit acknowledgement paths are
  unchanged.
- **INV-002 one authoritative state:** no change. Row state
  remains authoritative in-memory + WAL; checkpoint pages are a
  physical encoding whose compression is invisible to logical
  state.
- **INV-003 no authorization gap:** the recovery decoder uses
  caller-supplied `EnvelopeCaps` from the policy service, so
  the admission context propagates. The checkpoint writer
  resolves the policy under the current scope; if a per-tenant
  level is later added (ADR 0060 §5 item 1), the writer picks it
  up automatically.
- **INV-004 no unsafe automatic availability:** N/A; single-
  process scope.
- **INV-005 recoverable lifecycle:** mixed-generation lifecycle
  per ADR 0063 §6 is implemented verbatim. Downgrade to a
  v1-only build after v2 pages have been written requires a
  backup-and-restore cycle per ADR 0063 §6.2; that remains the
  operator's responsibility.

## 8. Dependencies this ADR would add

None. The implementation uses only existing workspace deps and
the already-accepted `bace::*` surface:

- `bace::envelope::{write_envelope, read_envelope, EnvelopeHeader,
  EnvelopeFlags, EnvelopeCaps, EnvelopeError}`
- `bace::codec::{Codec, CodecId, DecodeError}`
- `bace::codec::raw::RAW_ID`
- `bace::registry::CodecRegistry`
- `bace::policy::{PolicyService, PolicyKey, ScopeChain, CompressionProfile}`

## 9. What this ADR explicitly does NOT do

- Modify `btree.rs`, `buffer_pool.rs`, `index_recovery.rs`,
  `backup.rs`, `database_backup.rs`, `history_backup.rs`,
  `checkpoint_manifest.rs`, `legacy_migration.rs`,
  `history_migration.rs`, or any catalog / SQL code.
- Touch the WAL frame format (`BRWL`).
- Introduce any new codec.
- Implement per-page codec selection (BACE §5.1 adaptive cost
  model). The initial `pick_codec` returns the first registered
  codec in the allow-set; with only RAW registered, that is
  deterministic.
- Implement the BACE-SEL-002 avoid-expansion comparison. With
  RAW as the only codec, encoded size equals decoded size.
- Persist the policy tree in the catalog. Policy is in-memory,
  constructed by the caller.
- Emit any production trace records yet. The `tracing` audit
  wiring per ADR 0060 §5 item 7 is a sibling slice; this ADR
  structures the error paths so future tracing additions are
  one-line.
- Grow the `RowStore` public API beyond `open_with_bace` and the
  new `StoreError` variants. All existing methods keep their
  signatures.

## 10. Acceptance evidence required before marking Accepted

1. ADR 0064 accepted — **satisfied** (14 November 2026).
2. ADRs 0060, 0061, 0062, 0063 accepted — **satisfied**.
3. Owner sign-off on §6 open design questions.
4. Commitment to the 12-item test matrix in §4.
5. Explicit statement that opening a directory via the old
   `open` after a BACE-written checkpoint fails closed with
   `BaceOpenWithoutBaceWiring`. This is a backward-compatibility
   trade-off: a store that has ever used BACE cannot be reopened
   by a BACE-unaware caller. Confirm this is acceptable.

Only then can this ADR move from **PROPOSED** to **ACCEPTED**, and
only then can `row_store.rs` grow the BACE branches that produce
the first production BRPG v2 bytes on disk.
