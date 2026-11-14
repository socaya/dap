# ADR 0060 — BACE central policy service (design-only; **PROPOSED**)

**Status: PROPOSED — not accepted for implementation.**

Owner review required before any code lands, per CLAUDE.md §4.9
("Spec-first binary formats") and the
[comprehensive implementation plan](../implementation/BRADIAN_COMPREHENSIVE_IMPLEMENTATION_PLAN.md)
§13 and §16. This ADR specifies a type model and resolution
contract only; no persistent format, wire protocol, codec identifier
or envelope byte layout is frozen here. Those belong in separate
dedicated ADRs after this one is accepted.

## 1. Context

The Bradian Adaptive Compression Engine (BACE) SRS v1.0 §3.1
requires:

> **BACE-ARCH-001 — Central policy service.** A versioned policy
> model MUST resolve global, workload, database, table, column,
> tier and job-level options with explicit precedence. Security and
> format capabilities override requested codec choices.
> **Verification:** Policy inheritance, conflict and downgrade tests.

BACE §8 further specifies that the hierarchy is:

```
platform baseline → workload policy → database/schema/table →
    optional per-column override → explicit maintenance job
```

with suggested keys `compression.profile`,
`compression.max_cpu_percent`, `compression.max_decode_ns_per_kib`,
`compression.min_saving_percent`, `compression.max_decoded_page_bytes`,
`compression.background_io_mbps` and `compression.dictionary_scope`.

Today nothing in the workspace resolves any of these: the only
compression in-tree is a hardcoded `ZSTD-3` wrap on `.derc` pages
(BACE §6). No `BACE*` policy type exists in `bradian-relational`.

The gaps analysis §5.1 lists this as the first BACE prerequisite:
every §4 functional requirement depends on being able to resolve a
policy at the subsystem adapter boundary.

## 2. Scope of this ADR

**In scope:** the type model, scope chain, resolution semantics,
capability-veto rules and the API surface a subsystem adapter
(row store, column segment, WAL, backup, replication, …) will
invoke to answer "what policy applies here?".

**Explicitly out of scope** (each will need its own ADR):

- Codec identifier numbers and the codec-registry capability-exchange
  wire format. (Proposed ADR 0061.)
- Compressed-envelope byte layout: version, codec ID, flags, encoded
  length, decoded length, bounds, integrity material. (Proposed
  ADR 0062.)
- BRPG v2 feature-flag page layout. (Proposed ADR 0063.)
- Codec selection algorithm and adaptive cost model (BACE §5.1).
- Persistence encoding for policy records. (Will be a sub-ADR
  attached to catalog storage.)
- The first subsystem adapter to integrate. (Plan recommends
  `backup` — BACE §4.8, cold-path, reversible — but the choice is
  a §13 owner decision.)

## 3. Proposed design

### 3.1 Scope chain (seven levels)

```
LEVEL 0   PLATFORM_BASELINE        (immutable, shipped with the build)
LEVEL 1   WORKLOAD_POLICY          (named profile, e.g. FAST/BALANCED/MAX)
LEVEL 2   DATABASE_POLICY          (per database, in catalog)
LEVEL 3   SCHEMA_POLICY            (per schema, in catalog)
LEVEL 4   TABLE_POLICY             (per table, in catalog)
LEVEL 5   COLUMN_POLICY            (per column, optional, in catalog)
LEVEL 6   JOB_POLICY               (explicit maintenance job override)
```

Precedence: **higher number wins** per key. A policy at level 6
(job) overrides a conflicting policy at level 5 (column), which
overrides level 4, and so on. A key with no setting at any level
takes the platform baseline value.

Three special levels on top of 0–6:

- **CAPABILITY VETO** (always wins): if a requested codec is not
  available on the current build or is forbidden by the format
  capability set, the resolved value is replaced with the platform
  baseline for that key and an audit record is emitted (see §3.6).
- **SECURITY VETO** (always wins): if a policy would disable
  encryption, weaken authenticated integrity, or allow cross-tenant
  dictionary reuse (BACE-SEC-002), the requested value is replaced
  with a safe fallback.
- **CODEC-SPECIFIC DOWNGRADE**: if a codec is available but a
  policy requests parameters outside its declared range (e.g. an
  invalid ZSTD level), the parameter is clamped to the codec's
  declared range and an audit record is emitted.

