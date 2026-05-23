//! Pure game state and rules.
//!
//! Code in this module should not depend on terminal rendering or networking.
//! Keeping it pure makes the core behavior easy to unit test.

pub mod ai;
pub mod bonus;
pub mod effects;
pub mod engine;
pub mod item_effects;
pub mod items;
pub mod mods;
pub mod player;
pub mod race;
pub mod stats;
pub mod track;
pub mod typing;
pub mod words;
