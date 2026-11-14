//! BACE — Bradian Adaptive Compression Engine (accepted ADRs 0060–0063).
//!
//! **Component-by-component implementation.** This module grows one
//! sub-module at a time:
//!
//! - Component Q.1 — `codec`: codec types and the RAW codec (ADR 0061).
//! - Component Q.2 — `envelope`: envelope v1 byte layout (ADR 0062). TBD.
//! - Component Q.3 — `registry`: static codec registry (ADR 0061). TBD.
//! - Component Q.4 — `policy`: policy service skeleton (ADR 0060). TBD.
//!
//! **Standalone module.** No carrier integration (BRPG v2 pages, `.derc`
//! column segments, BRWL WAL frames, backup archives) lands in this
//! component or in Q.2–Q.4. Carrier integration is a separate workstream
//! per each future implementation ADR.
//!
//! See the comprehensive review package
//! `docs/implementation/BACE_ADR_REVIEW_PACKAGE.md` for the accepted
//! design decisions.

pub mod codec;
pub mod envelope;
pub mod policy;
pub mod registry;

pub use codec::{
    CapabilityFlags, Codec, CodecCapability, CodecId, DecodeError, Determinism, EncodeError,
};
pub use envelope::{
    read_envelope, write_envelope, Envelope, EnvelopeCaps, EnvelopeError, EnvelopeFlags,
    EnvelopeHeader, ENVELOPE_HEADER_SIZE, ENVELOPE_MAGIC, ENVELOPE_MIN_SIZE, ENVELOPE_TRAILER_SIZE,
    ENVELOPE_VERSION,
};
pub use policy::{
    CompressionProfile, DictionaryScope, InMemoryPolicyService, PlatformBaseline, PolicyKey,
    PolicyService, PolicyValue, Resolved, ScopeChain, ScopeLevel, ScopeOverrides, VetoReason,
};
pub use registry::{BuiltInRegistry, CodecRegistry, Negotiation};
