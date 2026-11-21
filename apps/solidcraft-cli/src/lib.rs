//! The parts of `intcad-cli` that tests exercise directly.
//!
//! The binary keeps the process-level command dispatch; validation is a library module so its
//! billing seam can be tested with inert fixtures instead of a private test hook in the shipped
//! binary.

pub mod validation;
