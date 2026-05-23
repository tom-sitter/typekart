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
    typing::{KeyAction, TypingEvent, apply_key},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceResultStatus {
    Finished,
    TimedOut,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceResultRow {
    pub placement: usize,
    pub player_id: RacePlayerId,
    pub name: String,
    pub color: PlayerColorId,
    pub status: RaceResultStatus,
    pub progress_words: usize,
    pub track_words: usize,
    pub wpm: u32,
    pub accuracy_percent: u32,
    pub typo_chars: usize,
    pub backspaces: usize,
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

pub fn build_race_result_rows(
    race: &RaceState,
    placements: &[RacePlayerId],
    now: Instant,
) -> Vec<RaceResultRow> {
    let mut ordered_ids = placements.to_vec();
    let mut remaining = race
        .players
        .iter()
        .map(|player| {
            (
                player.id,
                player.connected,
                player.state.word_index,
                player.state.input.chars().count(),
            )
        })
        .filter(|(id, _, _, _)| !ordered_ids.contains(id))
        .collect::<Vec<_>>();

    remaining.sort_by_key(|(_, connected, word_index, input_len)| {
        (
            // Active racers should appear before disconnected racers if a host
            // needs to synthesize rows before placement completion.
            !*connected,
            std::cmp::Reverse(*word_index),
            std::cmp::Reverse(*input_len),
        )
    });
    ordered_ids.extend(remaining.into_iter().map(|(id, _, _, _)| id));

    let track_words = race.track.len();
    ordered_ids
        .into_iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let player = race.players.iter().find(|player| player.id == id)?;
            let finished = player.state.is_finished();
            let status = if finished {
                RaceResultStatus::Finished
            } else if player.connected {
                RaceResultStatus::TimedOut
            } else {
                RaceResultStatus::Disconnected
            };
            let stats_until = player.state.finished_at.unwrap_or(now);
            let wpm = player
                .state
                .stats
                .words_per_minute(player.state.started_at, stats_until)
                .round()
                .clamp(0.0, u32::MAX as f64) as u32;
            let accuracy_percent = player.state.stats.accuracy().round().clamp(0.0, 100.0) as u32;
            let progress_words = if finished {
                track_words
            } else {
                player.state.word_index.min(track_words)
            };

            Some(RaceResultRow {
                placement: index + 1,
                player_id: id,
                name: player.name.clone(),
                color: player.color,
                status,
                progress_words,
                track_words,
                wpm,
                accuracy_percent,
                typo_chars: player.state.stats.typo_chars,
                backspaces: player.state.stats.backspaces,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{PlayerColorId, RacePlayerId, RaceResultStatus, RaceState};
    use crate::game::{stats::TypingStats, track::Track, typing::KeyAction};

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

    #[test]
    fn race_result_rows_order_finished_then_progress_and_include_stats() {
        let now = Instant::now();
        let mut race = RaceState::new(track(&["one", "two"]));
        race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
        race.add_player(RacePlayerId(2), "guest", PlayerColorId::Red, now);

        let guest = race
            .players
            .iter_mut()
            .find(|player| player.id == RacePlayerId(2))
            .unwrap();
        guest.state.finished_at = Some(now);
        guest.state.stats.completed_words = 2;

        let host = race
            .players
            .iter_mut()
            .find(|player| player.id == RacePlayerId(1))
            .unwrap();
        host.connected = false;
        host.state.word_index = 1;
        host.state.stats = TypingStats {
            typed_chars: 10,
            correct_chars: 8,
            typo_chars: 2,
            backspaces: 3,
            completed_words: 1,
        };

        let rows = super::build_race_result_rows(&race, &[RacePlayerId(2)], now);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].player_id, RacePlayerId(2));
        assert_eq!(rows[0].status, RaceResultStatus::Finished);
        assert_eq!(rows[0].progress_words, 2);
        assert_eq!(rows[1].player_id, RacePlayerId(1));
        assert_eq!(rows[1].status, RaceResultStatus::Disconnected);
        assert_eq!(rows[1].progress_words, 1);
        assert_eq!(rows[1].accuracy_percent, 80);
        assert_eq!(rows[1].typo_chars, 2);
        assert_eq!(rows[1].backspaces, 3);
    }
}
