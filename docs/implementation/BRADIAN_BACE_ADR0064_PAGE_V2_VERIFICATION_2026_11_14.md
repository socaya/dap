# ADR 0064 — `page.rs` BRPG v2 decoder/writer implementation verification

14 November 2026. Baseline: `48b43f2`.
First implementation ADR in the BACE carrier-integration workstream.

## Executed result

**914 Rust tests passed, 0 failed, 0 ignored.** Clippy clean on
`bradian-relational --all-targets`. The on-disk BRPG v1 writer bytes
are byte-identical to the pre-ADR-0064 writer (golden vector asserts
the header and payload region). No production path yet emits a v2
page — `row_store.rs` and every other carrier remains unchanged.

| Executed check | Result |
|---|---|
| `cargo check -p bradian-relational` | Passed |
| `cargo test --workspace` | 914 passed, 0 failed |
| `cargo test -p bradian-relational --lib page::adr0064_tests` | 13 passed |
| `cargo test -p bradian-relational --test page_codec` | 6 passed (all pre-existing v1 tests) |
| `cargo clippy -p bradian-relational --all-targets -- -D warnings` | Passed |

## What landed

`crates/bradian-relational/src/page.rs` grew the v2 branch per
accepted ADR 0064 §3:

- Constants: `PAGE_VERSION_V1` + `PAGE_VERSION_V2`; the historical
  `PAGE_VERSION` alias retained.
- `PageFlags` struct with `payload_is_bace_envelope` only
  (bits 1–7 reserved per ADR 0063 §3.3; bit 0 is the only v2.0
  feature). Methods: `empty`, `with_bace_envelope`, `is_empty`,
  `to_byte`, `from_byte` (enforces reserved-must-be-zero).
- New `flags: PageFlags` field on `Page`. Default value on v1
  pages is `PageFlags::empty()` so no existing caller needs to
  touch it.
- New constructor `Page::new_v2(page_id, lsn, kind, payload, flags)`
  returning `Result<Self, PageError>`.
- `Page::decode` auto-selects v1/v2 based on the `format_version`
  byte at offset `[4..=5]`. `format_version` outside `{1, 2}`
  continues to fail closed with `UnsupportedVersion(n)`.
- `Page::encode` picks v1 vs v2 on-disk format based on
  `self.flags.is_empty()` per ADR 0064 §3.5 — a page with no flags
  always emits v1 (wire-compatible with the pre-ADR-0064 reader);
  a page with flags set always emits v2.
- Two new `PageError` variants: `UnsupportedFeatureFlag(u8)`
  (v2 pages with reserved bits 1–7) and `BaceEnvelopeMagicMismatch
  { kind }` (v2 pages with `payload_is_bace_envelope` set but
  payload lacking `ENVELOPE_MAGIC`). The existing
  `UnsupportedFlags(u8)` variant is kept for v1 pages with
  non-zero byte `[7]`.
- Single new module-boundary dependency:
  `use crate::bace::envelope::ENVELOPE_MAGIC;` for the structural
  check only. `page.rs` does not call `read_envelope` or any
  other stateful `bace` symbol (ADR 0064 §3.7).

## 10-item test checklist — all satisfied

Every item from ADR 0064 §4 passes:

1. **Every existing `page_codec.rs` test continues to pass**
   unchanged (6/6). One assertion was updated: the "future
   version" fail-closed case now uses version 3 instead of 2
   because version 2 is now valid per ADR 0064 §3.4. The intent
   (unknown version → `UnsupportedVersion`) is preserved; only the
   test's choice of "unknown" moved forward.
2. A round-tripped v1 page with no flags decodes with
   `flags == PageFlags::empty()`. ✓
3. A round-tripped v2 page with `payload_is_bace_envelope=true`
   and a synthetic BACE-magic-prefixed payload decodes with the
   flag set. ✓
4. A v2 page with byte `[7]` set to a reserved-bit pattern fails
   with `UnsupportedFeatureFlag`. ✓ (verified for every bit 1..=7
   in `page_flags_from_byte_refuses_every_reserved_bit`).
5. A v2 page with `payload_is_bace_envelope=true` whose payload
   does not start with `ENVELOPE_MAGIC` fails with
   `BaceEnvelopeMagicMismatch`. ✓ — both the decoder path and the
   two writer paths (`new_v2` + `encode`) are tested.
6. A page with `format_version = 3` fails with
   `UnsupportedVersion(3)`. ✓
7. A v2 page CRC32 is computed exactly like v1 (whole page, CRC
   field zeroed). ✓
8. A v1 writer path still produces pages that decode under BRPG v1
   on an older build. ✓ — a golden vector asserts the header and
   payload region byte-by-byte.
9. `new_v2(..., PageFlags::empty())` emits a v1-shape page
   (on-disk `format_version == 1`). ✓
10. `new_v2(..., with_bace_envelope(true))` emits a v2 page
    (on-disk `format_version == 2`). ✓

Plus two supporting tests:

- `page_flags_byte_round_trip_covers_both_defined_bit_states` —
  `PageFlags::empty()` and `PageFlags::empty().with_bace_envelope(true)`
  both round-trip through `to_byte` + `from_byte`.
- `page_flags_from_byte_refuses_every_reserved_bit` — proves the
  strict reserved-must-be-zero rule for every bit position 1..=7.

## Compliance against ADR 0064

- §3.1 constants — ✓
- §3.2 `PageFlags` — ✓ (bit 0 only; bits 1–2 remain reserved per
  ADR 0063; bits 3–7 strict-fail-closed)
- §3.3 `Page.flags` field — ✓
- §3.4 decoder behaviour (auto-select v1/v2; v2 structural checks
  including envelope-magic) — ✓
- §3.5 writer behaviour (flags empty → v1; flags set → v2) — ✓
- §3.6 new `PageError` variants — ✓
- §3.7 module boundary (single import of `ENVELOPE_MAGIC`) — ✓
- §4 10-item test checklist — ✓ (all items green)
- §6 resolved decisions — all seven defaults implemented as
  specified.

## What this implementation explicitly does NOT do

- Modify `row_store.rs`, `btree.rs`, `buffer_pool.rs`,
  `index_recovery.rs`, `backup.rs`, `database_backup.rs`,
  `history_backup.rs`, or any other carrier.
- Make any production write path emit a v2 page. The shipped SQL
  engines, backup pipelines, catalog and migration flows all
  continue to invoke `Page::new(..)` (which returns `flags ==
  empty()`) and encode as v1. **Zero v2 bytes reach disk in
  production.**
- Introduce a `bace::Codec` or carrier-level envelope wiring.
- Implement overflow chains, sidecar dictionaries or
  multi-envelope pages (ADR 0063 §3.3 bits 1–2 remain reserved).
- Perform bounded decompression of BACE envelopes. Carriers call
  `bace::envelope::read_envelope` on `page.payload` themselves
  with their own caps.

## Next

Row_store carrier integration (ADR 0065) — the first production
write path that emits v2 pages. Needs:

1. A draft ADR 0065 covering mixed-generation reader + BACE-aware
   checkpoint writer semantics.
2. Owner review of ADR 0065's open design questions.
3. Implementation per the same workflow used for ADR 0064.

ADR 0065 is **not drafted yet**.

See [ADR 0064](../adr/0064-page-rs-brpg-v2-decoder-writer.md),
[ADR 0063](../adr/0063-brpg-v2-feature-flag-page.md),
[BACE_ADR_REVIEW_PACKAGE](BACE_ADR_REVIEW_PACKAGE.md).
