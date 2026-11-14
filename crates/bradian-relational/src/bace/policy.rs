//! BACE central policy service skeleton (accepted ADR 0060).
//!
//! Seven-level scope chain with higher-number-wins per-key precedence:
//!
//! ```text
//! LEVEL 0   PLATFORM_BASELINE
//! LEVEL 1   WORKLOAD_POLICY     (FAST / BALANCED / MAX / RAW)
//! LEVEL 2   DATABASE_POLICY     (persisted in PersistentCatalog — later ADR)
//! LEVEL 3   SCHEMA_POLICY       (persisted — later ADR)
//! LEVEL 4   TABLE_POLICY        (persisted — later ADR)
//! LEVEL 5   COLUMN_POLICY       (persisted — later ADR)
//! LEVEL 6   JOB_POLICY          (process-local override)
//! ```
//!
//! This component ships the type model, the resolution semantics and
//! an `InMemoryPolicyService` implementation. Levels 2–5 persistence
//! (catalog DDL event wiring) is deferred per ADR 0060 §5 item 3.

use std::collections::HashMap;

use super::codec::CodecId;
use super::registry::CodecRegistry;

/// Workload profile enum (ADR 0060 §5 item 2 — fixed enum, four
/// profiles). Operator-definable profiles are a future ADR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionProfile {
    /// Prefer latency over space.
    Fast,
    /// Default; balance space, CPU and latency.
    Balanced,
    /// Prefer space over latency.
    Max,
    /// Compression disabled; always emit RAW envelopes.
    Raw,
}

/// Dictionary scope (ADR 0060 §3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictionaryScope {
    None,
    Column,
    Table,
    Database,
}

/// The seven scope levels per ADR 0060 §3.1. Higher-numbered levels
/// win on conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeLevel {
    PlatformBaseline = 0,
    Workload = 1,
    Database = 2,
    Schema = 3,
    Table = 4,
    Column = 5,
    Job = 6,
}

/// Every policy key the service knows about. ADR 0060 §3.2 initial
/// catalogue; this list grows as new policies land in later ADRs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PolicyKey {
    CompressionProfile,
    CompressionMaxCpuPercent,
    CompressionMaxDecodeNsPerKib,
    CompressionMinSavingPercent,
    CompressionMaxDecodedPageBytes,
    CompressionBackgroundIoMbps,
    CompressionDictionaryScope,
    CompressionAllowedCodecs,
}

/// Type-tagged policy value. The policy service never produces a
/// value of the wrong variant for a given key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyValue {
    Profile(CompressionProfile),
    DictionaryScope(DictionaryScope),
    U8(u8),
    U32(u32),
    U64(u64),
    CodecSet(Vec<CodecId>),
}

impl PolicyValue {
    pub fn as_profile(&self) -> Option<CompressionProfile> {
        match self { Self::Profile(v) => Some(*v), _ => None }
    }
    pub fn as_dictionary_scope(&self) -> Option<DictionaryScope> {
        match self { Self::DictionaryScope(v) => Some(*v), _ => None }
    }
    pub fn as_u8(&self) -> Option<u8> {
        match self { Self::U8(v) => Some(*v), _ => None }
    }
    pub fn as_u32(&self) -> Option<u32> {
        match self { Self::U32(v) => Some(*v), _ => None }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match self { Self::U64(v) => Some(*v), _ => None }
    }
    pub fn as_codec_set(&self) -> Option<&[CodecId]> {
        match self { Self::CodecSet(v) => Some(v.as_slice()), _ => None }
    }
}

/// A resolution outcome. Records which scope level provided the
/// value and which veto (if any) fired during resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub value: PolicyValue,
    pub resolved_at: ScopeLevel,
    pub overridden_by: Option<VetoReason>,
}

/// Veto reason recorded on every forced override. Emitted to `tracing`
/// per ADR 0060 §5 item 7 — the first implementation uses `tracing`;
/// future integration with BRD-SEC-005 audit comes later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VetoReason {
    CapabilityUnavailable { requested_codec: CodecId },
    SecurityPolicy { reason: &'static str },
    CodecParameterOutOfRange { requested: PolicyValue, clamped: PolicyValue },
}

/// Per-level override map. An entry at level N with key K overrides
/// any value at levels < N for that key.
pub type ScopeOverrides = HashMap<PolicyKey, PolicyValue>;

