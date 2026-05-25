//! Local terminal session state.
//!
//! This type coordinates local typing, AI racers, bonus claims, items, timed
//! effects, and display-facing event history.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use rand::{Rng, thread_rng};

use crate::game::{
    ai::AiDifficulty,
    ai_driver::{ai_chars_per_second, ai_effective_wpm, next_ai_key_for_player},
    bonus::{BonusState, claim_bonus_choice},
    effects::ActiveEffect,
    host_session::{
        advance_host_race_lifecycle, begin_countdown_phase, connected_racer_count,
        countdown_should_cancel, countdown_start_plan, countdown_tick_phase,
        prepare_race_from_participants, return_to_lobby_outcome, start_race_from_countdown,
    },
    item_effects::{
        AttackDirection as SharedAttackDirection, RaceImpactCueKind, RaceItemCueKind,
        RaceItemEffectState, activate_item_pickup,
    },
    items::{HeldItem, ItemPickup, ItemRegistry, ItemRollContext, ItemUse, RacePositionBand},
    mods::ActiveModConfig,
    player::PlayerState,
    race::{
        PlayerColorId, RaceLifecycleState, RaceParticipant, RacePlayer, RacePlayerId, RaceState,
    },
    track::{Track, WordList},
    typing::{KeyAction, TypingEvent, apply_key, first_typo_index},
};
use typekart_protocol::NetworkRacePhase;

