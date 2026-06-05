use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use super::{RaceImpactCueKind, activate_item_pickup};
use crate::game::{
    items::{HeldItem, ItemPickup, ItemRegistry},
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::Track,
};

fn race(words: &[&str]) -> RaceState {
    let now = Instant::now();
    let mut race = RaceState::new(Track::new(
        words.iter().map(|word| word.to_string()).collect(),
    ));
    race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
    race.add_player(RacePlayerId(2), "guest", PlayerColorId::Red, now);
    race
}

#[test]
fn banana_resets_human_target_without_stun() {
    let now = Instant::now();
    let mut race = race(&["one", "two"]);
    race.players[1].state.input = "twx".to_string();
    race.players[1].state.typo_index = Some(2);
    let mut effects = HashMap::new();

    let report = activate_item_pickup(
        &mut race,
        &mut effects,
        &HashSet::new(),
        &ItemRegistry::builtin(),
        RacePlayerId(1),
        ItemPickup::Held(HeldItem::Banana),
        now,
    );

    assert!(
        report
            .events
            .iter()
            .any(|event| event.message() == "host hit guest")
    );
    assert_eq!(race.players[1].state.input, "");
    assert_eq!(race.players[1].state.typo_index, None);
    assert_eq!(effects[&RacePlayerId(2)].stunned_until, None);
    assert_eq!(
        effects[&RacePlayerId(2)].impact_cue.unwrap().kind,
        RaceImpactCueKind::Banana
    );
}

#[test]
fn shield_blocks_banana_and_is_consumed() {
    let now = Instant::now();
    let mut race = race(&["one", "two"]);
    let mut effects = HashMap::new();

    activate_item_pickup(
        &mut race,
        &mut effects,
        &HashSet::new(),
        &ItemRegistry::builtin(),
        RacePlayerId(2),
        ItemPickup::Shield,
        now,
    );
    activate_item_pickup(
        &mut race,
        &mut effects,
        &HashSet::new(),
        &ItemRegistry::builtin(),
        RacePlayerId(1),
        ItemPickup::Held(HeldItem::Banana),
        now,
    );

    assert!(!race.players[1].state.has_active_shield(now));
    assert_eq!(
        effects[&RacePlayerId(2)].impact_cue.unwrap().kind,
        RaceImpactCueKind::ShieldBlock
    );
}

#[test]
fn cyclone_targets_actual_first_place_human() {
    let now = Instant::now();
    let mut race = race(&["one", "two", "three"]);
    race.players[0].state.word_index = 1;
    race.players[1].state.word_index = 0;
    let mut effects = HashMap::new();

    let report = activate_item_pickup(
        &mut race,
        &mut effects,
        &HashSet::new(),
        &ItemRegistry::builtin(),
        RacePlayerId(2),
        ItemPickup::Held(HeldItem::Cyclone),
        now,
    );

    assert!(
        report
            .events
            .iter()
            .any(|event| event.message() == "guest hit host with Cyclone")
    );
    assert_eq!(race.players[0].state.word_override(1), Some("owt"));
    assert_eq!(race.players[1].state.word_override(0), None);
}

#[test]
fn cyclone_misses_when_attacker_is_first_place() {
    let now = Instant::now();
    let mut race = race(&["one", "two", "three"]);
    race.players[0].state.word_index = 2;
    race.players[1].state.word_index = 1;
    let mut effects = HashMap::new();

    let report = activate_item_pickup(
        &mut race,
        &mut effects,
        &HashSet::new(),
        &ItemRegistry::builtin(),
        RacePlayerId(1),
        ItemPickup::Held(HeldItem::Cyclone),
        now,
    );

    assert!(
        report
            .events
            .iter()
            .any(|event| event.message() == "host missed Cyclone")
    );
    assert_eq!(race.players[0].state.word_override(2), None);
    assert_eq!(race.players[1].state.word_override(1), None);
    assert!(!effects.contains_key(&RacePlayerId(2)));
}

#[test]
fn cyclone_targets_first_place_ai_and_resets_ai_budget() {
    let now = Instant::now();
    let mut race = race(&["one", "two", "three"]);
    race.players[0].state.word_index = 0;
    race.players[1].state.word_index = 1;
    let mut effects = HashMap::new();
    let ai_players = HashSet::from([RacePlayerId(2)]);

    let report = activate_item_pickup(
        &mut race,
        &mut effects,
        &ai_players,
        &ItemRegistry::builtin(),
        RacePlayerId(1),
        ItemPickup::Held(HeldItem::Cyclone),
        now,
    );

    assert_eq!(race.players[1].state.word_override(1), Some("owt"));
    assert_eq!(report.reset_ai_players, vec![RacePlayerId(2)]);
    assert!(
        effects
            .get(&RacePlayerId(2))
            .and_then(|effect| effect.stunned_until)
            .is_some_and(|until| until > now)
    );
}
