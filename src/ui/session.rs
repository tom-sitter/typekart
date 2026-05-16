//! Local terminal session state.
//!
//! Multiplayer will eventually have server snapshots and remote players. For
//! Milestone 3, this type coordinates local typing, bonus claims, items, timed
//! effects, and display-facing event history.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use rand::{thread_rng, Rng};

use crate::game::{
    ai::AiDifficulty,
    bonus::{claim_bonus_choice, BonusState},
    effects::{ActiveEffect, AttackWarning, PendingAttack},
    items::{
        select_nearest_banana_target, HeldItem, ItemPickup, ItemRegistry, ItemUse, RacerPosition,
    },
    mods::ActiveModConfig,
    player::PlayerState,
    track::{Track, WordList},
    typing::{apply_key, first_typo_index, KeyAction, TypingEvent},
};

const MUSHROOM_BOOST_WORDS: usize = 3;
const MUSHROOM_WPM: f64 = 180.0;
const AI_BANANA_STUN: Duration = Duration::from_secs(2);
const PLAYER_ATTACK_WARNING: Duration = Duration::from_millis(900);
const MAX_AI_RACERS: usize = 6;
const POST_FIRST_FINISH_TIMEOUT: Duration = Duration::from_secs(15);
const ITEM_IMPACT_BLINK: Duration = Duration::from_millis(1200);
const ITEM_CUE_DURATION: Duration = Duration::from_millis(1500);
const RACE_COUNTDOWN: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub struct LocalSession {
    pub track: Track,
    pub player: PlayerState,
    pub ai_racers: Vec<AiRacer>,
    pub bonuses: BonusState,
    pub bonus_attempt: Option<BonusAttempt>,
    pub attack_warning: Option<AttackWarning>,
    pub player_impact_until: Option<Instant>,
    pub player_item_cue: Option<ItemCue>,
    pub race_status: RaceStatus,
    pub race_phase: RacePhase,
    pub events: EventLog,
    pub run_log: RunLog,
    // Restart needs the same source word list and race length that created the
    // first track. Keeping them here lets the terminal loop reset in place.
    word_list: WordList,
    word_count: usize,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    item_registry: ItemRegistry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RacePhase {
    WaitingForHost,
    Countdown { starts_at: Instant },
    Racing,
}

impl RacePhase {
    pub fn countdown_label(self, now: Instant) -> Option<String> {
        let Self::Countdown { starts_at } = self else {
            return None;
        };

        let remaining = starts_at.saturating_duration_since(now);
        let seconds = remaining.as_secs_f64().ceil().clamp(1.0, 3.0) as u8;
        Some(seconds.to_string())
    }

    pub fn is_racing(self) -> bool {
        self == Self::Racing
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RaceStatus {
    pub first_finished_at: Option<Instant>,
    pub ended_at: Option<Instant>,
}

impl RaceStatus {
    pub fn is_ended(self) -> bool {
        self.ended_at.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct AiRacer {
    pub id: usize,
    pub name: String,
    pub player: PlayerState,
    pub difficulty: AiDifficulty,
    pub words_per_minute: f64,
    char_budget: f64,
    last_update: Instant,
    stunned_until: Option<Instant>,
    pub(crate) impact_until: Option<Instant>,
    pub item_cue: Option<ItemCue>,
}

impl AiRacer {
    pub(crate) fn new(
        id: usize,
        difficulty: AiDifficulty,
        words_per_minute: f64,
        now: Instant,
    ) -> Self {
        Self {
            id,
            name: format!("ai-{id}"),
            player: PlayerState::new(now),
            difficulty,
            words_per_minute,
            char_budget: 0.0,
            last_update: now,
            stunned_until: None,
            impact_until: None,
            item_cue: None,
        }
    }

    pub fn is_stunned(&self, now: Instant) -> bool {
        self.stunned_until.is_some_and(|until| until > now)
    }

    #[cfg(test)]
    pub fn is_impacted(&self, now: Instant) -> bool {
        self.impact_until.is_some_and(|until| until > now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemCue {
    pub kind: ItemCueKind,
    pub until: Instant,
}

impl ItemCue {
    fn new(kind: ItemCueKind, now: Instant) -> Self {
        Self {
            kind,
            until: now + ITEM_CUE_DURATION,
        }
    }

    pub fn is_visible(self, now: Instant) -> bool {
        self.until > now
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCueKind {
    Banana { direction: AttackDirection },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackDirection {
    Ahead,
    Behind,
    Overlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BananaResolution {
    SpunOut,
    Blocked,
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

fn build_ai_racers(count: usize, difficulty: AiDifficulty, now: Instant) -> Vec<AiRacer> {
    let mut rng = thread_rng();
    (1..=count)
        .map(|id| {
            let words_per_minute = rng.gen_range(difficulty.wpm_range());
            AiRacer::new(id, difficulty, words_per_minute, now)
        })
        .collect()
}

impl LocalSession {
    pub fn new(
        track: Track,
        player: PlayerState,
        word_list: WordList,
        ai_racer_count: usize,
        ai_difficulty: AiDifficulty,
        item_registry: ItemRegistry,
        active_mod_config: ActiveModConfig,
    ) -> Self {
        let mut events = EventLog::new(8);
        events.push("Press Space to start");

        let bonuses = BonusState::generate(&track, &word_list);
        let word_count = track.len();
        let now = player.started_at;
        let ai_racer_count = ai_racer_count.min(MAX_AI_RACERS);
        let ai_racers = build_ai_racers(ai_racer_count, ai_difficulty, now);
        let mut run_log = RunLog::new(now, 500);
        run_log.push(now, active_mod_config.log_summary());
        run_log.push(
            now,
            format!(
                "session created words={} ai_racers={} difficulty={ai_difficulty:?}",
                word_count, ai_racer_count
            ),
        );

        Self {
            track,
            player,
            ai_racers,
            bonuses,
            bonus_attempt: None,
            attack_warning: None,
            player_impact_until: None,
            player_item_cue: None,
            race_status: RaceStatus::default(),
            race_phase: RacePhase::WaitingForHost,
            events,
            run_log,
            word_list,
            word_count,
            ai_racer_count,
            ai_difficulty,
            item_registry,
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
        let mut run_log = RunLog::new(player.started_at, 500);
        let item_registry = ItemRegistry::builtin();
        run_log.push(
            player.started_at,
            format!("test session created words={word_count}"),
        );

        Self {
            track,
            player,
            ai_racers: Vec::new(),
            bonuses,
            bonus_attempt: None,
            attack_warning: None,
            player_impact_until: None,
            player_item_cue: None,
            race_status: RaceStatus::default(),
            race_phase: RacePhase::Racing,
            events,
            run_log,
            word_list,
            word_count,
            ai_racer_count: 0,
            ai_difficulty: AiDifficulty::Easy,
            item_registry,
        }
    }

    pub fn apply_action(&mut self, action: LocalAction, now: Instant) {
        if self.race_status.is_ended() && action != LocalAction::Restart {
            return;
        }

        if self.handle_prerace_action(action, now) {
            return;
        }

        if !self.race_phase.is_racing() && action != LocalAction::Restart {
            return;
        }

        match action {
            LocalAction::Typing(action) => self.apply_typing_action(action, now),
            LocalAction::ActivateItem => self.activate_item(ItemUse::Normal, now),
            LocalAction::ActivateModifiedItem => self.activate_item(ItemUse::Modified, now),
            LocalAction::Restart => self.restart(now),
        }
        self.update_race_status(now);
    }

    fn handle_prerace_action(&mut self, action: LocalAction, now: Instant) -> bool {
        match (self.race_phase, action) {
            (_, LocalAction::Restart) => {
                self.restart(now);
                true
            }
            (RacePhase::WaitingForHost, LocalAction::Typing(KeyAction::Space)) => {
                self.start_countdown(now);
                true
            }
            (RacePhase::WaitingForHost | RacePhase::Countdown { .. }, _) => true,
            (RacePhase::Racing, _) => false,
        }
    }

    fn start_countdown(&mut self, now: Instant) {
        self.race_phase = RacePhase::Countdown {
            starts_at: now + RACE_COUNTDOWN,
        };
        self.events.push("Race starts in 3");
        self.run_log.push(now, "host started countdown");
    }

    fn start_race(&mut self, now: Instant) {
        self.race_phase = RacePhase::Racing;
        self.player.started_at = now;
        for ai in &mut self.ai_racers {
            ai.player.started_at = now;
            ai.last_update = now;
            ai.char_budget = 0.0;
        }
        self.events.push("Go");
        self.run_log.push(now, "race started");
    }

    pub fn restart(&mut self, now: Instant) {
        let Ok(track) = Track::generate(&self.word_list, self.word_count) else {
            self.events.push("Restart failed");
            self.run_log.push(now, "restart failed: track generation");
            return;
        };
        let player = PlayerState::new(now);
        let bonuses = BonusState::generate(&track, &self.word_list);
        let ai_racers = build_ai_racers(self.ai_racer_count, self.ai_difficulty, now);

        self.track = track;
        self.player = player;
        self.ai_racers = ai_racers;
        self.bonuses = bonuses;
        self.bonus_attempt = None;
        self.attack_warning = None;
        self.player_impact_until = None;
        self.player_item_cue = None;
        self.race_status = RaceStatus::default();
        self.race_phase = RacePhase::WaitingForHost;
        self.events = EventLog::new(8);
        self.events.push("Press Space to start");
        self.run_log.push(
            now,
            format!(
                "restart words={} ai_racers={} difficulty={:?}",
                self.word_count, self.ai_racer_count, self.ai_difficulty
            ),
        );
    }

    pub fn tick(&mut self, now: Instant) {
        if self.race_status.is_ended() {
            return;
        }

        if let RacePhase::Countdown { starts_at } = self.race_phase {
            if now >= starts_at {
                self.start_race(starts_at);
            }
        }

        if !self.race_phase.is_racing() {
            return;
        }

        self.advance_mushroom(now);
        self.advance_ai_mushrooms(now);
        self.tick_ai_racers(now);

        let expired_choices = self.bonuses.expire_cooldowns(&self.track, now);
        if expired_choices > 0 {
            self.events.push("Bonus refreshed");
        }

        let expired_effects = self.player.expire_effects(now);
        if expired_effects > 0 {
            self.events.push("Shield expired");
        }

        self.expire_item_cues(now);

        if self
            .attack_warning
            .is_some_and(|warning| warning.resolves_at <= now)
        {
            self.apply_banana_to_player(now);
            self.attack_warning = None;
        }

        self.update_race_status(now);
    }

    fn expire_item_cues(&mut self, now: Instant) {
        if self.player_item_cue.is_some_and(|cue| !cue.is_visible(now)) {
            self.player_item_cue = None;
        }
        for ai in &mut self.ai_racers {
            if ai.item_cue.is_some_and(|cue| !cue.is_visible(now)) {
                ai.item_cue = None;
            }
        }
    }

    fn apply_banana_to_player(&mut self, now: Instant) {
        if self.player.has_active_shield(now) {
            self.player.active_effects.clear();
            self.events.push("Attack blocked");
            return;
        }

        self.player.input.clear();
        self.player.typo_index = None;
        self.bonus_attempt = None;
        self.player_impact_until = Some(now + ITEM_IMPACT_BLINK);
        self.events.push("Banana spun you out");
    }

    fn update_race_status(&mut self, now: Instant) {
        if self.race_status.ended_at.is_some() {
            return;
        }

        if self.race_status.first_finished_at.is_none() {
            self.race_status.first_finished_at = self.first_finished_at();
        }

        let Some(first_finished_at) = self.race_status.first_finished_at else {
            return;
        };

        if self.all_racers_finished()
            || now.saturating_duration_since(first_finished_at) >= POST_FIRST_FINISH_TIMEOUT
        {
            self.race_status.ended_at = Some(now);
            self.events.push("Race ended");
        }
    }

    fn first_finished_at(&self) -> Option<Instant> {
        std::iter::once(self.player.finished_at)
            .chain(self.ai_racers.iter().map(|ai| ai.player.finished_at))
            .flatten()
            .min()
    }

    fn all_racers_finished(&self) -> bool {
        self.player.is_finished() && self.ai_racers.iter().all(|ai| ai.player.is_finished())
    }

    fn tick_ai_racers(&mut self, now: Instant) {
        for ai_index in 0..self.ai_racers.len() {
            self.ai_try_claim_bonus(ai_index, now);
            self.ai_use_item(ai_index, now);
            self.advance_ai_typing(ai_index, now);
        }
    }

    fn advance_ai_typing(&mut self, ai_index: usize, now: Instant) {
        let Some(ai) = self.ai_racers.get_mut(ai_index) else {
            return;
        };

        let elapsed = now.saturating_duration_since(ai.last_update);
        ai.last_update = now;

        if ai.player.is_finished() || ai.is_stunned(now) {
            return;
        }

        ai.char_budget += elapsed.as_secs_f64() * ai_chars_per_second(ai.words_per_minute);
        while ai.char_budget >= 1.0 && !ai.player.is_finished() {
            let Some(action) = next_ai_key(&ai.player, &self.track) else {
                break;
            };
            let events = apply_key(&mut ai.player, &self.track, action, now);
            ai.char_budget -= 1.0;

            if events
                .iter()
                .any(|event| matches!(event, TypingEvent::RaceFinished))
            {
                self.events.push(format!("{} finished", ai.name));
                break;
            }
        }
    }

    fn ai_try_claim_bonus(&mut self, ai_index: usize, now: Instant) {
        let Some(ai) = self.ai_racers.get(ai_index) else {
            return;
        };
        if ai.player.held_item.is_some()
            || ai.player.has_active_shield(now)
            || player_has_active_mushroom(&ai.player)
            || ai.item_cue.is_some_and(|cue| cue.is_visible(now))
            || ai.player.is_finished()
            || ai.player.typo_index.is_some()
            || !ai.player.input.is_empty()
        {
            return;
        }

        let Some((point_index, point)) = self.bonuses.point_for_gap(ai.player.word_index) else {
            return;
        };
        let available_choices = point
            .choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| choice.is_available(now))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if available_choices.is_empty() {
            return;
        }

        let mut rng = thread_rng();
        let choice_index = available_choices[rng.gen_range(0..available_choices.len())];
        let has_nearby_racer = self.ai_has_nearby_racer(ai_index, 5);
        let Some(item) = claim_bonus_choice(
            &mut self.bonuses,
            point_index,
            choice_index,
            now,
            has_nearby_racer,
            &self.item_registry,
            &mut rng,
        ) else {
            return;
        };

        self.receive_ai_pickup(ai_index, item, now);
    }

    fn ai_has_nearby_racer(&self, ai_index: usize, max_distance_words: usize) -> bool {
        let Some(ai) = self.ai_racers.get(ai_index) else {
            return false;
        };
        (!self.player.is_finished()
            && self.player.word_index.abs_diff(ai.player.word_index) <= max_distance_words)
            || self
                .ai_racers
                .iter()
                .enumerate()
                .any(|(other_index, other)| {
                    other_index != ai_index
                        && !other.player.is_finished()
                        && other.player.word_index.abs_diff(ai.player.word_index)
                            <= max_distance_words
                })
    }

    fn receive_ai_pickup(&mut self, ai_index: usize, item: ItemPickup, now: Instant) {
        match item {
            ItemPickup::Held(held_item) => {
                let Some(ai) = self.ai_racers.get(ai_index) else {
                    return;
                };
                let ai_name = ai.name.clone();
                self.events
                    .push(format!("{ai_name} picked up {}", held_item.name()));
                self.run_log.push(
                    now,
                    format!(
                        "{ai_name} picked up {} at word={}",
                        held_item.name(),
                        ai.player.word_index
                    ),
                );
                self.activate_ai_held_item(ai_index, held_item, now);
            }
            ItemPickup::Shield => {
                let Some(ai) = self.ai_racers.get_mut(ai_index) else {
                    return;
                };
                let ai_name = ai.name.clone();
                let word_index = ai.player.word_index;
                ai.player.active_effects.push(ActiveEffect::Shield {
                    until: now + Duration::from_secs(5),
                });
                self.events.push(format!("{} shielded", ai.name));
                self.run_log.push(
                    now,
                    format!("{ai_name} picked up Shield at word={word_index}"),
                );
            }
        }
    }

    fn activate_ai_held_item(&mut self, ai_index: usize, item: HeldItem, now: Instant) {
        match item {
            HeldItem::Mushroom => self.activate_ai_mushroom(ai_index, now),
            HeldItem::Banana => self.ai_use_banana(ai_index, now),
        }
    }

    fn ai_use_item(&mut self, ai_index: usize, now: Instant) {
        let Some(item) = self.ai_racers[ai_index].player.held_item else {
            return;
        };

        match item {
            HeldItem::Mushroom => {
                self.ai_racers[ai_index].player.held_item = None;
                self.activate_ai_held_item(ai_index, item, now);
            }
            HeldItem::Banana => {
                self.ai_racers[ai_index].player.held_item = None;
                self.activate_ai_held_item(ai_index, item, now);
            }
        }
    }

    fn ai_use_banana(&mut self, ai_index: usize, now: Instant) {
        let Some(ai) = self.ai_racers.get(ai_index) else {
            return;
        };
        let ai_name = ai.name.clone();
        let attacker_word_index = ai.player.word_index;
        let racers = self.racer_positions_excluding_ai(ai_index, now);
        self.run_log.push(
            now,
            format!(
                "{ai_name} banana fired from word={attacker_word_index}; candidates={}",
                self.racer_positions_summary(&racers, now)
            ),
        );
        let Some(target) = select_nearest_banana_target(ai.player.word_index, &racers, 10) else {
            self.events.push(format!("{} missed Banana", ai.name));
            self.run_log.push(now, format!("{ai_name} banana missed"));
            return;
        };
        let direction = attack_direction(attacker_word_index, target.word_index);
        let distance = attacker_word_index.abs_diff(target.word_index);
        self.run_log.push(
            now,
            format!(
                "{ai_name} banana target={} target_word={} direction={direction:?} cue_placement={} distance_words={distance}",
                racer_name(target.id),
                target.word_index,
                banana_cue_placement(direction)
            ),
        );

        if target.id == 0 {
            if let Some(ai) = self.ai_racers.get_mut(ai_index) {
                ai.item_cue = Some(ItemCue::new(ItemCueKind::Banana { direction }, now));
            }
            self.attack_warning = Some(AttackWarning {
                attack: PendingAttack::BananaWordSwap,
                resolves_at: now + PLAYER_ATTACK_WARNING,
            });
            self.events.push(format!("{ai_name} threw Banana at you"));
        } else {
            if let Some(ai) = self.ai_racers.get_mut(ai_index) {
                ai.item_cue = Some(ItemCue::new(ItemCueKind::Banana { direction }, now));
            }
            if self.apply_banana_to_ai(target.id, now) == Some(BananaResolution::SpunOut) {
                self.events.push(format!("{ai_name} hit ai-{}", target.id));
            }
        }
    }

    fn racer_positions_excluding_ai(&self, ai_index: usize, now: Instant) -> Vec<RacerPosition> {
        let mut racers = Vec::new();
        if !self.player.is_finished() {
            racers.push(RacerPosition {
                id: 0,
                word_index: self.player.word_index,
            });
        }
        racers.extend(
            self.ai_racers
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != ai_index)
                .filter(|(_, ai)| !ai.player.is_finished())
                .filter(|(_, ai)| !ai.is_stunned(now))
                .map(|(_, ai)| RacerPosition {
                    id: ai.id,
                    word_index: ai.player.word_index,
                }),
        );
        racers
    }

    fn racer_positions_summary(&self, racers: &[RacerPosition], now: Instant) -> String {
        if racers.is_empty() {
            return "none".to_string();
        }

        racers
            .iter()
            .map(|racer| {
                if racer.id == 0 {
                    format!(
                        "player@{} shield={} finished={}",
                        racer.word_index,
                        self.player.has_active_shield(now),
                        self.player.is_finished()
                    )
                } else if let Some(ai) = self.ai_racers.iter().find(|ai| ai.id == racer.id) {
                    format!(
                        "{}@{} shield={} stunned={} finished={}",
                        ai.name,
                        racer.word_index,
                        ai.player.has_active_shield(now),
                        ai.is_stunned(now),
                        ai.player.is_finished()
                    )
                } else {
                    format!("ai-{}@{} missing", racer.id, racer.word_index)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn apply_banana_to_ai(&mut self, ai_id: usize, now: Instant) -> Option<BananaResolution> {
        let Some(ai) = self.ai_racers.iter_mut().find(|ai| ai.id == ai_id) else {
            self.run_log.push(
                now,
                format!("banana target ai-{ai_id} missing from session"),
            );
            return None;
        };
        let ai_name = ai.name.clone();
        let word_index = ai.player.word_index;

        if ai.player.has_active_shield(now) {
            ai.player.active_effects.clear();
            self.events.push(format!("{} blocked Banana", ai_name));
            self.run_log.push(
                now,
                format!("{ai_name} blocked Banana at word={word_index}; shield consumed"),
            );
            Some(BananaResolution::Blocked)
        } else {
            ai.stunned_until = Some(now + AI_BANANA_STUN);
            ai.impact_until = Some(now + ITEM_IMPACT_BLINK);
            ai.char_budget = 0.0;
            self.events.push(format!("{} spun out", ai_name));
            self.run_log.push(
                now,
                format!(
                    "{ai_name} spun out at word={word_index}; stun_ms={} impact_blink_ms={}",
                    AI_BANANA_STUN.as_millis(),
                    ITEM_IMPACT_BLINK.as_millis()
                ),
            );
            Some(BananaResolution::SpunOut)
        }
    }

    fn activate_ai_mushroom(&mut self, ai_index: usize, now: Instant) {
        let Some(ai) = self.ai_racers.get_mut(ai_index) else {
            return;
        };
        ai.player.input.clear();
        ai.player.typo_index = None;
        ai.player.active_effects.push(ActiveEffect::Mushroom {
            remaining_words: MUSHROOM_BOOST_WORDS,
            next_step_at: now,
            step_interval: mushroom_step_interval(),
        });
        self.events.push(format!("{} used Mushroom", ai.name));
        self.advance_ai_mushroom(ai_index, now);
    }

    fn advance_ai_mushrooms(&mut self, now: Instant) {
        for ai_index in 0..self.ai_racers.len() {
            loop {
                if !self.advance_ai_mushroom(ai_index, now) {
                    break;
                }
                if self.ai_racers[ai_index].player.is_finished() {
                    break;
                }
            }
        }
    }

    fn advance_ai_mushroom(&mut self, ai_index: usize, now: Instant) -> bool {
        let Some(effect_index) = self.ai_racers[ai_index]
            .player
            .active_effects
            .iter()
            .position(|effect| {
                matches!(
                    effect,
                    ActiveEffect::Mushroom {
                        remaining_words,
                        next_step_at,
                        ..
                    } if *remaining_words > 0 && *next_step_at <= now
                )
            })
        else {
            return false;
        };

        let ai = &mut self.ai_racers[ai_index];
        let remaining_track_words = self.track.len().saturating_sub(ai.player.word_index);
        if remaining_track_words == 0 {
            ai.player.active_effects.remove(effect_index);
            return false;
        }

        ai.player.word_index += 1;
        ai.player.stats.completed_words += 1;
        ai.player.input.clear();
        ai.player.typo_index = None;

        if ai.player.word_index >= self.track.len() {
            ai.player.finished_at = Some(now);
            ai.player.active_effects.remove(effect_index);
            self.events.push(format!("{} finished", ai.name));
            return false;
        }

        if let Some(ActiveEffect::Mushroom {
            remaining_words,
            next_step_at,
            step_interval,
        }) = ai.player.active_effects.get_mut(effect_index)
        {
            *remaining_words -= 1;
            if *remaining_words == 0 {
                ai.player.active_effects.remove(effect_index);
            } else {
                *next_step_at += *step_interval;
            }
        }

        true
    }

    fn apply_typing_action(&mut self, action: KeyAction, now: Instant) {
        if player_has_active_mushroom(&self.player) {
            return;
        }

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
                    self.apply_bonus_char(ch);
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
            KeyAction::Char(ch) => self.apply_bonus_char(ch),
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
            KeyAction::Space => {
                if self.completed_bonus_word_without_typo() {
                    if let Some(attempt) = self.bonus_attempt {
                        self.claim_bonus(attempt, now);
                    }
                } else {
                    self.apply_bonus_char(' ');
                }
            }
        }
    }

    fn apply_bonus_char(&mut self, ch: char) {
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
    }

    fn completed_bonus_word_without_typo(&self) -> bool {
        let Some(attempt) = self.bonus_attempt else {
            return false;
        };
        let Some(target) = self
            .bonuses
            .points
            .get(attempt.point_index)
            .and_then(|point| point.choices.get(attempt.choice_index))
            .map(|choice| choice.word.as_str())
        else {
            return false;
        };

        self.player.typo_index.is_none() && self.player.input == target
    }

    fn claim_bonus(&mut self, attempt: BonusAttempt, now: Instant) {
        let mut rng = thread_rng();
        let Some(item) = claim_bonus_choice(
            &mut self.bonuses,
            attempt.point_index,
            attempt.choice_index,
            now,
            false,
            &self.item_registry,
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
            && !player_has_active_mushroom(&self.player)
            && !self.player_item_cue.is_some_and(|cue| cue.is_visible(now))
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

        self.player.held_item = None;
        self.activate_held_item(item, item_use, now);
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
                self.events.push(format!("Picked up {}", held_item.name()));
                self.run_log.push(
                    now,
                    format!(
                        "player picked up {} at word={}",
                        held_item.name(),
                        self.player.word_index
                    ),
                );
                self.activate_held_item(held_item, ItemUse::Normal, now);
            }
            ItemPickup::Shield => {
                self.activate_shield(now);
                self.events.push("Picked up Shield");
                self.run_log.push(
                    now,
                    format!("player picked up Shield at word={}", self.player.word_index),
                );
            }
        }
    }

    fn activate_held_item(&mut self, item: HeldItem, _item_use: ItemUse, now: Instant) {
        match item {
            HeldItem::Mushroom => self.activate_mushroom(now),
            HeldItem::Banana => {
                let racers = self
                    .ai_racers
                    .iter()
                    .filter(|ai| !ai.player.is_finished())
                    .filter(|ai| !ai.is_stunned(now))
                    .map(|ai| RacerPosition {
                        id: ai.id,
                        word_index: ai.player.word_index,
                    })
                    .collect::<Vec<_>>();
                self.run_log.push(
                    now,
                    format!(
                        "player banana fired from word={}; candidates={}",
                        self.player.word_index,
                        self.racer_positions_summary(&racers, now)
                    ),
                );
                if let Some(target) =
                    select_nearest_banana_target(self.player.word_index, &racers, 10)
                {
                    let direction = attack_direction(self.player.word_index, target.word_index);
                    let distance = self.player.word_index.abs_diff(target.word_index);
                    self.run_log.push(
                        now,
                        format!(
                            "player banana target={} target_word={} direction={direction:?} cue_placement={} distance_words={distance}",
                            racer_name(target.id),
                            target.word_index,
                            banana_cue_placement(direction)
                        ),
                    );
                    self.player_item_cue =
                        Some(ItemCue::new(ItemCueKind::Banana { direction }, now));
                    if self.apply_banana_to_ai(target.id, now) == Some(BananaResolution::SpunOut) {
                        self.events.push(format!("Hit ai-{}", target.id));
                    }
                } else {
                    self.events.push("No racer in range");
                    self.run_log
                        .push(now, "player banana missed: no racer in range");
                }
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

fn ai_chars_per_second(words_per_minute: f64) -> f64 {
    words_per_minute * 5.0 / 60.0
}

fn attack_direction(attacker_word_index: usize, target_word_index: usize) -> AttackDirection {
    match target_word_index.cmp(&attacker_word_index) {
        std::cmp::Ordering::Greater => AttackDirection::Ahead,
        std::cmp::Ordering::Equal => AttackDirection::Overlap,
        std::cmp::Ordering::Less => AttackDirection::Behind,
    }
}

fn banana_cue_placement(direction: AttackDirection) -> &'static str {
    match direction {
        AttackDirection::Ahead => "after",
        AttackDirection::Behind => "before",
        // Same-word attacks are drawn after the marker, but the distinct
        // direction keeps logs and future rendering from calling them "ahead".
        AttackDirection::Overlap => "after-overlap",
    }
}

fn racer_name(id: usize) -> String {
    if id == 0 {
        "player".to_string()
    } else {
        format!("ai-{id}")
    }
}

fn player_has_active_mushroom(player: &PlayerState) -> bool {
    player.active_effects.iter().any(|effect| {
        matches!(
            effect,
            ActiveEffect::Mushroom {
                remaining_words,
                ..
            } if *remaining_words > 0
        )
    })
}

fn next_ai_key(player: &PlayerState, track: &Track) -> Option<KeyAction> {
    let target = track.current_word(player.word_index)?;
    if player.input == target {
        return Some(KeyAction::Space);
    }

    target
        .chars()
        .nth(player.input.chars().count())
        .map(KeyAction::Char)
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

#[derive(Debug, Clone)]
pub struct RunLog {
    started_at: Instant,
    entries: VecDeque<String>,
    capacity: usize,
}

impl RunLog {
    pub fn new(started_at: Instant, capacity: usize) -> Self {
        Self {
            started_at,
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, now: Instant, message: impl Into<String>) {
        if self.capacity == 0 {
            return;
        }

        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }

        let elapsed_ms = now.saturating_duration_since(self.started_at).as_millis();
        self.entries
            .push_back(format!("+{elapsed_ms:>6}ms {}", message.into()));
    }

    pub fn entries(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::{
        game::{
            ai::AiDifficulty,
            bonus::{BonusChoice, BonusPoint, BonusState},
            effects::ActiveEffect,
            items::{HeldItem, ItemPickup, ItemRegistry},
            mods::{ActiveModConfig, ContentMetadata},
            player::PlayerState,
            track::{Track, WordList},
            typing::KeyAction,
            words::WordSetDefinition,
        },
        ui::session::{
            AiRacer, AttackDirection, EventLog, ItemCue, ItemCueKind, LocalAction, LocalSession,
            RacePhase,
        },
    };

    fn track(words: &[&str]) -> Track {
        Track::new(words.iter().map(|word| word.to_string()).collect())
    }

    fn word_list() -> WordList {
        WordList {
            words: vec![
                "alpha".to_string(),
                "bravo".to_string(),
                "charlie".to_string(),
                "delta".to_string(),
                "echo".to_string(),
                "foxtrot".to_string(),
                "golf".to_string(),
                "hotel".to_string(),
            ],
        }
    }

    fn test_active_mod_config() -> ActiveModConfig {
        let item_registry = ItemRegistry::builtin();
        ActiveModConfig::new(
            &WordSetDefinition {
                metadata: ContentMetadata::built_in("classic", "Classic"),
                words: word_list(),
            },
            &item_registry,
            None,
        )
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
    fn race_waits_for_host_space_before_accepting_typing() {
        let now = Instant::now();
        let mut session = LocalSession::new(
            track(&["one", "two"]),
            PlayerState::new(now),
            word_list(),
            0,
            AiDifficulty::Easy,
            ItemRegistry::builtin(),
            test_active_mod_config(),
        );

        session.apply_action(LocalAction::Typing(KeyAction::Char('o')), now);

        assert_eq!(session.race_phase, RacePhase::WaitingForHost);
        assert!(session.player.input.is_empty());
    }

    #[test]
    fn host_space_starts_countdown_before_race_begins() {
        let now = Instant::now();
        let mut session = LocalSession::new(
            track(&["one", "two"]),
            PlayerState::new(now),
            word_list(),
            1,
            AiDifficulty::Hard,
            ItemRegistry::builtin(),
            test_active_mod_config(),
        );

        session.apply_action(LocalAction::Typing(KeyAction::Space), now);
        session.apply_action(
            LocalAction::Typing(KeyAction::Char('o')),
            now + std::time::Duration::from_secs(1),
        );
        session.tick(now + std::time::Duration::from_secs(1));

        assert!(matches!(session.race_phase, RacePhase::Countdown { .. }));
        assert!(session.player.input.is_empty());
        assert!(session.ai_racers[0].player.input.is_empty());

        session.tick(now + std::time::Duration::from_secs(3));
        session.apply_action(
            LocalAction::Typing(KeyAction::Char('o')),
            now + std::time::Duration::from_secs(3),
        );

        let started_at = now + std::time::Duration::from_secs(3);
        assert_eq!(session.race_phase, RacePhase::Racing);
        assert_eq!(session.player.started_at, started_at);
        assert_eq!(session.ai_racers[0].player.started_at, started_at);
        assert_eq!(session.player.input, "o");
    }

    #[test]
    fn race_ends_when_all_racers_finish() {
        let now = Instant::now();
        let mut session = LocalSession::with_bonuses(
            track(&["a"]),
            PlayerState::new(now),
            BonusState::with_points(vec![], vec![]),
        );

        session.apply_action(LocalAction::Typing(KeyAction::Char('a')), now);

        assert!(session.player.is_finished());
        assert!(session.race_status.is_ended());
    }

    #[test]
    fn race_ends_after_post_first_finish_timeout() {
        let now = Instant::now();
        let mut session = LocalSession::with_bonuses(
            track(&["a", "b"]),
            PlayerState::new(now),
            BonusState::with_points(vec![], vec![]),
        );
        session
            .ai_racers
            .push(AiRacer::new(1, AiDifficulty::Easy, 35.0, now));

        session.apply_action(LocalAction::Typing(KeyAction::Char('a')), now);
        session.apply_action(LocalAction::Typing(KeyAction::Space), now);
        session.apply_action(LocalAction::Typing(KeyAction::Char('b')), now);
        assert!(session.player.is_finished());
        assert!(!session.race_status.is_ended());

        session.tick(now + std::time::Duration::from_secs(16));

        assert!(session.race_status.is_ended());
    }

    #[test]
    fn local_session_caps_ai_racers_at_six() {
        let now = Instant::now();
        let session = LocalSession::new(
            track(&["one", "two"]),
            PlayerState::new(now),
            word_list(),
            8,
            AiDifficulty::Easy,
            ItemRegistry::builtin(),
            test_active_mod_config(),
        );

        assert_eq!(session.ai_racers.len(), 6);
    }

    #[test]
    fn ai_racers_sample_wpm_from_difficulty_range() {
        let now = Instant::now();
        let session = LocalSession::new(
            track(&["one", "two"]),
            PlayerState::new(now),
            word_list(),
            3,
            AiDifficulty::Hard,
            ItemRegistry::builtin(),
            test_active_mod_config(),
        );

        assert!(session.ai_racers.iter().all(|ai| {
            AiDifficulty::Hard
                .wpm_range()
                .contains(&ai.words_per_minute)
        }));
    }

    #[test]
    fn ai_racer_advances_from_wpm_budget() {
        let now = Instant::now();
        let mut session = LocalSession::new(
            track(&["abcdef", "road"]),
            PlayerState::new(now),
            word_list(),
            1,
            AiDifficulty::Hard,
            ItemRegistry::builtin(),
            test_active_mod_config(),
        );

        session.apply_action(LocalAction::Typing(KeyAction::Space), now);
        session.tick(now + std::time::Duration::from_secs(3));
        session.tick(now + std::time::Duration::from_secs(5));

        assert!(
            !session.ai_racers[0].player.input.is_empty()
                || session.ai_racers[0].player.stats.completed_words > 0
        );
    }

    #[test]
    fn player_banana_can_stun_ai_target() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 1;
        session.player.held_item = Some(HeldItem::Banana);
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.word_index = 2;
        session.ai_racers.push(ai);

        session.apply_action(LocalAction::ActivateModifiedItem, now);

        assert!(session.ai_racers[0].is_stunned(now));
        assert!(session.ai_racers[0].is_impacted(now));
        assert_eq!(session.player.held_item, None);
    }

    #[test]
    fn player_banana_targets_nearest_ai_regardless_of_activation_variant() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 10;
        session.player.held_item = Some(HeldItem::Banana);

        let mut closer_behind = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        closer_behind.player.word_index = 9;
        session.ai_racers.push(closer_behind);

        let mut farther_ahead = AiRacer::new(2, AiDifficulty::Easy, 35.0, now);
        farther_ahead.player.word_index = 12;
        session.ai_racers.push(farther_ahead);

        session.apply_action(LocalAction::ActivateModifiedItem, now);

        assert!(session.ai_racers[0].is_stunned(now));
        assert!(!session.ai_racers[1].is_stunned(now));
        assert_eq!(
            session.player_item_cue.unwrap().kind,
            ItemCueKind::Banana {
                direction: AttackDirection::Behind
            }
        );
    }

    #[test]
    fn player_banana_ignores_finished_ai_targets() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 10;
        session.player.held_item = Some(HeldItem::Banana);

        let mut finished_ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        finished_ai.player.word_index = 10;
        finished_ai.player.finished_at = Some(now);
        session.ai_racers.push(finished_ai);

        let mut active_ai = AiRacer::new(2, AiDifficulty::Easy, 35.0, now);
        active_ai.player.word_index = 12;
        session.ai_racers.push(active_ai);

        session.apply_action(LocalAction::ActivateItem, now);

        assert!(!session.ai_racers[0].is_stunned(now));
        assert!(session.ai_racers[1].is_stunned(now));
    }

    #[test]
    fn player_banana_ignores_stunned_ai_targets() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 10;
        session.player.held_item = Some(HeldItem::Banana);

        let mut stunned_ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        stunned_ai.player.word_index = 10;
        stunned_ai.stunned_until = Some(now + std::time::Duration::from_secs(1));
        session.ai_racers.push(stunned_ai);

        let mut active_ai = AiRacer::new(2, AiDifficulty::Easy, 35.0, now);
        active_ai.player.word_index = 12;
        session.ai_racers.push(active_ai);

        session.apply_action(LocalAction::ActivateItem, now);

        assert!(session.ai_racers[0].is_stunned(now));
        assert!(session.ai_racers[1].is_stunned(now));
        assert!(session
            .run_log
            .entries()
            .any(|entry| entry.contains("player banana target=ai-2")));
    }

    #[test]
    fn player_banana_reports_overlap_direction_for_same_word_target() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 10;
        session.player.held_item = Some(HeldItem::Banana);

        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.word_index = 10;
        session.ai_racers.push(ai);

        session.apply_action(LocalAction::ActivateItem, now);

        assert_eq!(
            session.player_item_cue.unwrap().kind,
            ItemCueKind::Banana {
                direction: AttackDirection::Overlap
            }
        );
        assert!(session
            .run_log
            .entries()
            .any(|entry| entry.contains("direction=Overlap cue_placement=after-overlap")));
    }

    #[test]
    fn shielded_ai_blocks_player_banana_without_hit_event() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 1;
        session.player.held_item = Some(HeldItem::Banana);

        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.word_index = 2;
        ai.player.active_effects.push(ActiveEffect::Shield {
            until: now + std::time::Duration::from_secs(1),
        });
        session.ai_racers.push(ai);

        session.apply_action(LocalAction::ActivateItem, now);

        let entries = session.events.entries().collect::<Vec<_>>();
        assert!(entries.contains(&"ai-1 blocked Banana"));
        assert!(!entries.contains(&"Hit ai-1"));
        assert!(!session.ai_racers[0].is_stunned(now));
        assert!(session.ai_racers[0].player.active_effects.is_empty());
        assert!(session
            .run_log
            .entries()
            .any(|entry| entry.contains("ai-1 blocked Banana")));
    }

    #[test]
    fn ai_banana_warning_can_clear_player_input() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 1;
        session.player.input = "t".to_string();
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.held_item = Some(HeldItem::Banana);
        session.ai_racers.push(ai);

        session.tick(now);
        assert!(session.attack_warning.is_some());

        session.tick(now + std::time::Duration::from_secs(1));

        assert!(session.player.input.is_empty());
        assert!(session.attack_warning.is_none());
        assert!(session.player_impact_until.is_some_and(|until| until > now));
    }

    #[test]
    fn ai_banana_ignores_finished_player_target() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 1;
        session.player.finished_at = Some(now);

        let mut attacker = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        attacker.player.word_index = 1;
        attacker.player.held_item = Some(HeldItem::Banana);
        session.ai_racers.push(attacker);

        let mut active_target = AiRacer::new(2, AiDifficulty::Easy, 35.0, now);
        active_target.player.word_index = 2;
        session.ai_racers.push(active_target);

        session.tick(now);

        assert!(session.attack_warning.is_none());
        assert!(!session.player_impact_until.is_some_and(|until| until > now));
        assert!(session.ai_racers[1].is_stunned(now));
    }

    #[test]
    fn ai_banana_ignores_stunned_ai_targets() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());

        let mut attacker = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        attacker.player.word_index = 10;
        attacker.player.held_item = Some(HeldItem::Banana);
        session.ai_racers.push(attacker);

        let mut stunned_target = AiRacer::new(2, AiDifficulty::Easy, 35.0, now);
        stunned_target.player.word_index = 10;
        stunned_target.stunned_until = Some(now + std::time::Duration::from_secs(1));
        session.ai_racers.push(stunned_target);

        let mut active_target = AiRacer::new(3, AiDifficulty::Easy, 35.0, now);
        active_target.player.word_index = 11;
        session.ai_racers.push(active_target);

        session.tick(now);

        assert!(session.ai_racers[1].is_stunned(now));
        assert!(session.ai_racers[2].is_stunned(now));
        assert!(session
            .run_log
            .entries()
            .any(|entry| entry.contains("ai-1 banana target=ai-3")));
    }

    #[test]
    fn ai_can_claim_bonus_pickup() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.word_index = 1;
        session.ai_racers.push(ai);

        session.tick(now);

        assert!(session.bonuses.points[0].choices.iter().any(|choice| {
            matches!(
                choice.status,
                crate::game::bonus::BonusChoiceStatus::Cooldown { .. }
            )
        }));
    }

    #[test]
    fn ai_cannot_claim_bonus_while_item_cue_is_visible() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.word_index = 1;
        ai.item_cue = Some(ItemCue::new(
            ItemCueKind::Banana {
                direction: AttackDirection::Ahead,
            },
            now,
        ));
        session.ai_racers.push(ai);

        session.tick(now);

        assert!(session.bonuses.points[0].choices.iter().all(|choice| {
            matches!(
                choice.status,
                crate::game::bonus::BonusChoiceStatus::Available
            )
        }));
    }

    #[test]
    fn ai_cannot_claim_bonus_while_mushroom_is_active() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.word_index = 1;
        ai.player.active_effects.push(ActiveEffect::Mushroom {
            remaining_words: 2,
            next_step_at: now + std::time::Duration::from_secs(1),
            step_interval: std::time::Duration::from_millis(400),
        });
        session.ai_racers.push(ai);

        session.tick(now);

        assert!(session.bonuses.points[0].choices.iter().all(|choice| {
            matches!(
                choice.status,
                crate::game::bonus::BonusChoiceStatus::Available
            )
        }));
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
        assert_eq!(session.race_phase, RacePhase::WaitingForHost);
        assert_eq!(
            session.events.entries().collect::<Vec<_>>(),
            vec!["Press Space to start"]
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

        assert!(session.bonus_attempt.is_some());
        assert_eq!(session.player.input, "drift");

        session.apply_action(LocalAction::Typing(KeyAction::Space), Instant::now());

        assert!(session.bonuses.points[0].choices.iter().any(|choice| {
            matches!(
                choice.status,
                crate::game::bonus::BonusChoiceStatus::Cooldown { .. }
            )
        }));
        assert!(session.player.input.is_empty());
    }

    #[test]
    fn held_pickup_auto_activates_immediately() {
        let track = track(&["one", "two", "three", "four"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));

        session.receive_pickup(ItemPickup::Held(HeldItem::Mushroom), Instant::now());

        assert_eq!(session.player.held_item, None);
        assert_eq!(session.player.word_index, 1);
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
        assert!(session
            .player
            .active_effects
            .iter()
            .any(|effect| matches!(effect, ActiveEffect::Mushroom { .. })));

        session.tick(now + std::time::Duration::from_secs_f64(0.4));
        assert_eq!(session.player.word_index, 2);

        session.tick(now + std::time::Duration::from_secs_f64(0.8));
        assert_eq!(session.player.word_index, 3);
        assert!(!session
            .player
            .active_effects
            .iter()
            .any(|effect| matches!(effect, ActiveEffect::Mushroom { .. })));
    }

    #[test]
    fn mushroom_pauses_player_typing_until_boost_finishes() {
        let track = track(&["one", "two", "three", "four", "five"]);
        let player = PlayerState::new(Instant::now());
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        session.player.held_item = Some(HeldItem::Mushroom);
        let now = Instant::now();

        session.apply_action(LocalAction::ActivateItem, now);
        session.apply_action(LocalAction::Typing(KeyAction::Char('t')), now);

        assert!(session.player.input.is_empty());

        session.tick(now + std::time::Duration::from_secs_f64(0.8));
        session.apply_action(
            LocalAction::Typing(KeyAction::Char('f')),
            now + std::time::Duration::from_secs_f64(0.8),
        );

        assert_eq!(session.player.input, "f");
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
    fn bonus_is_unavailable_while_item_cue_is_visible() {
        let now = Instant::now();
        let track = track(&["one", "two"]);
        let player = PlayerState::new(now);
        let mut session = LocalSession::with_bonuses(track, player, bonuses());
        session.player.word_index = 1;
        session.player_item_cue = Some(ItemCue::new(
            ItemCueKind::Banana {
                direction: AttackDirection::Ahead,
            },
            now,
        ));

        session.apply_action(LocalAction::Typing(KeyAction::Char('d')), now);

        assert!(session.bonus_attempt.is_none());
        assert_eq!(session.player.input, "d");
    }

    #[test]
    fn bonus_is_unavailable_while_mushroom_is_active() {
        let now = Instant::now();
        let track = track(&["one", "two"]);
        let player = PlayerState::new(now);
        let mut session = LocalSession::with_bonuses(track, player, bonuses());
        session.player.word_index = 1;
        session.player.active_effects.push(ActiveEffect::Mushroom {
            remaining_words: 2,
            next_step_at: now + std::time::Duration::from_secs(1),
            step_interval: std::time::Duration::from_millis(400),
        });

        session.apply_action(LocalAction::Typing(KeyAction::Char('d')), now);

        assert!(session.bonus_attempt.is_none());
        assert!(session.player.input.is_empty());
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
        assert!(session
            .events
            .entries()
            .any(|entry| entry == "No racer in range"));
    }
}
