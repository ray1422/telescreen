//! The Telescreen app: the controller and the wiring of core, transport, platform,
//! view, and renderer. `main.rs` is the binary.

pub mod controller;
#[cfg(feature = "debug-protocol")]
pub mod debug;
pub mod paths;
pub mod ui;
