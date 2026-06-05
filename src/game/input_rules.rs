//! Shared input gating rules.
//!
//! These helpers answer whether a racer can currently accept manual or
//! AI-generated typing input. They do not apply input or mutate state.

use std::{collections::HashMap, time::Instant};

use super::{
    item_effects::{RaceItemEffectState, player_has_active_mushroom_effect, player_is_stunned},
    race::{RacePlayerId, RaceState},
};

pub fn player_input_is_paused(
    race: &RaceState,
    player_effects: &HashMap<RacePlayerId, RaceItemEffectState>,
    player_id: RacePlayerId,
    now: Instant,
) -> bool {
    player_is_stunned(player_effects, player_id, now)
        || race
            .player(player_id)
            .is_some_and(|player| player_has_active_mushroom_effect(player, now))
}

pub fn player_input_is_paused_or_finished(
    race: &RaceState,
    player_effects: &HashMap<RacePlayerId, RaceItemEffectState>,
    player_id: RacePlayerId,
    now: Instant,
) -> bool {
    race.player(player_id)
        .is_none_or(|player| player.state.is_finished())
        || player_input_is_paused(race, player_effects, player_id, now)
}

#[cfg(test)]
mod tests;