/// The resolution scope passed to `PolicyService::resolve`. Each
/// level is optional; absent levels fall through. The platform
/// baseline is NOT in `ScopeChain` — it is owned by the policy
/// service implementation.
#[derive(Clone, Debug, Default)]
pub struct ScopeChain {
    pub workload: ScopeOverrides,
    pub database: ScopeOverrides,
    pub schema: ScopeOverrides,
    pub table: ScopeOverrides,
    pub column: ScopeOverrides,
    pub job: ScopeOverrides,
}

impl ScopeChain {
    /// A scope chain with no overrides; every key resolves to the
    /// platform baseline.
    pub fn baseline_only() -> Self { Self::default() }

    fn at(&self, level: ScopeLevel) -> Option<&ScopeOverrides> {
        match level {
            ScopeLevel::PlatformBaseline => None,
            ScopeLevel::Workload => Some(&self.workload),
            ScopeLevel::Database => Some(&self.database),
            ScopeLevel::Schema => Some(&self.schema),
            ScopeLevel::Table => Some(&self.table),
            ScopeLevel::Column => Some(&self.column),
            ScopeLevel::Job => Some(&self.job),
        }
    }
}

/// The policy service API per ADR 0060 §3.3.
pub trait PolicyService {
    /// Resolve one key against the scope chain. Never fails — missing
    /// overrides fall through to the platform baseline.
    fn resolve(&self, key: PolicyKey, scope: &ScopeChain) -> Resolved;

    /// Bulk resolve many keys at once. Implementations MUST take one
    /// atomic snapshot of the policy tree so a concurrent update
    /// cannot split the view (ADR 0060 §3.5 atomicity).
    fn resolve_snapshot(
        &self,
        keys: &[PolicyKey],
        scope: &ScopeChain,
    ) -> HashMap<PolicyKey, Resolved> {
        keys.iter().map(|k| (*k, self.resolve(*k, scope))).collect()
    }
}

/// Shipped platform baseline — the LEVEL 0 fallback. These values are
/// immutable and built into the process; ADR 0060 §5 item 3 excludes
/// them from persistence.
#[derive(Clone, Debug)]
pub struct PlatformBaseline {
    pub profile: CompressionProfile,
    pub max_cpu_percent: u8,
    pub max_decode_ns_per_kib: u64,
    pub min_saving_percent: u8,
    pub max_decoded_page_bytes: u32,
    pub background_io_mbps: u32,
    pub dictionary_scope: DictionaryScope,
    pub allowed_codecs: Vec<CodecId>,
}

impl PlatformBaseline {
    /// Default shipped baseline. Values from ADR 0060 §3.2.
    pub fn shipped() -> Self {
        Self {
            profile: CompressionProfile::Balanced,
            max_cpu_percent: 50,
            max_decode_ns_per_kib: 20_000,
            min_saving_percent: 5,
            max_decoded_page_bytes: 4_194_304,
            background_io_mbps: 100,
            dictionary_scope: DictionaryScope::Table,
            // Component Q.3 ships only RAW. Other codecs join the
            // default allow-set as they register in later ADRs.
            allowed_codecs: vec![super::codec::raw::RAW_ID],
        }
    }

    fn baseline_value(&self, key: PolicyKey) -> PolicyValue {
        match key {
            PolicyKey::CompressionProfile => PolicyValue::Profile(self.profile),
            PolicyKey::CompressionMaxCpuPercent => PolicyValue::U8(self.max_cpu_percent),
            PolicyKey::CompressionMaxDecodeNsPerKib => PolicyValue::U64(self.max_decode_ns_per_kib),
            PolicyKey::CompressionMinSavingPercent => PolicyValue::U8(self.min_saving_percent),
            PolicyKey::CompressionMaxDecodedPageBytes => PolicyValue::U32(self.max_decoded_page_bytes),
            PolicyKey::CompressionBackgroundIoMbps => PolicyValue::U32(self.background_io_mbps),
            PolicyKey::CompressionDictionaryScope => PolicyValue::DictionaryScope(self.dictionary_scope),
            PolicyKey::CompressionAllowedCodecs => PolicyValue::CodecSet(self.allowed_codecs.clone()),
        }
    }
}

impl Default for PlatformBaseline {
    fn default() -> Self { Self::shipped() }
}

