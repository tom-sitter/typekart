//! Terminal user interface.
//!
//! UI code translates between terminal events and game actions, then renders
//! game state. It should not own core game rules.

pub mod render;
pub mod session;
pub mod terminal;
