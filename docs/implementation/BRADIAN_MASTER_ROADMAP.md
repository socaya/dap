# Bradian fixed master roadmap

The gate IDs in Master SRS v1.1 Section 32 are fixed. Section 18 contains an earlier,
broader summary with different groupings; Section 32 is the working sequence because
it specifies the detailed dependency and exit conditions. ADR 0040 records this
reconciliation without rewriting historical specifications or implementation files.

BRD-G1 through BRD-G44 are historical implementation records, not 44 accepted master
gates. Their tests remain useful evidence. No new gate number is assigned to each
commit. New changes identify a task in the current master gate. Passing a subset of
tests does not accept a complete master requirement or permit skipping its exit checks.

| Master gate | Fixed scope | Current position |
|---|---|---|
| 0 | Evidence, code audit, risk and specification closure | Evidence baseline assembled; unresolved architecture decisions remain explicit blockers before their formats/protocols can be frozen |
| 1 | Portable kernel and correct single-node transaction | Active implementation; native local SQL command delivered; memory, identity and failure qualification still open |
| 2 | Relational compatibility and safe schema lifecycle | Not accepted; existing schema/SQL slices are supporting evidence only |
| 3 | Columnar engine and commit-consistent HTAP | Not accepted; inherited analytical engine is not an authoritative HTAP projection |
| 4 | Single-cluster replication and safe placement | Blocked on earlier gates and secure identity/protocol decisions |
| 5 | Distributed commit and global snapshot correctness | Blocked on proven replication and global-ordering prototype |
| 6 | Studio, durable operations and migration | Not accepted; existing UI and local migrations are partial evidence |
| 7 | Federation and site autonomy | Blocked on explicit installation identities and delta/conflict contracts |
| 8 | Cluster-consistent recovery and hardening | Blocked on complete trust, storage and replication designs |
| 9 | Qualification and external reproducibility | Not accepted; no production or cross-platform certification claimed |

The [comprehensive gaps analysis](BRADIAN_COMPREHENSIVE_GAPS_ANALYSIS.md) cross-references the SRS v1.1 normative MUST clauses, BACE SRS v1.0 and the gap register; use it as the single-pane-of-glass view of release-blocking vs feature-shortfall vs scoped-deviation items. The [BACE ADR review package](BACE_ADR_REVIEW_PACKAGE.md) aggregates the four BACE prerequisite ADRs (0060, 0061, 0062, 0063) into one navigation aid for owner review, including all 28 open design questions.

## Gate 0 evidence baseline

The [source inventory and compatibility map](BRADIAN_SOURCE_INVENTORY.md),
[complete master requirement matrix](BRADIAN_MASTER_TRACEABILITY.md),
[threat model](../security/BRADIAN_THREAT_MODEL.md), ADRs and executed verification
reports supply the previously missing auditable baseline. The legacy analytics
traceability file remains explicitly scoped to the inherited product.

Current page/WAL/backup experiments are demonstrated in page codec, atomic batch,
checkpoint, process-exit, corruption, durable MVCC, catalog activation and backup
tests. HTTP tests demonstrate only the bootstrap token boundary. They do not prove
SQL authorization propagation, a secure cluster or global transaction ordering.
Those uncertainties remain design blockers, with prototype/evidence requirements in
the threat model and source inventory. No distributed timestamp, cluster identity
or authorization wire format may be frozen from the current stubs/partial modules.

Automatic CI remains disabled by the owner's instruction. Manual workspace checks
are the evidence path; this is an explicit exception to the earlier baseline-CI
wording, not an unreported skipped check. Gate 0's evidence baseline is reviewable;
this does not sign off unresolved architecture decisions or production security.

## Gate 1 task order and acceptance

| Stable task | Status | Evidence or remaining work |
|---|---|---|
| G1-CATALOG | Implemented subset | Persistent catalog and schema/row validation tests; unsupported secondary/FK constraints remain explicit |
| G1-STORAGE | Implemented subset | BRPG/BRWL codecs, immutable B+tree checkpoints, CRCs and exclusive store locks |
| G1-TRANSACTION | Implemented subset | WAL atomicity, rollback, snapshots, conflict tests, savepoints and read validation; not full isolation-level conformance |
| G1-CLI | Implemented | Canonical bradian-db and legacy dap-admin executables, local SQL scripts, explicit transaction/history modes and process tests |
| G1-LOCAL-ACCESS | Partial | Shared `bradian_relational::local_access` enforces POSIX private-root invariants (symlink refusal, mode 0o700, non-directory refusal, effective-UID ownership, bounded create-if-missing) and the native SQL CLI delegates to it; POSIX ACLs, parent-path trust, Windows ACL and enterprise identity remain unqualified |
| G1-INSTALL | Partial | Linux development install is exercised; portable release installers/signing/upgrades remain open |
| G1-MEMORY | Partial | Query/history/staging, backing/recovery maps, build/metadata buffers, CLI lines, per-domain SQL/fallible-native active transactions, row-store + legacy-migration directory listings, decoded persistent-catalog state, decoded multi-table-domain state and in-memory MvccStore versions are controlled; I/O/cache, cross-engine totals and global budgets remain open |
| G1-FAULTS | Partial | Torn-frame/CRC/process-exit tests plus zero-padded WAL extension, zero-body CRC refusal, `.tmp` orphans at higher LSNs, stray incidental files and leftover pending manifest coverage; real ENOSPC/EIO propagation, fsync-on-power-loss, device-level torn writes and Windows/macOS qualification remain open |
| G1-REFERENCE | Partial | Checked SQL/decimal result corpora plus a single-file positive/negative SQL subset corpus (`docs/reference/BRADIAN_SQL_SUBSET.md`) with every claim backed by a passing test; external SQL-92/99/2016 reference-suite compliance and full documented isolation qualification remain open |
| G1-PORTABILITY | Open | Windows/macOS installation, lock lifecycle and recovery runs |

