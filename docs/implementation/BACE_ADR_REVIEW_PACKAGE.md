# BACE prerequisite ADR review package

14 November 2026. **Review complete.** All four BACE prerequisite
ADRs are now **ACCEPTED** with the §4 proposed defaults recorded as
resolved decisions in each ADR's §5 or §9. One exception: ADR 0061
question P1 (concrete codec ID assignments in range 16–47) was
explicitly left deferred; the first implementation allocates ID 48
for RAW only. No inner-encoding IDs are frozen until explicit owner
signoff on P1.

This document now serves as a historical record of the review
inputs. For the authoritative accepted state, see each ADR under
`docs/adr/`.

This document is a **navigation aid**. The authoritative text is
each ADR under `docs/adr/`. Where this package and an ADR conflict,
the ADR wins.

## 1. The four ADRs at a glance

| ADR | Commit | Scope | Lines |
|---|---|---|---|
| [0060 — BACE central policy service](../adr/0060-bace-central-policy-service.md) | `d0beded` | Type model for a versioned policy service with explicit scope-chain precedence. Addresses **BACE-ARCH-001**. | 308 |
| [0061 — BACE codec registry and capability negotiation](../adr/0061-bace-codec-registry-capability-negotiation.md) | `244862b` | `u16` codec ID namespace with `.derc`-legacy preservation, `CodecCapability` descriptor, negotiation algorithm with fail-closed on missing critical codecs. Addresses **BACE-ARCH-002**. | 319 |
| [0062 — BACE compressed envelope v1](../adr/0062-bace-compressed-envelope-v1.md) | `b694a90` | 32-byte fixed header + payload + 4-byte trailer CRC32. Little-endian. 14-step fail-closed read protocol. Host-local trust only. Addresses **BACE-FMT-001..005**, **BACE-ARCH-005/006**. | 346 |
| [0063 — BRPG v2 feature-flag page](../adr/0063-brpg-v2-feature-flag-page.md) | `1ad30b5` | Same 4 096 B page, same 32-byte header. Bump `format_version` to 2, reinterpret byte `[7]` as `feature_flags`. Bit 0 = `payload_is_bace_envelope`. Natural fail-closed via v1 reader. Addresses **BACE-FR-ROW-002**, **BACE-FMT-004**. | 386 |

Combined total: **1 359 lines of specification, zero lines of
implementation code.** Per CLAUDE.md §4.9 that is the required
sequence.

## 2. Dependency graph

```
                   ┌────────────────────────────────┐
                   │ 0060 Central policy service    │
                   │   (type model only)            │
                   └────────────┬───────────────────┘
                                │
                                ▼
                   ┌────────────────────────────────┐
                   │ 0061 Codec registry +          │
                   │   capability negotiation       │
                   └────────────┬───────────────────┘
                                │
                                ▼
                   ┌────────────────────────────────┐
                   │ 0062 Compressed envelope v1    │
                   │   (freezes byte layout)        │
                   └────────────┬───────────────────┘
                                │
                                ▼
                   ┌────────────────────────────────┐
                   │ 0063 BRPG v2 feature-flag page │
                   │   (freezes page layout)        │
                   └────────────────────────────────┘
```

