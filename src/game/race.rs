//! Server-authoritative race state.
//!
//! Local play currently coordinates race rules in `ui::session`, because that
//! was the fastest way to build and tune the first playable loop. Multiplayer
//! needs the same rules to be driven by a server instead of a terminal UI. This
//! module is the migration point: pure race state that can eventually power
//! local play, AI simulation, and network-hosted races.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use super::{
    item_effects::RaceItemEffectState,
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct RaceParticipant {
    pub id: RacePlayerId,
    pub name: String,
    pub color: PlayerColorId,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceLifecycleStatus {
    Running,
    Finished {
        all_connected_finished: bool,
        all_connected_disconnected: bool,
        timeout_expired: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceLifecycleUpdate {
    pub newly_finished: Vec<RacePlayerId>,
    pub status: RaceLifecycleStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RaceLifecycleState {
    pub placements: Vec<RacePlayerId>,
    pub first_finished_at: Option<Instant>,
}

impl RaceLifecycleState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.placements.clear();
        self.first_finished_at = None;
    }

    pub fn update(
        &mut self,
        race: &RaceState,
        now: Instant,
        post_first_finish_timeout: Duration,
    ) -> RaceLifecycleUpdate {
        update_race_lifecycle(
            race,
            &mut self.placements,
            &mut self.first_finished_at,
            now,
            post_first_finish_timeout,
        )
    }
}

#[derive(Debug, Clone)]
pub struct RaceRuntimeState<PlayerId, BonusAttempt> {
    pub lifecycle: RaceLifecycleState,
    pub bonus_attempts: HashMap<PlayerId, BonusAttempt>,
    pub spent_bonus_gaps: HashMap<PlayerId, usize>,
    pub player_effects: HashMap<RacePlayerId, RaceItemEffectState>,
}

impl<PlayerId, BonusAttempt> Default for RaceRuntimeState<PlayerId, BonusAttempt> {
    fn default() -> Self {
        Self {
            lifecycle: RaceLifecycleState::new(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
        }
    }
}

impl<PlayerId, BonusAttempt> RaceRuntimeState<PlayerId, BonusAttempt> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.lifecycle.reset();
        self.bonus_attempts.clear();
        self.spent_bonus_gaps.clear();
        self.player_effects.clear();
    }
}

#[allow(dead_code)]
impl RaceState {
    pub fn new(track: Track) -> Self {
        Self {
            track,
            players: Vec::new(),
        }
    }

    pub fn from_participants(
        track: Track,
        participants: impl IntoIterator<Item = RaceParticipant>,
        now: Instant,
    ) -> Self {
        Self {
            track,
            players: participants
                .into_iter()
                .map(|participant| RacePlayer {
                    id: participant.id,
                    name: participant.name,
                    color: participant.color,
                    state: PlayerState::new(now),
                    connected: participant.connected,
                })
                .collect(),
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

    pub fn player_mut(&mut self, id: RacePlayerId) -> Option<&mut RacePlayer> {
        self.players.iter_mut().find(|player| player.id == id)
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

pub fn update_race_lifecycle(
    race: &RaceState,
    placements: &mut Vec<RacePlayerId>,
    first_finished_at: &mut Option<Instant>,
    now: Instant,
    post_first_finish_timeout: Duration,
) -> RaceLifecycleUpdate {
    let mut newly_finished = Vec::new();
    for id in race
        .players
        .iter()
        .filter(|player| player.connected && player.state.is_finished())
        .map(|player| player.id)
    {
        if !placements.contains(&id) {
            placements.push(id);
            newly_finished.push(id);
        }
    }

    if first_finished_at.is_none() && !placements.is_empty() {
        *first_finished_at = Some(now);
    }

    let connected_racers = race
        .players
        .iter()
        .filter(|player| player.connected)
        .count();
    let connected_finished = race
        .players
        .iter()
        .filter(|player| player.connected && player.state.is_finished())
        .count();
    let all_connected_finished = connected_racers > 0 && connected_finished == connected_racers;
    let all_connected_disconnected = connected_racers == 0;
    let timeout_expired = first_finished_at.is_some_and(|first_finished_at| {
        now.duration_since(first_finished_at) >= post_first_finish_timeout
    });

    let status = if all_connected_finished || all_connected_disconnected || timeout_expired {
        append_unfinished_connected_placements(race, placements);
        RaceLifecycleStatus::Finished {
            all_connected_finished,
            all_connected_disconnected,
            timeout_expired,
        }
    } else {
        RaceLifecycleStatus::Running
    };

    RaceLifecycleUpdate {
        newly_finished,
        status,
    }
}

pub fn append_unfinished_connected_placements(
    race: &RaceState,
    placements: &mut Vec<RacePlayerId>,
) {
    let mut remaining = race
        .players
        .iter()
        .filter(|player| player.connected)
        .map(|player| {
            (
                player.id,
                player.state.word_index,
                player.state.input.chars().count(),
            )
        })
        .filter(|(id, _, _)| !placements.contains(id))
        .collect::<Vec<_>>();

    remaining.sort_by_key(|(_, word_index, input_len)| {
        (
            std::cmp::Reverse(*word_index),
            std::cmp::Reverse(*input_len),
        )
    });

    placements.extend(remaining.into_iter().map(|(id, _, _)| id));
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
mod tests;
