//! Browser-safe host game loop facade.
//!
//! This module is intentionally small. It wraps the existing pure `RaceState`
//! with a command/tick API that does not depend on the terminal UI, native TCP,
//! or relay code. Future browser-hosted games can grow this facade until it owns
//! the full authoritative race loop.

use std::time::Instant;

use super::{
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::Track,
    typing::{KeyAction, TypingEvent},
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
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{CoreHost, CoreHostCommand, CoreHostEvent};
    use crate::game::{
        race::{PlayerColorId, RacePlayerId},
        track::Track,
        typing::KeyAction,
    };

    fn track(words: &[&str]) -> Track {
        Track::new(words.iter().map(|word| word.to_string()).collect())
    }

    #[test]
    fn core_host_can_drive_race_without_terminal_or_network() {
        let now = Instant::now();
        let player_id = RacePlayerId(1);
        let mut host = CoreHost::new(track(&["go"]));

        let events = host.tick(
            now,
            [
                CoreHostCommand::AddPlayer {
                    id: player_id,
                    name: "host".to_string(),
                    color: PlayerColorId::Cyan,
                },
                CoreHostCommand::KeyInput {
                    id: player_id,
                    action: KeyAction::Char('g'),
                },
                CoreHostCommand::KeyInput {
                    id: player_id,
                    action: KeyAction::Char('o'),
                },
            ],
        );

        assert_eq!(
            events,
            [
                CoreHostEvent::PlayerAdded { id: player_id },
                CoreHostEvent::InputChanged { id: player_id },
                CoreHostEvent::InputChanged { id: player_id },
                CoreHostEvent::WordCompleted { id: player_id },
                CoreHostEvent::RaceFinished { id: player_id },
            ]
        );
        assert!(host.race().player(player_id).unwrap().state.is_finished());
    }

    #[test]
    fn core_host_reports_missing_players() {
        let now = Instant::now();
        let missing = RacePlayerId(99);
        let mut host = CoreHost::new(track(&["go"]));

        let events = host.tick(
            now,
            [CoreHostCommand::KeyInput {
                id: missing,
                action: KeyAction::Char('g'),
            }],
        );

        assert_eq!(events, [CoreHostEvent::PlayerMissing { id: missing }]);
    }
}
