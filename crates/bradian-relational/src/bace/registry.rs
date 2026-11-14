//! BACE codec registry and capability negotiation (accepted ADR 0061
//! §3.4 and §3.5).
//!
//! The registry is **build-time static**: a process's codec set is a
//! function of its build, matching ADR 0061 §3.8. No runtime plugin
//! registration path exists. `BuiltInRegistry::new()` installs the
//! shipped codec set (RAW only in this component); tests may install
//! alternate sets via `BuiltInRegistry::with_codecs`.

use super::codec::{Codec, CodecCapability, CodecId};
use super::codec::raw::RawCodec;

/// Result of negotiating codec capabilities with a remote peer.
///
/// `agreed` is the intersection of IDs where both sides publish the
/// same `format_version`. `missing_critical` names IDs that one side
/// marked `critical` but that the other side does not know. Per
/// ADR 0061 §3.5, a non-empty `missing_critical` **fails the session
/// closed**; the caller MUST NOT silently downgrade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Negotiation {
    pub agreed: Vec<CodecId>,
    pub missing_critical: Vec<CodecId>,
}

impl Negotiation {
    /// Returns `true` when the negotiation is safe to proceed.
    pub fn is_viable(&self) -> bool {
        self.missing_critical.is_empty()
    }
}

/// Codec lookup and negotiation surface per ADR 0061 §3.4.
pub trait CodecRegistry {
    /// Enumerate every codec in this registry. Immutable after
    /// construction; concurrent readers see the same set.
    fn capabilities(&self) -> &[CodecCapability];

    /// Resolve one codec ID. Callers MUST treat `None` as fail-closed
    /// (ADR 0061 §3.6 invariant 1); there is no silent fallback.
    fn capability(&self, id: CodecId) -> Option<&CodecCapability>;

    /// Return the codec implementation for `id`, if registered. This
    /// is the lookup that actually encodes/decodes payloads.
    fn codec(&self, id: CodecId) -> Option<&dyn Codec>;

    /// Negotiate the intersection with a remote capability set.
    /// Per ADR 0061 §3.5:
    ///
    /// 1. Agreed IDs are those present on both sides with matching
    ///    `format_version`.
    /// 2. Missing-critical IDs are those marked `critical` on either
    ///    side but absent on the other.
    /// 3. The caller MUST abort the session if `missing_critical` is
    ///    non-empty.
    fn negotiate(&self, remote: &[CodecCapability]) -> Negotiation {
        let local = self.capabilities();
        let mut agreed = Vec::new();
        let mut missing_critical = Vec::new();

        for lc in local {
            match remote.iter().find(|rc| rc.id == lc.id) {
                Some(rc) if rc.format_version == lc.format_version => {
                    agreed.push(lc.id);
                }
                Some(_) => {
                    // Known on both sides but format_version differs —
                    // not in the agreed set. If either side marks this
                    // codec critical, treat as missing-critical.
                    if lc.flags.critical {
                        missing_critical.push(lc.id);
                    }
                }
                None => {
                    if lc.flags.critical {
                        missing_critical.push(lc.id);
                    }
                }
            }
        }

        for rc in remote {
            if local.iter().any(|lc| lc.id == rc.id) {
                continue;
            }
            if rc.flags.critical {
                missing_critical.push(rc.id);
            }
        }

        agreed.sort_by_key(|id| id.0);
        missing_critical.sort_by_key(|id| id.0);
        missing_critical.dedup();

        Negotiation { agreed, missing_critical }
    }
}

/// The default built-in BACE codec registry.
///
/// Shipped set: `RAW` (ID 48) only in this component. Additional
/// codecs are added by later implementation ADRs, each one landing
/// with its own golden-corpus commitment per ADR 0061 §3.7.
pub struct BuiltInRegistry {
    codecs: Vec<Box<dyn Codec>>,
    capabilities: Vec<CodecCapability>,
}

impl BuiltInRegistry {
    /// Construct the default shipped registry.
    pub fn new() -> Self {
        Self::with_codecs(vec![Box::new(RawCodec)])
    }

    /// Construct a registry from an explicit codec set. Test helper.
    pub fn with_codecs(codecs: Vec<Box<dyn Codec>>) -> Self {
        let capabilities = codecs.iter().map(|c| c.capability().clone()).collect();
        Self { codecs, capabilities }
    }
}

impl Default for BuiltInRegistry {
    fn default() -> Self { Self::new() }
}

impl CodecRegistry for BuiltInRegistry {
    fn capabilities(&self) -> &[CodecCapability] {
        &self.capabilities
    }

    fn capability(&self, id: CodecId) -> Option<&CodecCapability> {
        self.capabilities.iter().find(|c| c.id == id)
    }

