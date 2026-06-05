use std::time::{Duration, Instant};

use crate::{
    game::{
        ai::AiDifficulty,
        ai_driver::ai_effective_wpm,
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
fn fogged_ai_racer_hesitates_from_reduced_wpm_budget() {
    let now = Instant::now();
    let mut session = LocalSession::with_bonuses(
        track(&["one", "two"]),
        PlayerState::new(now),
        BonusState::with_points(vec![], vec![]),
    );
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 60.0, now);
    ai.player.fogged_word_index = Some(0);
    ai.player.fogged_until = Some(now + Duration::from_secs(5));
    session.ai_racers.push(ai);

    session.tick(now + Duration::from_secs(1));

    assert_eq!(session.ai_racers[0].player.word_index, 0);
    assert_eq!(session.ai_racers[0].player.input, "one");
}

#[test]
fn focused_ai_racer_gets_small_wpm_boost() {
    assert_eq!(ai_effective_wpm(60.0, true, false, 10, 70), 70.0);
    assert_eq!(ai_effective_wpm(60.0, false, false, 10, 70), 60.0);
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
    assert!(
        session
            .events
            .entries()
            .any(|entry| entry == "you hit ai-2")
    );
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
    assert!(!entries.contains(&"you hit ai-1"));
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

    session.receive_pickup(Some(ItemPickup::Held(HeldItem::Mushroom)), Instant::now());

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
fn ai_mushroom_resets_typing_budget_after_shared_interruption() {
    let track = track(&["one", "two", "three", "four", "five"]);
    let player = PlayerState::new(Instant::now());
    let start = Instant::now();
    let mut session =
        LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));
    session.ai_racers.push(AiRacer::new(
        1,
        AiDifficulty::Easy,
        35.0,
        start - std::time::Duration::from_secs(1),
    ));
    session.ai_racers[0].driver.char_budget = 4.0;

    session.receive_ai_pickup(0, ItemPickup::Held(HeldItem::Mushroom), start);

    assert_eq!(session.ai_racers[0].player.word_index, 1);
    assert_eq!(session.ai_racers[0].driver.char_budget, 0.0);
    assert_eq!(session.ai_racers[0].driver.last_update, Some(start));
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

    session.receive_pickup(Some(ItemPickup::Shield), Instant::now());

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
    session.receive_pickup(Some(ItemPickup::Shield), now);

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

    session.receive_pickup(Some(ItemPickup::Held(HeldItem::Focus)), now);
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
            .any(|entry| entry == "you hit ai-1 with Cyclone")
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
fn fog_hits_all_ai_racers_in_range() {
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
    session.player.held_item = Some(HeldItem::Fog);

    session.apply_action(LocalAction::ActivateItem, now);

    assert!(session.ai_racers[0].player.is_fogged_at(now));
    assert!(!session.ai_racers[1].player.is_fogged_at(now));
    assert!(matches!(
        session.ai_racers[0].impact_cue.map(|cue| cue.kind),
        Some(ImpactCueKind::Fog)
    ));
    assert!(matches!(
        session.player_item_cue.as_ref().map(|cue| cue.kind),
        Some(ItemCueKind::Fog)
    ));
}

#[test]
fn fog_persists_after_current_word_is_completed() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let mut player = PlayerState::new(now);
    player.fogged_word_index = Some(0);
    player.fogged_until = Some(now + Duration::from_secs(5));
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

    assert!(session.player.is_fogged_at(now));
    assert_eq!(session.player.word_index, 1);
    assert_eq!(session.player.fogged_word_index, Some(0));
}

#[test]
fn fog_expires_after_duration() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let mut player = PlayerState::new(now);
    player.fogged_word_index = Some(0);
    player.fogged_until = Some(now + Duration::from_secs(5));
    let mut session =
        LocalSession::with_bonuses(track, player, BonusState::with_points(vec![], vec![]));

    session.tick(now + Duration::from_secs(5));

    assert!(!session.player.is_fogged_at(now + Duration::from_secs(5)));
    assert_eq!(session.player.fogged_word_index, None);
    assert_eq!(session.player.fogged_until, None);
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
            .any(|entry| entry == "you missed Banana")
    );
}
