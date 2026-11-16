//! The Telescreen app: the controller and the wiring of core, transport, platform,
//! view, and renderer. `main.rs` is the binary.

pub mod controller;
#[cfg(feature = "debug-protocol")]
pub mod debug;
pub mod discovery;
pub mod paths;
pub mod pointer_mailbox;
#[cfg(target_os = "linux")]
pub mod stage;
pub mod stage_agent;
#[cfg(target_os = "linux")]
pub mod stage_record;
pub mod startup;
pub mod ui;
