# BACE Component Q.4 — policy service + scope chain verification

14 November 2026. Baseline: `06369e2`.
Fourth BACE implementation step against accepted ADR 0060.
**Completes Slice Q** — the standalone BACE module.

## Executed result

**901 Rust tests passed, 0 failed, 0 ignored.** Clippy clean on
`bradian-relational --all-targets`. No persistent format change
reaches disk. The BACE module is now **feature-complete at the
module level**: codec, envelope, registry and policy service are
all implemented and composed. Carrier integration (BRPG v2,
row_store, `.derc`, WAL, backup) remains untouched.

| Executed check | Result |
|---|---|
| `cargo check -p bradian-relational` | Passed |
| `cargo test --workspace` | 901 passed, 0 failed |
| `cargo test -p bradian-relational --lib bace::policy` | 9 passed |
| `cargo test -p bradian-relational --lib bace` | 38 passed (Q.1: 4, Q.2: 17, Q.3: 8, Q.4: 9) |
| `cargo clippy -p bradian-relational --all-targets -- -D warnings` | Passed |

## What landed

New submodule `crates/bradian-relational/src/bace/policy.rs`:

**Type model (ADR 0060 §3.1–§3.2):**

- `CompressionProfile { Fast, Balanced, Max, Raw }` — fixed enum per
  ADR 0060 §5 item 2.
- `DictionaryScope { None, Column, Table, Database }`.
- `ScopeLevel { PlatformBaseline, Workload, Database, Schema, Table,
  Column, Job }` — seven levels, higher wins.
- `PolicyKey` — eight initial keys from ADR 0060 §3.2.
- `PolicyValue` — type-tagged enum with `as_profile`, `as_u8`,
  `as_u32`, `as_u64`, `as_dictionary_scope`, `as_codec_set` typed
  accessors.
- `Resolved { value, resolved_at, overridden_by }`.
- `VetoReason { CapabilityUnavailable, SecurityPolicy,
  CodecParameterOutOfRange }`.
- `ScopeOverrides = HashMap<PolicyKey, PolicyValue>`.
- `ScopeChain { workload, database, schema, table, column, job }` —
  one `ScopeOverrides` per non-baseline level.
- `PlatformBaseline` — shipped platform defaults for every key.

**API surface (ADR 0060 §3.3):**

- `PolicyService` trait: `resolve(key, scope)` and
  `resolve_snapshot(keys, scope)` with default impl that batches
  individual resolves (correct for the in-memory impl; sibling
  implementations may override for atomicity).

**Implementation:**

- `InMemoryPolicyService<R: CodecRegistry>` — holds a
  `PlatformBaseline` and composes with a codec registry so the
  capability veto can enforce "requested codec must be
  registered".

**Veto application order (ADR 0060 §3.4):**

1. **Capability veto** — applies only to
   `CompressionAllowedCodecs`. Drops any codec not in the
   registry. If every requested codec is absent, falls back to the
   platform baseline allow-set. Records the first absent codec as
   `VetoReason::CapabilityUnavailable`.
2. **Security veto** — stub (no encryption pipeline exists yet).
   The code path is in place; the first security check fires when
   the encryption workstream (BRD-SEC-003) lands.
3. **Codec parameter clamp** — enforces declared ranges for
   `CompressionMaxCpuPercent` (≤ 100),
   `CompressionMinSavingPercent` (≤ 99), and
   `CompressionMaxDecodedPageBytes` (4 096..=16 777 216). Records
   the pre- and post-clamp values in `VetoReason::CodecParameterOutOfRange`.

## 9 unit tests

- `baseline_only_scope_resolves_every_key_at_platform_level` — all
  eight keys resolve to `ScopeLevel::PlatformBaseline` with no
  veto.
- `job_level_wins_over_every_lower_level` — JOB (level 6) override
  wins against every lower-level setting of the same key.
- `precedence_order_is_job_column_table_schema_database_workload_platform`
  — five iterations clearing the top level in turn, verifying the
  full precedence chain surfaces each lower level.
- `resolve_snapshot_returns_atomic_view_across_many_keys` — three
  keys resolved simultaneously, each at its correct level.
- `capability_veto_drops_unregistered_codecs_and_records_the_first_offender`
  — RAW survives, unknown codecs dropped, first unknown codec ID
  recorded.
