//! The egui renderer of the Telescreen view models (`docs/plan.md`, "View boundary").
//!
//! This is the only crate that uses egui. It draws a view model on a winit window
//! and returns the intents of the user. It does not place, size, or raise windows:
//! the app and the platform crates do that.

mod screens;
mod window;

pub use window::{RenderError, WindowRenderer};
