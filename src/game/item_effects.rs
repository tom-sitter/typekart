//! Shared item activation rules for authoritative race hosts.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use super::{
    effects::ActiveEffect,
    items::{
        BananaDisplayConfig, HeldItem, ItemPickup, ItemRegistry, RacerPosition,
        select_nearest_banana_target,
    },
    race::{RacePlayerId, RaceState},
};

#[derive(Debug, Clone, Default)]
pub struct RaceItemEffectState {
    pub stunned_until: Option<Instant>,
    pub impact_cue: Option<RaceImpactCue>,
    pub item_cue: Option<RaceItemCue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaceImpactCue {
    pub kind: RaceImpactCueKind,
    pub until: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceImpactCueKind {
    Banana,
    Cyclone,
    SquidInk,
    ShieldBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceItemCue {
    pub kind: RaceItemCueKind,
    pub ascii_label: String,
    pub unicode_label: String,
    pub placement: RaceItemCuePlacement,
    pub until: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceItemCueKind {
    Banana { direction: AttackDirection },
    Cyclone { direction: AttackDirection },
    SquidInk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceItemCuePlacement {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackDirection {
    Ahead,
    Behind,
    Overlap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemActivationReport {
    pub events: Vec<String>,
    pub interrupted_players: Vec<RacePlayerId>,
    pub reset_ai_players: Vec<RacePlayerId>,
}

impl ItemActivationReport {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            interrupted_players: Vec::new(),
            reset_ai_players: Vec::new(),
        }
    }

    fn interrupt(&mut self, player_id: RacePlayerId) {
        if !self.interrupted_players.contains(&player_id) {
            self.interrupted_players.push(player_id);
        }
    }

    fn reset_ai(&mut self, player_id: RacePlayerId) {
        if !self.reset_ai_players.contains(&player_id) {
            self.reset_ai_players.push(player_id);
        }
    }
}

pub fn activate_item_pickup(
    race: &mut RaceState,
    effects: &mut HashMap<RacePlayerId, RaceItemEffectState>,
    ai_players: &HashSet<RacePlayerId>,
    item_registry: &ItemRegistry,
    player_id: RacePlayerId,
    item: ItemPickup,
    now: Instant,
) -> ItemActivationReport {
    match item {
        ItemPickup::Held(HeldItem::Mushroom) => {
            activate_mushroom(race, player_id, item_registry, now)
        }
        ItemPickup::Held(HeldItem::Banana) => {
            activate_banana(race, effects, ai_players, item_registry, player_id, now)
        }
        ItemPickup::Held(HeldItem::Focus) => activate_focus(race, player_id, item_registry, now),
        ItemPickup::Held(HeldItem::Cyclone) => {
            activate_cyclone(race, effects, ai_players, item_registry, player_id, now)
        }
        ItemPickup::Held(HeldItem::SquidInk) => {
            activate_squid_ink(race, effects, item_registry, player_id, now)
        }
        ItemPickup::Shield => activate_shield(race, player_id, item_registry, now),
    }
}

pub fn player_has_active_mushroom_effect(
    player: &crate::game::race::RacePlayer,
    _now: Instant,
) -> bool {
    player.state.active_effects.iter().any(|effect| {
        matches!(
            effect,
            ActiveEffect::Mushroom {
                remaining_words,
                ..
            } if *remaining_words > 0
        )
    })
}

pub fn player_is_stunned(
    effects: &HashMap<RacePlayerId, RaceItemEffectState>,
    player_id: RacePlayerId,
    now: Instant,
) -> bool {
    effects
        .get(&player_id)
        .and_then(|effects| effects.stunned_until)
        .is_some_and(|until| until > now)
}

pub fn advance_mushrooms(race: &mut RaceState, now: Instant) -> Vec<RacePlayerId> {
    let player_ids = race
        .players
        .iter()
        .map(|player| player.id)
        .collect::<Vec<_>>();
    let mut interrupted = Vec::new();

    for player_id in player_ids {
        loop {
            if !advance_mushroom_one_word(race, player_id, now) {
                break;
            }
            if !interrupted.contains(&player_id) {
                interrupted.push(player_id);
            }
            if race
                .player(player_id)
                .is_some_and(|player| player.state.is_finished())
            {
                break;
            }
        }
    }

    interrupted
}

fn activate_shield(
    race: &mut RaceState,
    player_id: RacePlayerId,
    item_registry: &ItemRegistry,
    now: Instant,
) -> ItemActivationReport {
    let report = ItemActivationReport::new();
    let Some(player) = race
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return report;
    };

    player.state.active_effects.push(ActiveEffect::Shield {
        until: now + Duration::from_millis(item_registry.shield_effect().duration_ms),
    });
    report
}

fn activate_mushroom(
    race: &mut RaceState,
    player_id: RacePlayerId,
    item_registry: &ItemRegistry,
    now: Instant,
) -> ItemActivationReport {
    let mut report = ItemActivationReport::new();
    let Some(player) = race
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return report;
    };

    player.state.input.clear();
    player.state.typo_index = None;
    let mushroom = item_registry.mushroom_effect();
    player.state.active_effects.push(ActiveEffect::Mushroom {
        remaining_words: mushroom.boost_words,
        next_step_at: now,
        step_interval: mushroom_step_interval(mushroom.wpm),
    });
    report.interrupt(player_id);
    for interrupted in advance_mushrooms(race, now) {
        report.interrupt(interrupted);
    }
    report
}

fn activate_focus(
    race: &mut RaceState,
    player_id: RacePlayerId,
    item_registry: &ItemRegistry,
    now: Instant,
) -> ItemActivationReport {
    let report = ItemActivationReport::new();
    let Some(player) = race
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return report;
    };

    player.state.active_effects.push(ActiveEffect::Focus {
        until: now + Duration::from_millis(item_registry.focus_effect().duration_ms),
    });
    report
}

fn activate_cyclone(
    race: &mut RaceState,
    effects: &mut HashMap<RacePlayerId, RaceItemEffectState>,
    ai_players: &HashSet<RacePlayerId>,
    item_registry: &ItemRegistry,
    player_id: RacePlayerId,
    now: Instant,
) -> ItemActivationReport {
    let mut report = ItemActivationReport::new();
    let attacker_name = player_label(race, player_id);
    let Some(target_id) = first_place_target(race, Some(player_id)) else {
        report
            .events
            .push(format!("{attacker_name} missed Cyclone"));
        return report;
    };

    let attacker_word_index = race
        .player(player_id)
        .map(|player| player.state.word_index)
        .unwrap_or_default();
    let target_word_index = race
        .player(target_id)
        .map(|player| player.state.word_index)
        .unwrap_or_default();
    let direction = attack_direction(attacker_word_index, target_word_index);
    effects.entry(player_id).or_default().item_cue = Some(RaceItemCue {
        kind: RaceItemCueKind::Cyclone { direction },
        ascii_label: cyclone_cue_label(direction, false),
        unicode_label: cyclone_cue_label(direction, true),
        placement: item_cue_placement(direction),
        until: now + Duration::from_millis(1_500),
    });

    let target_name = player_label(race, target_id);
    if apply_cyclone_to_player(
        race,
        effects,
        ai_players,
        item_registry,
        target_id,
        now,
        &mut report,
    ) {
        report
            .events
            .push(format!("{attacker_name} hit {target_name} with Cyclone"));
    }
    report
}

fn activate_banana(
    race: &mut RaceState,
    effects: &mut HashMap<RacePlayerId, RaceItemEffectState>,
    ai_players: &HashSet<RacePlayerId>,
    item_registry: &ItemRegistry,
    player_id: RacePlayerId,
    now: Instant,
) -> ItemActivationReport {
    let mut report = ItemActivationReport::new();
    let Some(attacker) = race.player(player_id) else {
        return report;
    };
    let attacker_word_index = attacker.state.word_index;
    let attacker_name = attacker.name.clone();
    let candidates = race
        .players
        .iter()
        .filter(|player| player.id != attacker.id)
        .filter(|player| player.connected)
        .filter(|player| !player.state.is_finished())
        .filter(|player| !player_is_stunned(effects, player.id, now))
        .map(|player| RacerPosition {
            id: player.id.0 as usize,
            word_index: player.state.word_index,
        })
        .collect::<Vec<_>>();

    let banana = item_registry.banana_effect();
    let Some(target) =
        select_nearest_banana_target(attacker_word_index, &candidates, banana.range_words)
    else {
        report.events.push(format!("{attacker_name} missed Banana"));
        return report;
    };

    let target_id = RacePlayerId(target.id as u64);
    let direction = attack_direction(attacker_word_index, target.word_index);
    let (ascii_label, unicode_label) = banana_cue_labels(direction, item_registry.banana_display());
    effects.entry(player_id).or_default().item_cue = Some(RaceItemCue {
        kind: RaceItemCueKind::Banana { direction },
        ascii_label,
        unicode_label,
        placement: item_cue_placement(direction),
        until: now + Duration::from_millis(banana.cue_ms),
    });

    match apply_banana_to_player(
        race,
        effects,
        ai_players,
        item_registry,
        target_id,
        now,
        &mut report,
    ) {
        Some(BananaResolution::SpunOut) => {
            let target_name = player_label(race, target_id);
            report
                .events
                .push(format!("{attacker_name} hit {target_name}"));
        }
        Some(BananaResolution::Blocked) | None => {}
    }

    report
}

fn activate_squid_ink(
    race: &mut RaceState,
    effects: &mut HashMap<RacePlayerId, RaceItemEffectState>,
    item_registry: &ItemRegistry,
    player_id: RacePlayerId,
    now: Instant,
) -> ItemActivationReport {
    let mut report = ItemActivationReport::new();
    let Some(attacker) = race.player(player_id) else {
        return report;
    };
    let attacker_word_index = attacker.state.word_index;
    let attacker_name = attacker.name.clone();
    let squid_ink = item_registry.squid_ink_effect();
    let targets = race
        .players
        .iter()
        .filter(|player| player.id != attacker.id)
        .filter(|player| player.connected)
        .filter(|player| !player.state.is_finished())
        .filter(|player| {
            attacker_word_index.abs_diff(player.state.word_index) <= squid_ink.range_words
        })
        .map(|player| player.id)
        .collect::<Vec<_>>();

    effects.entry(player_id).or_default().item_cue = Some(RaceItemCue {
        kind: RaceItemCueKind::SquidInk,
        ascii_label: " ink ".to_string(),
        unicode_label: " 🦑 ".to_string(),
        placement: RaceItemCuePlacement::After,
        until: now + Duration::from_millis(squid_ink.cue_ms),
    });

    let mut hit_count = 0;
    for target_id in targets {
        if apply_squid_ink_to_player(race, effects, item_registry, target_id, now, &mut report) {
            hit_count += 1;
        }
    }

    if hit_count == 0 {
        report
            .events
            .push(format!("{attacker_name} missed Squid Ink"));
    } else {
        report
            .events
            .push(format!("{attacker_name} inked {hit_count} racer(s)"));
    }
    report
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BananaResolution {
    SpunOut,
    Blocked,
}

fn apply_banana_to_player(
    race: &mut RaceState,
    effects: &mut HashMap<RacePlayerId, RaceItemEffectState>,
    ai_players: &HashSet<RacePlayerId>,
    item_registry: &ItemRegistry,
    target_id: RacePlayerId,
    now: Instant,
    report: &mut ItemActivationReport,
) -> Option<BananaResolution> {
    let target_index = race
        .players
        .iter()
        .position(|player| player.id == target_id)?;
    let target_name = race.players[target_index].name.clone();

    if race.players[target_index].state.has_active_shield(now) {
        race.players[target_index]
            .state
            .active_effects
            .retain(|effect| !matches!(effect, ActiveEffect::Shield { .. }));
        effects.entry(target_id).or_default().impact_cue = Some(RaceImpactCue {
            kind: RaceImpactCueKind::ShieldBlock,
            until: now + Duration::from_millis(700),
        });
        report.events.push(format!("{target_name} blocked Banana"));
        return Some(BananaResolution::Blocked);
    }

    let target = &mut race.players[target_index];
    target.state.input.clear();
    target.state.typo_index = None;
    report.interrupt(target_id);
    let target_is_ai = ai_players.contains(&target_id);
    if target_is_ai {
        report.reset_ai(target_id);
    }
    let effects = effects.entry(target_id).or_default();
    let banana = item_registry.banana_effect();
    if target_is_ai {
        effects.stunned_until = Some(now + Duration::from_millis(banana.stun_ms));
    } else {
        effects.stunned_until = None;
    }
    effects.impact_cue = Some(RaceImpactCue {
        kind: RaceImpactCueKind::Banana,
        until: now + Duration::from_millis(banana.impact_blink_ms),
    });
    Some(BananaResolution::SpunOut)
}

fn first_place_target(race: &RaceState, exclude: Option<RacePlayerId>) -> Option<RacePlayerId> {
    race.players
        .iter()
        .filter(|player| Some(player.id) != exclude)
        .filter(|player| player.connected)
        .filter(|player| !player.state.is_finished())
        .max_by_key(|player| (player.state.word_index, player.state.input.chars().count()))
        .map(|player| player.id)
}

fn apply_cyclone_to_player(
    race: &mut RaceState,
    effects: &mut HashMap<RacePlayerId, RaceItemEffectState>,
    ai_players: &HashSet<RacePlayerId>,
    item_registry: &ItemRegistry,
    target_id: RacePlayerId,
    now: Instant,
    report: &mut ItemActivationReport,
) -> bool {
    let Some(target_index) = race
        .players
        .iter()
        .position(|player| player.id == target_id)
    else {
        return false;
    };
    let target_name = race.players[target_index].name.clone();

    if race.players[target_index].state.has_active_shield(now) {
        race.players[target_index]
            .state
            .active_effects
            .retain(|effect| !matches!(effect, ActiveEffect::Shield { .. }));
        effects.entry(target_id).or_default().impact_cue = Some(RaceImpactCue {
            kind: RaceImpactCueKind::ShieldBlock,
            until: now + Duration::from_millis(700),
        });
        report.events.push(format!("{target_name} blocked Cyclone"));
        return false;
    }

    let affected_words = item_registry.cyclone_effect().affected_words;
    let target = &mut race.players[target_index].state;
    let mut applied = false;
    for word_index in target.word_index..target.word_index.saturating_add(affected_words) {
        let Some(word) = race.track.current_word(word_index) else {
            break;
        };
        target
            .word_overrides
            .insert(word_index, word.chars().rev().collect());
        applied = true;
    }
    if applied {
        target.input.clear();
        target.typo_index = None;
        report.interrupt(target_id);
        if ai_players.contains(&target_id) {
            report.reset_ai(target_id);
        }
        let cyclone = item_registry.cyclone_effect();
        let effects = effects.entry(target_id).or_default();
        effects.stunned_until = Some(now + Duration::from_millis(cyclone.stun_ms));
        effects.impact_cue = Some(RaceImpactCue {
            kind: RaceImpactCueKind::Cyclone,
            until: now + Duration::from_millis(1_200),
        });
    }
    applied
}

fn apply_squid_ink_to_player(
    race: &mut RaceState,
    effects: &mut HashMap<RacePlayerId, RaceItemEffectState>,
    item_registry: &ItemRegistry,
    target_id: RacePlayerId,
    now: Instant,
    report: &mut ItemActivationReport,
) -> bool {
    let Some(target_index) = race
        .players
        .iter()
        .position(|player| player.id == target_id)
    else {
        return false;
    };
    let target_name = race.players[target_index].name.clone();

    if race.players[target_index].state.has_active_shield(now) {
        race.players[target_index]
            .state
            .active_effects
            .retain(|effect| !matches!(effect, ActiveEffect::Shield { .. }));
        effects.entry(target_id).or_default().impact_cue = Some(RaceImpactCue {
            kind: RaceImpactCueKind::ShieldBlock,
            until: now + Duration::from_millis(700),
        });
        report
            .events
            .push(format!("{target_name} blocked Squid Ink"));
        return false;
    }

    let target = &mut race.players[target_index].state;
    let squid_ink = item_registry.squid_ink_effect();
    target.inked_word_index = Some(target.word_index);
    target.inked_until = Some(now + Duration::from_millis(squid_ink.duration_ms));
    effects.entry(target_id).or_default().impact_cue = Some(RaceImpactCue {
        kind: RaceImpactCueKind::SquidInk,
        until: now + Duration::from_millis(squid_ink.impact_blink_ms),
    });
    true
}

fn advance_mushroom_one_word(race: &mut RaceState, player_id: RacePlayerId, now: Instant) -> bool {
    let Some(player_index) = race
        .players
        .iter()
        .position(|player| player.id == player_id)
    else {
        return false;
    };
    let Some(effect_index) = race.players[player_index]
        .state
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

    let remaining = race
        .track
        .len()
        .saturating_sub(race.players[player_index].state.word_index);
    if remaining == 0 {
        race.players[player_index]
            .state
            .active_effects
            .remove(effect_index);
        return false;
    }

    let player = &mut race.players[player_index];
    player.state.word_index += 1;
    player.state.stats.completed_words += 1;
    player.state.input.clear();
    player.state.typo_index = None;

    if player.state.word_index >= race.track.len() {
        player.state.finished_at = Some(now);
        player.state.active_effects.remove(effect_index);
        return false;
    }

    if let Some(ActiveEffect::Mushroom {
        remaining_words,
        next_step_at,
        step_interval,
    }) = player.state.active_effects.get_mut(effect_index)
    {
        *remaining_words -= 1;
        if *remaining_words == 0 {
            player.state.active_effects.remove(effect_index);
        } else {
            *next_step_at += *step_interval;
        }
    }

    true
}

fn player_label(race: &RaceState, player_id: RacePlayerId) -> String {
    race.player(player_id)
        .map(|player| player.name.clone())
        .unwrap_or_else(|| format!("player {}", player_id.0))
}

fn attack_direction(attacker_word_index: usize, target_word_index: usize) -> AttackDirection {
    match target_word_index.cmp(&attacker_word_index) {
        std::cmp::Ordering::Greater => AttackDirection::Ahead,
        std::cmp::Ordering::Less => AttackDirection::Behind,
        std::cmp::Ordering::Equal => AttackDirection::Overlap,
    }
}

fn mushroom_step_interval(wpm: u32) -> Duration {
    Duration::from_secs_f64(60.0 / f64::from(wpm))
}

fn banana_cue_labels(direction: AttackDirection, display: BananaDisplayConfig) -> (String, String) {
    match direction {
        AttackDirection::Ahead => (display.ascii_ahead, display.unicode_ahead),
        AttackDirection::Behind => (display.ascii_behind, display.unicode_behind),
        AttackDirection::Overlap => (display.ascii_overlap, display.unicode_overlap),
    }
}

fn cyclone_cue_label(direction: AttackDirection, unicode: bool) -> String {
    match (direction, unicode) {
        (AttackDirection::Ahead, false) => " cy>>".to_string(),
        (AttackDirection::Behind, false) => "<<cy ".to_string(),
        (AttackDirection::Overlap, false) => " cy<>".to_string(),
        (AttackDirection::Ahead, true) => " 🌀 >>".to_string(),
        (AttackDirection::Behind, true) => "<< 🌀 ".to_string(),
        (AttackDirection::Overlap, true) => " 🌀 <>".to_string(),
    }
}

fn item_cue_placement(direction: AttackDirection) -> RaceItemCuePlacement {
    match direction {
        AttackDirection::Ahead | AttackDirection::Overlap => RaceItemCuePlacement::After,
        AttackDirection::Behind => RaceItemCuePlacement::Before,
    }
}

#[cfg(test)]
mod tests {
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

        assert!(report.events.iter().any(|event| event == "host hit guest"));
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
}
