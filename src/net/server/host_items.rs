//! Network-host item adapter.
//!
//! Item targeting and effect rules live in `game::item_effects`. This module
//! handles the network host's side effects around those shared item reports.

use std::{collections::HashSet, time::Instant};

use crate::game::{
    host_session::{HostItemPickupInput, HostItemPickupState, apply_host_item_pickup},
    item_effects::advance_mushrooms,
    items::ItemPickup,
    race::RacePlayerId,
};
use crate::net::{log::push_network_log, protocol::PlayerId};

use super::{HostState, push_event};

pub(super) fn activate_network_pickup(
    state: &mut HostState,
    player_id: PlayerId,
    item: ItemPickup,
    now: Instant,
) {
    let ai_players = state
        .ai_racers
        .keys()
        .map(|player_id| RacePlayerId(player_id.0))
        .collect::<HashSet<_>>();
    let report = apply_host_item_pickup(
        &mut HostItemPickupState {
            race: &mut state.race,
            effects: &mut state.runtime.player_effects,
            ai_players: &ai_players,
            item_registry: &state.item_registry,
        },
        HostItemPickupInput {
            player_id: RacePlayerId(player_id.0),
            item,
            now,
        },
    );

    for interrupted in report.interrupted_players {
        state
            .runtime
            .bonus_attempts
            .remove(&PlayerId(interrupted.0));
    }
    for ai_id in report.reset_ai_players {
        if let Some(ai) = state.ai_racers.get_mut(&PlayerId(ai_id.0)) {
            ai.char_budget = 0.0;
        }
    }
    for event in report.events {
        push_network_log(&state.debug_log, event.clone());
        push_event(state, event);
    }
}

pub(super) fn advance_network_mushrooms(state: &mut HostState, now: Instant) {
    for interrupted in advance_mushrooms(&mut state.race, now) {
        state
            .runtime
            .bonus_attempts
            .remove(&PlayerId(interrupted.0));
    }
}
