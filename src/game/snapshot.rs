//! Shared protocol snapshot builders.
//!
//! Hosts own when snapshots are emitted and which event messages are attached.
//! This module owns the browser-safe conversion from authoritative race state
//! into protocol-shaped race/player/bonus snapshots.

use std::{collections::HashMap, time::Instant};

use typekart_protocol::{
    AssignedColor, AttackDirectionSnapshot, BonusChoiceSnapshot, BonusChoiceSnapshotStatus,
    BonusPointSnapshot, ImpactCueSnapshot, ImpactCueSnapshotKind, ItemCuePlacementSnapshot,
    ItemCueSnapshot, ItemCueSnapshotKind, ModConfigSnapshot, NetworkRacePhase, PlayerId,
    PlayerKind, PlayerSnapshot, RaceDeltaSnapshot, RaceSnapshot, WordOverrideSnapshot,
};

use super::{
    bonus::{BonusChoiceStatus, BonusState},
    item_effects::{
        AttackDirection, RaceImpactCue, RaceImpactCueKind, RaceItemCue, RaceItemCueKind,
        RaceItemCuePlacement, RaceItemEffectState, player_has_active_mushroom_effect,
    },
    race::{PlayerColorId, RacePlayerId, RaceState},
};

pub struct RaceSnapshotInput<'a> {
    pub sequence: u64,
    pub phase: NetworkRacePhase,
    pub mod_config: ModConfigSnapshot,
    pub race: &'a RaceState,
    pub bonuses: &'a BonusState,
    pub player_effects: &'a HashMap<RacePlayerId, RaceItemEffectState>,
    pub events: Vec<String>,
    pub now: Instant,
}

pub struct RaceDeltaSnapshotInput<'a> {
    pub sequence: u64,
    pub phase: NetworkRacePhase,
    pub race: &'a RaceState,
    pub bonuses: &'a BonusState,
    pub player_effects: &'a HashMap<RacePlayerId, RaceItemEffectState>,
    pub events: Vec<String>,
    pub now: Instant,
}

pub fn build_race_snapshot(
    input: RaceSnapshotInput<'_>,
    player_kind: impl Fn(PlayerId) -> PlayerKind,
) -> RaceSnapshot {
    RaceSnapshot {
        sequence: input.sequence,
        phase: input.phase,
        mod_config: input.mod_config,
        track_words: input.race.track.words.clone(),
        bonuses: build_bonus_snapshots(input.bonuses, input.now),
        players: build_player_snapshots(input.race, input.player_effects, input.now, player_kind),
        events: input.events,
    }
}

pub fn build_race_delta_snapshot(
    input: RaceDeltaSnapshotInput<'_>,
    player_kind: impl Fn(PlayerId) -> PlayerKind,
) -> RaceDeltaSnapshot {
    RaceDeltaSnapshot {
        sequence: input.sequence,
        phase: input.phase,
        bonuses: build_bonus_snapshots(input.bonuses, input.now),
        players: build_player_snapshots(input.race, input.player_effects, input.now, player_kind),
        events: input.events,
    }
}

pub fn build_player_snapshots(
    race: &RaceState,
    player_effects: &HashMap<RacePlayerId, RaceItemEffectState>,
    now: Instant,
    player_kind: impl Fn(PlayerId) -> PlayerKind,
) -> Vec<PlayerSnapshot> {
    race.players
        .iter()
        .map(|player| {
            let player_id = PlayerId(player.id.0);
            let effects = player_effects.get(&player.id).cloned().unwrap_or_default();
            PlayerSnapshot {
                id: player_id,
                name: player.name.clone(),
                kind: player_kind(player_id),
                color: assigned_color(player.color),
                word_index: player.state.word_index,
                input: player.state.input.clone(),
                typo_index: player.state.typo_index,
                word_overrides: player
                    .state
                    .word_overrides
                    .iter()
                    .map(|(word_index, word)| WordOverrideSnapshot {
                        word_index: *word_index,
                        word: word.clone(),
                    })
                    .collect(),
                finished: player.state.is_finished(),
                connected: player.connected,
                shielded: player.state.has_active_shield(now),
                focused: player.state.has_active_focus(now),
                inked: player.state.is_inked_at(now),
                boosted: player_has_active_mushroom_effect(player, now),
                stunned: effects.stunned_until.is_some_and(|until| until > now),
                impact_remaining_ms: remaining_ms(effects.impact_cue.map(|cue| cue.until), now),
                impact_cue: build_impact_cue_snapshot(effects.impact_cue, now),
                item_cue: build_item_cue_snapshot(effects.item_cue, now),
            }
        })
        .collect()
}

pub fn build_bonus_snapshots(bonuses: &BonusState, now: Instant) -> Vec<BonusPointSnapshot> {
    bonuses
        .points
        .iter()
        .map(|point| BonusPointSnapshot {
            after_word_index: point.after_word_index,
            choices: point
                .choices
                .iter()
                .map(|choice| BonusChoiceSnapshot {
                    word: choice.word.clone(),
                    status: match choice.status {
                        BonusChoiceStatus::Available => BonusChoiceSnapshotStatus::Available,
                        BonusChoiceStatus::Cooldown { until } if until <= now => {
                            BonusChoiceSnapshotStatus::Available
                        }
                        BonusChoiceStatus::Cooldown { until } => {
                            BonusChoiceSnapshotStatus::Cooldown {
                                remaining_ms: remaining_ms(Some(until), now),
                            }
                        }
                    },
                })
                .collect(),
        })
        .collect()
}

