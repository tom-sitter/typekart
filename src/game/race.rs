//! Server-authoritative race state.
//!
//! Local play currently coordinates race rules in `ui::session`, because that
//! was the fastest way to build and tune the first playable loop. Multiplayer
//! needs the same rules to be driven by a server instead of a terminal UI. This
//! module is the migration point: pure race state that can eventually power
//! local play, AI simulation, and network-hosted races.

use std::time::Instant;

use super::{
    player::PlayerState,
    track::Track,
    typing::{apply_key, KeyAction, TypingEvent},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct RacePlayerId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PlayerColorId {
    Cyan,
    Red,
    Green,
    Blue,
    Yellow,
    Magenta,
}

#[allow(dead_code)]
pub const PLAYER_COLOR_ROTATION: [PlayerColorId; 6] = [
    PlayerColorId::Cyan,
    PlayerColorId::Red,
    PlayerColorId::Green,
    PlayerColorId::Blue,
    PlayerColorId::Yellow,
    PlayerColorId::Magenta,
];

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RacePlayer {
    pub id: RacePlayerId,
    pub name: String,
    pub color: PlayerColorId,
    pub state: PlayerState,
    pub connected: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RaceState {
    pub track: Track,
    pub players: Vec<RacePlayer>,
}

#[allow(dead_code)]
impl RaceState {
    pub fn new(track: Track) -> Self {
        Self {
            track,
            players: Vec::new(),
        }
    }

    pub fn add_player(
        &mut self,
        id: RacePlayerId,
        name: impl Into<String>,
        color: PlayerColorId,
        now: Instant,
    ) {
        self.players.push(RacePlayer {
            id,
            name: name.into(),
            color,
            state: PlayerState::new(now),
            connected: true,
        });
    }

    pub fn player(&self, id: RacePlayerId) -> Option<&RacePlayer> {
        self.players.iter().find(|player| player.id == id)
    }

    pub fn apply_key_input(
        &mut self,
        id: RacePlayerId,
        action: KeyAction,
        now: Instant,
    ) -> Option<Vec<TypingEvent>> {
        let player_index = self.players.iter().position(|player| player.id == id)?;
        let track = &self.track;
        let player = &mut self.players[player_index];

        Some(apply_key(&mut player.state, track, action, now))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{PlayerColorId, RacePlayerId, RaceState};
    use crate::game::{track::Track, typing::KeyAction};

    fn track(words: &[&str]) -> Track {
        Track::new(words.iter().map(|word| word.to_string()).collect())
    }

    #[test]
    fn race_state_adds_players() {
        let now = Instant::now();
        let mut race = RaceState::new(track(&["one", "two"]));

        race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);

        let player = race.player(RacePlayerId(1)).unwrap();
        assert_eq!(player.name, "tom");
        assert_eq!(player.color, PlayerColorId::Cyan);
        assert!(player.connected);
        assert_eq!(player.state.word_index, 0);
    }

    #[test]
    fn race_state_applies_key_input_to_selected_player() {
        let now = Instant::now();
        let mut race = RaceState::new(track(&["one", "two"]));
        race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);
        race.add_player(RacePlayerId(2), "alex", PlayerColorId::Red, now);

        race.apply_key_input(RacePlayerId(2), KeyAction::Char('o'), now)
            .unwrap();

        assert_eq!(race.player(RacePlayerId(1)).unwrap().state.input, "");
        assert_eq!(race.player(RacePlayerId(2)).unwrap().state.input, "o");
    }

    #[test]
    fn race_state_returns_none_for_unknown_player_input() {
        let now = Instant::now();
        let mut race = RaceState::new(track(&["one", "two"]));

        let events = race.apply_key_input(RacePlayerId(99), KeyAction::Char('o'), now);

        assert_eq!(events, None);
    }

    #[test]
    fn race_state_uses_existing_final_word_finish_rule() {
        let now = Instant::now();
        let mut race = RaceState::new(track(&["a"]));
        race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);

        race.apply_key_input(RacePlayerId(1), KeyAction::Char('a'), now)
            .unwrap();

        let player = race.player(RacePlayerId(1)).unwrap();
        assert!(player.state.is_finished());
        assert_eq!(player.state.stats.completed_words, 1);
    }
}
