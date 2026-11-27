//! The parts of `intcad-cli` that tests exercise directly.
//!
//! The binary keeps the process-level command dispatch. These modules expose the same
//! composition to tests with inert product transports, without a test hook in the binary.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod account;
pub mod validation;
