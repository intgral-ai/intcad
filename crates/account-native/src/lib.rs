//! Native account adapters.
//!
//! This crate owns the durable operation ledger and, in later tasks, the OS credential store and
//! HTTP account service. It is native-only and must never enter the wasm closure (`xtask/src/layers.rs`,
//! `Class::Exempt`). The engine keeps the pure interfaces and test doubles.
#![cfg(not(target_arch = "wasm32"))]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod account_paths;
mod billing;
mod contract;
mod credentials;
mod device_code;
mod form;
mod http;
mod identity_journal;
mod ledger;
mod session_verification;
mod token_expiry;

#[cfg(test)]
mod credentials_tests;
#[cfg(test)]
mod device_code_tests;
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod identity_journal_tests;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod session_verification_tests;

pub use account_paths::AccountPaths;
pub use billing::SessionBilling;
pub use contract::{ContractArtifacts, ContractPin, VerifiedContract};
pub use credentials::{
    ACCOUNT_KEYRING_SERVICE, BlockedProductExchange, CredentialSession, KeyringStore, MemorySession, NoPersistentStore, ProductExchangeError,
    ProductLogin, ProductSession, ProductSessionExchange, ProviderLogin, ProviderRefresh, ProviderSession,
};
pub use device_code::{
    DeviceCodeError, DeviceCodeFlow, DeviceCodeMode, DeviceCodePoll, DeviceCodePrompt, DeviceCodeTransport, UreqDeviceCodeTransport,
    WORKOS_DEVICE_AUTHORIZATION_URL, WORKOS_DEVICE_GRANT_TYPE, WORKOS_TOKEN_URL, WorkOsPublicClient,
};
pub use http::{
    BootstrapPlatform, CredentialHandoff, FixedCredentials, HttpAccountService, HttpConfigError, InstallationBootstrap, OperationRenewal, Secret,
    Transport, TransportFailure, UreqTransport, WireConnection, WireCredentials, WireRequest, WireResponse,
};
pub use ledger::{FileOperationStore, InMemoryOperationStore};
pub use session_verification::ServerVerifiedProductSession;
pub use solidcraft_engine::entitlement::{
    AuthenticatedHostScope, Capability, EntryCategory, FreshIntent, OperationId, OperationRecord, OperationStatus, OperationStore,
    OperationStoreError, RetryDecision, RetryMapping, RetryScopeKey, ReviewOperation, RunId, SaveOutcome, TransportCorrelation,
};
