//! Network protocol and transport code.
//!
//! Milestone 4 starts with serializable protocol types and a minimal TCP
//! handshake. The race loop will be layered on after the host/join path is
//! proven.

pub mod client;
pub mod log;
pub mod protocol;
pub mod server;
