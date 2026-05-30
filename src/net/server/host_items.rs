//! Network-host item adapter.
//!
//! Item targeting and effect rules live in `game::item_effects`. This module
//! handles the network host's side effects around those shared item reports.

#[cfg(test)]
use std::collections::HashSet;
use std::time::Instant;

use crate::game::{host_session::HostItemAftermath, item_effects::advance_mushrooms};
#[cfg(test)]
use crate::game::{
    host_session::{
        HostItemPickupInput, HostItemPickupState, apply_host_item_pickup,
        host_item_aftermath_actions,
    },
    items::ItemPickup,
    race::RacePlayerId,
};
use crate::net::{log::push_network_log, protocol::PlayerId};

use super::{HostState, push_event};

#[cfg(test)]
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

    let aftermath = host_item_aftermath_actions(report);
    apply_network_item_aftermath(state, aftermath);
}

pub(super) fn apply_network_item_aftermath(state: &mut HostState, aftermath: HostItemAftermath) {
    for interrupted in aftermath.interrupted_players {
        state
            .runtime
            .bonus_attempts
            .remove(&PlayerId(interrupted.0));
    }
    for ai_id in aftermath.reset_ai_players {
        if let Some(ai) = state.ai_racers.get_mut(&PlayerId(ai_id.0)) {
            ai.char_budget = 0.0;
        }
    }
    for event in aftermath.events {
        let message = event.message();
        push_network_log(&state.debug_log, message.clone());
        push_event(state, message);
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
