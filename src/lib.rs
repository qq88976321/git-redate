//! git-redate: interactively edit git commit dates.
//!
//! The crate is split into a thin binary (`src/main.rs`) over this
//! library so the pure logic (time math, edit operations, range
//! parsing) can be unit-tested without a terminal or a real repository.
//! Modules are declared here as they are implemented.

pub mod cli;
pub mod config;
pub mod datetime;
pub mod error;