Gate 1 stays active until its exit conditions have evidence. The next implementation
work must address G1-MEMORY, G1-LOCAL-ACCESS and G1-FAULTS before new Gate 2 features.
The [isolation reference cross-engine confirmations verification](BRADIAN_GATE1_ISOLATION_CROSS_ENGINE_VERIFICATION_2026_11_14.md) records the latest Gate 1 slice. The [fault recovery bounds verification](BRADIAN_GATE1_FAULT_RECOVERY_BOUNDS_VERIFICATION_2026_11_14.md) records the preceding slice. The [workspace usage snapshot verification](BRADIAN_GATE1_WORKSPACE_USAGE_VERIFICATION_2026_11_14.md) records the preceding slice. The [DomainSqlEngine admission propagation verification](BRADIAN_GATE1_DOMAIN_SQL_LIMITS_VERIFICATION_2026_11_14.md) records the preceding slice. The [transitive admission propagation verification](BRADIAN_GATE1_TRANSITIVE_ADMISSION_VERIFICATION_2026_11_14.md) records the preceding slice. The [operations guides for 0051/0054/0056/0058 verification](BRADIAN_GATE1_OPERATIONS_GUIDES_VERIFICATION_2026_11_14.md) records the preceding slice. The [operations guide index verification](BRADIAN_GATE1_OPERATIONS_INDEX_VERIFICATION_2026_11_14.md) records the preceding slice. The [TimestampTz/Uuid SQL-reachability scrub verification](BRADIAN_GATE1_TIMESTAMPTZ_UUID_SCRUB_VERIFICATION_2026_11_14.md) records the preceding slice. The [UPDATE expression 128-node boundary verification](BRADIAN_GATE1_EXPRESSION_NODE_BOUNDARY_VERIFICATION_2026_11_14.md) records the preceding slice. The [in-memory MvccStore admission verification](BRADIAN_GATE1_MVCC_LIMITS_VERIFICATION_2026_11_14.md) records the preceding slice. The [effective-UID local-access verification](BRADIAN_GATE1_LOCAL_ACCESS_UID_VERIFICATION_2026_11_14.md) records the preceding slice. The [fault recovery matrix extension verification](BRADIAN_GATE1_FAULT_RECOVERY_EXTENSION_VERIFICATION_2026_11_14.md) records the preceding slice. The [isolation levels written statement verification](BRADIAN_GATE1_ISOLATION_LEVELS_VERIFICATION_2026_11_14.md) records the preceding slice. The [SQL subset corpus extension verification](BRADIAN_GATE1_SQL_SUBSET_EXTENSION_VERIFICATION_2026_11_14.md) records the preceding slice. The [decoded domain admission verification](BRADIAN_GATE1_DOMAIN_LIMITS_VERIFICATION_2026_11_14.md) records the preceding slice. The [SQL subset corpus verification](BRADIAN_GATE1_SQL_SUBSET_CORPUS_VERIFICATION_2026_11_14.md) records the preceding slice. The [decoded catalog admission verification](BRADIAN_GATE1_CATALOG_LIMITS_VERIFICATION_2026_11_14.md) records the preceding slice. The [private-root local access verification](BRADIAN_GATE1_LOCAL_ACCESS_VERIFICATION_2026_11_14.md) records the preceding slice. The [deterministic fault recovery verification](BRADIAN_GATE1_FAULT_RECOVERY_VERIFICATION_2026_11_14.md) records the preceding slice. The [directory listing admission verification](BRADIAN_GATE1_DIRECTORY_LIMITS_VERIFICATION_2026_11_14.md) records the preceding runtime slice. The [historical transaction admission verification](BRADIAN_GATE1_TRANSACTION_ADMISSION_VERIFICATION_2026_11_13.md) records the prior runtime slice. The [SQL CLI output verification](BRADIAN_GATE1_SQL_OUTPUT_LIMITS_VERIFICATION_2026_11_13.md) retains its previous evidence. The [metadata verification](BRADIAN_GATE1_METADATA_LIMITS_VERIFICATION_2026_11_13.md) retains its previous evidence. The [checkpoint/build verification](BRADIAN_GATE1_CHECKPOINT_LIMITS_VERIFICATION_2026_11_13.md) retains its previous evidence. The [index recovery verification](BRADIAN_GATE1_INDEX_RECOVERY_LIMITS_VERIFICATION_2026_11_13.md) retains its previous evidence. The [backing RowStore verification](BRADIAN_GATE1_ROW_STORE_LIMITS_VERIFICATION_2026_11_13.md) retains its previous evidence. The [transaction staging verification](BRADIAN_GATE1_STAGING_LIMITS_VERIFICATION_2026_11_13.md) retains its previous evidence. The [retained-history admission verification](BRADIAN_GATE1_HISTORY_ADMISSION_VERIFICATION_2026_11_13.md) retains its previous evidence. The [single-table limit verification](BRADIAN_GATE1_SINGLE_TABLE_LIMITS_VERIFICATION_2026_11_13.md) retains its prior evidence.
The [history scan verification](BRADIAN_GATE1_SCAN_LIMITS_VERIFICATION_2026_11_13.md) retains its earlier evidence.
The [Gate 1 local CLI verification](BRADIAN_GATE1_LOCAL_CLI_VERIFICATION_2026_11_13.md)
records what was actually run. No existing analytical, schema or SQL work is deleted
merely because it was implemented before this ordering was reconciled.
