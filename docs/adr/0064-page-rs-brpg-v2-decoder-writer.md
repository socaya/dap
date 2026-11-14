# ADR 0064 — `page.rs` BRPG v2 decoder/writer (implementation; **PROPOSED**)

**Status: PROPOSED — not accepted for implementation.**

Blocked on **ADR 0063** (BRPG v2 feature-flag page) which is already
accepted. This ADR is the **first implementation ADR** in the BACE
workstream: it turns the ADR 0063 byte layout into an actual Rust
decoder/writer in `crates/bradian-relational/src/page.rs`.

Owner review required before any code lands, per CLAUDE.md §4.9. This
ADR freezes the **Rust API surface** that every carrier (row_store,
backup, `.derc` later) will use to read/write pages.

## 1. Context

- ADR 0063 froze the BRPG v2 byte layout: same 4 096 B page, same
  32-byte header; `format_version` bumps 1 → 2, and byte `[7]` is
  reinterpreted as a `feature_flags` bitfield.
- Today `crates/bradian-relational/src/page.rs` implements v1 only.
  A v1 reader presented with a v2 page returns
  `PageError::UnsupportedVersion(2)` — that is the ADR 0063 natural
  fail-closed signal.
- Slice Q (components Q.1–Q.4) built the standalone `bace` module
  (`bace::envelope`, `bace::codec`, `bace::registry`, `bace::policy`).
  Nothing in production writes a BACE envelope to disk yet because
  no carrier integrates with `bace` — this ADR is that integration
  for `page.rs`, and only `page.rs`. `row_store.rs` integration is
  a separate implementation ADR (0065).

## 2. Scope of this ADR

**In scope:**

- A `page.rs` decoder path for BRPG v2 pages alongside the existing
  v1 path. Both paths remain callable.
- A `page.rs` writer path that emits BRPG v2 pages when explicitly
  requested by the caller; the default writer continues to emit v1.
- The Rust API surface that carriers use: fields on `Page`, new
  constants, new `PageError` variants.
- Round-trip tests for v2 pages across every valid flag combination.
- Explicit preservation of every existing v1 `page_codec.rs` test.

**Explicitly out of scope** (each needs its own ADR):

- `row_store.rs` integration — ADR 0065.
- `btree.rs` v2 Index-page key encoding — a later ADR.
- `buffer_pool.rs` accounting changes — carrier integration ADRs.
- Any new codec beyond RAW — ADR 0066 (ZSTD) and successors.
- `backup.rs` / `database_backup.rs` BACE adapter — ADR 0067.
- `.derc` / BRWL BACE integration — follow-ups.
- Any use of the `bace::policy::PolicyService` to decide which
  pages to encode. This ADR exposes the mechanism; the policy
  wiring is a carrier concern.
- Any production write of a v2 page. This ADR makes v2 writes
  possible via the Rust API; it does **not** make any caller use
  them.

## 3. Proposed Rust API additions

### 3.1 New constants

```rust
pub const PAGE_VERSION_V1: u16 = 1;   // unchanged from today's PAGE_VERSION
pub const PAGE_VERSION_V2: u16 = 2;
```

The existing `pub const PAGE_VERSION: u16 = 1;` is kept as an alias
for `PAGE_VERSION_V1` so no caller in the current workspace breaks.

### 3.2 `PageFlags` bitfield

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageFlags {
    /// Bit 0 — payload is exactly one BACE v1 envelope
    /// (ADR 0062). `payload_length == envelope.encoded_length + 36`.
    pub payload_is_bace_envelope: bool,
    // Bits 1–2 (`has_overflow_chain`, `has_sidecar_dictionary`) are
    // reserved per ADR 0063 §3.3. They will become fields in a
    // later ADR; this struct does not expose them yet.
}

impl PageFlags {
    pub const fn empty() -> Self;
    pub const fn with_bace_envelope(self, on: bool) -> Self;
    pub fn to_byte(self) -> u8;
    pub fn from_byte(b: u8) -> Result<Self, PageError>;
}
```

`PageFlags::from_byte` applies ADR 0063 §3.3 strict-reserved-must-be-zero:
any bit set beyond bit 0 fails with `PageError::UnsupportedFlags(b)`.

### 3.3 New `Page` field

The existing `Page` struct gains **one** new field. Default value on
v1-decoded pages is `PageFlags::empty()`, so no existing caller
needs to touch it.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub page_id: u64,
    pub lsn: u64,
    pub kind: PageKind,
    pub payload: Vec<u8>,
    pub flags: PageFlags,     // NEW
}
```

### 3.4 Decoder behaviour

`Page::decode(buf)` auto-selects v1 or v2 based on the
`format_version` byte. Compatibility guarantees:

- A v1 page continues to decode exactly as today, returning a
  `Page` with `flags = PageFlags::empty()`.
- A v2 page follows the ADR 0063 §4 10-step read protocol: magic,
  version, kind, feature_flags (via `PageFlags::from_byte`),
  payload_length cap, page CRC32, zero-padded tail check, and
  (when `payload_is_bace_envelope` is set) a check that the first
  four payload bytes are `ENVELOPE_MAGIC`.
