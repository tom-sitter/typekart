//! Network-host AI racer adapter.
//!
//! Core AI typing behavior lives in `game::ai_driver` and lobby policy lives
//! in `game::lobby`. This module owns the network host's bookkeeping around
//! those shared rules: bot roster entries, sampled WPM, and host event output.

use std::{collections::HashMap, time::Instant};

use anyhow::Result;
use rand::{Rng, thread_rng};

use crate::game::{
    ai::AiDifficulty,
    ai_driver::{AiDriverConfig, AiDriverState},
    host_session::{HostAiTickInput, advance_host_ai_racer_tick},
    lobby::{
        add_ai_lobby_player as shared_add_ai_lobby_player, color_for_lobby_slot,
        set_lobby_ai_difficulty as shared_set_lobby_ai_difficulty,
    },
    race::{RacePlayerId, RaceState},
};
use crate::net::{
    log::push_network_log,
    protocol::{LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind},
};

use super::{HostState, host_bonus, host_input::player_label, push_event};

#[derive(Debug, Clone)]
pub(super) struct NetworkAiRacer {
    pub(super) difficulty: AiDifficulty,
    pub(super) words_per_minute: f64,
    pub(super) char_budget: f64,
    pub(super) last_update: Instant,
}

pub(super) fn add_network_ai_racers(
    race: &mut RaceState,
    players: &mut Vec<LobbyPlayer>,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    now: Instant,
) -> HashMap<PlayerId, NetworkAiRacer> {
    let mut ai_racers = HashMap::new();
    let mut rng = thread_rng();
    for index in 0..ai_racer_count {
        let player_id = PlayerId(index as u64 + 2);
        let name = format!("ai-{}", index + 1);
        let color = color_for_lobby_slot(index + 1);
        race.add_player(RacePlayerId(player_id.0), name.clone(), color.into(), now);
        ai_racers.insert(
            player_id,
            NetworkAiRacer {
                words_per_minute: rng.gen_range(ai_difficulty.wpm_range()),
                difficulty: ai_difficulty,
                char_budget: 0.0,
                last_update: now,
            },
        );
        players.push(LobbyPlayer {
            id: player_id,
            name,
            kind: PlayerKind::Bot,
            color,
            ready: true,
            connected: true,
            ai_difficulty: Some(ai_difficulty.into()),
            ai_wpm: ai_racers
                .get(&player_id)
                .map(|racer| racer.words_per_minute.round() as u32),
        });
    }
    ai_racers
}

pub(super) fn add_lobby_ai_racer(state: &mut HostState) -> Result<()> {
    let now = Instant::now();
    let mut rng = thread_rng();
    let words_per_minute = rng.gen_range(state.ai_difficulty.wpm_range());
    let added = shared_add_ai_lobby_player(
        &mut state.players,
        state.phase,
        state.max_players,
        state.ai_difficulty.into(),
        words_per_minute.round() as u32,
    )?;
    let player = added.player;

    state.race.add_player(
        RacePlayerId(player.id.0),
        player.name.clone(),
        player.color.into(),
        now,
    );
    state.ai_racers.insert(
        player.id,
        NetworkAiRacer {
            difficulty: state.ai_difficulty,
            words_per_minute,
            char_budget: 0.0,
            last_update: now,
        },
    );
    push_event(state, format!("{} added", player.name));
    push_network_log(
        &state.debug_log,
        format!(
            "ai added player={} difficulty={} wpm={:.0}",
            player.id.0,
            state.ai_difficulty.name(),
            words_per_minute
        ),
    );

    Ok(())
}

pub(super) fn set_lobby_ai_difficulty(
    state: &mut HostState,
    player_id: Option<PlayerId>,
    difficulty: AiDifficulty,
) -> Result<()> {
    let mut rng = thread_rng();
    let words_per_minute = rng.gen_range(difficulty.wpm_range());
    let outcome = shared_set_lobby_ai_difficulty(
        &mut state.players,
        state.phase,
        player_id,
        difficulty.into(),
        words_per_minute.round() as u32,
    )?;

    match outcome {
        crate::game::lobby::LobbyAiDifficultyOutcome::DefaultChanged { .. } => {
            state.ai_difficulty = difficulty;
            push_event(
                state,
                format!("New AI difficulty set to {}", difficulty.name()),
            );
            push_network_log(
                &state.debug_log,
                format!("default ai difficulty={}", difficulty.name()),
            );
        }
        crate::game::lobby::LobbyAiDifficultyOutcome::PlayerChanged {
            player_id, name, ..
        } => {
            if let Some(ai) = state.ai_racers.get_mut(&player_id) {
                ai.difficulty = difficulty;
                ai.words_per_minute = words_per_minute;
                ai.char_budget = 0.0;
                ai.last_update = Instant::now();
            }
            push_event(state, format!("{name} set to {}", difficulty.name()));
            push_network_log(
                &state.debug_log,
                format!(
                    "ai difficulty player={} difficulty={} wpm={:.0}",
                    player_id.0,
                    difficulty.name(),
                    words_per_minute
                ),
            );
        }
    }

    Ok(())
}

pub(super) fn advance_network_ai_racers(state: &mut HostState, now: Instant) {
    if state.phase != NetworkRacePhase::Racing {
        reset_network_ai_timing(state, now);
        return;
    }

    let player_ids = state.ai_racers.keys().copied().collect::<Vec<_>>();
    for player_id in player_ids {
        host_bonus::network_ai_try_claim_bonus(state, player_id, now);
        advance_network_ai_typing(state, player_id, now);
    }
}

pub(super) fn reset_network_ai_timing(state: &mut HostState, now: Instant) {
    for ai in state.ai_racers.values_mut() {
        ai.char_budget = 0.0;
        ai.last_update = now;
    }
}

fn advance_network_ai_typing(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let race_player_id = RacePlayerId(player_id.0);
    let Some(ai) = state.ai_racers.get(&player_id) else {
        return;
    };
    let words_per_minute = ai.words_per_minute;
    let char_budget = ai.char_budget;
    let last_update = ai.last_update;
    let elapsed = now.saturating_duration_since(ai.last_update);
    let mut driver = AiDriverState {
        char_budget,
        last_update: Some(last_update),
    };
    if let Some(ai) = state.ai_racers.get_mut(&player_id) {
        ai.last_update = now;
    }

    let config = AiDriverConfig {
        base_wpm: words_per_minute,
        focus_boost_wpm: state.item_registry.focus_effect().ai_wpm_boost,
        ink_multiplier_percent: state
            .item_registry
            .squid_ink_effect()
            .ai_wpm_multiplier_percent,
    };
    let advance = advance_host_ai_racer_tick(
        &mut state.race,
        &state.runtime.player_effects,
        &mut driver,
        HostAiTickInput {
            player_id: race_player_id,
            config,
            now,
            elapsed,
        },
    );

    if let Some(ai) = state.ai_racers.get_mut(&player_id) {
        ai.char_budget = driver.char_budget;
    }

    if advance.finished() {
        push_event(
            state,
            format!("{} finished", player_label(state, player_id)),
        );
    }
}
