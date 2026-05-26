//! Shared bonus-attempt typing flow.
//!
//! Bonus words are displayed by UI adapters, but the rules for entering,
//! correcting, cancelling, and claiming those words are gameplay rules. This
//! module keeps that state machine shared between terminal and browser hosts.

use std::{collections::HashMap, hash::Hash, time::Instant};

use rand::Rng;

use super::{
    bonus::{BonusState, claim_bonus_choice},
    items::{ItemPickup, ItemRegistry, ItemRollContext},
    race::{RacePlayerId, RaceState},
    typing::{KeyAction, first_typo_index},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BonusAttempt {
    pub point_index: usize,
    pub choice_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BonusClaimOutcome {
    pub pickup: Option<ItemPickup>,
    pub after_word_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BonusFlowEvent {
    AttemptStarted(BonusAttempt),
    InputChanged,
    TypoStarted,
    TypoCleared,
    AttemptCancelled,
    ClaimResolved(BonusClaimOutcome),
    AttemptInvalidated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BonusFlowOutcome {
    pub handled: bool,
    pub events: Vec<BonusFlowEvent>,
}

impl BonusFlowOutcome {
    fn ignored() -> Self {
        Self {
            handled: false,
            events: Vec::new(),
        }
    }

    fn handled(events: Vec<BonusFlowEvent>) -> Self {
        Self {
            handled: true,
            events,
        }
    }
}

pub struct BonusFlowState<'a, PlayerKey> {
    pub race: &'a mut RaceState,
    pub bonuses: &'a mut BonusState,
    pub bonus_attempts: &'a mut HashMap<PlayerKey, BonusAttempt>,
    pub spent_bonus_gaps: &'a mut HashMap<PlayerKey, usize>,
}

pub struct BonusClaimRoll<'a, R> {
    pub item_context: ItemRollContext,
    pub item_registry: &'a ItemRegistry,
    pub rng: &'a mut R,
}

pub fn apply_bonus_key<PlayerKey, R>(
    state: &mut BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
    action: KeyAction,
    now: Instant,
    roll: BonusClaimRoll<'_, R>,
) -> BonusFlowOutcome
where
    PlayerKey: Copy + Eq + Hash,
    R: Rng,
{
    if state.bonus_attempts.contains_key(&player_key) {
        return BonusFlowOutcome::handled(apply_existing_bonus_key(
            state,
            player_key,
            race_player_id,
            action,
            now,
            roll,
        ));
    }

    let KeyAction::Char(ch) = action else {
        return BonusFlowOutcome::ignored();
    };
    let Some(attempt) = bonus_attempt_start(
        state.race,
        state.bonuses,
        state.spent_bonus_gaps,
        player_key,
        race_player_id,
        ch,
        now,
    ) else {
        return BonusFlowOutcome::ignored();
    };

    state.bonus_attempts.insert(player_key, attempt);
    let mut events = vec![BonusFlowEvent::AttemptStarted(attempt)];
    events.extend(apply_bonus_char(state, player_key, race_player_id, ch));
    BonusFlowOutcome::handled(events)
}

pub fn bonus_attempt_start<PlayerKey>(
    race: &RaceState,
    bonuses: &BonusState,
    spent_bonus_gaps: &HashMap<PlayerKey, usize>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
    ch: char,
    now: Instant,
) -> Option<BonusAttempt>
where
    PlayerKey: Copy + Eq + Hash,
{
    let player = race.player(race_player_id)?;
    if player.state.held_item.is_some()
        || player.state.has_active_shield(now)
        || player.state.has_active_focus(now)
        || player.state.typo_index.is_some()
        || !player.state.input.is_empty()
        || player.state.is_finished()
    {
        return None;
    }

    let (point_index, point) = bonuses.point_for_gap(player.state.word_index)?;
    if spent_bonus_gaps
        .get(&player_key)
        .is_some_and(|after_word_index| *after_word_index == point.after_word_index)
    {
        return None;
    }

    point
        .available_choice_starting_with(ch, now)
        .map(|(choice_index, _)| BonusAttempt {
            point_index,
            choice_index,
        })
}

pub fn claim_active_bonus<PlayerKey, R>(
    state: &mut BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
    now: Instant,
    roll: BonusClaimRoll<'_, R>,
) -> Option<BonusClaimOutcome>
where
    PlayerKey: Copy + Eq + Hash,
    R: Rng,
{
    let attempt = state.bonus_attempts.remove(&player_key)?;
    Some(resolve_bonus_claim(
        state,
        player_key,
        race_player_id,
        attempt,
        now,
        roll,
    ))
}

pub fn claim_random_available_bonus<PlayerKey, R>(
    state: &mut BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
    now: Instant,
    roll: BonusClaimRoll<'_, R>,
) -> Option<BonusClaimOutcome>
where
    PlayerKey: Copy + Eq + Hash,
    R: Rng,
{
    if state.bonus_attempts.contains_key(&player_key) {
        return None;
    }
    let player = state.race.player(race_player_id)?;
    if player.state.held_item.is_some()
        || player.state.has_active_shield(now)
        || player.state.has_active_focus(now)
        || player.state.typo_index.is_some()
        || !player.state.input.is_empty()
        || player.state.is_finished()
    {
        return None;
    }

    let (point_index, point) = state.bonuses.point_for_gap(player.state.word_index)?;
    if state
        .spent_bonus_gaps
        .get(&player_key)
        .is_some_and(|after_word_index| *after_word_index == point.after_word_index)
    {
        return None;
    }

    let available_choices = point
        .choices
        .iter()
        .enumerate()
        .filter(|(_, choice)| choice.is_available(now))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if available_choices.is_empty() {
        return None;
    }

    let choice_index = available_choices[roll.rng.gen_range(0..available_choices.len())];
    let attempt = BonusAttempt {
        point_index,
        choice_index,
    };
    Some(resolve_bonus_claim(
        state,
        player_key,
        race_player_id,
        attempt,
        now,
        roll,
    ))
}

fn apply_existing_bonus_key<PlayerKey, R>(
    state: &mut BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
    action: KeyAction,
    now: Instant,
    roll: BonusClaimRoll<'_, R>,
) -> Vec<BonusFlowEvent>
where
    PlayerKey: Copy + Eq + Hash,
    R: Rng,
{
    match action {
        KeyAction::Char(ch) => apply_bonus_char(state, player_key, race_player_id, ch),
        KeyAction::Backspace => apply_bonus_backspace(state, player_key, race_player_id),
        KeyAction::Space => {
            if bonus_completed_without_typo(state, player_key, race_player_id) {
                claim_active_bonus(state, player_key, race_player_id, now, roll)
                    .map(|outcome| vec![BonusFlowEvent::ClaimResolved(outcome)])
                    .unwrap_or_else(|| vec![BonusFlowEvent::AttemptInvalidated])
            } else {
                apply_bonus_char(state, player_key, race_player_id, ' ')
            }
        }
    }
}

fn apply_bonus_char<PlayerKey>(
    state: &mut BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
    ch: char,
) -> Vec<BonusFlowEvent>
where
    PlayerKey: Copy + Eq + Hash,
{
    let Some(attempt) = state.bonus_attempts.get(&player_key).copied() else {
        return Vec::new();
    };
    let Some(target) = bonus_target(state.bonuses, attempt).map(str::to_owned) else {
        state.bonus_attempts.remove(&player_key);
        return vec![BonusFlowEvent::AttemptInvalidated];
    };
    let Some(player) = state.race.player_mut(race_player_id) else {
        state.bonus_attempts.remove(&player_key);
        return vec![BonusFlowEvent::AttemptInvalidated];
    };

    let previous_typo = player.state.typo_index;
    let input_index = player.state.input.chars().count();
    let is_correct = previous_typo.is_none() && target.chars().nth(input_index) == Some(ch);

    player.state.stats.typed_chars += 1;
    if is_correct {
        player.state.stats.correct_chars += 1;
    } else {
        player.state.stats.typo_chars += 1;
    }

    player.state.input.push(ch);
    player.state.typo_index = first_typo_index(&player.state.input, &target);

    let mut events = vec![BonusFlowEvent::InputChanged];
    if previous_typo.is_none() && player.state.typo_index.is_some() {
        events.push(BonusFlowEvent::TypoStarted);
    }
    events
}

fn apply_bonus_backspace<PlayerKey>(
    state: &mut BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
) -> Vec<BonusFlowEvent>
where
    PlayerKey: Copy + Eq + Hash,
{
    let Some(attempt) = state.bonus_attempts.get(&player_key).copied() else {
        return Vec::new();
    };
    let target = bonus_target(state.bonuses, attempt).map(str::to_owned);
    let Some(player) = state.race.player_mut(race_player_id) else {
        state.bonus_attempts.remove(&player_key);
        return vec![BonusFlowEvent::AttemptInvalidated];
    };

    let previous_typo = player.state.typo_index;
    if player.state.input.pop().is_some() {
        player.state.stats.backspaces += 1;
    }

    player.state.typo_index = target
        .as_deref()
        .and_then(|target| first_typo_index(&player.state.input, target));

    let mut events = vec![BonusFlowEvent::InputChanged];
    if previous_typo.is_some() && player.state.typo_index.is_none() {
        events.push(BonusFlowEvent::TypoCleared);
    }
    if player.state.input.is_empty() {
        state.bonus_attempts.remove(&player_key);
        events.push(BonusFlowEvent::AttemptCancelled);
    }
    events
}

fn bonus_completed_without_typo<PlayerKey>(
    state: &BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
) -> bool
where
    PlayerKey: Copy + Eq + Hash,
{
    let Some(attempt) = state.bonus_attempts.get(&player_key).copied() else {
        return false;
    };
    let Some(target) = bonus_target(state.bonuses, attempt) else {
        return false;
    };
    let Some(player) = state.race.player(race_player_id) else {
        return false;
    };

    player.state.typo_index.is_none() && player.state.input == target
}

fn resolve_bonus_claim<PlayerKey, R>(
    state: &mut BonusFlowState<'_, PlayerKey>,
    player_key: PlayerKey,
    race_player_id: RacePlayerId,
    attempt: BonusAttempt,
    now: Instant,
    roll: BonusClaimRoll<'_, R>,
) -> BonusClaimOutcome
where
    PlayerKey: Copy + Eq + Hash,
    R: Rng,
{
    let after_word_index = state
        .bonuses
        .points
        .get(attempt.point_index)
        .map(|point| point.after_word_index);
    let pickup = claim_bonus_choice(
        state.bonuses,
        attempt.point_index,
        attempt.choice_index,
        now,
        roll.item_context,
        roll.item_registry,
        roll.rng,
    );

    if let Some(player) = state.race.player_mut(race_player_id) {
        player.state.input.clear();
        player.state.typo_index = None;
    }

    if let Some(after_word_index) = after_word_index {
        state.spent_bonus_gaps.insert(player_key, after_word_index);
    }

    BonusClaimOutcome {
        pickup,
        after_word_index,
    }
}

fn bonus_target(bonuses: &BonusState, attempt: BonusAttempt) -> Option<&str> {
    bonuses
        .points
        .get(attempt.point_index)?
        .choices
        .get(attempt.choice_index)
        .map(|choice| choice.word.as_str())
}

#[cfg(test)]
mod tests {
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
}