- A page with `format_version` outside {1, 2} fails closed with
  `PageError::UnsupportedVersion`.
- A v2 page with any reserved flag bit set fails closed with
  `PageError::UnsupportedFlags`.
- A v2 page with `payload_is_bace_envelope` but whose payload
  does not start with `ENVELOPE_MAGIC` fails closed with
  `PageError::BaceEnvelopeMagicMismatch { kind }` — a new variant.

**Important:** this ADR does **not** make `Page::decode` call
`bace::envelope::read_envelope`. The v2 decoder only verifies that
the flag/payload match structurally. Callers that want to decode the
envelope call `bace::envelope::read_envelope(&page.payload, caps)`
themselves. This keeps `page.rs` free of a dependency on the policy
service and keeps the ADR 0062 bounded-decompression caps a caller
concern.

### 3.5 Writer behaviour

Today `Page::new(page_id, lsn, kind, payload)` + `Page::encode(buf)`
writes a v1 page. That path **stays**. The default construction
produces a v1-encoded page.

New constructor for v2:

```rust
impl Page {
    pub fn new_v2(
        page_id: u64,
        lsn: u64,
        kind: PageKind,
        payload: Vec<u8>,
        flags: PageFlags,
    ) -> Result<Self, PageError>;
}
```

The encoder picks the on-disk version based on the `flags` field:

- If `flags == PageFlags::empty()`, the encoder emits a **v1 page**
  (back-compatible). Operators can still write v1-shape pages
  through the v2 constructor without unnecessarily tripping older
  readers.
- If `flags != PageFlags::empty()`, the encoder emits a **v2 page**
  with `format_version = 2` and the correct feature_flags byte.

This rule keeps the common "no flags" path wire-compatible with v1
and makes every v2-shape page's on-disk presence an **intentional**
operator decision.

### 3.6 New `PageError` variants

```rust
pub enum PageError {
    // existing variants unchanged
    ...
    UnsupportedFeatureFlag(u8),         // NEW — bits 1-7 of v2 feature_flags
    BaceEnvelopeMagicMismatch { kind: PageKind },  // NEW — v2 page with
                                                   // payload_is_bace_envelope
                                                   // but payload lacks "BACE"
}
```

The existing `UnsupportedFlags(u8)` variant is kept for v1 pages
with non-zero byte `[7]` (its original semantics). The new
`UnsupportedFeatureFlag` is used for v2 pages with reserved bits
set; the two paths are distinct so the error message stays clear.

### 3.7 Module boundaries

- `page.rs` MAY import `bace::envelope::ENVELOPE_MAGIC` (a
  `[u8; 4]` constant) solely to perform the structural check in
  §3.4 when `payload_is_bace_envelope` is set. It MUST NOT import
  `bace::envelope::read_envelope` or any other `bace` symbol that
  carries state or allocates. This keeps `page.rs` a pure codec.
- `bace::*` does NOT import `page.rs`.

## 4. Fail-closed invariants (implementation checklist)

An implementation of this ADR MUST demonstrate every item below
through a passing test:

1. Every existing `page_codec.rs` test continues to pass unchanged.
2. A round-tripped v1 page with no flags decodes with
   `flags == PageFlags::empty()`.
3. A round-tripped v2 page with `payload_is_bace_envelope=true`
   and a synthetic BACE-magic-prefixed payload decodes with the
   flag set.
4. A v2 page with byte `[7]` set to a reserved-bit pattern fails
   with `UnsupportedFeatureFlag`.
5. A v2 page with `payload_is_bace_envelope=true` whose payload
   does not start with `ENVELOPE_MAGIC` fails with
   `BaceEnvelopeMagicMismatch`.
6. A page with `format_version = 3` fails with `UnsupportedVersion(3)`.
7. A v2 page CRC32 is computed exactly like v1 (whole page,
   CRC field zeroed).
8. A v1 writer path still produces pages that decode under BRPG v1
   on an older build. Verified by a golden-vector test: encoding
   through `Page::new(..).encode(..)` yields bytes identical to
   the v1-only predecessor's output.
9. `new_v2(..., PageFlags::empty())` emits a v1-shape page (on-disk
   `format_version == 1`). This proves §3.5 compatibility.
10. `new_v2(..., PageFlags::with_bace_envelope(true))` emits a v2
    page (on-disk `format_version == 2`).

## 5. Alternatives considered

**A. Keep two separate `PageV1` and `PageV2` structs.** Cleaner
type-safety. Rejected: every carrier (`row_store`, `backup`,
`index_recovery`, `btree`) currently refers to `Page` as one type.
Splitting would ripple through six modules in one commit. A
single `Page` with a `flags` field minimises blast radius.

**B. Decode the BACE envelope inside `page.rs`.** Would make `Page`
carry structured envelope content. Rejected: it forces `page.rs`
to depend on the policy service (for `EnvelopeCaps`), ties
page-level decoding to admission concerns that belong to carriers,
and makes `page_codec.rs` tests more expensive. The current
design keeps the page codec a pure byte codec; carriers call
`bace::envelope::read_envelope` on `page.payload` themselves.

