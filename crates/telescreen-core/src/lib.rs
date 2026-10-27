//! Platform-independent core of Telescreen.
//!
//! This crate owns the config, the domain model, the session rules, and the wire
//! messages. It has no I/O threads, sockets, windows, or OS input APIs. The
//! platform crates implement the traits in [`platform`].

pub mod clipboard;
pub mod config;
pub mod driver;
pub mod ids;
pub mod input;
pub mod key_route;
pub mod keys;
pub mod platform;
pub mod pointer;
pub mod routes;
pub mod session;
pub mod wire;