pub fn build_item_cue_snapshot(cue: Option<RaceItemCue>, now: Instant) -> Option<ItemCueSnapshot> {
    let cue = cue.filter(|cue| cue.until > now)?;
    Some(ItemCueSnapshot {
        kind: match cue.kind {
            RaceItemCueKind::Banana { direction } => ItemCueSnapshotKind::Banana {
                direction: attack_direction(direction),
            },
            RaceItemCueKind::Cyclone { direction } => ItemCueSnapshotKind::Cyclone {
                direction: attack_direction(direction),
            },
            RaceItemCueKind::SquidInk => ItemCueSnapshotKind::SquidInk,
        },
        ascii_label: cue.ascii_label,
        unicode_label: cue.unicode_label,
        placement: match cue.placement {
            RaceItemCuePlacement::Before => ItemCuePlacementSnapshot::Before,
            RaceItemCuePlacement::After => ItemCuePlacementSnapshot::After,
        },
        remaining_ms: remaining_ms(Some(cue.until), now),
    })
}

pub fn build_impact_cue_snapshot(
    cue: Option<RaceImpactCue>,
    now: Instant,
) -> Option<ImpactCueSnapshot> {
    let cue = cue.filter(|cue| cue.until > now)?;
    Some(ImpactCueSnapshot {
        kind: match cue.kind {
            RaceImpactCueKind::Banana => ImpactCueSnapshotKind::Banana,
            RaceImpactCueKind::Cyclone => ImpactCueSnapshotKind::Cyclone,
            RaceImpactCueKind::SquidInk => ImpactCueSnapshotKind::SquidInk,
            RaceImpactCueKind::ShieldBlock => ImpactCueSnapshotKind::ShieldBlock,
        },
        remaining_ms: remaining_ms(Some(cue.until), now),
    })
}

pub fn assigned_color(color: PlayerColorId) -> AssignedColor {
    match color {
        PlayerColorId::Cyan => AssignedColor::Cyan,
        PlayerColorId::Red => AssignedColor::Red,
        PlayerColorId::Green => AssignedColor::Green,
        PlayerColorId::Blue => AssignedColor::Blue,
        PlayerColorId::Yellow => AssignedColor::Yellow,
        PlayerColorId::Magenta => AssignedColor::Magenta,
    }
}

pub fn player_color_id(color: AssignedColor) -> PlayerColorId {
    match color {
        AssignedColor::Cyan => PlayerColorId::Cyan,
        AssignedColor::Red => PlayerColorId::Red,
        AssignedColor::Green => PlayerColorId::Green,
        AssignedColor::Blue => PlayerColorId::Blue,
        AssignedColor::Yellow => PlayerColorId::Yellow,
        AssignedColor::Magenta => PlayerColorId::Magenta,
    }
}

pub fn remaining_ms(until: Option<Instant>, now: Instant) -> u64 {
    until
        .filter(|until| *until > now)
        .map(|until| until.saturating_duration_since(now).as_millis() as u64)
        .unwrap_or(0)
}

fn attack_direction(direction: AttackDirection) -> AttackDirectionSnapshot {
    match direction {
        AttackDirection::Ahead => AttackDirectionSnapshot::Ahead,
        AttackDirection::Behind => AttackDirectionSnapshot::Behind,
        AttackDirection::Overlap => AttackDirectionSnapshot::Overlap,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use super::{build_bonus_snapshots, build_player_snapshots};
    use crate::game::{
        bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
        race::{PlayerColorId, RacePlayerId, RaceState},
        track::Track,
    };

    #[test]
    fn bonus_snapshot_reports_expired_cooldowns_as_available() {
        let now = std::time::Instant::now();
        let bonuses = BonusState::with_points(
            vec![BonusPoint::new(
                0,
                [
                    BonusChoice {
                        word: "dash".to_string(),
                        status: BonusChoiceStatus::Cooldown {
                            until: now - Duration::from_secs(1),
                        },
                    },
                    BonusChoice::available("spin"),
                    BonusChoice::available("zoom"),
                ],
            )],
            Vec::new(),
        );

        let snapshot = build_bonus_snapshots(&bonuses, now);

        assert!(matches!(
            snapshot[0].choices[0].status,
            typekart_protocol::BonusChoiceSnapshotStatus::Available
        ));
    }

    #[test]
    fn player_snapshot_contains_core_typing_state() {
        let now = std::time::Instant::now();
        let mut race = RaceState::new(Track::new(vec!["one".to_string(), "two".to_string()]));
        race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);
        race.players[0].state.word_index = 1;
        race.players[0].state.input = "tw".to_string();

        let snapshot = build_player_snapshots(&race, &HashMap::new(), now, |_| {
            typekart_protocol::PlayerKind::Human
        });

        assert_eq!(snapshot[0].id, typekart_protocol::PlayerId(1));
        assert_eq!(snapshot[0].word_index, 1);
        assert_eq!(snapshot[0].input, "tw");
        assert_eq!(snapshot[0].color, typekart_protocol::AssignedColor::Cyan);
    }
}