### 3.2 Policy key catalogue (initial set)

Every key has a declared type, default value, legal range or enum,
and a short description. This set is a starting point drawn from
BACE §8 and is intended to grow in subsequent ADRs.

| Key | Type | Default | Range / enum |
|---|---|---|---|
| `compression.profile` | enum | `BALANCED` | `FAST`, `BALANCED`, `MAX`, `RAW` |
| `compression.max_cpu_percent` | u8 | 50 | 0..=100 |
| `compression.max_decode_ns_per_kib` | u64 | 20_000 | > 0 |
| `compression.min_saving_percent` | u8 | 5 | 0..=99 |
| `compression.max_decoded_page_bytes` | u32 | 4_194_304 | 4096..=16_777_216 |
| `compression.background_io_mbps` | u32 | 100 | > 0 |
| `compression.dictionary_scope` | enum | `TABLE` | `NONE`, `COLUMN`, `TABLE`, `DATABASE` |
| `compression.allowed_codecs` | bitset | all codecs available in build | registry-defined |

### 3.3 API surface (proposed)

Name is illustrative — the committed shape waits on owner review.

```rust
/// A resolved value for one key at one scope chain.
pub struct Resolved<T> {
    pub value: T,
    pub resolved_at: ScopeLevel,
    pub overridden_by: Option<VetoReason>,
}

pub enum ScopeLevel {
    PlatformBaseline, Workload, Database, Schema, Table, Column, Job,
}

pub enum VetoReason {
    CapabilityUnavailable { requested_codec: CodecId },
    SecurityPolicy { reason: &'static str },
    CodecParameterOutOfRange { requested: Value, clamped: Value },
}

pub trait PolicyService {
    /// Resolve one key against a scope chain. Never fails: missing
    /// settings fall back through the chain to the platform baseline.
    fn resolve<T: PolicyValue>(&self, key: PolicyKey, scope: &ScopeChain)
        -> Resolved<T>;

    /// Bulk resolve many keys at once (for a single page/batch
    /// encoding decision). Must take exactly one atomic snapshot of
    /// the policy tree so a concurrent update cannot split the view.
    fn resolve_snapshot(&self, keys: &[PolicyKey], scope: &ScopeChain)
        -> ResolvedSnapshot;
}
```

### 3.4 Resolution semantics

1. Build the scope chain from innermost (job, if any) to outermost
   (platform baseline).
2. For each key, walk the chain from innermost out; the first level
   with a declared value for that key wins.
3. Apply the **capability veto**: if the resolved codec is not in
   the current build's codec registry, fall back to the platform
   baseline.
4. Apply the **security veto**: if the resolved value would weaken
   encryption, integrity or tenant isolation, fall back to the
   platform baseline.
5. Apply the **codec parameter clamp**: if the resolved parameter
   is outside the codec's declared range, clamp to the nearest
   legal value.
6. Record the resolved value, the scope level that provided it, and
   any veto that fired.

### 3.5 Atomicity

A single page/batch encoding decision resolves multiple keys. The
policy service MUST guarantee that one `resolve_snapshot` call
returns values that all come from the same policy tree generation.
A concurrent policy update that changes `compression.profile` and
`compression.allowed_codecs` must either both be visible or both be
invisible to a single resolution.

Implementation note (non-normative): a copy-on-write policy tree
with an atomic `Arc<PolicyTree>` swap is one way to satisfy this.
The final implementation ADR will decide.

### 3.6 Audit

Every veto (`CapabilityUnavailable`, `SecurityPolicy`,
`CodecParameterOutOfRange`) MUST emit an audit record carrying:

- resolved key,
- requested value,
- replaced value,
- veto reason,
- scope chain identity (not its full contents, to avoid logging
  sensitive policy bodies).

The audit channel itself is a separate ADR (ties to BRD-SEC-005).
This ADR specifies the record contents; the channel wiring is
deferred.

### 3.7 Persistence

**Not frozen by this ADR.** Levels 2–5 (database/schema/table/column)
are persistent and must live in the catalog or in a sibling
policy store. The on-disk encoding, versioning and migration
protocol will be specified in a subsequent ADR once the catalog's
BACE integration is designed. Levels 0 (platform baseline) and 6
(job) are process-local.

