//! Reusable TypeKart library surface.
//!
//! The command-line binary is still the primary application, but the browser UI
//! needs the game rules and protocol to compile without going through `main`.
//! Keeping these modules behind a library boundary is the first step toward a
//! shared Rust/WASM core.

pub mod game;

#[cfg(feature = "cli")]
pub mod app;
#[cfg(feature = "cli")]
pub mod net;
#[cfg(feature = "cli")]
pub mod ui;
