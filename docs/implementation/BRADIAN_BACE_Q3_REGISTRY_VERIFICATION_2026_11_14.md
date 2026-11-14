# BACE Component Q.3 — codec registry + negotiation verification

14 November 2026. Baseline: `1622eef`.
Third BACE implementation step against accepted ADR 0061 §3.4 / §3.5
and the registry obligation surfaced in ADR 0062 §4 steps 6–7.

## Executed result

**892 Rust tests passed, 0 failed, 0 ignored.** Clippy clean on
`bradian-relational --all-targets`. No persistent format change.

| Executed check | Result |
|---|---|
| `cargo check -p bradian-relational` | Passed |
| `cargo test --workspace` | 892 passed, 0 failed |
| `cargo test -p bradian-relational --lib bace::registry` | 8 passed |
| `cargo clippy -p bradian-relational --all-targets -- -D warnings` | Passed |

## What landed

New submodule `crates/bradian-relational/src/bace/registry.rs`:

- `Negotiation { agreed: Vec<CodecId>, missing_critical: Vec<CodecId> }`
  with `is_viable()` convenience accessor. `missing_critical`
  non-empty is the ADR 0061 §3.5 fail-closed signal for the caller.
- `CodecRegistry` trait exposing:
  - `capabilities()` — the full list;
  - `capability(id)` — per-ID capability lookup, returns `None` for
    unregistered IDs (ADR 0061 §3.6 invariant 1 fail-closed);
  - `codec(id)` — concrete `&dyn Codec` lookup, same fail-closed
    contract;
  - `negotiate(remote)` — default implementation runs the ADR 0061
    §3.5 five-step algorithm.
- `BuiltInRegistry` concrete type with `new()` installing the
  shipped codec set and `with_codecs(Vec<Box<dyn Codec>>)` for test
  injection.

The shipped set is **RAW only** (ID 48) in this component. Every
subsequent codec arrives with its own golden-corpus commitment per
ADR 0061 §3.7 and its own implementation ADR.

## Negotiation algorithm (ADR 0061 §3.5)

```text
let A = ∅
let M = ∅
for each lc in local:
    if some rc in remote with rc.id == lc.id:
        if rc.format_version == lc.format_version:
            A ← A ∪ { lc.id }
        else if lc.flags.critical:
            M ← M ∪ { lc.id }
    else if lc.flags.critical:
        M ← M ∪ { lc.id }
for each rc in remote where no lc has lc.id == rc.id:
    if rc.flags.critical:
        M ← M ∪ { rc.id }
return (A sorted, M sorted and deduplicated)
```

A caller MUST treat `missing_critical.is_empty() == false` as a
release-blocking fail-closed signal.

## 8 unit tests

Positive:
- `default_registry_registers_raw_at_id_48` — RAW is present,
  format_version 1, critical flag set.
- `codec_lookup_returns_working_codec_that_round_trips_via_envelope`
  — **end-to-end integration**: registry `codec()` → encode →
  `write_envelope` → `read_envelope` → registry `codec()` → decode.
  Proves the registry correctly wires the codec and the envelope
  together.
- `negotiate_identical_sets_agrees_on_every_id_and_misses_none` —
  local == remote agrees on every ID.
- `with_codecs_test_helper_installs_the_provided_set_only` — the
  test injection surface works for empty and single-codec sets.

Fail-closed:
- `capability_lookup_for_unregistered_id_returns_none` — both
  `capability()` and `codec()` return `None` for unknown IDs.
- `negotiate_when_remote_is_missing_a_local_critical_codec_fails_closed`
  — remote with no codecs → `missing_critical == [RAW_ID]`,
  `is_viable() == false`.
- `negotiate_when_remote_insists_on_a_codec_this_build_lacks_fails_closed`
  — covers both directions: a remote critical codec unknown
  locally shows up in `missing_critical`; a non-critical unknown
  remote codec does not.
- `negotiate_rejects_matching_ids_with_different_format_versions`
  — same ID, bumped `format_version` → not in `agreed`; because
  RAW is critical on both sides, surfaces as `missing_critical`.

## Compliance check against ADR 0061

- §3.4 API surface — `capabilities()`, `capability(id)`,
  `negotiate(remote)`. Added `codec(id)` because the registry must
  own the actual codec implementations to decode envelopes. ✓
- §3.5 negotiation algorithm — implemented verbatim with
  deterministic sort order and dedup on `missing_critical`. ✓
- §3.6 fail-closed invariant 1 (Unknown codec ID) — `capability()`
  and `codec()` both return `Option`, surfacing the invariant to
  the caller. ✓
- §3.7 portability — RAW ships with a `Deterministic` claim; a
  golden-file corpus for RAW's byte-exact behaviour ships via the
  envelope test suite (Q.2) and the round-trip test here. ✓
- §3.8 build-time static — `BuiltInRegistry::new()` is the only
  production constructor; the test helper `with_codecs` is
  non-production. ✓

## What this component explicitly does NOT do

- No codec selection algorithm (BACE §5.1 adaptive cost model).
  Deferred to a later component.
- No `supported_types`, `parameters`, `decode_cpu_ns_per_kib_estimate`
  or `simd_availability` fields on `CodecCapability` yet. These
  are added when the selection algorithm needs them — this
  component's lookup + negotiation paths do not.
- No policy service yet — Component Q.4.
- No carrier integration (BRPG v2, row_store, `.derc`, BRWL,
  backup).
- No ID assignment in the 16–47 inner-encoding range. P1 still
  deferred.

## Next

Component Q.4 — `PolicyService` skeleton with the seven-level
`ScopeChain` from ADR 0060 §3.1 and an in-memory implementation.

See [BACE_ADR_REVIEW_PACKAGE](BACE_ADR_REVIEW_PACKAGE.md),
[ADR 0061](../adr/0061-bace-codec-registry-capability-negotiation.md).