Level 1 (workload policy) is a named profile. Whether these are
shipped in the build (immutable), managed by the operator (durable),
or declared per session is deferred to the subsequent persistence
ADR.

## 4. Alternatives considered

**A. Flat key/value config map.** Simpler, but gives no path to
capability veto or codec-parameter clamping. Rejected: fails
BACE-ARCH-001's "security and format capabilities override" clause.

**B. Per-codec policy objects.** Every codec owns its own policy
tree. Cleaner object model, but prevents cross-codec rules like
`compression.min_saving_percent` from applying uniformly. Rejected
as a primary model; may be used internally by the codec registry.

**C. SQL-level ALTER TABLE SET COMPRESSION =** as the only
interface. BACE §8 explicitly flags this SQL surface as
"non-implemented illustration". Rejected as the sole interface;
a programmatic policy service must exist first, and SQL can grow
on top of it later.

**D. No central service; let each subsystem adapter own its own
policy.** This is the current state. Rejected because BACE-ARCH-001
requires a central versioned service and because without one, cross-
subsystem invariants (encryption-after-compression, tenant isolation)
cannot be enforced consistently.

## 5. Open design questions (owner review)

These are the decisions this ADR deliberately does **not** take.
Each needs an answer before implementation begins.

1. **Scope-chain cardinality.** The proposed seven levels match
   BACE §8. Is a per-tenant level required between workload and
   database? Would eight levels be too many for operators to reason
   about?
2. **Named workload profiles** (`FAST` / `BALANCED` / `MAX` / `RAW`
   as in §3.2) — are these a fixed enum set, or should operators
   be able to define their own named profiles?
3. **Persistence surface.** Is the policy tree persisted in the
   catalog alongside DDL events (reusing `PersistentCatalog`), or
   in a sibling policy store with its own WAL?
4. **Compatibility with existing `.derc` ZSTD-3 hardcode.** Should
   the first integration migrate `.derc` to policy-governed
   compression, or leave it in place and apply the policy service
   to new subsystems only?
5. **First subsystem to integrate.** Plan §12.1 recommends `backup`
   (cold path, reversible, ZSTD already proven). Confirm or pick
   another.
6. **SQL surface.** Defer `ALTER TABLE ... SET COMPRESSION` to a
   later ADR, or scope it now?
7. **Audit channel.** Use a new structured log, extend `tracing`,
   or wait for the BRD-SEC-005 audit workstream?

## 6. Impact on invariants

Nothing in this ADR violates SRS v1.1 §20 invariants:

- **INV-001** commit correctness: no commit path changes.
- **INV-002** one authoritative state: policies are a protocol
  layer; row state remains authoritative.
- **INV-003** no authorization gap: policy resolution does not
  bypass authorization; it only chooses an encoding within what
  authorization allowed.
- **INV-004** no unsafe automatic availability: N/A — no
  distributed decision.
- **INV-005** recoverable lifecycle: policy changes must have a
  state machine; §3.7 defers the persistence mechanism but the
  invariant obligation is noted here.

## 7. Dependencies this ADR would add

None in design. If accepted, implementation will not introduce new
runtime dependencies beyond what is already in CLAUDE.md §5.

## 8. What this ADR explicitly does NOT do

- Freeze any on-disk or wire format.
- Choose codec identifier numbers.
- Decide the compressed-envelope layout.
- Alter `.derc` or BRPG today.
- Introduce any `BACE*` admission / `ResourceError` variant.
- Modify any existing ADR.

If accepted, this ADR produces design material only. The next
proposed ADRs are **0061 (codec registry capability negotiation)**,
**0062 (compressed envelope byte layout)**, and **0063 (BRPG v2
feature-flag page)**, each of which must freeze a specific format.

## 9. Acceptance evidence required before marking Accepted

1. Owner sign-off on §5 open design questions.
2. Workload-profile set finalized.
3. Scope-chain cardinality confirmed.
4. First-subsystem choice confirmed.
5. Audit channel decision.

Only then can this ADR move from **PROPOSED** to **ACCEPTED**, and
only then can an implementation ADR follow.