/// An in-memory policy service suitable for process-local tests and
/// administrative paths. Persistence (ADR 0060 §5 item 3, levels 2–5
/// via `PersistentCatalog`) is a later implementation ADR.
///
/// This service composes with a `CodecRegistry` so capability vetoes
/// can enforce "the requested codec must be registered in this build"
/// per ADR 0060 §3.4 step 3.
pub struct InMemoryPolicyService<R: CodecRegistry> {
    baseline: PlatformBaseline,
    registry: R,
}

impl<R: CodecRegistry> InMemoryPolicyService<R> {
    pub fn new(baseline: PlatformBaseline, registry: R) -> Self {
        Self { baseline, registry }
    }

    /// Apply the ADR 0060 §3.4 step 3 capability veto. For
    /// `CompressionAllowedCodecs`, drop any codec not in the
    /// registry. If the resulting set is empty, replace the value
    /// with the platform baseline's allow-set and record the first
    /// dropped codec as the veto reason.
    fn apply_capability_veto(&self, key: PolicyKey, resolved: Resolved) -> Resolved {
        if key != PolicyKey::CompressionAllowedCodecs {
            return resolved;
        }
        let codecs = match resolved.value.as_codec_set() {
            Some(v) => v,
            None => return resolved, // type mismatch — leave unchanged
        };
        let (kept, first_missing): (Vec<_>, Option<CodecId>) = {
            let mut kept = Vec::new();
            let mut first_missing = None;
            for id in codecs {
                if self.registry.capability(*id).is_some() {
                    kept.push(*id);
                } else if first_missing.is_none() {
                    first_missing = Some(*id);
                }
            }
            (kept, first_missing)
        };
        match first_missing {
            None => resolved,
            Some(requested_codec) => {
                if kept.is_empty() {
                    Resolved {
                        value: PolicyValue::CodecSet(self.baseline.allowed_codecs.clone()),
                        resolved_at: ScopeLevel::PlatformBaseline,
                        overridden_by: Some(VetoReason::CapabilityUnavailable { requested_codec }),
                    }
                } else {
                    Resolved {
                        value: PolicyValue::CodecSet(kept),
                        resolved_at: resolved.resolved_at,
                        overridden_by: Some(VetoReason::CapabilityUnavailable { requested_codec }),
                    }
                }
            }
        }
    }

    /// Apply the ADR 0060 §3.4 step 5 codec-parameter clamp. Each
    /// key with a declared range clamps its value to the legal
    /// range; the veto reason carries the pre-clamp and post-clamp
    /// values.
    fn apply_parameter_clamp(&self, key: PolicyKey, resolved: Resolved) -> Resolved {
        let (requested, clamped) = match key {
            PolicyKey::CompressionMaxCpuPercent => {
                match resolved.value.as_u8() {
                    Some(v) if v > 100 => (PolicyValue::U8(v), PolicyValue::U8(100)),
                    _ => return resolved,
                }
            }
            PolicyKey::CompressionMinSavingPercent => {
                match resolved.value.as_u8() {
                    Some(v) if v > 99 => (PolicyValue::U8(v), PolicyValue::U8(99)),
                    _ => return resolved,
                }
            }
            PolicyKey::CompressionMaxDecodedPageBytes => {
                match resolved.value.as_u32() {
                    Some(v) if !(4096..=16_777_216).contains(&v) => {
                        let clamped_value = v.clamp(4096, 16_777_216);
                        (PolicyValue::U32(v), PolicyValue::U32(clamped_value))
                    }
                    _ => return resolved,
                }
            }
            _ => return resolved,
        };
        Resolved {
            value: clamped.clone(),
            resolved_at: resolved.resolved_at,
            overridden_by: Some(VetoReason::CodecParameterOutOfRange { requested, clamped }),
        }
    }
}

