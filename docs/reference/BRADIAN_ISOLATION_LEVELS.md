# Bradian isolation levels — documented statement

Normative test backing: every claim below cites an existing passing test
in `crates/bradian-relational/tests/`. This document does **not** change
engine behaviour; it is a reader-facing statement of what the shipped
modes guarantee. Any future change to engine behaviour must update both
the owning test and the row here.

## 1. Modes

Bradian's three native SQL engines expose two concurrency modes:

- **Snapshot mode (default).** `BEGIN` or `BEGIN TRANSACTION` starts a
  consistent snapshot. Reads see a point-in-time view; writes conflict
  check against the last-committed version of the keys they modify.
  Scope: `SqlEngine`, `DomainSqlEngine`, `HistorySqlEngine`.
- **Bounded read-validation mode (opt-in).** `BEGIN TRANSACTION WITH
  READ VALIDATION` additionally tracks every observed key and
  revalidates it under the commit mutex; a concurrent change to any
  observed key fails the commit. Scope: `HistorySqlEngine` only, and
  only within one domain. See [ADR 0037](../adr/0037-bounded-transaction-read-validation.md).

Both modes are **single-process**. Multi-process coordination relies on
the exclusive advisory lock each store takes at open; Bradian does not
yet claim distributed isolation.

## 2. Anomaly matrix

For every classical isolation anomaly, the row below states what the two
modes do and cites the owning test.

| Anomaly | Snapshot (default) | Read-validation (opt-in) | Owning test |
|---|---|---|---|
| **Dirty read** — read uncommitted data | Not possible: reads only see committed snapshots | Not possible | Implicit in the snapshot model; exercised by `read_validation::independent_validated_writes_commit_and_read_only_validation_does_not_write_wal` and the DurableMvccStore commit protocol |
| **Non-repeatable read** — a key changes value within one transaction | Not possible: same snapshot | Not possible | `read_validation::absent_keys_tombstones_and_aba_values_are_conflicts` |
| **Phantom read** — a scan predicate sees a different set of rows twice | Not possible within one snapshot; next transaction sees current state | Not possible; a scan that observes a key-set revalidates that key-set at commit | `read_validation::scans_including_empty_results_validate_read_only_and_writing_commits` and `read_validation_sql::scan_phantoms_validate_even_after_rollback_to_an_earlier_marker` |
| **Lost update** — two transactions overwrite the same key | **Prevented**: the conflict check aborts the second committer's whole transaction | **Prevented** | `read_validation::independent_validated_writes_commit_and_read_only_validation_does_not_write_wal`, `history_savepoint_rollback_to_discards_only_post_marker_work` |
| **Write skew** — two transactions read overlapping keys and write different ones in a way that violates a shared invariant | **Possible**: snapshot mode does not re-check observed reads at commit | **Prevented** within the bounded observation window (4096 distinct keys per domain); above that, falls back to whole-domain validation | `read_validation::concurrent_validating_writers_cannot_both_commit_write_skew`, `read_validation_sql::validated_cross_table_point_reads_reject_write_skew_and_default_begin_stays_compatible` |
| **Read-only anomalies** (e.g. observing an impossible combination of committed values) | Can appear in snapshot mode | Validator catches them on read-only transactions too | `read_validation::absent_keys_tombstones_and_aba_values_are_conflicts`, `read_validation_sql::read_only_and_absent_row_observations_fail_without_partial_writes` |
| **ABA observation** — a key observed at value V, then changed to V' and back to V | **Possible**: a snapshot sees the final V; it cannot distinguish | **Detected**: version/LSN is part of the recorded observation, so an ABA cycle fails validation | `read_validation::absent_keys_tombstones_and_aba_values_are_conflicts` |

## 3. Observation budget

Read-validation tracks up to **4096 distinct observed keys per domain**.
Beyond that, it falls back to whole-domain validation at commit — a
successful commit requires that no other transaction committed into the
domain between BEGIN and commit. See
[ADR 0037](../adr/0037-bounded-transaction-read-validation.md).

| Boundary | Covering test |
|---|---|
| Within budget: validation is per-key | `read_validation::concurrent_validating_writers_cannot_both_commit_write_skew` |
| Beyond budget: fallback to whole-domain | `read_validation::observation_limit_falls_back_to_bounded_domain_validation` |

## 4. Savepoint interaction

Savepoint rollback **does not forget observations** in read-validation
mode. A transaction that read a key, created a savepoint, modified
state and rolled back to the savepoint still validates the original
read at commit.

| Behaviour | Covering test |
|---|---|
| Savepoint rollback preserves observations | `read_validation::savepoint_rollback_and_release_cannot_forget_observations` |
| Scan observations survive rollback to marker | `read_validation_sql::scan_phantoms_validate_even_after_rollback_to_an_earlier_marker` |