const MAX_AI_RACERS: usize = 6;
const POST_FIRST_FINISH_TIMEOUT: Duration = Duration::from_secs(15);
const RACE_COUNTDOWN: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub struct LocalSession {
    pub track: Track,
    pub player: PlayerState,
    pub ai_racers: Vec<AiRacer>,
    pub bonuses: BonusState,
    pub bonus_attempt: Option<BonusAttempt>,
    player_stunned_until: Option<Instant>,
    pub player_impact_cue: Option<ImpactCue>,
    pub player_item_cue: Option<ItemCue>,
    pub race_status: RaceStatus,
    pub race_phase: NetworkRacePhase,
    pub events: EventLog,
    pub run_log: RunLog,
    countdown_ends_at: Option<Instant>,
    // Restart needs the same source word list and race length that created the
    // first track. Keeping them here lets the terminal loop reset in place.
    word_list: WordList,
    word_count: usize,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    item_registry: ItemRegistry,
    selected_ai_index: usize,
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
    pub(crate) impact_cue: Option<ImpactCue>,
    pub item_cue: Option<ItemCue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImpactCue {
    pub kind: ImpactCueKind,
    pub until: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactCueKind {
    Banana,
    Cyclone,
    SquidInk,
    ShieldBlock,
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
            impact_cue: None,
            item_cue: None,
        }
    }

    pub fn is_stunned(&self, now: Instant) -> bool {
        self.stunned_until.is_some_and(|until| until > now)
    }

    #[cfg(test)]
    pub fn is_impacted(&self, now: Instant) -> bool {
        self.impact_cue.is_some_and(|cue| cue.until > now)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemCue {
    pub kind: ItemCueKind,
    pub until: Instant,
    pub ascii_label: String,
    pub unicode_label: String,
}

impl ItemCue {
    #[cfg(test)]
    pub(crate) fn new(kind: ItemCueKind, now: Instant) -> Self {
        match kind {
            ItemCueKind::Banana { direction } => {
                Self::banana(direction, now, &ItemRegistry::builtin())
            }
            ItemCueKind::Cyclone { direction } => Self::cyclone(direction, now),
            ItemCueKind::SquidInk => Self::squid_ink(now, &ItemRegistry::builtin()),
        }
    }

    #[cfg(test)]
    fn banana(direction: AttackDirection, now: Instant, item_registry: &ItemRegistry) -> Self {
        let effect = item_registry.banana_effect();
        let display = item_registry.banana_display();
        let (ascii_label, unicode_label) = match direction {
            AttackDirection::Ahead => (display.ascii_ahead, display.unicode_ahead),
            AttackDirection::Behind => (display.ascii_behind, display.unicode_behind),
            AttackDirection::Overlap => (display.ascii_overlap, display.unicode_overlap),
        };

        Self {
            kind: ItemCueKind::Banana { direction },
            until: now + Duration::from_millis(effect.cue_ms),
            ascii_label,
            unicode_label,
        }
    }

    #[cfg(test)]
    fn cyclone(direction: AttackDirection, now: Instant) -> Self {
        let (ascii_label, unicode_label) = match direction {
            AttackDirection::Ahead => (" cy>>".to_string(), " 🌀 >>".to_string()),
            AttackDirection::Behind => ("<<cy ".to_string(), "<< 🌀 ".to_string()),
            AttackDirection::Overlap => (" cy<>".to_string(), " 🌀 <>".to_string()),
        };

        Self {
            kind: ItemCueKind::Cyclone { direction },
            until: now + Duration::from_millis(1_500),
            ascii_label,
            unicode_label,
        }
    }

    #[cfg(test)]
    fn squid_ink(now: Instant, item_registry: &ItemRegistry) -> Self {
        let effect = item_registry.squid_ink_effect();
        Self {
            kind: ItemCueKind::SquidInk,
            until: now + Duration::from_millis(effect.cue_ms),
            ascii_label: " ink ".to_string(),
            unicode_label: " 🦑 ".to_string(),
        }
    }

    pub fn is_visible(&self, now: Instant) -> bool {
        self.until > now
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCueKind {
    Banana { direction: AttackDirection },
    Cyclone { direction: AttackDirection },
    SquidInk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackDirection {
    Ahead,
    Behind,
    Overlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAction {
    Typing(KeyAction),
    ActivateItem,
    ActivateModifiedItem,
    SelectPreviousRacer,
    SelectNextRacer,
    AddAi,
    RemoveSelectedRacer,
    SetSelectedAiDifficulty(AiDifficulty),
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

fn next_local_ai_id(ai_racers: &[AiRacer]) -> usize {
    let mut id = 1;
    while ai_racers.iter().any(|ai| ai.id == id) {
        id += 1;
    }
    id
}

fn local_ai_color(id: usize) -> PlayerColorId {
    match id % 6 {
        1 => PlayerColorId::Red,
        2 => PlayerColorId::Green,
        3 => PlayerColorId::Blue,
        4 => PlayerColorId::Yellow,
        5 => PlayerColorId::Magenta,
        _ => PlayerColorId::Cyan,
    }
}

fn local_race_participants(ai_racers: &[AiRacer]) -> Vec<RaceParticipant> {
    let mut participants = Vec::with_capacity(ai_racers.len() + 1);
    participants.push(RaceParticipant {
        id: RacePlayerId(1),
        name: "you".to_string(),
        color: PlayerColorId::Cyan,
        connected: true,
    });
    participants.extend(ai_racers.iter().map(|ai| RaceParticipant {
        id: RacePlayerId((ai.id + 1) as u64),
        name: ai.name.clone(),
        color: local_ai_color(ai.id),
        connected: true,
    }));
    participants
}

fn apply_prepared_player_states(
    race: &RaceState,
    player: &mut PlayerState,
    ai_racers: &mut [AiRacer],
) {
    if let Some(race_player) = race.player(RacePlayerId(1)) {
        *player = race_player.state.clone();
    }

    for ai in ai_racers {
        if let Some(race_player) = race.player(RacePlayerId((ai.id + 1) as u64)) {
            ai.player = race_player.state.clone();
        }
    }
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

        let word_count = track.len();
        let now = player.started_at;
        let ai_racer_count = ai_racer_count.min(MAX_AI_RACERS);
        let mut ai_racers = build_ai_racers(ai_racer_count, ai_difficulty, now);
        let prepared = prepare_race_from_participants(
            local_race_participants(&ai_racers),
            track,
            &word_list,
            now,
        );
        let mut player = player;
        apply_prepared_player_states(&prepared.race, &mut player, &mut ai_racers);
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
            track: prepared.race.track,
            player,
            ai_racers,
            bonuses: prepared.bonuses,
            bonus_attempt: None,
            player_stunned_until: None,
            player_impact_cue: None,
            player_item_cue: None,
            race_status: RaceStatus::default(),
            race_phase: NetworkRacePhase::WaitingForHost,
            events,
            run_log,
            countdown_ends_at: None,
            word_list,
            word_count,
            ai_racer_count,
            ai_difficulty,
            item_registry,
            selected_ai_index: 0,
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
            player_stunned_until: None,
            player_impact_cue: None,
            player_item_cue: None,
            race_status: RaceStatus::default(),
            race_phase: NetworkRacePhase::Racing,
            events,
            run_log,
            countdown_ends_at: None,
            word_list,
            word_count,
            ai_racer_count: 0,
            ai_difficulty: AiDifficulty::Easy,
            item_registry,
            selected_ai_index: 0,
        }
    }

    pub fn apply_action(&mut self, action: LocalAction, now: Instant) {
        if self.race_status.is_ended() && action != LocalAction::Restart {
            return;
        }

        if self.handle_prerace_action(action, now) {
            return;
        }

        if self.race_phase != NetworkRacePhase::Racing && action != LocalAction::Restart {
            return;
        }

        match action {
            LocalAction::Typing(action) => self.apply_typing_action(action, now),
            LocalAction::ActivateItem => self.activate_item(ItemUse::Normal, now),
            LocalAction::ActivateModifiedItem => self.activate_item(ItemUse::Modified, now),
            LocalAction::SelectPreviousRacer
            | LocalAction::SelectNextRacer
            | LocalAction::AddAi
            | LocalAction::RemoveSelectedRacer
            | LocalAction::SetSelectedAiDifficulty(_) => {}
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
            (NetworkRacePhase::WaitingForHost, LocalAction::Typing(KeyAction::Space)) => {
                self.start_countdown(now);
                true
            }
            (NetworkRacePhase::WaitingForHost, LocalAction::SelectPreviousRacer) => {
                self.select_previous_ai();
                true
            }
            (NetworkRacePhase::WaitingForHost, LocalAction::SelectNextRacer) => {
                self.select_next_ai();
                true
            }
            (NetworkRacePhase::WaitingForHost, LocalAction::AddAi) => {
                self.add_ai_racer(now);
                true
            }
            (NetworkRacePhase::WaitingForHost, LocalAction::RemoveSelectedRacer) => {
                self.remove_selected_ai(now);
                true
            }
            (
                NetworkRacePhase::WaitingForHost,
                LocalAction::SetSelectedAiDifficulty(difficulty),
            ) => {
                self.set_selected_ai_difficulty(difficulty, now);
                true
            }
            (NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost, _) => true,
            (NetworkRacePhase::Countdown { .. }, _) => true,
            (NetworkRacePhase::Racing | NetworkRacePhase::Finished, _) => false,
        }
    }

    fn start_countdown(&mut self, now: Instant) {
        if countdown_start_plan(self.race_phase).is_err() {
            return;
        }
        let race = self.shared_race_state();
        if begin_countdown_phase(connected_racer_count(&race)).is_err() {
            self.events.push("No racers ready");
            return;
        }

        self.race_phase = countdown_tick_phase(3);
        self.countdown_ends_at = Some(now + RACE_COUNTDOWN);
        self.events.push("Race starts in 3");
        self.run_log.push(now, "host started countdown");
    }

    fn start_race(&mut self, now: Instant) {
        let race = self.shared_race_state();
        let Ok(outcome) =
            start_race_from_countdown(self.race_phase, !countdown_should_cancel(&race))
        else {
            return;
        };

        self.race_phase = outcome.phase;
        self.countdown_ends_at = None;
        self.player.started_at = now;
        for ai in &mut self.ai_racers {
            ai.player.started_at = now;
            ai.last_update = now;
            ai.char_budget = 0.0;
        }
        self.events.push(outcome.event);
        self.run_log.push(now, "race started");
    }

    pub fn restart(&mut self, now: Instant) {
        if let Some(outcome) = return_to_lobby_outcome(self.race_phase) {
            self.run_log.push(now, outcome.event);
        }

        let Ok(track) = Track::generate(&self.word_list, self.word_count) else {
            self.events.push("Restart failed");
            self.run_log.push(now, "restart failed: track generation");
            return;
        };
        let mut player = PlayerState::new(now);
        let mut ai_racers = build_ai_racers(self.ai_racer_count, self.ai_difficulty, now);
        let prepared = prepare_race_from_participants(
            local_race_participants(&ai_racers),
            track,
            &self.word_list,
            now,
        );
        apply_prepared_player_states(&prepared.race, &mut player, &mut ai_racers);

        self.track = prepared.race.track;
        self.player = player;
        self.ai_racers = ai_racers;
        self.selected_ai_index = self
            .selected_ai_index
            .min(self.ai_racers.len().saturating_sub(1));
        self.bonuses = prepared.bonuses;
        self.bonus_attempt = None;
        self.player_stunned_until = None;
        self.player_impact_cue = None;
        self.player_item_cue = None;
        self.race_status = RaceStatus::default();
        self.race_phase = NetworkRacePhase::WaitingForHost;
        self.countdown_ends_at = None;
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

    pub fn selected_ai_index(&self) -> Option<usize> {
        (!self.ai_racers.is_empty()).then_some(self.selected_ai_index)
    }

    fn select_previous_ai(&mut self) {
        self.selected_ai_index = self.selected_ai_index.saturating_sub(1);
    }

    fn select_next_ai(&mut self) {
        if !self.ai_racers.is_empty() {
            self.selected_ai_index = (self.selected_ai_index + 1).min(self.ai_racers.len() - 1);
        }
    }

    fn add_ai_racer(&mut self, now: Instant) {
        if self.ai_racers.len() >= MAX_AI_RACERS {
            self.events.push("AI roster is full");
            return;
        }

        self.ai_racer_count += 1;
        let id = next_local_ai_id(&self.ai_racers);
        let mut rng = thread_rng();
        let words_per_minute = rng.gen_range(self.ai_difficulty.wpm_range());
        let ai = AiRacer::new(id, self.ai_difficulty, words_per_minute, now);
        let name = ai.name.clone();
        self.ai_racers.push(ai);
        self.selected_ai_index = self.ai_racers.len() - 1;
        self.events.push(format!("{name} added"));
        self.run_log.push(
            now,
            format!(
                "{name} added difficulty={} wpm={:.0}",
                self.ai_difficulty.name(),
                words_per_minute
            ),
        );
    }

    fn remove_selected_ai(&mut self, now: Instant) {
        if self.ai_racers.is_empty() {
            self.events.push("No AI selected");
            return;
        }

        let ai = self.ai_racers.remove(self.selected_ai_index);
        self.ai_racer_count = self.ai_racer_count.saturating_sub(1);
        self.selected_ai_index = self
            .selected_ai_index
            .min(self.ai_racers.len().saturating_sub(1));
        self.events.push(format!("{} removed", ai.name));
        self.run_log.push(now, format!("{} removed", ai.name));
    }

    fn set_selected_ai_difficulty(&mut self, difficulty: AiDifficulty, now: Instant) {
        self.ai_difficulty = difficulty;
        if self.ai_racers.is_empty() {
            self.events
                .push(format!("New AI difficulty {}", difficulty.name()));
            self.run_log
                .push(now, format!("default ai difficulty={}", difficulty.name()));
            return;
        }

        let Some(ai) = self.ai_racers.get_mut(self.selected_ai_index) else {
            return;
        };
        let mut rng = thread_rng();
        ai.difficulty = difficulty;
        ai.words_per_minute = rng.gen_range(difficulty.wpm_range());
        ai.char_budget = 0.0;
        ai.last_update = now;
        self.events
            .push(format!("{} set to {}", ai.name, difficulty.name()));
        self.run_log.push(
            now,
            format!(
                "{} difficulty={} wpm={:.0}",
                ai.name,
                difficulty.name(),
                ai.words_per_minute
            ),
        );
    }

    pub fn tick(&mut self, now: Instant) {
        if self.race_status.is_ended() {
            return;
        }

        if let Some(countdown_ends_at) = self.countdown_ends_at
            && matches!(self.race_phase, NetworkRacePhase::Countdown { .. })
        {
            if now >= countdown_ends_at {
                if countdown_should_cancel(&self.shared_race_state()) {
                    self.restart(now);
                    return;
                }
                self.start_race(countdown_ends_at);
            } else {
                let remaining_seconds = countdown_ends_at
                    .saturating_duration_since(now)
                    .as_secs_f64()
                    .ceil()
                    .clamp(1.0, 3.0) as u8;
                self.race_phase = countdown_tick_phase(remaining_seconds);
            }
        }

        if self.race_phase != NetworkRacePhase::Racing {
            return;
        }

        self.advance_mushroom(now);
        self.advance_ai_mushrooms(now);
        self.tick_ai_racers(now);

        let expired_choices = self.bonuses.expire_cooldowns(&self.track, now);
        if expired_choices > 0 {
            self.run_log
                .push(now, format!("bonus refreshed choices={expired_choices}"));
        }

        let expired_effects = self.player.expire_effects(now);
        if expired_effects > 0 {
            self.events.push("Shield expired");
        }
        for ai in &mut self.ai_racers {
            ai.player.expire_effects(now);
        }

        self.expire_item_cues(now);

        self.update_race_status(now);
    }

    fn expire_item_cues(&mut self, now: Instant) {
        if self
            .player_item_cue
            .as_ref()
            .is_some_and(|cue| !cue.is_visible(now))
        {
            self.player_item_cue = None;
        }
        for ai in &mut self.ai_racers {
            if ai.item_cue.as_ref().is_some_and(|cue| !cue.is_visible(now)) {
                ai.item_cue = None;
            }
        }
    }

    fn update_race_status(&mut self, now: Instant) {
        if self.race_status.ended_at.is_some() {
            return;
        }

        let race = self.shared_race_state();
        let mut lifecycle = RaceLifecycleState {
            placements: Vec::new(),
            first_finished_at: self.race_status.first_finished_at,
        };
        let outcome = advance_host_race_lifecycle(
            &mut lifecycle,
            &race,
            self.race_phase,
            now,
            POST_FIRST_FINISH_TIMEOUT,
        );
        self.race_status.first_finished_at = lifecycle.first_finished_at;
        self.race_phase = outcome.phase;

        if outcome.flow.finished.is_some() {
            self.race_status.ended_at = Some(now);
            if let Some(event) = outcome.finish_event {
                self.events.push(event);
            }
        }
    }

    fn shared_race_state(&self) -> RaceState {
        let mut players = Vec::with_capacity(self.ai_racers.len() + 1);
        players.push(RacePlayer {
            id: RacePlayerId(1),
            name: "you".to_string(),
            color: PlayerColorId::Cyan,
            state: self.player.clone(),
            connected: true,
        });
        players.extend(self.ai_racers.iter().map(|ai| RacePlayer {
            id: RacePlayerId((ai.id + 1) as u64),
            name: ai.name.clone(),
            color: local_ai_color(ai.id),
            state: ai.player.clone(),
            connected: true,
        }));

        RaceState {
            track: self.track.clone(),
            players,
        }
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

        let effective_wpm = ai_effective_wpm(
            ai.words_per_minute,
            ai.player.has_active_focus(now),
            ai.player.is_inked_at(now),
            self.item_registry.focus_effect().ai_wpm_boost,
            self.item_registry
                .squid_ink_effect()
                .ai_wpm_multiplier_percent,
        );
        ai.char_budget += elapsed.as_secs_f64() * ai_chars_per_second(effective_wpm);
        while ai.char_budget >= 1.0 && !ai.player.is_finished() {
            let Some(action) = next_ai_key_for_player(&ai.player, &self.track) else {
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
            || ai.player.has_active_focus(now)
            || player_has_active_mushroom(&ai.player)
            || ai.item_cue.as_ref().is_some_and(|cue| cue.is_visible(now))
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
        let item_context = self.ai_item_roll_context(ai_index, 5);
        let Some(item) = claim_bonus_choice(
            &mut self.bonuses,
            point_index,
            choice_index,
            now,
            item_context,
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

    fn ai_item_roll_context(&self, ai_index: usize, max_distance_words: usize) -> ItemRollContext {
        ItemRollContext {
            has_nearby_racer: self.ai_has_nearby_racer(ai_index, max_distance_words),
            position: self.ai_position_band(ai_index),
        }
    }

    fn ai_position_band(&self, ai_index: usize) -> RacePositionBand {
        let Some(ai) = self.ai_racers.get(ai_index) else {
            return RacePositionBand::Middle;
        };
        racer_position_band(
            ai.player.word_index,
            self.active_racer_word_indices_excluding_ai(ai_index),
        )
    }

    fn receive_ai_pickup(&mut self, ai_index: usize, item: ItemPickup, now: Instant) {
        match item {
            ItemPickup::Held(held_item) => {
                let Some(ai) = self.ai_racers.get(ai_index) else {
                    return;
                };
                let ai_name = ai.name.clone();
                self.events
                    .push(format!("{ai_name} got {}", held_item.name()));
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
                    until: now
                        + Duration::from_millis(self.item_registry.shield_effect().duration_ms),
                });
                self.events.push(format!("{ai_name} got Shield"));
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
            HeldItem::Banana => self.use_banana(
                self.ai_racers
                    .get(ai_index)
                    .map(|ai| RacePlayerId((ai.id + 1) as u64)),
                now,
            ),
            HeldItem::Focus => self.activate_ai_focus(ai_index, now),
            HeldItem::Cyclone => self.use_cyclone(Some(ai_index), now),
            HeldItem::SquidInk => self.use_squid_ink(
                self.ai_racers
                    .get(ai_index)
                    .map(|ai| RacePlayerId((ai.id + 1) as u64)),
                now,
            ),
        }
    }

    fn ai_use_item(&mut self, ai_index: usize, now: Instant) {
        let Some(item) = self.ai_racers[ai_index].player.held_item else {
            return;
        };

        self.ai_racers[ai_index].player.held_item = None;
        self.activate_ai_held_item(ai_index, item, now);
    }

    fn activate_ai_focus(&mut self, ai_index: usize, now: Instant) {
        let Some(ai) = self.ai_racers.get_mut(ai_index) else {
            return;
        };
        ai.player.active_effects.push(ActiveEffect::Focus {
            until: now + Duration::from_millis(self.item_registry.focus_effect().duration_ms),
        });
    }

    fn use_cyclone(&mut self, attacker_ai_index: Option<usize>, now: Instant) {
        let player_id = attacker_ai_index
            .and_then(|index| self.ai_racers.get(index))
            .map(|ai| RacePlayerId((ai.id + 1) as u64))
            .unwrap_or(RacePlayerId(1));
        self.activate_shared_item_pickup(player_id, ItemPickup::Held(HeldItem::Cyclone), now);
    }

    fn use_banana(&mut self, player_id: Option<RacePlayerId>, now: Instant) {
        let Some(player_id) = player_id else {
            return;
        };
        self.activate_shared_item_pickup(player_id, ItemPickup::Held(HeldItem::Banana), now);
    }

    fn use_squid_ink(&mut self, player_id: Option<RacePlayerId>, now: Instant) {
        let Some(player_id) = player_id else {
            return;
        };
        self.activate_shared_item_pickup(player_id, ItemPickup::Held(HeldItem::SquidInk), now);
    }

    fn activate_shared_item_pickup(
        &mut self,
        player_id: RacePlayerId,
        item: ItemPickup,
        now: Instant,
    ) {
        let mut race = self.shared_race_state();
        let ai_players = self
            .ai_racers
            .iter()
            .map(|ai| RacePlayerId((ai.id + 1) as u64))
            .collect::<HashSet<_>>();
        let mut effects = self.shared_item_effects();
        let report = activate_item_pickup(
            &mut race,
            &mut effects,
            &ai_players,
            &self.item_registry,
            player_id,
            item,
            now,
        );

        self.sync_from_shared_item_race(race);
        for interrupted in report.interrupted_players {
            if interrupted == RacePlayerId(1) {
                self.bonus_attempt = None;
            }
        }
        for reset_ai in report.reset_ai_players {
            if let Some(ai) = self.local_ai_mut(reset_ai) {
                ai.char_budget = 0.0;
            }
        }
        self.apply_shared_item_effects(effects);
        for event in report.events {
            let event = local_item_event(event);
            self.run_log.push(now, event.clone());
            self.events.push(event);
        }
    }

    fn shared_item_effects(&self) -> HashMap<RacePlayerId, RaceItemEffectState> {
        let mut effects = HashMap::new();
        if self.player_stunned_until.is_some() {
            effects.insert(
                RacePlayerId(1),
                RaceItemEffectState {
                    stunned_until: self.player_stunned_until,
                    ..RaceItemEffectState::default()
                },
            );
        }
        for ai in &self.ai_racers {
            if ai.stunned_until.is_some() {
                effects.insert(
                    RacePlayerId((ai.id + 1) as u64),
                    RaceItemEffectState {
                        stunned_until: ai.stunned_until,
                        ..RaceItemEffectState::default()
                    },
                );
            }
        }
        effects
    }

    fn sync_from_shared_item_race(&mut self, race: RaceState) {
        for player in race.players {
            if player.id == RacePlayerId(1) {
                self.player = player.state;
            } else if let Some(ai) = self.local_ai_mut(player.id) {
                ai.player = player.state;
            }
        }
    }

    fn apply_shared_item_effects(
        &mut self,
        effects: HashMap<RacePlayerId, crate::game::item_effects::RaceItemEffectState>,
    ) {
        for (player_id, effect) in effects {
            if player_id == RacePlayerId(1) {
                self.player_stunned_until = effect.stunned_until;
                if let Some(cue) = effect.impact_cue {
                    self.player_impact_cue = Some(ImpactCue {
                        kind: impact_cue_kind(cue.kind),
                        until: cue.until,
                    });
                }
                if let Some(cue) = effect.item_cue {
                    self.player_item_cue = Some(item_cue_from_shared(cue));
                }
            } else if let Some(ai) = self.local_ai_mut(player_id) {
                ai.stunned_until = effect.stunned_until;
                if let Some(cue) = effect.impact_cue {
                    ai.impact_cue = Some(ImpactCue {
                        kind: impact_cue_kind(cue.kind),
                        until: cue.until,
                    });
                }
                if let Some(cue) = effect.item_cue {
                    ai.item_cue = Some(item_cue_from_shared(cue));
                }
            }
        }
    }

    fn local_ai_mut(&mut self, player_id: RacePlayerId) -> Option<&mut AiRacer> {
        let ai_id = usize::try_from(player_id.0).ok()?.checked_sub(1)?;
        self.ai_racers.iter_mut().find(|ai| ai.id == ai_id)
    }

    fn activate_ai_mushroom(&mut self, ai_index: usize, now: Instant) {
        let Some(ai) = self.ai_racers.get_mut(ai_index) else {
            return;
        };
        ai.player.input.clear();
        ai.player.typo_index = None;
        ai.player.active_effects.push(ActiveEffect::Mushroom {
            remaining_words: self.item_registry.mushroom_effect().boost_words,
            next_step_at: now,
            step_interval: mushroom_step_interval(self.item_registry.mushroom_effect().wpm),
        });
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
        if player_has_active_mushroom(&self.player) || self.player_is_stunned(now) {
            return;
        }

        if self.bonus_attempt.is_some() {
            self.apply_bonus_typing_action(action, now);
            return;
        }

        if let KeyAction::Char(ch) = action
            && self.can_start_bonus_attempt(now)
            && let Some((point_index, choice_index)) = self.match_bonus_start(ch, now)
        {
            self.bonus_attempt = Some(BonusAttempt {
                point_index,
                choice_index,
            });
            self.apply_bonus_char(ch);
            return;
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
                    let _ = previous_typo;
                }
                if self.player.input.is_empty() {
                    self.bonus_attempt = None;
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
        let _ = previous_typo;
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
        let item_context = self.player_item_roll_context(5);
        let Some(item) = claim_bonus_choice(
            &mut self.bonuses,
            attempt.point_index,
            attempt.choice_index,
            now,
            item_context,
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

    fn player_item_roll_context(&self, max_distance_words: usize) -> ItemRollContext {
        ItemRollContext {
            has_nearby_racer: self.player_has_nearby_racer(max_distance_words),
            position: self.player_position_band(),
        }
    }

    fn player_has_nearby_racer(&self, max_distance_words: usize) -> bool {
        !self.player.is_finished()
            && self.ai_racers.iter().any(|ai| {
                !ai.player.is_finished()
                    && self.player.word_index.abs_diff(ai.player.word_index) <= max_distance_words
            })
    }

    fn player_position_band(&self) -> RacePositionBand {
        racer_position_band(self.player.word_index, self.active_ai_word_indices())
    }

    fn active_ai_word_indices(&self) -> Vec<usize> {
        self.ai_racers
            .iter()
            .filter(|ai| !ai.player.is_finished())
            .map(|ai| ai.player.word_index)
            .collect()
    }

    fn active_racer_word_indices_excluding_ai(
        &self,
        ai_index: usize,
    ) -> impl Iterator<Item = usize> + '_ {
        let player_word = (!self.player.is_finished()).then_some(self.player.word_index);
        player_word.into_iter().chain(
            self.ai_racers
                .iter()
                .enumerate()
                .filter(move |(other_index, ai)| {
                    *other_index != ai_index && !ai.player.is_finished()
                })
                .map(|(_, ai)| ai.player.word_index),
        )
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
            && !self.player.has_active_focus(now)
            && !player_has_active_mushroom(&self.player)
            && !self
                .player_item_cue
                .as_ref()
                .is_some_and(|cue| cue.is_visible(now))
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
            until: now + Duration::from_millis(self.item_registry.shield_effect().duration_ms),
        });
    }

    fn receive_pickup(&mut self, item: ItemPickup, now: Instant) {
        match item {
            ItemPickup::Held(held_item) => {
                self.events.push(format!("Got {}", held_item.name()));
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
                self.events.push("Got Shield");
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
            HeldItem::Focus => self.activate_focus(now),
            HeldItem::Cyclone => self.use_cyclone(None, now),
            HeldItem::SquidInk => self.use_squid_ink(Some(RacePlayerId(1)), now),
            HeldItem::Banana => self.use_banana(Some(RacePlayerId(1)), now),
        }
    }

    fn activate_focus(&mut self, now: Instant) {
        self.player.active_effects.push(ActiveEffect::Focus {
            until: now + Duration::from_millis(self.item_registry.focus_effect().duration_ms),
        });
    }

    fn activate_mushroom(&mut self, now: Instant) {
        self.player.input.clear();
        self.player.typo_index = None;
        self.bonus_attempt = None;
        self.player.active_effects.push(ActiveEffect::Mushroom {
            remaining_words: self.item_registry.mushroom_effect().boost_words,
            next_step_at: now,
            step_interval: mushroom_step_interval(self.item_registry.mushroom_effect().wpm),
        });
        self.advance_mushroom(now);
    }

    fn player_is_stunned(&self, now: Instant) -> bool {
        self.player_stunned_until.is_some_and(|until| until > now)
    }

    fn advance_mushroom(&mut self, now: Instant) {
        while let Some(effect_index) = self.player.active_effects.iter().position(|effect| {
            matches!(
                effect,
                ActiveEffect::Mushroom {
                    remaining_words,
                    next_step_at,
                    ..
                } if *remaining_words > 0 && *next_step_at <= now
            )
        }) {
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

fn mushroom_step_interval(wpm: u32) -> std::time::Duration {
    std::time::Duration::from_secs_f64(60.0 / f64::from(wpm))
}

fn racer_position_band(
    word_index: usize,
    other_word_indices: impl IntoIterator<Item = usize>,
) -> RacePositionBand {
    let mut ahead = 0;
    let mut behind = 0;
    for other_word_index in other_word_indices {
        if other_word_index > word_index {
            ahead += 1;
        } else if other_word_index < word_index {
            behind += 1;
        }
    }

    if ahead == 0 && behind > 0 {
        RacePositionBand::First
    } else if behind == 0 && ahead > 0 {
        RacePositionBand::Trailing
    } else {
        RacePositionBand::Middle
    }
}

fn impact_cue_kind(kind: RaceImpactCueKind) -> ImpactCueKind {
    match kind {
        RaceImpactCueKind::Banana => ImpactCueKind::Banana,
        RaceImpactCueKind::Cyclone => ImpactCueKind::Cyclone,
        RaceImpactCueKind::SquidInk => ImpactCueKind::SquidInk,
        RaceImpactCueKind::ShieldBlock => ImpactCueKind::ShieldBlock,
    }
}

fn item_cue_from_shared(cue: crate::game::item_effects::RaceItemCue) -> ItemCue {
    ItemCue {
        kind: item_cue_kind(cue.kind),
        until: cue.until,
        ascii_label: cue.ascii_label,
        unicode_label: cue.unicode_label,
    }
}

fn item_cue_kind(kind: RaceItemCueKind) -> ItemCueKind {
    match kind {
        RaceItemCueKind::Banana { direction } => ItemCueKind::Banana {
            direction: attack_direction_from_shared(direction),
        },
        RaceItemCueKind::Cyclone { direction } => ItemCueKind::Cyclone {
            direction: attack_direction_from_shared(direction),
        },
        RaceItemCueKind::SquidInk => ItemCueKind::SquidInk,
    }
}

fn attack_direction_from_shared(direction: SharedAttackDirection) -> AttackDirection {
    match direction {
        SharedAttackDirection::Ahead => AttackDirection::Ahead,
        SharedAttackDirection::Behind => AttackDirection::Behind,
        SharedAttackDirection::Overlap => AttackDirection::Overlap,
    }
}

fn local_item_event(event: String) -> String {
    if event == "you missed Cyclone" {
        "Missed Cyclone".to_string()
    } else if event == "you missed Banana" {
        "Missed Banana".to_string()
    } else if event == "you missed Squid Ink" {
        "Missed Squid Ink".to_string()
    } else if event == "you blocked Cyclone" {
        "Blocked Cyclone".to_string()
    } else if let Some(target) = event
        .strip_prefix("you hit ")
        .and_then(|suffix| suffix.strip_suffix(" with Cyclone"))
    {
        format!("Hit {target} with Cyclone")
    } else if let Some(target) = event.strip_prefix("you hit ") {
        format!("Hit {target}")
    } else if let Some(count) = event
        .strip_prefix("you inked ")
        .and_then(|suffix| suffix.strip_suffix(" racer(s)"))
    {
        format!("Squid Ink hit {count} racer(s)")
    } else {
        event
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

fn event_message(event: TypingEvent, previous_word: Option<&str>) -> Option<String> {
    match event {
        TypingEvent::InputChanged => None,
        TypingEvent::WordCompleted => {
            let _ = previous_word;
            None
        }
        TypingEvent::RaceFinished => Some("Race finished".to_string()),
        TypingEvent::TypoStarted { .. } | TypingEvent::TypoCleared => None,
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
    use std::time::{Duration, Instant};

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
            AiRacer, AttackDirection, EventLog, ImpactCueKind, ItemCue, ItemCueKind, LocalAction,
            LocalSession,
        },
    };
    use typekart_protocol::NetworkRacePhase;

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
        assert!(!entries.contains(&"Typo started"));
        assert!(!entries.contains(&"Typo cleared"));
        assert!(!entries.iter().any(|entry| entry.starts_with("Completed ")));
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

        assert_eq!(session.race_phase, NetworkRacePhase::WaitingForHost);
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

        assert!(matches!(
            session.race_phase,
            NetworkRacePhase::Countdown { .. }
        ));
        assert!(session.player.input.is_empty());
        assert!(session.ai_racers[0].player.input.is_empty());

        session.tick(now + std::time::Duration::from_secs(3));
        session.apply_action(
            LocalAction::Typing(KeyAction::Char('o')),
            now + std::time::Duration::from_secs(3),
        );

        let started_at = now + std::time::Duration::from_secs(3);
        assert_eq!(session.race_phase, NetworkRacePhase::Racing);
        assert_eq!(session.player.started_at, started_at);
        assert_eq!(session.ai_racers[0].player.started_at, started_at);
        assert_eq!(session.player.input, "o");
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "Race started")
        );
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
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "Race finished")
        );
    }

    #[test]
    fn restart_uses_shared_return_to_lobby_outcome() {
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

        session.apply_action(LocalAction::Typing(KeyAction::Space), now);
        session.tick(now + std::time::Duration::from_secs(3));
        session.apply_action(
            LocalAction::Restart,
            now + std::time::Duration::from_secs(4),
        );

        assert_eq!(session.race_phase, NetworkRacePhase::WaitingForHost);
        assert!(session.player.input.is_empty());
        assert!(
            session
                .run_log
                .entries()
                .any(|entry| entry.ends_with("Race cancelled"))
        );

        session.player.finished_at = Some(now + std::time::Duration::from_secs(5));
        session.race_status.ended_at = Some(now + std::time::Duration::from_secs(5));
        session.race_phase = NetworkRacePhase::Finished;
        session.apply_action(
            LocalAction::Restart,
            now + std::time::Duration::from_secs(6),
        );

        assert!(
            session
                .run_log
                .entries()
                .any(|entry| entry.ends_with("Returned to lobby"))
        );
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
    fn local_lobby_can_add_remove_and_retune_ai_racers() {
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

        session.apply_action(LocalAction::AddAi, now);
        assert_eq!(session.ai_racers.len(), 1);
        assert_eq!(session.selected_ai_index(), Some(0));

        session.apply_action(
            LocalAction::SetSelectedAiDifficulty(AiDifficulty::Hard),
            now,
        );
        assert_eq!(session.ai_racers[0].difficulty, AiDifficulty::Hard);
        assert!(session.ai_racers[0].words_per_minute >= 55.0);

        session.apply_action(LocalAction::RemoveSelectedRacer, now);
        assert!(session.ai_racers.is_empty());
        assert_eq!(session.selected_ai_index(), None);
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
    fn inked_ai_racer_hesitates_from_reduced_wpm_budget() {
        let now = Instant::now();
        let mut session = LocalSession::with_bonuses(
            track(&["one", "two"]),
            PlayerState::new(now),
            BonusState::with_points(vec![], vec![]),
        );
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 60.0, now);
        ai.player.inked_word_index = Some(0);
        ai.player.inked_until = Some(now + Duration::from_secs(5));
        session.ai_racers.push(ai);

        session.tick(now + Duration::from_secs(1));

        assert_eq!(session.ai_racers[0].player.word_index, 0);
        assert_eq!(session.ai_racers[0].player.input, "one");
    }

    #[test]
    fn focused_ai_racer_gets_small_wpm_boost() {
        assert_eq!(super::ai_effective_wpm(60.0, true, false, 10, 70), 70.0);
        assert_eq!(super::ai_effective_wpm(60.0, false, false, 10, 70), 60.0);
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
        assert!(session.events.entries().any(|entry| entry == "Hit ai-2"));
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
        assert!(session.ai_racers[0].is_impacted(now));
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
        assert!(
            session
                .run_log
                .entries()
                .any(|entry| entry.contains("ai-1 blocked Banana"))
        );
    }

    #[test]
    fn ai_banana_immediately_clears_player_input() {
        let now = Instant::now();
        let mut session =
            LocalSession::with_bonuses(track(&["one", "two"]), PlayerState::new(now), bonuses());
        session.player.word_index = 1;
        session.player.input = "t".to_string();
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
        ai.player.held_item = Some(HeldItem::Banana);
        session.ai_racers.push(ai);

        session.tick(now);

        assert!(session.player.input.is_empty());
        assert!(
            session
                .player_impact_cue
                .is_some_and(|cue| cue.kind == ImpactCueKind::Banana && cue.until > now)
        );
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "ai-1 hit you")
        );
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

        assert!(session.player_impact_cue.is_none_or(|cue| cue.until <= now));
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
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "ai-1 hit ai-3")
        );
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
        assert_eq!(session.race_phase, NetworkRacePhase::WaitingForHost);
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
    fn focus_pickup_activates_and_forgives_wrong_keys() {
        let now = Instant::now();
        let track = track(&["one", "two"]);
        let player = PlayerState::new(now);
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));

        session.receive_pickup(ItemPickup::Held(HeldItem::Focus), now);
        session.apply_action(LocalAction::Typing(KeyAction::Char('x')), now);

        assert!(session.player.has_active_focus(now));
        assert_eq!(session.player.input, "");
        assert_eq!(session.player.typo_index, None);
        assert_eq!(session.player.stats.typo_chars, 1);
    }

    #[test]
    fn player_cyclone_reverses_first_place_ai_word() {
        let now = Instant::now();
        let track = track(&["one", "two", "three"]);
        let player = PlayerState::new(now);
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 80.0, now);
        ai.player.word_index = 1;
        session.ai_racers.push(ai);
        session.player.held_item = Some(HeldItem::Cyclone);

        session.apply_action(LocalAction::ActivateItem, now);

        assert_eq!(session.player.held_item, None);
        assert_eq!(session.ai_racers[0].player.word_override(1), Some("owt"));
        assert!(session.ai_racers[0].is_stunned(now));
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "Hit ai-1 with Cyclone")
        );
    }

    #[test]
    fn first_place_ai_cyclone_misses_instead_of_hitting_player() {
        let now = Instant::now();
        let track = track(&["one", "two", "three"]);
        let player = PlayerState::new(now);
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 80.0, now);
        ai.player.word_index = 1;
        ai.player.held_item = Some(HeldItem::Cyclone);
        session.ai_racers.push(ai);

        session.ai_use_item(0, now);

        assert_eq!(session.player.word_override(0), None);
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "ai-1 missed Cyclone")
        );
    }

    #[test]
    fn cyclone_is_blocked_by_shield_and_consumes_shield() {
        let now = Instant::now();
        let track = track(&["one", "two", "three"]);
        let player = PlayerState::new(now);
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 80.0, now);
        ai.player.word_index = 1;
        ai.player.active_effects.push(ActiveEffect::Shield {
            until: now + std::time::Duration::from_secs(5),
        });
        session.ai_racers.push(ai);
        session.player.held_item = Some(HeldItem::Cyclone);

        session.apply_action(LocalAction::ActivateItem, now);

        assert_eq!(session.ai_racers[0].player.word_override(1), None);
        assert!(!session.ai_racers[0].player.has_active_shield(now));
        assert!(
            session
                .events
                .entries()
                .any(|entry| entry == "ai-1 blocked Cyclone")
        );
    }

    #[test]
    fn squid_ink_hits_all_ai_racers_in_range() {
        let now = Instant::now();
        let track = track(&["one", "two", "three", "four"]);
        let player = PlayerState::new(now);
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
        let mut near_ai = AiRacer::new(1, AiDifficulty::Easy, 80.0, now);
        near_ai.player.word_index = 1;
        let mut far_ai = AiRacer::new(2, AiDifficulty::Easy, 80.0, now);
        far_ai.player.word_index = 6;
        session.ai_racers.push(near_ai);
        session.ai_racers.push(far_ai);
        session.player.held_item = Some(HeldItem::SquidInk);

        session.apply_action(LocalAction::ActivateItem, now);

        assert!(session.ai_racers[0].player.is_inked_at(now));
        assert!(!session.ai_racers[1].player.is_inked_at(now));
        assert!(matches!(
            session.ai_racers[0].impact_cue.map(|cue| cue.kind),
            Some(ImpactCueKind::SquidInk)
        ));
        assert!(matches!(
            session.player_item_cue.as_ref().map(|cue| cue.kind),
            Some(ItemCueKind::SquidInk)
        ));
    }

    #[test]
    fn squid_ink_persists_after_current_word_is_completed() {
        let now = Instant::now();
        let track = track(&["one", "two", "three"]);
        let mut player = PlayerState::new(now);
        player.inked_word_index = Some(0);
        player.inked_until = Some(now + Duration::from_secs(5));
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));

        for action in [
            KeyAction::Char('o'),
            KeyAction::Char('n'),
            KeyAction::Char('e'),
            KeyAction::Space,
        ] {
            session.apply_action(LocalAction::Typing(action), now);
        }
        session.tick(now);

        assert!(session.player.is_inked_at(now));
        assert_eq!(session.player.word_index, 1);
        assert_eq!(session.player.inked_word_index, Some(0));
    }

    #[test]
    fn squid_ink_expires_after_duration() {
        let now = Instant::now();
        let track = track(&["one", "two", "three"]);
        let mut player = PlayerState::new(now);
        player.inked_word_index = Some(0);
        player.inked_until = Some(now + Duration::from_secs(5));
        let mut session =
            LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));

        session.tick(now + Duration::from_secs(5));

        assert!(!session.player.is_inked_at(now + Duration::from_secs(5)));
        assert_eq!(session.player.inked_word_index, None);
        assert_eq!(session.player.inked_until, None);
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
                .any(|entry| entry == "Missed Banana")
        );
    }
}
