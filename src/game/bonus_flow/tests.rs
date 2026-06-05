use std::{collections::HashMap, time::Instant};

use rand::{SeedableRng, rngs::StdRng};

use super::{
    BonusAttempt, BonusClaimRoll, BonusFlowEvent, BonusFlowState, apply_bonus_key,
    claim_random_available_bonus,
};
use crate::game::{
    bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
    items::{ItemRegistry, ItemRollContext, RacePositionBand},
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::Track,
    typing::KeyAction,
};

fn race_with_bonus(now: Instant) -> (RaceState, BonusState) {
    let mut race = RaceState::new(Track::new(vec!["one".into(), "two".into()]));
    race.add_player(RacePlayerId(1), "player", PlayerColorId::Cyan, now);
    race.players[0].state.word_index = 1;
    let bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice::available("dash"),
                BonusChoice::available("spin"),
                BonusChoice::available("zoom"),
            ],
        )],
        vec!["dash".into(), "spin".into(), "zoom".into()],
    );
    (race, bonuses)
}

fn item_context() -> ItemRollContext {
    ItemRollContext {
        has_nearby_racer: false,
        position: RacePositionBand::Middle,
    }
}

#[test]
fn bonus_flow_starts_from_first_matching_character() {
    let now = Instant::now();
    let (mut race, mut bonuses) = race_with_bonus(now);
    let mut attempts = HashMap::new();
    let mut spent = HashMap::new();
    let mut rng = StdRng::seed_from_u64(1);
    let registry = ItemRegistry::builtin();

    let outcome = apply_bonus_key(
        &mut BonusFlowState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent,
        },
        1_u64,
        RacePlayerId(1),
        KeyAction::Char('d'),
        now,
        BonusClaimRoll {
            item_context: item_context(),
            item_registry: &registry,
            rng: &mut rng,
        },
    );

    assert!(outcome.handled);
    assert_eq!(
        attempts.get(&1_u64),
        Some(&BonusAttempt {
            point_index: 0,
            choice_index: 0
        })
    );
    assert!(
        outcome
            .events
            .contains(&BonusFlowEvent::AttemptStarted(BonusAttempt {
                point_index: 0,
                choice_index: 0
            }))
    );
    assert_eq!(race.players[0].state.input, "d");
}

#[test]
fn random_available_bonus_claim_resolves_pickup_and_spends_gap() {
    let now = Instant::now();
    let (mut race, mut bonuses) = race_with_bonus(now);
    let mut attempts = HashMap::new();
    let mut spent = HashMap::new();
    let mut rng = StdRng::seed_from_u64(2);
    let registry = ItemRegistry::builtin();

    let outcome = claim_random_available_bonus(
        &mut BonusFlowState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent,
        },
        1_u64,
        RacePlayerId(1),
        now,
        BonusClaimRoll {
            item_context: item_context(),
            item_registry: &registry,
            rng: &mut rng,
        },
    );

    assert!(outcome.is_some_and(|outcome| outcome.pickup.is_some()));
    assert_eq!(spent.get(&1_u64), Some(&0));
}

#[test]
fn random_available_bonus_claim_respects_spent_gap() {
    let now = Instant::now();
    let (mut race, mut bonuses) = race_with_bonus(now);
    let mut attempts = HashMap::new();
    let mut spent = HashMap::from([(1_u64, 0_usize)]);
    let mut rng = StdRng::seed_from_u64(2);
    let registry = ItemRegistry::builtin();

    let outcome = claim_random_available_bonus(
        &mut BonusFlowState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent,
        },
        1_u64,
        RacePlayerId(1),
        now,
        BonusClaimRoll {
            item_context: item_context(),
            item_registry: &registry,
            rng: &mut rng,
        },
    );

    assert!(outcome.is_none());
}

#[test]
fn backspace_bails_out_when_bonus_input_becomes_empty() {
    let now = Instant::now();
    let (mut race, mut bonuses) = race_with_bonus(now);
    let mut attempts = HashMap::from([(
        1_u64,
        BonusAttempt {
            point_index: 0,
            choice_index: 0,
        },
    )]);
    let mut spent = HashMap::new();
    let mut rng = StdRng::seed_from_u64(1);
    let registry = ItemRegistry::builtin();
    race.players[0].state.input = "d".into();

    let outcome = apply_bonus_key(
        &mut BonusFlowState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent,
        },
        1_u64,
        RacePlayerId(1),
        KeyAction::Backspace,
        now,
        BonusClaimRoll {
            item_context: item_context(),
            item_registry: &registry,
            rng: &mut rng,
        },
    );

    assert!(outcome.handled);
    assert!(!attempts.contains_key(&1_u64));
    assert!(outcome.events.contains(&BonusFlowEvent::AttemptCancelled));
}

#[test]
fn space_claims_completed_bonus_and_marks_gap_spent() {
    let now = Instant::now();
    let (mut race, mut bonuses) = race_with_bonus(now);
    let mut attempts = HashMap::from([(
        1_u64,
        BonusAttempt {
            point_index: 0,
            choice_index: 0,
        },
    )]);
    let mut spent = HashMap::new();
    let mut rng = StdRng::seed_from_u64(1);
    let registry = ItemRegistry::builtin();
    race.players[0].state.input = "dash".into();

    let outcome = apply_bonus_key(
        &mut BonusFlowState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent,
        },
        1_u64,
        RacePlayerId(1),
        KeyAction::Space,
        now,
        BonusClaimRoll {
            item_context: item_context(),
            item_registry: &registry,
            rng: &mut rng,
        },
    );

    assert!(outcome.handled);
    assert!(!attempts.contains_key(&1_u64));
    assert_eq!(spent.get(&1_u64), Some(&0));
    assert_eq!(race.players[0].state.input, "");
    assert!(matches!(
        bonuses.points[0].choices[0].status,
        BonusChoiceStatus::Cooldown { .. }
    ));
    assert!(
        outcome
            .events
            .iter()
            .any(|event| matches!(event, BonusFlowEvent::ClaimResolved(_)))
    );
}
