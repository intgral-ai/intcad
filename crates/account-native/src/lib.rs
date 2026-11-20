//! Native account adapters.
//!
//! This crate owns the durable operation ledger and, in later tasks, the OS credential store and
//! HTTP account service. It is native-only and must never enter the wasm closure (`xtask/src/layers.rs`,
//! `Class::Exempt`). The engine keeps the pure interfaces and test doubles.
#![cfg(not(target_arch = "wasm32"))]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod contract;
mod http;
mod ledger;

#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod ledger_tests;

pub use contract::{ContractArtifacts, ContractPin, VerifiedContract};
pub use http::{
    BootstrapPlatform, FixedCredentials, HttpAccountService, HttpConfigError, InstallationBootstrap, Secret, Transport, TransportFailure,
    WireConnection, WireCredentials, WireRequest, WireResponse,
};
pub use ledger::{FileOperationStore, InMemoryOperationStore};
pub use solidcraft_engine::entitlement::{
    AuthenticatedHostScope, Capability, EntryCategory, FreshIntent, OperationId, OperationRecord, OperationStatus, OperationStore,
    OperationStoreError, RetryDecision, RetryMapping, RetryScopeKey, ReviewOperation, RunId, SaveOutcome, TransportCorrelation,
};
