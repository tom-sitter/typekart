//! Pure game state and rules.
//!
//! Code in this module should not depend on terminal rendering or networking.
//! Keeping it pure makes the core behavior easy to unit test.

pub mod ai;
pub mod ai_driver;
pub mod bonus;
pub mod bonus_flow;
pub mod effects;
pub mod engine;
pub mod host_events;
pub mod host_session;
pub mod input_rules;
pub mod item_effects;
pub mod items;
pub mod lobby;
pub mod mods;
pub mod player;
pub mod race;
pub mod race_flow;
pub mod snapshot;
pub mod stats;
pub mod tick;
pub mod track;
pub mod typing;
pub mod words;