- `capability_veto_falls_back_to_baseline_when_every_requested_codec_is_unknown`
  — all-unknown allow-set falls back to the baseline allow-set with
  the veto recorded.
- `parameter_clamp_bounds_u8_cpu_percent_and_records_the_veto` —
  request 250, clamped to 100, veto recorded.
- `parameter_clamp_bounds_max_decoded_page_bytes_into_the_declared_range`
  — below-min clamped to 4 096, above-max clamped to 16 777 216.
- `values_already_within_range_pass_the_clamp_without_firing_a_veto`
  — in-range values resolve cleanly with `overridden_by == None`.

## Compliance check against ADR 0060

- §3.1 seven-level scope chain — implemented. ✓
- §3.2 initial policy key catalogue — eight keys shipped with their
  declared types. ✓
- §3.3 API surface — `resolve` + `resolve_snapshot`. ✓
- §3.4 resolution semantics — walk from innermost (Job) out; apply
  capability veto, then security veto (stub), then clamp. ✓
- §3.5 atomicity — the in-memory implementation reads immutable
  `ScopeOverrides` maps without a lock; a concurrent update would
  need to publish a new `ScopeChain` by value. Future persistent
  implementations will need to enforce this more strongly.
- §3.6 audit — the veto reasons are carried on `Resolved`. Writing
  them to `tracing` is a sibling slice; the data is in place.
- §5 resolved decisions — profiles are fixed enum (item 2),
  persistence deferred (item 3), `.derc` left alone (item 4),
  first subsystem = `backup` (item 5), SQL deferred (item 6),
  audit via `tracing` (item 7).

## What this component explicitly does NOT do

- No persistence. Levels 2–5 (database/schema/table/column) live
  in the caller's `ScopeChain` for now. The `PersistentCatalog`
  wiring per ADR 0060 §5 item 3 is a later ADR.
- No SQL `ALTER TABLE ... SET COMPRESSION` (deferred, L1).
- No `tracing` emission on vetoes yet — the data is on `Resolved`
  but the call site has not been wired. One-line addition when a
  caller needs it.
- No security veto implementation (stub only). The first security
  check fires when the encryption pipeline lands.
- No workload-profile → value expansion. `CompressionProfile::Fast`
  / `Balanced` / `Max` / `Raw` are returned as enum values; a
  future ADR defines what each profile implies for the other keys.
- No carrier integration.

## Slice Q completion summary

Four components, four commits, four verification reports.

| Component | Commit | Lines added | Tests added |
|---|---|---|---|
| Q.1 codec types + RAW | `b6a7ee2` | ~200 | 4 |
| Q.2 envelope v1 | `1622eef` | ~500 | 17 |
| Q.3 registry + negotiation | `06369e2` | ~300 | 8 |
| Q.4 policy service | (this) | ~450 | 9 |

**Total Slice Q: 4 commits, ~1 450 lines of code, 38 new tests.**
Workspace: 863 → **901 tests, 0 failures throughout.** Clippy
clean on every commit.

## Honest status statement

The standalone `bace` module implements the accepted ADRs 0060,
0061 and 0062 at the module level. **No production path writes a
BACE envelope to disk; no carrier has been integrated.** The
module exposes a clean seam that future carrier ADRs (BRPG v2 page
integration, backup adapter, `.derc` adapter, WAL adapter) can
pick up.

Further implementation past this point requires new ADRs per the
acceptance text of ADR 0063 §13 item 5:

> No production write will emit v2 pages until implementation ADRs
> for the page, row-store, B+tree and backup modules are also
> accepted.

The next natural ADRs in the chain are:

- **ADR 0064** — `page.rs` BRPG v2 decoder/writer (first carrier
  integration; implements ADR 0063 bytes in production code).
- **ADR 0065** — `row_store.rs` mixed-generation reader and
  BACE-aware checkpoint writer.
- **ADR 0066** — second codec: ZSTD (needs determinism corpus on
  every supported target per ADR 0061 §3.7).
- **ADR 0067** — `backup.rs` BACE adapter (the plan §12.1
  recommended first subsystem, now unblocked at the module level).

None of 0064–0067 is drafted. The honest next step is: pause and
decide which carrier adapter to draft first, or accept that the
standalone module is where Slice Q stops.

See [BACE_ADR_REVIEW_PACKAGE](BACE_ADR_REVIEW_PACKAGE.md),
[ADR 0060](../adr/0060-bace-central-policy-service.md).