impl<R: CodecRegistry> PolicyService for InMemoryPolicyService<R> {
    fn resolve(&self, key: PolicyKey, scope: &ScopeChain) -> Resolved {
        // Walk from JOB (level 6) down to WORKLOAD (level 1). First
        // match wins. If none matches, fall back to the platform
        // baseline at level 0.
        for level in [
            ScopeLevel::Job,
            ScopeLevel::Column,
            ScopeLevel::Table,
            ScopeLevel::Schema,
            ScopeLevel::Database,
            ScopeLevel::Workload,
        ] {
            if let Some(overrides) = scope.at(level) {
                if let Some(value) = overrides.get(&key) {
                    let resolved = Resolved {
                        value: value.clone(),
                        resolved_at: level,
                        overridden_by: None,
                    };
                    // Vetoes run in the ADR-ordered sequence: capability,
                    // then security (stub; no vetoes fire until an
                    // encryption pipeline exists), then parameter clamp.
                    let resolved = self.apply_capability_veto(key, resolved);
                    return self.apply_parameter_clamp(key, resolved);
                }
            }
        }
        let resolved = Resolved {
            value: self.baseline.baseline_value(key),
            resolved_at: ScopeLevel::PlatformBaseline,
            overridden_by: None,
        };
        // The baseline values are already within range by construction,
        // but apply the capability veto in case the registry has
        // fewer codecs than the baseline allow-set.
        let resolved = self.apply_capability_veto(key, resolved);
        self.apply_parameter_clamp(key, resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::codec::{raw::RAW_ID, CodecId};
    use super::super::registry::BuiltInRegistry;

    fn service() -> InMemoryPolicyService<BuiltInRegistry> {
        InMemoryPolicyService::new(PlatformBaseline::shipped(), BuiltInRegistry::new())
    }

    fn put(level: &mut ScopeOverrides, key: PolicyKey, value: PolicyValue) {
        level.insert(key, value);
    }

    #[test]
    fn baseline_only_scope_resolves_every_key_at_platform_level() {
        let svc = service();
        let scope = ScopeChain::baseline_only();
        for key in [
            PolicyKey::CompressionProfile,
            PolicyKey::CompressionMaxCpuPercent,
            PolicyKey::CompressionMaxDecodeNsPerKib,
            PolicyKey::CompressionMinSavingPercent,
            PolicyKey::CompressionMaxDecodedPageBytes,
            PolicyKey::CompressionBackgroundIoMbps,
            PolicyKey::CompressionDictionaryScope,
            PolicyKey::CompressionAllowedCodecs,
        ] {
            let r = svc.resolve(key, &scope);
            assert_eq!(r.resolved_at, ScopeLevel::PlatformBaseline);
            assert!(r.overridden_by.is_none());
        }
    }

    #[test]
    fn job_level_wins_over_every_lower_level() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        // Set the same key at every level; JOB should win.
        for overrides in [&mut scope.workload, &mut scope.database, &mut scope.schema,
                          &mut scope.table, &mut scope.column] {
            put(overrides, PolicyKey::CompressionProfile, PolicyValue::Profile(CompressionProfile::Fast));
        }
        put(&mut scope.job, PolicyKey::CompressionProfile, PolicyValue::Profile(CompressionProfile::Max));
        let r = svc.resolve(PolicyKey::CompressionProfile, &scope);
        assert_eq!(r.resolved_at, ScopeLevel::Job);
        assert_eq!(r.value.as_profile(), Some(CompressionProfile::Max));
    }

    #[test]
    fn precedence_order_is_job_column_table_schema_database_workload_platform() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        // Fill from workload up, removing one at a time to prove the
        // precedence chain top-down.
        put(&mut scope.workload, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(10));
        put(&mut scope.database, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(20));
        put(&mut scope.schema, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(30));
        put(&mut scope.table, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(40));
        put(&mut scope.column, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(50));
        put(&mut scope.job, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(60));

        for (clear_level, expected_level, expected_value) in [
            (ScopeLevel::Job, ScopeLevel::Column, 50),
            (ScopeLevel::Column, ScopeLevel::Table, 40),
            (ScopeLevel::Table, ScopeLevel::Schema, 30),
            (ScopeLevel::Schema, ScopeLevel::Database, 20),
            (ScopeLevel::Database, ScopeLevel::Workload, 10),
        ] {
            let bucket = match clear_level {
                ScopeLevel::Job => &mut scope.job,
                ScopeLevel::Column => &mut scope.column,
                ScopeLevel::Table => &mut scope.table,
                ScopeLevel::Schema => &mut scope.schema,
                ScopeLevel::Database => &mut scope.database,
                _ => unreachable!(),
            };
            bucket.remove(&PolicyKey::CompressionMaxCpuPercent);
            let r = svc.resolve(PolicyKey::CompressionMaxCpuPercent, &scope);
            assert_eq!(r.resolved_at, expected_level);
            assert_eq!(r.value.as_u8(), Some(expected_value));
        }
    }

