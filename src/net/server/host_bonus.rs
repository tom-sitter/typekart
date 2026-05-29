//! Network-host bonus adapter.
//!
//! The shared bonus state machine lives in `game::bonus_flow`. This module
//! handles the network host's feed/log text and wires bonus claims into
//! network-host item activation.

use std::time::Instant;

use rand::thread_rng;

use crate::game::{
    bonus_flow::{BonusClaimRoll, BonusFlowEvent, BonusFlowState, claim_random_available_bonus},
    host_session::{HostPlayerKeyInput, HostPlayerKeyState, apply_host_player_key},
    item_effects::player_has_active_mushroom_effect,
    items::{ItemPickup, ItemRollContext, RacePositionBand},
    race::RacePlayerId,
    typing::KeyAction,
};
use crate::net::{log::push_network_log, protocol::PlayerId};

use super::{
    HostState,
    host_input::{player_input_is_paused, player_label, player_name},
    host_items, push_event,
};

pub(super) fn apply_network_key_input(
    state: &mut HostState,
    player_id: PlayerId,
    action: KeyAction,
    now: Instant,
) -> bool {
    if player_input_is_paused(state, player_id, now) {
        return true;
    }

    let item_context = network_item_roll_context(state, player_id, 5);
    let item_registry = state.item_registry.clone();
    let mut rng = thread_rng();
    let outcome = apply_host_player_key(
        &mut HostPlayerKeyState {
            race: &mut state.race,
            bonuses: &mut state.bonuses,
            bonus_attempts: &mut state.runtime.bonus_attempts,
            spent_bonus_gaps: &mut state.runtime.spent_bonus_gaps,
        },
        HostPlayerKeyInput {
            player_key: player_id,
            race_player_id: RacePlayerId(player_id.0),
            action,
            now,
        },
        BonusClaimRoll {
            item_context,
            item_registry: &item_registry,
            rng: &mut rng,
        },
    );
    if !outcome.bonus_events.is_empty() {
        handle_network_bonus_events(state, player_id, outcome.bonus_events, now);
    }

    outcome.handled
}

pub(super) fn network_ai_try_claim_bonus(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(player) = state.race.player(RacePlayerId(player_id.0)) else {
        return;
    };
    if player.state.held_item.is_some()
        || player.state.has_active_shield(now)
        || player.state.has_active_focus(now)
        || player_has_active_mushroom_effect(player, now)
        || player_is_stunned(state, player_id, now)
        || state
            .runtime
            .player_effects
            .get(&RacePlayerId(player_id.0))
            .and_then(|effects| effects.item_cue.as_ref())
            .is_some_and(|cue| cue.until > now)
        || player.state.typo_index.is_some()
        || !player.state.input.is_empty()
        || player.state.is_finished()
        || state.runtime.bonus_attempts.contains_key(&player_id)
    {
        return;
    }

    let mut rng = thread_rng();
    let item_context = network_item_roll_context(state, player_id, 5);
    let item_registry = state.item_registry.clone();
    let Some(outcome) = claim_random_available_bonus(
        &mut BonusFlowState {
            race: &mut state.race,
            bonuses: &mut state.bonuses,
            bonus_attempts: &mut state.runtime.bonus_attempts,
            spent_bonus_gaps: &mut state.runtime.spent_bonus_gaps,
        },
        player_id,
        RacePlayerId(player_id.0),
        now,
        BonusClaimRoll {
            item_context,
            item_registry: &item_registry,
            rng: &mut rng,
        },
    ) else {
        return;
    };
    handle_network_bonus_claim_outcome(state, player_id, outcome.pickup, now);
}

fn handle_network_bonus_events(
    state: &mut HostState,
    player_id: PlayerId,
    events: Vec<BonusFlowEvent>,
    now: Instant,
) {
    for event in events {
        match event {
            BonusFlowEvent::TypoStarted => {
                push_network_log(
                    &state.debug_log,
                    format!("{} bonus typo started", player_label(state, player_id)),
                );
            }
            BonusFlowEvent::TypoCleared => {
                push_network_log(
                    &state.debug_log,
                    format!("{} bonus typo cleared", player_label(state, player_id)),
                );
            }
            BonusFlowEvent::AttemptCancelled => {
                push_network_log(
                    &state.debug_log,
                    format!("{} bonus attempt cancelled", player_label(state, player_id)),
                );
            }
            BonusFlowEvent::ClaimResolved(outcome) => {
                handle_network_bonus_claim_outcome(state, player_id, outcome.pickup, now);
            }
            BonusFlowEvent::AttemptInvalidated => {
                push_network_log(
                    &state.debug_log,
                    format!(
                        "{} bonus attempt invalidated",
                        player_label(state, player_id)
                    ),
                );
            }
            BonusFlowEvent::AttemptStarted(_) | BonusFlowEvent::InputChanged => {}
        }
    }
}

fn handle_network_bonus_claim_outcome(
    state: &mut HostState,
    player_id: PlayerId,
    pickup: Option<ItemPickup>,
    now: Instant,
) {
    let name = player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0));
    match pickup {
        Some(item) => {
            let item_name = item_pickup_name(item);
            push_event(state, format!("{name} got {item_name}"));
            push_network_log(
                &state.debug_log,
                format!("{name} picked up {item_name} from network bonus"),
            );
            host_items::activate_network_pickup(state, player_id, item, now);
        }
        None => {
            push_event(state, format!("{name} missed the bonus"));
            push_network_log(
                &state.debug_log,
                format!("{name} missed network bonus; choice was unavailable"),
            );
        }
    }
}

fn player_has_nearby_racer(
    state: &HostState,
    player_id: PlayerId,
    max_distance_words: usize,
) -> bool {
    let Some(player) = state.race.player(RacePlayerId(player_id.0)) else {
        return false;
    };

    state.race.players.iter().any(|other| {
        other.id != player.id
            && other.connected
            && !other.state.is_finished()
            && player.state.word_index.abs_diff(other.state.word_index) <= max_distance_words
    })
}

fn network_item_roll_context(
    state: &HostState,
    player_id: PlayerId,
    max_distance_words: usize,
) -> ItemRollContext {
    ItemRollContext {
        has_nearby_racer: player_has_nearby_racer(state, player_id, max_distance_words),
        position: network_position_band(state, player_id),
    }
}

fn network_position_band(state: &HostState, player_id: PlayerId) -> RacePositionBand {
    let active_racers = state
        .race
        .players
        .iter()
        .filter(|player| player.connected && !player.state.is_finished())
        .collect::<Vec<_>>();
    if active_racers.len() <= 1 {
        return RacePositionBand::Middle;
    }

    let Some(player) = active_racers
        .iter()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return RacePositionBand::Middle;
    };

    let ahead = active_racers
        .iter()
        .filter(|other| other.state.word_index > player.state.word_index)
        .count();
    let behind = active_racers
        .iter()
        .filter(|other| other.state.word_index < player.state.word_index)
        .count();

    if ahead == 0 && behind > 0 {
        RacePositionBand::First
    } else if behind == 0 && ahead > 0 {
        RacePositionBand::Trailing
    } else {
        RacePositionBand::Middle
    }
}

fn player_is_stunned(state: &HostState, player_id: PlayerId, now: Instant) -> bool {
    crate::game::item_effects::player_is_stunned(
        &state.runtime.player_effects,
        RacePlayerId(player_id.0),
        now,
    )
}

fn item_pickup_name(item: ItemPickup) -> &'static str {
    match item {
        ItemPickup::Held(held_item) => held_item.name(),
        ItemPickup::Shield => "Shield",
    }
}
