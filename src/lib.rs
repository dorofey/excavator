//! Shared file-manager core. This library has no GPUI dependency.
pub mod connections;
pub mod credentials;
pub mod domain;
pub mod persistence;
pub mod platform;
pub mod providers;
pub mod transfers;

#[cfg(feature = "tui")]
pub mod tui;