    #[test]
    fn resolve_snapshot_returns_atomic_view_across_many_keys() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        put(&mut scope.table, PolicyKey::CompressionProfile, PolicyValue::Profile(CompressionProfile::Fast));
        put(&mut scope.table, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(25));
        let keys = [PolicyKey::CompressionProfile, PolicyKey::CompressionMaxCpuPercent, PolicyKey::CompressionMinSavingPercent];
        let snap = svc.resolve_snapshot(&keys, &scope);
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[&PolicyKey::CompressionProfile].resolved_at, ScopeLevel::Table);
        assert_eq!(snap[&PolicyKey::CompressionMaxCpuPercent].resolved_at, ScopeLevel::Table);
        assert_eq!(snap[&PolicyKey::CompressionMinSavingPercent].resolved_at, ScopeLevel::PlatformBaseline);
    }

    #[test]
    fn capability_veto_drops_unregistered_codecs_and_records_the_first_offender() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        // Ask for RAW (registered) + some unknown codec IDs.
        let requested = vec![RAW_ID, CodecId(32), CodecId(33)];
        put(&mut scope.table, PolicyKey::CompressionAllowedCodecs, PolicyValue::CodecSet(requested));
        let r = svc.resolve(PolicyKey::CompressionAllowedCodecs, &scope);
        // RAW survives; the unknowns are dropped. Veto records the first.
        assert_eq!(r.value.as_codec_set(), Some(&[RAW_ID][..]));
        assert_eq!(r.overridden_by, Some(VetoReason::CapabilityUnavailable { requested_codec: CodecId(32) }));
    }

    #[test]
    fn capability_veto_falls_back_to_baseline_when_every_requested_codec_is_unknown() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        let requested = vec![CodecId(32), CodecId(33)];
        put(&mut scope.table, PolicyKey::CompressionAllowedCodecs, PolicyValue::CodecSet(requested));
        let r = svc.resolve(PolicyKey::CompressionAllowedCodecs, &scope);
        // All unknown — fall back to the baseline allow-set (RAW only).
        assert_eq!(r.resolved_at, ScopeLevel::PlatformBaseline);
        assert_eq!(r.value.as_codec_set(), Some(&[RAW_ID][..]));
        assert!(matches!(r.overridden_by, Some(VetoReason::CapabilityUnavailable { .. })));
    }

    #[test]
    fn parameter_clamp_bounds_u8_cpu_percent_and_records_the_veto() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        put(&mut scope.job, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(250));
        let r = svc.resolve(PolicyKey::CompressionMaxCpuPercent, &scope);
        assert_eq!(r.value.as_u8(), Some(100));
        assert_eq!(r.overridden_by, Some(VetoReason::CodecParameterOutOfRange {
            requested: PolicyValue::U8(250),
            clamped: PolicyValue::U8(100),
        }));
    }

    #[test]
    fn parameter_clamp_bounds_max_decoded_page_bytes_into_the_declared_range() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        // Below the declared 4 KiB minimum.
        put(&mut scope.job, PolicyKey::CompressionMaxDecodedPageBytes, PolicyValue::U32(1024));
        let r = svc.resolve(PolicyKey::CompressionMaxDecodedPageBytes, &scope);
        assert_eq!(r.value.as_u32(), Some(4096));
        assert!(matches!(r.overridden_by, Some(VetoReason::CodecParameterOutOfRange { .. })));

        // Above the declared 16 MiB maximum.
        put(&mut scope.job, PolicyKey::CompressionMaxDecodedPageBytes, PolicyValue::U32(u32::MAX));
        let r = svc.resolve(PolicyKey::CompressionMaxDecodedPageBytes, &scope);
        assert_eq!(r.value.as_u32(), Some(16_777_216));
        assert!(matches!(r.overridden_by, Some(VetoReason::CodecParameterOutOfRange { .. })));
    }

    #[test]
    fn values_already_within_range_pass_the_clamp_without_firing_a_veto() {
        let svc = service();
        let mut scope = ScopeChain::baseline_only();
        put(&mut scope.job, PolicyKey::CompressionMaxCpuPercent, PolicyValue::U8(75));
        let r = svc.resolve(PolicyKey::CompressionMaxCpuPercent, &scope);
        assert_eq!(r.value.as_u8(), Some(75));
        assert!(r.overridden_by.is_none());
    }
}