See [ADR 0036](../adr/0036-bounded-history-transaction-savepoints.md).

## 5. AS OF reads

`AS OF <lsn>` is a read-only historical snapshot at a WAL LSN. It is
refused in a read-validation session because the validator operates on
the current snapshot. Use a separate snapshot session for historical
queries that need both AS OF and strong read behaviour.

| Behaviour | Covering test |
|---|---|
| AS OF in snapshot session returns committed rows at the LSN | `sql_subset_corpus::as_of_returns_committed_rows_at_lsn` |
| AS OF refused in read-validation session | `sql_subset_corpus::as_of_is_rejected_in_a_read_validation_session` and `read_validation_sql::historical_reads_and_invalid_mode_syntax_cannot_silently_bypass_validation` |

## 6. All-twenty-interleaving coverage

`read_validation::all_twenty_interleavings_of_two_transactions_preserve_shared_rule`
exhaustively exercises the twenty distinct interleavings of two
two-statement transactions sharing an invariant. Every interleaving
either commits both transactions in a serially-equivalent order or
aborts the second committer with a `Conflict` error. There is no
schedule that violates the shared rule.

## 7. Claims this document does not make

- **Serializable (SQL-standard SERIALIZABLE).** Snapshot mode is not
  Serializable; write skew is possible. Read-validation mode is bounded
  Serializable within its observation budget and over the shared-WAL
  domains that opt in; it is **not** a fully qualified Serializable
  isolation level in the sense of a published external reference suite.
- **Distributed isolation.** Both modes assume one process holds the
  exclusive directory lock. Cross-process or cross-node coordination is
  outside Bradian's current scope.
- **Repeatable read / read committed / read uncommitted names.** These
  are SQL-standard level labels. The corpus describes behaviour at the
  anomaly level, not by label. Mapping to external labels requires
  external reference-suite runs (G1-REFERENCE, still open).

## 8. Cross-engine confirmations

The anomaly matrix in §2 cites tests from `read_validation.rs` and
`read_validation_sql.rs`, which exercise the explicit opt-in
read-validation surface. The same invariants are independently
confirmed by other suites that cover the native durable MVCC kernel
(`durable_mvcc.rs`, `history_admission.rs`), the savepoint API
(`savepoints.rs`, `savepoint_crash.rs`) and the historical SQL
engine (`history_sql.rs`). Operators reading this reference can
treat the citations below as additional evidence for the same
behaviours.

| Property | Owning test(s) |
|---|---|
| **Snapshot-scoped disjoint writes commit without conflict** — scalability of snapshot mode | `durable_mvcc::disjoint_writers_commit_from_same_snapshot` |
| **Same-key lost-update prevention** in the durable kernel (not only in read-validation) | `durable_mvcc::conflicting_same_key_writes_are_rejected_without_partial_commit` |
| **Phantom prevention via conservative whole-store LSN check** after a full scan | `durable_mvcc::full_scan_then_write_conflicts_with_intervening_commit` |
| **Pinned-reader retention** — a long-lived snapshot pins the version it sees, blocking retention-floor advancement | `durable_mvcc::pinned_reader_prevents_advancing_floor_past_its_start` |
| **Read-only transactions do not append WAL** — a snapshot read that writes nothing does not produce durable bytes | `durable_mvcc::read_only_transactions_do_not_append_wal_or_change_snapshot` |
| **Savepoint rollback preserves scan conflict observations** — in the durable kernel | `savepoints::rollback_to_never_forgets_full_scan_conflict_observation` |
| **Savepoint rollback preserves original snapshot conflicts and retention pin** | `savepoints::restored_writes_keep_original_snapshot_conflicts_and_retention_pin` |
| **Savepoint + admission interaction** — coalesced writes across savepoints charge only the final committed key set | `history_admission::savepoints_and_coalesced_writes_charge_only_the_final_committed_key_set` |
| **ACID D under process exit** — a process that exits mid-transaction keeps only the outer committed work | `savepoint_crash::process_exit_keeps_only_outer_committed_work` |
| **Historical SQL rollback never publishes staged changes** | `history_sql::deletion_history_and_rollback_behave_correctly` |
| **AS OF before any committed data returns no rows** — historical reads do not fabricate pre-existence | `history_sql::as_of_before_first_import_visibility_returns_no_rows` |

Each cited test is a passing integration test in the current workspace.

See the [comprehensive implementation plan](../implementation/BRADIAN_COMPREHENSIVE_IMPLEMENTATION_PLAN.md)
§4.4 for the full isolation workstream.
