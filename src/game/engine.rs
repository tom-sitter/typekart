//! Browser-safe host game loop facade.
//!
//! This module is intentionally small. It wraps the existing pure `RaceState`
//! with a command/tick API that does not depend on the terminal UI, native TCP,
//! or relay code. Future browser-hosted games can grow this facade until it owns
//! the full authoritative race loop.

use std::time::Instant;

use super::{
    ai_driver::{AiDriverConfig, AiDriverState, advance_ai_driver},
    race::{PlayerColorId, RacePlayerId, RaceState},
    race_flow::update_race_flow,
    track::Track,
    typing::{KeyAction, TypingEvent},
};

pub use super::{
    ai_driver::{AiDriverAdvance, ai_effective_wpm, next_ai_key},
    race_flow::{race_flow_is_finished, reset_race_runtime},
};

#[derive(Debug, Clone)]
pub struct CoreHost {
    race: RaceState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreHostCommand {
    AddPlayer {
        id: RacePlayerId,
        name: String,
        color: PlayerColorId,
    },
    KeyInput {
        id: RacePlayerId,
        action: KeyAction,
    },
    SetConnected {
        id: RacePlayerId,
        connected: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreHostEvent {
    PlayerAdded { id: RacePlayerId },
    PlayerConnectionChanged { id: RacePlayerId, connected: bool },
    PlayerMissing { id: RacePlayerId },
    InputChanged { id: RacePlayerId },
    WordCompleted { id: RacePlayerId },
    RaceFinished { id: RacePlayerId },
    TypoStarted { id: RacePlayerId, index: usize },
    TypoCleared { id: RacePlayerId },
}

impl CoreHost {
    pub fn new(track: Track) -> Self {
        Self {
            race: RaceState::new(track),
        }
    }

    pub fn race(&self) -> &RaceState {
        &self.race
    }

    pub fn race_mut(&mut self) -> &mut RaceState {
        &mut self.race
    }

    pub fn tick(
        &mut self,
        now: Instant,
        commands: impl IntoIterator<Item = CoreHostCommand>,
    ) -> Vec<CoreHostEvent> {
        let mut events = Vec::new();

        for command in commands {
            match command {
                CoreHostCommand::AddPlayer { id, name, color } => {
                    self.race.add_player(id, name, color, now);
                    events.push(CoreHostEvent::PlayerAdded { id });
                }
                CoreHostCommand::KeyInput { id, action } => {
                    let Some(typing_events) = self.race.apply_key_input(id, action, now) else {
                        events.push(CoreHostEvent::PlayerMissing { id });
                        continue;
                    };
                    events.extend(typing_events.into_iter().map(|event| match event {
                        TypingEvent::InputChanged => CoreHostEvent::InputChanged { id },
                        TypingEvent::WordCompleted => CoreHostEvent::WordCompleted { id },
                        TypingEvent::RaceFinished => CoreHostEvent::RaceFinished { id },
                        TypingEvent::TypoStarted { index } => {
                            CoreHostEvent::TypoStarted { id, index }
                        }
                        TypingEvent::TypoCleared => CoreHostEvent::TypoCleared { id },
                    }));
                }
                CoreHostCommand::SetConnected { id, connected } => {
                    let Some(player) = self.race.players.iter_mut().find(|player| player.id == id)
                    else {
                        events.push(CoreHostEvent::PlayerMissing { id });
                        continue;
                    };
                    player.connected = connected;
                    events.push(CoreHostEvent::PlayerConnectionChanged { id, connected });
                }
            }
        }

        events
    }

    pub fn advance_ai(
        &mut self,
        player_id: RacePlayerId,
        driver: &mut AiDriverState,
        config: AiDriverConfig,
        now: Instant,
        elapsed: std::time::Duration,
    ) -> super::ai_driver::AiDriverAdvance {
        advance_ai_driver(&mut self.race, player_id, driver, config, now, elapsed)
    }

    pub fn update_lifecycle(
        &self,
        lifecycle: &mut super::race::RaceLifecycleState,
        now: Instant,
        post_first_finish_timeout: std::time::Duration,
    ) -> super::race::RaceLifecycleUpdate {
        update_race_flow(lifecycle, &self.race, now, post_first_finish_timeout)
    }
}

#[cfg(test)]
mod tests;