**C. Default `encode()` emits v2 on any build that supports it.**
Rejected: silently bumping on-disk version on every write would
break operators who rely on byte-compatible rolling restarts. The
proposed rule — v2 only when flags demand it — keeps the common
path identical to today.

**D. Add runtime version selection via `encode_with_version`.**
Rejected: the §3.5 flags-based rule makes the version choice a
**function of the content**, not a free parameter. A page with
BACE-envelope-payload MUST be v2; a page with no flags MAY be v1.
There is no legitimate case for `encode_with_version(V2, PageFlags::empty())`
other than testing, where callers can construct the bytes directly.

## 6. Open design questions (owner review)

1. **Should `Page::encode()` with `flags != empty()` ALWAYS produce
   a v2 page, even when the carrier is about to write it into a
   generation whose manifest still admits v1-only readers?** The
   proposed answer is "yes" — the carrier is responsible for
   gating v2-flag writes behind the generation's capability set.
   `page.rs` does not know which generation it writes into.
   Confirm.
2. **New constructor name.** `Page::new_v2(..., flags)` vs
   `Page::new_with_flags(..., flags)` vs replacing `new` with a
   single constructor that takes `PageFlags` and keeps a
   zero-flag convenience method. Picking any of the three is
   fine; the first is the draft default.
3. **`PageError::UnsupportedFlags` vs `UnsupportedFeatureFlag`.**
   The draft keeps them distinct. Alternative: merge into one
   variant and let the error message disambiguate. The current
   name separation makes the error path explicit; confirm.
4. **BACE-envelope-magic structural check in `Page::decode`.**
   Draft runs the check when `payload_is_bace_envelope` is set.
   Alternative: do no structural check at the page layer; let
   the carrier discover the mismatch on
   `bace::envelope::read_envelope`. The draft version catches the
   error one layer earlier and makes `page_codec.rs` tests
   simpler. Confirm.
5. **New `page.rs` → `bace::envelope::ENVELOPE_MAGIC` dependency.**
   Draft permits a single `use` of this constant. Alternative:
   duplicate the four-byte literal in `page.rs`. The draft
   centralises the constant. Confirm.
6. **`new_v2` return type.** Draft is `Result<Self, PageError>`.
   The error path covers only `InvalidPayloadLength`; if `flags`
   are always valid (constructed through `PageFlags` methods),
   the function may never error. Alternative: return `Self` and
   validate separately.
7. **Should the v2 writer refuse to emit a page whose
   `payload_is_bace_envelope` is set but whose payload does not
   start with `ENVELOPE_MAGIC`?** Draft: yes. Writer-side
   fail-closed matches decoder-side fail-closed. Confirm.

## 7. Impact on invariants

- **INV-001 commit correctness:** no change. Pages still land
  through the carrier's existing WAL + checkpoint + manifest path.
- **INV-002 one authoritative state:** no change. `Page` is a
  physical encoding; row state remains authoritative.
- **INV-003 no authorization gap:** no change. The page decoder
  does not carry authorization; carriers do.
- **INV-004 no unsafe automatic availability:** N/A.
- **INV-005 recoverable lifecycle:** v1 pages continue to decode
  under the existing recovery path; v2 pages decode under the new
  branch. Mixed-generation coexistence (ADR 0063 §6) is a
  row_store concern; this ADR only exposes the mechanism.

## 8. Dependencies this ADR would add

None. The implementation uses only existing workspace deps:
`crc32fast` (page CRC, unchanged), `thiserror` (error enum), and
the new `bace::envelope::ENVELOPE_MAGIC` constant (same workspace
crate).

## 9. What this ADR explicitly does NOT do

- Modify `row_store.rs`, `btree.rs`, `buffer_pool.rs`,
  `index_recovery.rs`, `backup.rs` or any other carrier.
- Make any production write path emit a v2 page. The default
  `Page::encode()` continues to produce v1 output.
- Introduce any new codec beyond RAW.
- Implement overflow chains, sidecar dictionaries or multi-envelope
  pages (ADR 0063 §3.3 bits 1–2 remain reserved).
- Perform bounded decompression of BACE envelopes. Carriers call
  `bace::envelope::read_envelope` themselves with their own caps
  from the policy service.
- Modify the existing `PageError::UnsupportedFlags(u8)` variant;
  it remains for v1 pages with non-zero byte `[7]`.

## 10. Acceptance evidence required before marking Accepted

1. ADR 0063 accepted — **satisfied** (14 November 2026).
2. Owner sign-off on §6 open design questions, especially Q2
   (constructor name), Q4 (envelope magic structural check), and
   Q6 (`new_v2` return type).
3. Commitment to a mixed-content test matrix: the implementation
   will include at minimum the 10 fail-closed / round-trip tests
   enumerated in §4, all of which must pass before the commit
   lands.
4. Explicit statement that no production write path will emit a
   v2 page until the carrier-integration ADR (0065) is also
   accepted — this keeps BRPG v2 bytes out of real stores until
   row_store is intentionally wired.

Only then can this ADR move from **PROPOSED** to **ACCEPTED**, and
only then can `page.rs` grow the v2 branches.
