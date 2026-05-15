//! Local terminal session state.
//!
//! Multiplayer will eventually have server snapshots and remote players. For
//! Milestone 3, this type coordinates local typing, bonus claims, items, timed
//! effects, and display-facing event history.

use std::{collections::VecDeque, time::Instant};

use rand::thread_rng;

use crate::game::{
    bonus::{BonusState, claim_bonus_choice},
    effects::{ActiveEffect, AttackWarning},
    items::{HeldItem, ItemPickup, ItemUse, banana_direction, select_banana_target},
    player::PlayerState,
    track::{Track, WordList},
    typing::{KeyAction, TypingEvent, apply_key, first_typo_index},
};

const MUSHROOM_BOOST_WORDS: usize = 3;
const MUSHROOM_WPM: f64 = 180.0;

#[derive(Debug)]
pub struct LocalSession {
    pub track: Track,
    pub player: PlayerState,
    pub bonuses: BonusState,
    pub bonus_attempt: Option<BonusAttempt>,
    pub attack_warning: Option<AttackWarning>,
    pub events: EventLog,
    // Restart needs the same source word list and race length that created the
    // first track. Keeping them here lets the terminal loop reset in place.
    word_list: WordList,
    word_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAction {
    Typing(KeyAction),
    ActivateItem,
    ActivateModifiedItem,
    // A full local reset: new track, new player state, new bonus layout.
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BonusAttempt {
    pub point_index: usize,
    pub choice_index: usize,
}

impl LocalSession {
    pub fn new(track: Track, player: PlayerState, word_list: WordList) -> Self {
        let mut events = EventLog::new(8);
        events.push("Race started");

        let bonuses = BonusState::generate(&track, &word_list);
        let word_count = track.len();

        Self {
            track,
            player,
            bonuses,
            bonus_attempt: None,
            attack_warning: None,
            events,
            word_list,
            word_count,
        }
    }

    #[cfg(test)]
    pub fn with_bonuses(track: Track, player: PlayerState, bonuses: BonusState) -> Self {
        let mut events = EventLog::new(8);
        events.push("Race started");
        let word_count = track.len();
        let word_list = WordList {
            words: vec![
                "alpha".to_string(),
                "bravo".to_string(),
                "charlie".to_string(),
                "delta".to_string(),
                "echo".to_string(),
                "foxtrot".to_string(),
            ],
        };

        Self {
            track,
            player,
            bonuses,
            bonus_attempt: None,
            attack_warning: None,
            events,
            word_list,
            word_count,
        }
    }

    pub fn apply_action(&mut self, action: LocalAction, now: Instant) {
        match action {
            LocalAction::Typing(action) => self.apply_typing_action(action, now),
            LocalAction::ActivateItem => self.activate_item(ItemUse::Normal, now),
            LocalAction::ActivateModifiedItem => self.activate_item(ItemUse::Modified, now),
            LocalAction::Restart => self.restart(now),
        }
    }

    pub fn restart(&mut self, now: Instant) {
        let Ok(track) = Track::generate(&self.word_list, self.word_count) else {
            self.events.push("Restart failed");
            return;
        };
        let player = PlayerState::new(now);
        let bonuses = BonusState::generate(&track, &self.word_list);

        self.track = track;
        self.player = player;
        self.bonuses = bonuses;
        self.bonus_attempt = None;
        self.attack_warning = None;
        self.events = EventLog::new(8);
        self.events.push("Race restarted");
    }

    pub fn tick(&mut self, now: Instant) {
        self.advance_mushroom(now);

        let expired_choices = self.bonuses.expire_cooldowns(&self.track, now);
        if expired_choices > 0 {
            self.events.push("Bonus refreshed");
        }

        let expired_effects = self.player.expire_effects(now);
        if expired_effects > 0 {
            self.events.push("Shield expired");
        }

        if self
            .attack_warning
            .is_some_and(|warning| warning.resolves_at <= now)
        {
            if self.player.has_active_shield(now) {
                self.player.active_effects.clear();
                self.events.push("Attack blocked");
            } else {
                self.events.push("Attack landed");
            }
            self.attack_warning = None;
        }
    }

    fn apply_typing_action(&mut self, action: KeyAction, now: Instant) {
        if self.bonus_attempt.is_some() {
            self.apply_bonus_typing_action(action, now);
            return;
        }

        if let KeyAction::Char(ch) = action {
            if self.can_start_bonus_attempt(now) {
                if let Some((point_index, choice_index)) = self.match_bonus_start(ch, now) {
                    self.bonus_attempt = Some(BonusAttempt {
                        point_index,
                        choice_index,
                    });
                    self.apply_bonus_char(ch, now);
                    return;
                }
            }
        }

        let previous_word_index = self.player.word_index;
        let previous_word = self
            .track
            .current_word(previous_word_index)
            .map(str::to_owned);
        let events = apply_key(&mut self.player, &self.track, action, now);

        for event in events {
            if let Some(message) = event_message(event, previous_word.as_deref()) {
                self.events.push(message);
            }
        }
    }

    fn apply_bonus_typing_action(&mut self, action: KeyAction, now: Instant) {
        match action {
            KeyAction::Char(ch) => self.apply_bonus_char(ch, now),
            KeyAction::Backspace => {
                let previous_typo = self.player.typo_index;
                if self.player.input.pop().is_some() {
                    self.player.stats.backspaces += 1;
                    self.recalculate_bonus_typo();
                    if previous_typo.is_some() && self.player.typo_index.is_none() {
                        self.events.push("Typo cleared");
                    }
                }
                if self.player.input.is_empty() {
                    self.bonus_attempt = None;
                    self.events.push("Bonus attempt cancelled");
                }
            }
            KeyAction::Space => self.apply_bonus_char(' ', now),
        }
    }

    fn apply_bonus_char(&mut self, ch: char, now: Instant) {
        let Some(attempt) = self.bonus_attempt else {
            return;
        };
        let Some(target) = self
            .bonuses
            .points
            .get(attempt.point_index)
            .and_then(|point| point.choices.get(attempt.choice_index))
            .map(|choice| choice.word.clone())
        else {
            self.bonus_attempt = None;
            return;
        };

        let previous_typo = self.player.typo_index;
        let input_index = self.player.input.chars().count();
        let is_correct = previous_typo.is_none() && target.chars().nth(input_index) == Some(ch);

        self.player.stats.typed_chars += 1;
        if is_correct {
            self.player.stats.correct_chars += 1;
        } else {
            self.player.stats.typo_chars += 1;
        }

        self.player.input.push(ch);
        self.player.typo_index = first_typo_index(&self.player.input, &target);
        if previous_typo.is_none() && self.player.typo_index.is_some() {
            self.events.push("Typo started");
        }

        if self.player.typo_index.is_none() && self.player.input == target {
            self.claim_bonus(attempt, now);
        }
    }

    fn claim_bonus(&mut self, attempt: BonusAttempt, now: Instant) {
        let mut rng = thread_rng();
        let Some(item) = claim_bonus_choice(
            &mut self.bonuses,
            attempt.point_index,
            attempt.choice_index,
            now,
            false,
            &mut rng,
        ) else {
            self.bonus_attempt = None;
            self.player.input.clear();
            return;
        };

        self.receive_pickup(item, now);
        self.player.input.clear();
        self.player.typo_index = None;
        self.bonus_attempt = None;
    }

    fn recalculate_bonus_typo(&mut self) {
        let Some(attempt) = self.bonus_attempt else {
            self.player.typo_index = None;
            return;
        };
        let Some(target) = self
            .bonuses
            .points
            .get(attempt.point_index)
            .and_then(|point| point.choices.get(attempt.choice_index))
            .map(|choice| choice.word.as_str())
        else {
            self.player.typo_index = None;
            return;
        };

        self.player.typo_index = first_typo_index(&self.player.input, target);
    }

    fn can_start_bonus_attempt(&self, now: Instant) -> bool {
        self.player.held_item.is_none()
            && !self.player.has_active_shield(now)
            && self.player.typo_index.is_none()
            && self.player.input.is_empty()
            && self
                .bonuses
                .point_for_gap(self.player.word_index)
                .is_some_and(|(_, point)| {
                    point.choices.iter().any(|choice| choice.is_available(now))
                })
    }

    fn match_bonus_start(&self, ch: char, now: Instant) -> Option<(usize, usize)> {
        let (point_index, point) = self.bonuses.point_for_gap(self.player.word_index)?;
        point
            .available_choice_starting_with(ch, now)
            .map(|(choice_index, _)| (point_index, choice_index))
    }

    fn activate_item(&mut self, item_use: ItemUse, now: Instant) {
        let Some(item) = self.player.held_item else {
            self.events.push("No item");
            return;
        };

        match item {
            HeldItem::Mushroom => {
                self.player.held_item = None;
                self.activate_mushroom(now);
            }
            HeldItem::Banana => {
                self.player.held_item = None;
                let direction = banana_direction(item_use);
                let target = select_banana_target(self.player.word_index, &[], direction, 10);
                if target.is_none() {
                    self.events.push("No racer in range");
                }
            }
        }
    }

    fn activate_shield(&mut self, now: Instant) {
        self.player.active_effects.push(ActiveEffect::Shield {
            until: now + std::time::Duration::from_secs(5),
        });
        self.events.push("Shield activated");
    }

    fn receive_pickup(&mut self, item: ItemPickup, now: Instant) {
        match item {
            ItemPickup::Held(held_item) => {
                self.player.held_item = Some(held_item);
                self.events.push(format!("Picked up {}", item.name()));
            }
            ItemPickup::Shield => {
                self.activate_shield(now);
                self.events.push("Picked up Shield");
            }
        }
    }

    fn activate_mushroom(&mut self, now: Instant) {
        self.player.input.clear();
        self.player.typo_index = None;
        self.bonus_attempt = None;
        self.player.active_effects.push(ActiveEffect::Mushroom {
            remaining_words: MUSHROOM_BOOST_WORDS,
            next_step_at: now,
            step_interval: mushroom_step_interval(),
        });
        self.events.push("Used Mushroom");
        self.advance_mushroom(now);
    }

    fn advance_mushroom(&mut self, now: Instant) {
        loop {
            let Some(effect_index) = self.player.active_effects.iter().position(|effect| {
                matches!(
                    effect,
                    ActiveEffect::Mushroom {
                        remaining_words,
                        next_step_at,
                        ..
                    } if *remaining_words > 0 && *next_step_at <= now
                )
            }) else {
                break;
            };

            self.advance_mushroom_one_word(now, effect_index);

            if self.player.is_finished() {
                break;
            }
        }
    }

    fn advance_mushroom_one_word(&mut self, now: Instant, effect_index: usize) {
        let remaining = self.track.len().saturating_sub(self.player.word_index);
        if remaining == 0 {
            self.player.active_effects.remove(effect_index);
            return;
        }

        self.player.word_index += 1;
        self.player.stats.completed_words += 1;
        self.player.input.clear();
        self.player.typo_index = None;
        self.bonus_attempt = None;

        if self.player.word_index >= self.track.len() {
            self.player.finished_at = Some(now);
            self.events.push("Race finished");
            self.player.active_effects.remove(effect_index);
            return;
        }

        if let Some(ActiveEffect::Mushroom {
            remaining_words,
            next_step_at,
            step_interval,
        }) = self.player.active_effects.get_mut(effect_index)
        {
            *remaining_words -= 1;
            if *remaining_words == 0 {
                self.player.active_effects.remove(effect_index);
            } else {
                *next_step_at += *step_interval;
            }
        }
    }
}

fn mushroom_step_interval() -> std::time::Duration {
    std::time::Duration::from_secs_f64(60.0 / MUSHROOM_WPM)
}

fn event_message(event: TypingEvent, previous_word: Option<&str>) -> Option<String> {
    match event {
        TypingEvent::InputChanged => None,
        TypingEvent::WordCompleted => previous_word.map(|word| format!("Completed {word}")),
        TypingEvent::RaceFinished => Some("Race finished".to_string()),
        TypingEvent::TypoStarted { .. } => Some("Typo started".to_string()),
        TypingEvent::TypoCleared => Some("Typo cleared".to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct EventLog {
    entries: VecDeque<String>,
    capacity: usize,
}

impl EventLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, message: impl Into<String>) {
        if self.capacity == 0 {
            return;
        }

        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(message.into());
    }

    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::{
        game::{
            bonus::{BonusChoice, BonusPoint, BonusState},
            effects::ActiveEffect,
            items::{HeldItem, ItemPickup},
            player::PlayerState,
            track::Track,
            typing::KeyAction,
        },
        ui::session::{EventLog, LocalAction, LocalSession},
    };

    fn track(words: &[&str]) -> Track {
        Track::new(words.iter().map(|word| word.to_string()).collect())
    }

    fn bonuses() -> BonusState {
        BonusState::with_points(
            vec![BonusPoint::new(
                0,
                [
                    BonusChoice::available("drift"),
                    BonusChoice::available("spark"),
                    BonusChoice::available("turbo"),
                ],
            )],
            vec!["boost".to_string()],
        )
    }

    #[test]
    fn event_log_keeps_only_capacity_entries() {
        let mut log = EventLog::new(2);

        log.push("one");
        log.push("two");
        log.push("three");

        assert_eq!(log.entries().collect::<Vec<_>>(), vec!["two", "three"]);
    }

    #[test]
    fn local_session_logs_meaningful_typing_events() {
        let track = track(&["fox", "road"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));

        session.apply_action(LocalAction::Typing(KeyAction::Char('f')), Instant::now());
        session.apply_action(LocalAction::Typing(KeyAction::Char('a')), Instant::now());
        session.apply_action(LocalAction::Typing(KeyAction::Backspace), Instant::now());
        session.apply_action(LocalAction::Typing(KeyAction::Char('o')), Instant::now());
        session.apply_action(LocalAction::Typing(KeyAction::Char('x')), Instant::now());
        session.apply_action(LocalAction::Typing(KeyAction::Space), Instant::now());

        let entries = session.events.entries().collect::<Vec<_>>();
        assert!(entries.contains(&"Race started"));
        assert!(entries.contains(&"Typo started"));
        assert!(entries.contains(&"Typo cleared"));
        assert!(entries.contains(&"Completed fox"));
    }

    #[test]
    fn restart_builds_new_race_state() {
        let track = track(&["fox", "road"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        session.player.word_index = 1;
        session.player.input = "ro".to_string();
        session.player.held_item = Some(HeldItem::Banana);
        session.player.active_effects.push(ActiveEffect::Shield {
            until: Instant::now() + std::time::Duration::from_secs(5),
        });

        session.apply_action(LocalAction::Restart, Instant::now());

        assert_eq!(session.track.len(), 2);
        assert_eq!(session.player.word_index, 0);
        assert!(session.player.input.is_empty());
        assert_eq!(session.player.held_item, None);
        assert!(session.player.active_effects.is_empty());
        assert!(session.bonus_attempt.is_none());
        assert!(session.attack_warning.is_none());
        assert_eq!(
            session.events.entries().collect::<Vec<_>>(),
            vec!["Race restarted"]
        );
    }

    #[test]
    fn completing_bonus_grants_item_or_activates_shield() {
        let track = track(&["one", "two"]);
        let player = PlayerState::new(Instant::now());
        let mut session = LocalSession::with_bonuses(track, player, bonuses());
        session.player.word_index = 1;

        for ch in "drift".chars() {
            session.apply_action(LocalAction::Typing(KeyAction::Char(ch)), Instant::now());
        }

        assert!(session.player.held_item.is_some() || !session.player.active_effects.is_empty());
        assert!(session.player.input.is_empty());
    }

    #[test]
    fn bonus_is_unavailable_while_holding_item() {
        let track = track(&["one", "two"]);
        let player = PlayerState::new(Instant::now());
        let mut session = LocalSession::with_bonuses(track, player, bonuses());
        session.player.word_index = 1;
        session.player.held_item = Some(HeldItem::Mushroom);

        session.apply_action(LocalAction::Typing(KeyAction::Char('d')), Instant::now());

        assert!(session.bonus_attempt.is_none());
        assert_eq!(session.player.input, "d");
    }

    #[test]
    fn backspace_can_bail_out_of_bonus_attempt() {
        let track = track(&["one", "two"]);
        let player = PlayerState::new(Instant::now());
        let mut session = LocalSession::with_bonuses(track, player, bonuses());
        session.player.word_index = 1;

        session.apply_action(LocalAction::Typing(KeyAction::Char('d')), Instant::now());
        session.apply_action(LocalAction::Typing(KeyAction::Backspace), Instant::now());

        assert!(session.bonus_attempt.is_none());
        assert!(session.player.input.is_empty());
    }

    #[test]
    fn mushroom_advances_three_words_one_step_at_a_time() {
        let track = track(&["one", "two", "three", "four", "five"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        session.player.held_item = Some(HeldItem::Mushroom);
        let now = Instant::now();

        session.apply_action(LocalAction::ActivateItem, now);

        assert_eq!(session.player.word_index, 1);
        assert_eq!(session.player.stats.completed_words, 1);
        assert_eq!(session.player.held_item, None);
        assert!(
            session
                .player
                .active_effects
                .iter()
                .any(|effect| matches!(effect, ActiveEffect::Mushroom { .. }))
        );

        session.tick(now + std::time::Duration::from_secs_f64(0.4));
        assert_eq!(session.player.word_index, 2);

        session.tick(now + std::time::Duration::from_secs_f64(0.8));
        assert_eq!(session.player.word_index, 3);
        assert!(
            !session
                .player
                .active_effects
                .iter()
                .any(|effect| matches!(effect, ActiveEffect::Mushroom { .. }))
        );
    }

    #[test]
    fn mushroom_can_finish_race() {
        let track = track(&["one", "two"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        session.player.word_index = 1;
        session.player.stats.completed_words = 1;
        session.player.held_item = Some(HeldItem::Mushroom);

        session.apply_action(LocalAction::ActivateItem, Instant::now());

        assert!(session.player.is_finished());
        assert_eq!(session.player.stats.completed_words, 2);
    }

    #[test]
    fn shield_pickup_activates_immediately_without_held_item() {
        let track = track(&["one", "two"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));

        session.receive_pickup(ItemPickup::Shield, Instant::now());

        assert_eq!(session.player.held_item, None);
        assert!(matches!(
            session.player.active_effects.first(),
            Some(ActiveEffect::Shield { .. })
        ));
    }

    #[test]
    fn bonus_is_unavailable_while_shield_is_active() {
        let now = Instant::now();
        let track = track(&["one", "two"]);
        let player = PlayerState::new(now);
        let mut session = LocalSession::with_bonuses(track, player, bonuses());
        session.player.word_index = 1;
        session.receive_pickup(ItemPickup::Shield, now);

        session.apply_action(LocalAction::Typing(KeyAction::Char('d')), now);

        assert!(session.bonus_attempt.is_none());
        assert_eq!(session.player.input, "d");
    }

    #[test]
    fn banana_with_no_target_is_consumed() {
        let track = track(&["one", "two"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        session.player.held_item = Some(HeldItem::Banana);

        session.apply_action(LocalAction::ActivateItem, Instant::now());

        assert_eq!(session.player.held_item, None);
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "No racer in range")
        );
    }
}
