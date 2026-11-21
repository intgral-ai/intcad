//! Native account adapters.
//!
//! This crate owns the durable operation ledger and, in later tasks, the OS credential store and
//! HTTP account service. It is native-only and must never enter the wasm closure (`xtask/src/layers.rs`,
//! `Class::Exempt`). The engine keeps the pure interfaces and test doubles.
#![cfg(not(target_arch = "wasm32"))]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod contract;
mod credentials;
mod device_code;
mod http;
mod ledger;
mod session_verification;

#[cfg(test)]
mod credentials_tests;
#[cfg(test)]
mod device_code_tests;
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod session_verification_tests;

pub use contract::{ContractArtifacts, ContractPin, VerifiedContract};
pub use credentials::{
    ACCOUNT_KEYRING_SERVICE, BlockedProductExchange, CredentialSession, KeyringStore, MemorySession, NoPersistentStore, ProductExchangeError,
    ProductLogin, ProductSession, ProductSessionExchange, ProviderLogin, ProviderSession,
};
pub use device_code::{
    DeviceCodeError, DeviceCodeFlow, DeviceCodeMode, DeviceCodePoll, DeviceCodePrompt, DeviceCodeTransport, UreqDeviceCodeTransport,
    WORKOS_DEVICE_AUTHORIZATION_URL, WORKOS_DEVICE_GRANT_TYPE, WORKOS_TOKEN_URL, WorkOsPublicClient,
};
pub use http::{
    BootstrapPlatform, CredentialHandoff, FixedCredentials, HttpAccountService, HttpConfigError, InstallationBootstrap, Secret, Transport,
    TransportFailure, WireConnection, WireCredentials, WireRequest, WireResponse,
};
pub use ledger::{FileOperationStore, InMemoryOperationStore};
pub use session_verification::ServerVerifiedProductSession;
pub use solidcraft_engine::entitlement::{
    AuthenticatedHostScope, Capability, EntryCategory, FreshIntent, OperationId, OperationRecord, OperationStatus, OperationStore,
    OperationStoreError, RetryDecision, RetryMapping, RetryScopeKey, ReviewOperation, RunId, SaveOutcome, TransportCorrelation,
};
