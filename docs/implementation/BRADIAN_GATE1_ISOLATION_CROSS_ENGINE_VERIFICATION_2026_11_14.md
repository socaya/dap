# Master Gate 1 — Isolation reference cross-engine confirmations

14 November 2026. Baseline: `b77f69a`.
Docs-only slice. Gate 1 remains active.

## Executed result

**863 Rust tests passed, 0 failed, 0 ignored** (unchanged by
docs-only slice). No code change.

| Executed check | Result |
|---|---|
| `cargo test --workspace` | 863 passed, 0 failed |
| `cargo test -p bradian-relational --test durable_mvcc --test history_admission --test savepoints --test savepoint_crash --test history_sql` | 68 passed (18 + 8 + 36 + 1 + 5) |
| Link validation on cited test names | 11 of 11 verified to exist and pass |

## Action

`docs/reference/BRADIAN_ISOLATION_LEVELS.md` previously cited 12 tests
from `read_validation.rs` and `read_validation_sql.rs`. The same
isolation invariants are proven independently by other suites;
operators reading the reference could not see that evidence.

Added **§8 Cross-engine confirmations** to the reference with an
11-row table citing passing tests from four additional suites:

- `durable_mvcc.rs` (5 citations): disjoint writes, same-key lost-update
  prevention, full-scan phantom prevention, pinned-reader retention,
  read-only transactions do not append WAL.
- `savepoints.rs` (2 citations): savepoint rollback preserves scan
  conflict observations and original snapshot conflicts + retention pin.
- `history_admission.rs` (1 citation): savepoints + coalesced writes
  charge only the final committed key set.
- `savepoint_crash.rs` (1 citation): process-exit keeps only outer
  committed work (ACID durability boundary).
- `history_sql.rs` (2 citations): historical SQL rollback never
  publishes staged changes; AS OF before any committed data returns
  no rows.

### §7 preservation

The reference originally ended with "§7 Claims this document does
not make". That section is cited by external references in
`BRADIAN_COMPREHENSIVE_GAPS_ANALYSIS.md` (two citations, lines 162
and 329). The new cross-engine section is inserted as **§8, after
§7**, so external references continue to resolve correctly.

## Remaining scope

- External SQL-standard isolation labels remain unmapped (G1-REFERENCE
  open).
- Distributed / cross-process isolation statements remain blocked on
  Gate 5 (consensus + global timestamp).

See [ADR 0037](../adr/0037-bounded-transaction-read-validation.md),
[ADR 0036](../adr/0036-bounded-history-transaction-savepoints.md),
[comprehensive gaps analysis](BRADIAN_COMPREHENSIVE_GAPS_ANALYSIS.md) §9.
