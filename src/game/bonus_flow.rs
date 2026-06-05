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
mod tests;