    fn codec(&self, id: CodecId) -> Option<&dyn Codec> {
        self.codecs.iter().find_map(|c| {
            if c.capability().id == id { Some(c.as_ref()) } else { None }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::codec::{
        raw::{RawCodec, RAW_ID},
        CapabilityFlags, CodecCapability, Determinism,
    };
    use super::super::envelope::{
        read_envelope, write_envelope, EnvelopeCaps, EnvelopeFlags, EnvelopeHeader,
    };

    fn fake_capability(id: u16, version: u16, critical: bool) -> CodecCapability {
        CodecCapability {
            id: CodecId(id),
            format_version: version,
            max_input_bytes: u32::MAX,
            max_output_bytes: u32::MAX,
            determinism: Determinism::Deterministic,
            flags: CapabilityFlags { critical, ..CapabilityFlags::default_outer() },
            name: "fake",
        }
    }

    #[test]
    fn default_registry_registers_raw_at_id_48() {
        let r = BuiltInRegistry::new();
        assert_eq!(r.capabilities().len(), 1);
        let cap = r.capability(RAW_ID).expect("RAW must be registered");
        assert_eq!(cap.id, RAW_ID);
        assert_eq!(cap.format_version, 1);
        assert!(cap.flags.critical, "RAW must be critical — it is the fallback");
    }

    #[test]
    fn capability_lookup_for_unregistered_id_returns_none() {
        let r = BuiltInRegistry::new();
        assert!(r.capability(CodecId(16)).is_none());
        assert!(r.codec(CodecId(16)).is_none());
    }

    #[test]
    fn codec_lookup_returns_working_codec_that_round_trips_via_envelope() {
        // End-to-end: encode via registry-looked-up codec, write envelope,
        // read envelope, decode via registry-looked-up codec. Proves the
        // registry wires the codec and the envelope together correctly.
        let r = BuiltInRegistry::new();
        let codec = r.codec(RAW_ID).expect("RAW must be registered");
        let input = b"the quick brown fox";

        let mut encoded = Vec::new();
        codec.encode(input, &mut encoded).unwrap();
        assert_eq!(encoded, input);

        let mut buf = Vec::new();
        write_envelope(&mut buf, &EnvelopeHeader {
            codec_id: RAW_ID,
            codec_format_version: codec.capability().format_version,
            flags: EnvelopeFlags::default(),
            encoded_length: encoded.len() as u32,
            decoded_length: input.len() as u32,
        }, &encoded).unwrap();

        let env = read_envelope(&buf, EnvelopeCaps::unbounded()).unwrap();
        assert_eq!(env.header.codec_id, RAW_ID);

        let codec = r.codec(env.header.codec_id).expect("codec must resolve");
        let mut decoded = Vec::new();
        codec.decode(env.payload, env.header.decoded_length, &mut decoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn negotiate_identical_sets_agrees_on_every_id_and_misses_none() {
        let r = BuiltInRegistry::new();
        let remote: Vec<_> = r.capabilities().to_vec();
        let n = r.negotiate(&remote);
        assert!(n.is_viable());
        assert_eq!(n.agreed, vec![RAW_ID]);
        assert!(n.missing_critical.is_empty());
    }

    #[test]
    fn negotiate_when_remote_is_missing_a_local_critical_codec_fails_closed() {
        // Local has RAW (critical). Remote has no codecs. Negotiation
        // must mark RAW as missing_critical and refuse to proceed.
        let r = BuiltInRegistry::new();
        let n = r.negotiate(&[]);
        assert!(!n.is_viable());
        assert_eq!(n.missing_critical, vec![RAW_ID]);
        assert!(n.agreed.is_empty());
    }

    #[test]
    fn negotiate_when_remote_insists_on_a_codec_this_build_lacks_fails_closed() {
        // Local has RAW only. Remote also publishes a non-critical codec
        // this build does not know. Negotiation should agree on RAW and
        // NOT report missing_critical (non-critical).
        let r = BuiltInRegistry::new();
        let remote = vec![
            r.capabilities()[0].clone(),
            fake_capability(32, 1, false),
        ];
        let n = r.negotiate(&remote);
        assert!(n.is_viable());
        assert_eq!(n.agreed, vec![RAW_ID]);
        assert!(n.missing_critical.is_empty());

        // Now mark that same remote codec critical — negotiation must
        // fail closed.
        let remote_with_critical_unknown = vec![
            r.capabilities()[0].clone(),
            fake_capability(32, 1, true),
        ];
        let n = r.negotiate(&remote_with_critical_unknown);
        assert!(!n.is_viable());
        assert_eq!(n.missing_critical, vec![CodecId(32)]);
    }

    #[test]
    fn negotiate_rejects_matching_ids_with_different_format_versions() {
        // Remote publishes RAW at a bumped format_version. The IDs match
        // but the versions differ — not in the agreed set. Because RAW
        // is critical on both sides, the mismatch surfaces as
        // missing_critical on the local side.
        let r = BuiltInRegistry::new();
        let mismatch = vec![fake_capability(RAW_ID.0, 2, true)];
        let n = r.negotiate(&mismatch);
        assert!(!n.is_viable());
        assert_eq!(n.missing_critical, vec![RAW_ID]);
        assert!(n.agreed.is_empty());
    }

    #[test]
    fn with_codecs_test_helper_installs_the_provided_set_only() {
        // Install a registry without RAW. capability() for RAW must
        // return None, and any future caller must fail closed.
        let empty = BuiltInRegistry::with_codecs(Vec::new());
        assert_eq!(empty.capabilities().len(), 0);
        assert!(empty.codec(RAW_ID).is_none());

        // Install an alternate set with one non-RAW critical codec at
        // ID 300 (vendor range). Not something production does; this
        // is strictly a test helper.
        let alt = BuiltInRegistry::with_codecs(vec![Box::new(RawCodec)]);
        assert!(alt.codec(RAW_ID).is_some());
    }
}