Acceptance **must** proceed in order. Each ADR explicitly lists
its predecessors in §13 ("Acceptance evidence required before
marking Accepted"). Signing 0062 without 0060 + 0061 is not
permitted; signing 0063 without the three predecessors is not
permitted.

## 3. What this package would authorize (if all four accepted)

Nothing immediately — **acceptance of these four ADRs produces
zero lines of implementation code.** They are the design frame.

What acceptance *unblocks* is a second-tier workstream:

| Second-tier ADR | Freezes | Depends on |
|---|---|---|
| 0064 — `page.rs` v2 decoder (impl) | Rust code path for reading/writing BRPG v2 pages | 0060..0063 accepted |
| 0065 — `row_store.rs` mixed-generation reader (impl) | WAL replay + checkpoint over v1 + v2 pages | 0064 |
| 0066 — First BACE codec implementation (impl) | A single concrete codec (likely RAW or ZSTD) with its determinism corpus | 0061 accepted |
| 0067 — Backup subsystem BACE adapter (impl) | First carrier integration per plan §12.1 — the cold-path proving ground | 0066 + 0063 |
| 0068 — Authenticated envelope variant | A separate on-disk layout for off-host / untrusted payloads | BRD-SEC-003 workstream |
| 0069 — B+tree v2 key encoding | Prefix / delta encoding inside a v2 Index page | 0063 Q5 |
| 0070 — Dictionary scope and lifecycle | Where dictionaries live, how they are authorised | 0060 + 0061 |
| 0071 — `.derc` BACE integration | Column segment migration to the BACE envelope | 0062 |
| 0072 — BRWL BACE integration | WAL frame carrying a BACE envelope | 0062 |

**None of ADRs 0064–0072 is drafted.** They become writable only
after this package is signed.

## 4. Aggregated open design questions

All 28 open questions from the four ADRs, grouped thematically for
single-sitting review. Each question is reproduced verbatim from
the owning ADR §5 or §9.

### 4.1 Scope and integration (seven questions)

| # | ADR | Question | Default proposed |
|---|---|---|---|
| S1 | 0060 Q1 | Scope-chain cardinality — the proposed seven levels match BACE §8. Is a per-tenant level required between workload and database? | 7 levels (no per-tenant) |
| S2 | 0060 Q2 | Named workload profiles — fixed enum set `FAST` / `BALANCED` / `MAX` / `RAW`, or operator-definable profiles? | Fixed enum, four profiles |
| S3 | 0060 Q3 | Persistence surface — levels 2–5 live in `PersistentCatalog`, or in a sibling policy store with its own WAL? | Reuse `PersistentCatalog` |
| S4 | 0060 Q4 | Compatibility with existing `.derc` ZSTD-3 hardcode — migrate `.derc` to policy-governed compression, or leave in place and apply the policy service to new subsystems only? | Leave `.derc` in place; new subsystems only |
| S5 | 0060 Q5 | First subsystem to integrate — `backup` (plan §12.1 recommendation: cold path, reversible, ZSTD proven), or another? | `backup` |
| S6 | 0063 Q1 | Initial coverage — v2.0 target only `PageKind::Data`, or also `Index` and `FreeMap`? | All three (`Data`, `Index`, `FreeMap`); `Metadata` excluded |
| S7 | 0063 Q6 | `.derc` coexistence — single table's BRPG v2 and `.derc` projection negotiate policy together, or independently? | Independently |

### 4.2 Protocol and identifier numbers (six questions)

| # | ADR | Question | Default proposed |
|---|---|---|---|
| P1 | 0061 Q1 | **Concrete codec ID assignments** inside ranges 16–63. These are permanent, forever decisions. Need a signed table. | No default — owner must choose |
| P2 | 0061 Q2 | Format-version strategy — monotone per codec, or major/minor? | Monotone u16, bumped on breaking change only |
| P3 | 0061 Q3 | Which codecs ship in the Gate-3 initial build? For the `backup` adapter, minimum is RAW + ZSTD + mandatory inner encodings. | RAW + ZSTD; defer inner encodings until BRPG v2 impl |
| P4 | 0062 Q2 | `encoded_length` sizing — `u32` (4 GiB cap) vs `u64`. | `u32` |
| P5 | 0062 Q3 | `decoded_length` sizing — same question. | `u32` |
| P6 | 0063 Q5 | B+tree v2 key encoding — draft immediately after 0063, or defer? | Draft after 0063 acceptance |

### 4.3 Security and authenticity (three questions)

| # | ADR | Question | Default proposed |
|---|---|---|---|
| A1 | 0062 Q1 | Separate magic for the authenticated variant up front (e.g. `"BACA"`) vs single `"BACE"` + flag? | Reserve `"BACA"` now for the authenticated variant. Avoid repurposing a flag bit under an already-published magic. |
| A2 | 0062 Q5 | Vendor bits in `flags[16..=31]` — permit when `codec_id >= 256`, or forbid in v1? | Permit only when `codec_id >= 256` |
| A3 | 0063 Q7 | Downgrade mechanism — formally verified backup/restore cycle only, vs an explicit page-by-page downgrade decoder? | Backup/restore only (BACE-FMT-004 default) |

### 4.4 Determinism and portability (two questions)

| # | ADR | Question | Default proposed |
|---|---|---|---|
| D1 | 0061 Q4 | Determinism bar — can LZ4 / ZSTD at a pinned level be `Deterministic` today, or must they be `HardwareDependent`? | `HardwareDependent` for ZSTD; `Deterministic` for RAW only. LZ4 TBD per empirical corpus. |
| D2 | 0061 Q5 | Experimental flag semantics — prevents durable writes only, or also prevents reads of previously-written experimental payloads? | Writes only; reads always honored |

### 4.5 Strictness policy (four questions)

| # | ADR | Question | Default proposed |
|---|---|---|---|
| X1 | 0062 Q4 | Reserved-must-be-zero — strict (any non-zero reserved byte fails) vs lenient (ignore)? | Strict (fail-closed) |
| X2 | 0062 Q6 | Empty payload (`encoded_length = 0`) — permit or forbid? | Permit (36-byte minimum envelope) |
| X3 | 0062 Q7 | Byte-order marker via ASCII magic — `"BACE"` as ASCII doubles as endianness check. Acceptable? | Acceptable |
| X4 | 0063 Q2 | Reserved-bit strictness for BRPG v2 — strict fail-closed (draft) vs ignore unknown bits for easier v2.1 forward-compat? | Strict (fail-closed) |

### 4.6 Lifecycle and operations (four questions)

| # | ADR | Question | Default proposed |
|---|---|---|---|
| L1 | 0060 Q6 | SQL surface — defer `ALTER TABLE ... SET COMPRESSION` to a later ADR, or scope it now? | Defer |
| L2 | 0060 Q7 | Audit channel — new structured log, extend `tracing`, or wait for BRD-SEC-005? | Extend `tracing` now; migrate to BRD-SEC-005 audit when that workstream lands |
| L3 | 0063 Q3 | Mixed-generation lifetime — v1 and v2 pages coexist forever, until next checkpoint, or until operator migration? | Forever (the manifest decides which is authoritative per generation) |
| L4 | 0063 Q4 | Background recompression — mandatory in BACE-FR-BKP-003 ADR (impl), or operator-driven opt-in? | Operator-driven opt-in |

### 4.7 Extension model (two questions)

| # | ADR | Question | Default proposed |
|---|---|---|---|
| E1 | 0061 Q6 | Vendor / third-party codec IDs (256–65 535) — open to third-party BACE implementations, or reserved for Bradian? | Open to third parties; must set `experimental` flag by default |
| E2 | 0061 Q7 | Plugin future — Gate 9 item, post-1.0 item, or out of scope forever? | Post-1.0 with a signed-plugin ADR; not in the Gate 0–9 roadmap |

**28 questions total.** The "Default proposed" column is the
author's recommendation; the owner may override any of them. P1
(codec ID assignments) has no default and must be decided.

## 5. Acceptance workflow

Recommended owner workflow:

1. Read each ADR in dependency order (0060 → 0061 → 0062 → 0063).
   Each has §5 or §9 Open design questions for its scope.
2. Answer the 28 aggregated questions in §4 above in any order.
   Record answers in a `BACE_DECISIONS_v1.md` or inline comments
   on each ADR.
3. For each ADR, edit its §9 "Open design questions" section into
   a §9 "Design decisions (resolved)" section with the answer.
4. For each ADR, confirm the §10 (or §12 for 0063) "Impact on
   invariants" analysis is still correct given the decisions.
5. For each ADR, confirm the §13 "Acceptance evidence" checklist
   is satisfied.
6. Change the status line at the top of each ADR from
   **PROPOSED** to **ACCEPTED** with the date and the git hash of
   the decisions commit.
7. Only then are implementation ADRs (0064+) writable.

**Order matters.** Accepting 0063 before 0062 would leave 0063
pointing at an unaccepted envelope format. CLAUDE.md §4.9 does
not permit that.

## 6. What each ADR commits to — quick reference

### 6.1 ADR 0060 — central policy service

**Freezes:** a type model only (no bytes, no wire).

- Seven-level scope chain: platform baseline → workload → database
  → schema → table → column → job. Higher-number-wins per key.
- Three always-win vetoes: capability unavailable, security
  policy, codec parameter out-of-range.
- Initial key catalogue from BACE §8 (`compression.profile`,
  `max_cpu_percent`, etc.).
- Proposed `PolicyService` trait with `resolve<T>` and
  `resolve_snapshot` (atomicity required).
- Audit record contents per veto; channel wiring deferred.

**Does NOT:** freeze any on-disk / wire format; choose codec
numbers; decide envelope layout; alter `.derc` or BRPG today;
introduce admission variants; modify existing ADRs.

### 6.2 ADR 0061 — codec registry

**Freezes:** a `u16` codec ID namespace partition and the
negotiation algorithm.

- Ranges: `0–5` FROZEN (mirror `.derc` v1 legacy), `6–15` compat
  buffer, `16–31` core inner, `32–47` specialized, `48–63` outer,
  `64–255` BACE future, `256+` vendor/experimental.
- `CodecCapability` descriptor with every BACE-ARCH-002 field.
- Negotiation: intersection by (ID, format_version) + parameter
  intersection + fail-closed on any codec marked `critical` on
  one side but absent on the other.
- Five fail-closed invariants (unknown ID, unsupported version,
  out-of-range parameter, length mismatch, determinism violation).
- Build-time static registry; no runtime plugin registration.

**Does NOT:** freeze codec ID numbers in 16–63 range; freeze
envelope layout; freeze BRPG v2; implement any codec; modify
`.derc` or any existing format; introduce admission variants.

### 6.3 ADR 0062 — compressed envelope v1

**Freezes:** a durable byte layout. **Every byte position is
forever.**

- 32-byte header + payload + 4-byte trailer CRC32 = 36-byte
  overhead.
- Little-endian throughout.
- Header fields: `magic "BACE"`, `envelope_version=1`, `codec_id`,
  `codec_format_version`, `reserved_a=0`, `flags u32`,
  `encoded_length u32`, `decoded_length u32`, `reserved_b=0`,
  `header_crc32 u32`.
- Flag bits: `payload_is_raw`, `critical`, `encryption_ordered`,
  `has_sidecar_dictionary`.
- 14-step read protocol with pre-allocation bounds check per
  **BACE-FMT-003**.
- v1 is CRC-only, host-local trust only. Authenticated variant
  deferred.

**Does NOT:** freeze authenticated variant; specify dictionary
storage; specify BRPG v2 / `.derc` / BRWL embedding; freeze codec
ID numbers; implement any codec; modify any existing format.

### 6.4 ADR 0063 — BRPG v2 feature-flag page

**Freezes:** the row-page extension. Same 4 096 B. Same 32-byte
header position layout. **Only `format_version` (1→2) and byte
`[7]` (reserved→`feature_flags`) change meaning.**

- Fail-closed entry: BRPG v1 reader already rejects unknown
  `format_version` AND non-zero byte at `[7]`. v2 trips both
  rejections, so a v1 reader presented with a v2 page returns
  `UnsupportedVersion(2)` with no partial data.
- Flag bit 0: `payload_is_bace_envelope`. Bits 1–2 reserved for
  overflow-chain and sidecar-dictionary. Bits 3–7 reserved,
  must be zero in v2.0.
- 10-step read protocol with fail-closed at every step.
- Mixed-generation coexistence: v1 and v2 pages may live side by
  side in a directory; the checkpoint manifest decides
  authoritative generation.
- v2 pages NEVER replace v1 pages in place. Downgrade requires
  a formally verified backup/restore cycle.

**Does NOT:** freeze authenticated page variant; define
overflow-chain or sidecar-dictionary semantics; define
multi-envelope pages; specify B+tree v2 key encoding; specify
`.derc` or BRWL embedding; freeze codec ID numbers; implement any
decoder or writer; modify any existing on-disk format.

## 7. Invariant check (SRS v1.1 §20)

Reviewed by the author across all four ADRs. **None** of the
following invariants is violated by any of the four ADRs:

- **INV-001 commit correctness** — none of the four ADRs touches
  commit semantics. BRPG v2 pages still land through the existing
  WAL + checkpoint + manifest path.
- **INV-002 one authoritative state** — compression is a format
  concern; row state remains authoritative. Pages are the physical
  encoding, not the state of record.
- **INV-003 no authorization gap** — the policy service (0060)
  evaluates within already-authorised scope; the registry (0061)
  carries no auth context; the envelope (0062) does not carry auth
  context (`flags.encryption_ordered` signals an outer wrapper
  only); BRPG v2 (0063) relies on caller-supplied caps from the
  policy service before allocating.
- **INV-004 no unsafe automatic availability** — none of the four
  makes a distributed availability decision.
- **INV-005 recoverable lifecycle** — every format bump includes
  a documented rollback / downgrade path. 0062 defers v1→v2
  envelope migration semantics to a future ADR; 0063 commits to
  copy-and-replace at generation boundaries with no in-place
  reinterpretation.

## 8. Risks the author wants to flag explicitly

1. **Codec ID assignments (P1) are permanent.** Once an ID is
   published into any durable envelope anywhere, changing its
   meaning is a release-breaking act. Spend time on P1.
2. **Determinism (D1) is load-bearing for BACE-NFR-PORT-001.** A
   decision to mark LZ4/ZSTD as `Deterministic` commits the build
   to a golden corpus that must reproduce on every target. This
   may require pinning library versions and SIMD paths.
3. **Mixed-generation lifetime (L3) interacts with recompression
   (L4).** Defaulting both to "operator opt-in forever" means v1
   pages may linger indefinitely. That is acceptable if the
   operator has good telemetry via the policy service's audit
   channel (L2).
4. **Authenticated envelope (A1) is deferred.** Reserving `"BACA"`
   now means the authenticated workstream cannot be derailed by a
   future magic collision. Deferring without reserving would be
   cheaper now but risky later. The default proposes reserving.
5. **BACE is an uncommitted workstream.** Even after accepting
   all four ADRs, no commitment to a schedule, a Gate 3+ decision,
   or an external deliverable follows. The master roadmap §32
   Gate 3 remains blocked on other prerequisites too.

## 9. If the owner decides NOT to accept

That outcome is legitimate. BACE is a parallel SRS with no
backing today; the only compression in-tree is a hardcoded
ZSTD-3 wrap on `.derc`. If BACE is deferred or deprioritised:

- All four ADRs stay in-tree as PROPOSED, dated and committed.
  They remain available if a future decision revives the
  workstream.
- The gaps analysis §5 and §9 continue to list BACE as a parallel
  SRS with zero backing.
- The `.derc` ZSTD-3 hardcode continues to work. No regression.

Nothing in `main` depends on BACE acceptance.

## 10. Reading order

1. **This document** (`BACE_ADR_REVIEW_PACKAGE.md`) — the
   navigation aid you are reading.
2. **ADR 0060** — the type model. Shortest to read.
3. **ADR 0061** — the codec registry. Includes the identifier
   number allocation table.
4. **ADR 0062** — the envelope. Contains the byte layout table.
5. **ADR 0063** — the page. Contains the mixed-generation policy.
6. Return to **this document** §4 and decide the 28 questions.
7. Update each ADR's status line and §9 / §5 sections.
8. Commit the decisions. Only then can implementation ADRs 0064+
   be drafted.

---

Pointers:

- [BACE SRS v1.0](../BACE_Comprehensive_Requirements_SRS_v1.0.md)
- [Master SRS v1.1](../Bradian_Master_Architecture_SRS_v1.1.md)
- [Comprehensive implementation plan](BRADIAN_COMPREHENSIVE_IMPLEMENTATION_PLAN.md) §12.1 (BACE workstream)
- [Comprehensive gaps analysis](BRADIAN_COMPREHENSIVE_GAPS_ANALYSIS.md) §5 (BACE integration gaps)
- [ADR 0060](../adr/0060-bace-central-policy-service.md)
- [ADR 0061](../adr/0061-bace-codec-registry-capability-negotiation.md)
- [ADR 0062](../adr/0062-bace-compressed-envelope-v1.md)
- [ADR 0063](../adr/0063-brpg-v2-feature-flag-page.md)
