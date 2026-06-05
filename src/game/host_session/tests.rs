use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use rand::{SeedableRng, rngs::StdRng};
use typekart_protocol::{
    AiDifficultySnapshot, AssignedColor, LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind,
};

use super::prepare_race_from_selected_lobby_players;
use super::{
    CountdownAdvanceRejection, CountdownRacePreparation, CountdownStartRejection,
    HostRaceTickAction, ReturnToLobbyDecision, advance_active_race_tick,
    advance_countdown_to_racing, advance_host_race_lifecycle, begin_countdown_phase,
    cancel_countdown_outcome, connected_racer_count, countdown_should_cancel, countdown_start_plan,
    countdown_tick_phase, has_connected_active_racer, host_race_tick_outcome,
    prepare_race_from_lobby, prepare_waiting_race_outcome, return_to_lobby_decision,
    return_to_lobby_outcome, start_active_race_runtime_outcome, start_race_from_countdown,
};
use crate::game::{
    ai_driver::{AiDriverConfig, AiDriverState},
    bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
    bonus_flow::{BonusAttempt, BonusClaimRoll, BonusFlowEvent},
    effects::ActiveEffect,
    item_effects::{ItemActivationReport, RaceItemEffectState},
    items::{HeldItem, ItemPickup, ItemRegistry, ItemRollContext, RacePositionBand},
    race::{RaceLifecycleState, RacePlayerId, RaceState},
    track::{Track, WordList},
    typing::{KeyAction, TypingEvent},
};

fn lobby_player(
    id: u64,
    name: &str,
    ready: bool,
    connected: bool,
    kind: PlayerKind,
) -> LobbyPlayer {
    LobbyPlayer {
        id: PlayerId(id),
        name: name.to_string(),
        color: AssignedColor::Cyan,
        ready,
        connected,
        kind,
        ai_difficulty: match kind {
            PlayerKind::Bot => Some(AiDifficultySnapshot::Easy),
            PlayerKind::Human => None,
        },
        ai_wpm: None,
    }
}

#[test]
fn prepare_race_from_lobby_selects_ready_connected_players() {
    let now = Instant::now();
    let players = vec![
        lobby_player(1, "host", true, true, PlayerKind::Human),
        lobby_player(2, "waiting", false, true, PlayerKind::Human),
        lobby_player(3, "offline", true, false, PlayerKind::Human),
        lobby_player(4, "ai-1", true, true, PlayerKind::Bot),
    ];

    let prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string(), "fast".to_string()]),
        &WordList::from_static("go\nfast\nbonus"),
        now,
    );

    assert_eq!(prepared.participant_count(), 2);
    assert_eq!(
        prepared
            .race
            .players
            .iter()
            .map(|player| player.name.as_str())
            .collect::<Vec<_>>(),
        ["host", "ai-1"]
    );
    assert_eq!(prepared.bonuses.points.len(), 0);
}

#[test]
fn host_runtime_reset_outcomes_describe_adapter_work() {
    assert_eq!(
        prepare_waiting_race_outcome(),
        super::PrepareWaitingRaceOutcome {
            phase: NetworkRacePhase::WaitingForHost,
            reset_runtime: true,
            clear_results: true,
            clear_events: true,
            reset_ai_timing: true,
        }
    );
    assert_eq!(
        start_active_race_runtime_outcome(),
        super::StartActiveRaceRuntimeOutcome {
            reset_runtime: true,
            clear_ai_timing: true,
            set_ai_timing_now: true,
        }
    );
}

#[test]
fn connected_active_racer_excludes_finished_players() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let mut prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string()]),
        &WordList::from_static("go\nbonus\nword"),
        now,
    );

    assert_eq!(connected_racer_count(&prepared.race), 1);
    assert!(has_connected_active_racer(&prepared.race));

    prepared.race.players[0].state.finished_at = Some(now);

    assert!(!has_connected_active_racer(&prepared.race));
}

#[test]
fn prepare_race_from_selected_players_keeps_unready_selected_racers() {
    let now = Instant::now();
    let players = vec![
        lobby_player(1, "host", false, true, PlayerKind::Human),
        lobby_player(2, "guest", false, true, PlayerKind::Human),
    ];

    let prepared = prepare_race_from_selected_lobby_players(
        &players,
        Track::new(vec!["go".to_string()]),
        &WordList::from_static("go\nbonus\nword"),
        now,
    );

    assert_eq!(prepared.participant_count(), 2);
}

#[test]
fn countdown_start_policy_identifies_when_race_preparation_is_needed() {
    assert_eq!(
        countdown_start_plan(NetworkRacePhase::Lobby),
        Ok(CountdownRacePreparation::PrepareRace)
    );
    assert_eq!(
        countdown_start_plan(NetworkRacePhase::Finished),
        Ok(CountdownRacePreparation::PrepareRace)
    );
    assert_eq!(
        countdown_start_plan(NetworkRacePhase::WaitingForHost),
        Ok(CountdownRacePreparation::UseCurrentRace)
    );
    assert_eq!(
        countdown_start_plan(NetworkRacePhase::Racing),
        Err(CountdownStartRejection::RaceAlreadyActive)
    );
}

#[test]
fn countdown_begin_requires_connected_racers() {
    assert_eq!(
        begin_countdown_phase(0),
        Err(CountdownStartRejection::NoConnectedRacers)
    );
    assert_eq!(
        begin_countdown_phase(1),
        Ok(NetworkRacePhase::Countdown {
            remaining_seconds: 3
        })
    );
}

#[test]
fn countdown_advance_policy_requires_countdown_and_active_racer() {
    assert_eq!(
        countdown_tick_phase(2),
        NetworkRacePhase::Countdown {
            remaining_seconds: 2
        }
    );
    assert_eq!(
        advance_countdown_to_racing(NetworkRacePhase::Racing, true),
        Err(CountdownAdvanceRejection::NotCountingDown)
    );
    assert_eq!(
        advance_countdown_to_racing(
            NetworkRacePhase::Countdown {
                remaining_seconds: 1
            },
            false,
        ),
        Err(CountdownAdvanceRejection::NoConnectedRacers)
    );
    assert_eq!(
        advance_countdown_to_racing(
            NetworkRacePhase::Countdown {
                remaining_seconds: 1
            },
            true,
        ),
        Ok(NetworkRacePhase::Racing)
    );
    assert_eq!(
        start_race_from_countdown(
            NetworkRacePhase::Countdown {
                remaining_seconds: 1
            },
            true,
        ),
        Ok(super::StartRaceOutcome {
            phase: NetworkRacePhase::Racing,
            event: "Race started"
        })
    );
}

#[test]
fn return_to_lobby_policy_distinguishes_cancel_and_results_return() {
    assert_eq!(
        return_to_lobby_decision(NetworkRacePhase::Racing),
        ReturnToLobbyDecision::CancelRace
    );
    assert_eq!(
        return_to_lobby_decision(NetworkRacePhase::Finished),
        ReturnToLobbyDecision::ReturnFromResults
    );
    assert_eq!(
        return_to_lobby_decision(NetworkRacePhase::Lobby),
        ReturnToLobbyDecision::Ignore
    );
    assert_eq!(
        return_to_lobby_outcome(NetworkRacePhase::Racing).map(|outcome| outcome.event),
        Some("Race cancelled")
    );
    assert_eq!(return_to_lobby_outcome(NetworkRacePhase::Lobby), None);
}

#[test]
fn countdown_cancel_policy_uses_connected_active_racers() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let mut prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string()]),
        &WordList::from_static("go\nbonus\nword"),
        now,
    );

    assert!(!countdown_should_cancel(&prepared.race));

    prepared.race.players[0].connected = false;

    assert!(countdown_should_cancel(&prepared.race));
    assert_eq!(
        cancel_countdown_outcome(),
        super::CountdownCancelOutcome {
            phase: NetworkRacePhase::WaitingForHost,
            event: "Countdown cancelled"
        }
    );
}

#[test]
fn race_tick_policy_selects_adapter_action() {
    assert_eq!(
        host_race_tick_outcome(NetworkRacePhase::Racing, false, 0).action,
        HostRaceTickAction::Ignore
    );
    assert_eq!(
        host_race_tick_outcome(NetworkRacePhase::Racing, true, 0).action,
        HostRaceTickAction::BroadcastDelta
    );
    assert_eq!(
        host_race_tick_outcome(NetworkRacePhase::Racing, false, 2).action,
        HostRaceTickAction::BroadcastDelta
    );
    assert_eq!(
        host_race_tick_outcome(NetworkRacePhase::Finished, false, 0).action,
        HostRaceTickAction::BroadcastResults
    );
}

#[test]
fn active_race_tick_expires_bonuses_effects_and_advances_lifecycle() {
    let now = Instant::now();
    let players = vec![
        lobby_player(1, "host", true, true, PlayerKind::Human),
        lobby_player(2, "joiner", true, true, PlayerKind::Human),
    ];
    let prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string(), "fast".to_string()]),
        &WordList::from_static("go\nfast\nbonus\none\ntwo\nthree"),
        now,
    );
    let mut race = prepared.race;
    race.players[0]
        .state
        .active_effects
        .push(ActiveEffect::Shield {
            until: now - Duration::from_millis(1),
        });
    race.players[0].state.finished_at = Some(now);
    let mut bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice {
                    word: "one".to_string(),
                    status: BonusChoiceStatus::Cooldown {
                        until: now - Duration::from_millis(1),
                    },
                },
                BonusChoice::available("two"),
                BonusChoice::available("three"),
            ],
        )],
        vec!["one".to_string()],
    );
    let mut lifecycle = RaceLifecycleState::default();

    let outcome = advance_active_race_tick(
        &mut lifecycle,
        &mut race,
        &mut bonuses,
        NetworkRacePhase::Racing,
        now,
        Duration::from_secs(15),
        false,
    );

    assert_eq!(outcome.tick.bonus_choices_refreshed, 1);
    assert_eq!(outcome.expired_effect_players, vec![RacePlayerId(1)]);
    assert_eq!(outcome.lifecycle.phase, NetworkRacePhase::Racing);
    assert_eq!(lifecycle.first_finished_at, Some(now));
}

#[test]
fn host_player_key_applies_normal_typing_when_no_bonus_is_claimed() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string()]),
        &WordList::from_static("go\nbonus\nword"),
        now,
    );
    let mut race = prepared.race;
    let mut bonuses = prepared.bonuses;
    let mut attempts = HashMap::new();
    let mut spent_bonus_gaps = HashMap::new();
    let registry = ItemRegistry::builtin();
    let mut rng = StdRng::seed_from_u64(1);

    let outcome = super::apply_host_player_key(
        &mut super::HostPlayerKeyState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent_bonus_gaps,
        },
        super::HostPlayerKeyInput {
            player_key: PlayerId(1),
            race_player_id: RacePlayerId(1),
            action: KeyAction::Char('g'),
            now,
        },
        BonusClaimRoll {
            item_context: item_context(),
            item_registry: &registry,
            rng: &mut rng,
        },
    );

    assert!(outcome.handled);
    assert_eq!(outcome.typing_events, vec![TypingEvent::InputChanged]);
    assert!(outcome.bonus_events.is_empty());
    assert_eq!(race.players[0].state.input, "g");
}

#[test]
fn host_player_key_prefers_available_bonus_attempt_over_track_typing() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string(), "fast".to_string()]),
        &WordList::from_static("go\nfast\ndash\nspin\nzoom"),
        now,
    );
    let mut race = prepared.race;
    race.players[0].state.word_index = 1;
    let mut bonuses = BonusState::with_points(
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
    let mut attempts = HashMap::new();
    let mut spent_bonus_gaps = HashMap::new();
    let registry = ItemRegistry::builtin();
    let mut rng = StdRng::seed_from_u64(1);

    let outcome = super::apply_host_player_key(
        &mut super::HostPlayerKeyState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent_bonus_gaps,
        },
        super::HostPlayerKeyInput {
            player_key: PlayerId(1),
            race_player_id: RacePlayerId(1),
            action: KeyAction::Char('d'),
            now,
        },
        BonusClaimRoll {
            item_context: item_context(),
            item_registry: &registry,
            rng: &mut rng,
        },
    );

    assert!(outcome.handled);
    assert!(outcome.typing_events.is_empty());
    assert_eq!(race.players[0].state.input, "d");
    assert!(attempts.contains_key(&PlayerId(1)));
    assert!(outcome.bonus_events.iter().any(|event| {
        matches!(
            event,
            BonusFlowEvent::AttemptStarted(BonusAttempt {
                point_index: 0,
                choice_index: 0
            })
        )
    }));
}

#[test]
fn host_item_pickup_applies_shared_item_rules() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string(), "fast".to_string()]),
        &WordList::from_static("go\nfast\nbonus"),
        now,
    );
    let mut race = prepared.race;
    let mut effects = HashMap::new();
    let ai_players = HashSet::new();
    let registry = ItemRegistry::builtin();

    let report = super::apply_host_item_pickup(
        &mut super::HostItemPickupState {
            race: &mut race,
            effects: &mut effects,
            ai_players: &ai_players,
            item_registry: &registry,
        },
        super::HostItemPickupInput {
            player_id: RacePlayerId(1),
            item: ItemPickup::Shield,
            now,
        },
    );

    assert!(report.events.is_empty());
    assert!(race.players[0].state.has_active_shield(now));
}

#[test]
fn host_ai_tick_advances_typing_with_shared_effect_gating() {
    let now = Instant::now();
    let mut race = RaceState::new(Track::new(vec!["go".to_string(), "fast".to_string()]));
    race.add_player(
        RacePlayerId(1),
        "ai-1",
        crate::game::race::PlayerColorId::Cyan,
        now,
    );
    let mut driver = AiDriverState::default();

    let advance = super::advance_host_ai_racer_tick(
        &mut race,
        &HashMap::new(),
        &mut driver,
        super::HostAiTickInput {
            player_id: RacePlayerId(1),
            config: AiDriverConfig {
                base_wpm: 12.0,
                focus_boost_wpm: 0,
                fog_multiplier_percent: 100,
            },
            now,
            elapsed: Duration::from_secs(1),
        },
    );

    assert!(advance.changed());
    assert_eq!(race.players[0].state.input, "g");
}

#[test]
fn host_ai_tick_pauses_when_item_effects_stun_player() {
    let now = Instant::now();
    let mut race = RaceState::new(Track::new(vec!["go".to_string(), "fast".to_string()]));
    race.add_player(
        RacePlayerId(1),
        "ai-1",
        crate::game::race::PlayerColorId::Cyan,
        now,
    );
    let mut driver = AiDriverState::default();
    let mut effects = HashMap::new();
    effects.insert(
        RacePlayerId(1),
        RaceItemEffectState {
            stunned_until: Some(now + Duration::from_secs(1)),
            ..Default::default()
        },
    );

    let advance = super::advance_host_ai_racer_tick(
        &mut race,
        &effects,
        &mut driver,
        super::HostAiTickInput {
            player_id: RacePlayerId(1),
            config: AiDriverConfig {
                base_wpm: 600.0,
                focus_boost_wpm: 0,
                fog_multiplier_percent: 100,
            },
            now,
            elapsed: Duration::from_secs(1),
        },
    );

    assert!(!advance.changed());
    assert_eq!(race.players[0].state.input, "");
    assert_eq!(driver.char_budget, 0.0);
}

#[test]
fn item_aftermath_actions_preserve_interruption_reset_and_events() {
    let report = ItemActivationReport {
        interrupted_players: vec![RacePlayerId(1)],
        reset_ai_players: vec![RacePlayerId(2)],
        events: vec![super::HostEvent::ItemHit {
            attacker_id: RacePlayerId(2),
            attacker_name: "ai-1".to_string(),
            target_id: RacePlayerId(1),
            target_name: "host".to_string(),
            item: HeldItem::Banana,
        }],
    };

    let aftermath = super::host_item_aftermath_actions(report);

    assert_eq!(aftermath.interrupted_players, vec![RacePlayerId(1)]);
    assert_eq!(aftermath.reset_ai_players, vec![RacePlayerId(2)]);
    assert_eq!(aftermath.events[0].message(), "ai-1 hit host");
}

#[test]
fn host_aftermath_adapter_actions_preserve_shared_order() {
    let actions = super::host_aftermath_adapter_actions(super::HostItemAftermath {
        interrupted_players: vec![RacePlayerId(1)],
        reset_ai_players: vec![RacePlayerId(2)],
        events: vec![super::HostEvent::ItemBlocked {
            target_id: RacePlayerId(3),
            target_name: "guest".to_string(),
            item: HeldItem::Cyclone,
        }],
    });

    assert_eq!(
        actions,
        vec![
            super::HostAftermathAction::ClearBonusAttempt(RacePlayerId(1)),
            super::HostAftermathAction::ResetAiDriver(RacePlayerId(2)),
            super::HostAftermathAction::EmitEvent(super::HostEvent::ItemBlocked {
                target_id: RacePlayerId(3),
                target_name: "guest".to_string(),
                item: HeldItem::Cyclone,
            }),
        ]
    );
}

#[test]
fn host_bonus_claim_reports_pickup_and_applies_item() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string(), "fast".to_string()]),
        &WordList::from_static("go\nfast\nbonus"),
        now,
    );
    let mut race = prepared.race;
    let mut effects = HashMap::new();
    let ai_players = HashSet::new();
    let registry = ItemRegistry::builtin();

    let outcome = super::apply_host_bonus_claim(
        &mut super::HostItemPickupState {
            race: &mut race,
            effects: &mut effects,
            ai_players: &ai_players,
            item_registry: &registry,
        },
        super::HostBonusClaimInput {
            player_id: RacePlayerId(1),
            player_name: "host".to_string(),
            pickup: Some(ItemPickup::Shield),
            now,
        },
    );

    assert_eq!(outcome.pickup, Some(ItemPickup::Shield));
    assert_eq!(outcome.aftermath.events[0].message(), "host got Shield");
    assert!(race.players[0].state.has_active_shield(now));
}

#[test]
fn host_bonus_claim_reports_missed_bonus_without_item_effects() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string(), "fast".to_string()]),
        &WordList::from_static("go\nfast\nbonus"),
        now,
    );
    let mut race = prepared.race;
    let mut effects = HashMap::new();
    let ai_players = HashSet::new();
    let registry = ItemRegistry::builtin();

    let outcome = super::apply_host_bonus_claim(
        &mut super::HostItemPickupState {
            race: &mut race,
            effects: &mut effects,
            ai_players: &ai_players,
            item_registry: &registry,
        },
        super::HostBonusClaimInput {
            player_id: RacePlayerId(1),
            player_name: "host".to_string(),
            pickup: None,
            now,
        },
    );

    assert_eq!(outcome.pickup, None);
    assert_eq!(
        outcome.aftermath.events[0].message(),
        "host missed the bonus"
    );
    assert!(!race.players[0].state.has_active_shield(now));
}

#[test]
fn host_ai_bonus_claim_rolls_and_applies_pickup() {
    let now = Instant::now();
    let mut race = RaceState::new(Track::new(vec!["go".to_string(), "fast".to_string()]));
    race.add_player(
        RacePlayerId(1),
        "ai-1",
        crate::game::race::PlayerColorId::Cyan,
        now,
    );
    race.players[0].state.word_index = 1;
    let mut bonuses = BonusState::with_points(
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
    let mut attempts = HashMap::new();
    let mut spent_bonus_gaps = HashMap::new();
    let mut effects = HashMap::new();
    let ai_players = HashSet::from([RacePlayerId(1)]);
    let registry = ItemRegistry::builtin();
    let mut rng = StdRng::seed_from_u64(1);

    let outcome = super::try_host_ai_bonus_claim(
        &mut super::HostAiBonusClaimState {
            race: &mut race,
            bonuses: &mut bonuses,
            bonus_attempts: &mut attempts,
            spent_bonus_gaps: &mut spent_bonus_gaps,
            effects: &mut effects,
            ai_players: &ai_players,
            item_registry: &registry,
        },
        super::HostAiBonusClaimInput {
            player_key: RacePlayerId(1),
            player_id: RacePlayerId(1),
            player_name: "ai-1".to_string(),
            item_context: item_context(),
            now,
        },
        &mut rng,
    )
    .expect("AI should claim an available bonus");

    assert!(outcome.pickup.is_some());
    assert!(
        outcome.aftermath.events[0]
            .message()
            .starts_with("ai-1 got ")
    );
    assert_eq!(spent_bonus_gaps.get(&RacePlayerId(1)), Some(&0));
}

#[test]
fn finalized_host_race_results_use_shared_protocol_projection() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let mut prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string()]),
        &WordList::from_static("go\nbonus"),
        now,
    );
    prepared.race.players[0].state.finished_at = Some(now);

    let results = super::finalize_host_race_results(&prepared.race, &[RacePlayerId(1)], now);

    assert_eq!(results.placements, vec![PlayerId(1)]);
    assert_eq!(results.rows.len(), 1);
    assert_eq!(results.events, vec![super::HostEvent::RaceFinished]);
}

#[test]
fn host_race_lifecycle_advances_only_while_racing() {
    let now = Instant::now();
    let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
    let mut prepared = prepare_race_from_lobby(
        &players,
        Track::new(vec!["go".to_string()]),
        &WordList::from_static("go\nbonus\nword"),
        now,
    );
    prepared.race.players[0].state.finished_at = Some(now);
    let mut lifecycle = RaceLifecycleState::new();

    let waiting = advance_host_race_lifecycle(
        &mut lifecycle,
        &prepared.race,
        NetworkRacePhase::WaitingForHost,
        now,
        Duration::from_secs(30),
    );

    assert_eq!(waiting.phase, NetworkRacePhase::WaitingForHost);
    assert!(waiting.flow.finished.is_none());
    assert!(lifecycle.placements.is_empty());

    let racing = advance_host_race_lifecycle(
        &mut lifecycle,
        &prepared.race,
        NetworkRacePhase::Racing,
        now,
        Duration::from_secs(30),
    );

    assert_eq!(racing.phase, NetworkRacePhase::Finished);
    assert_eq!(racing.flow.newly_finished[0].player_id, RacePlayerId(1));
    assert!(racing.flow.finished.is_some());
    assert_eq!(racing.finish_event, Some("Race finished"));
}

fn item_context() -> ItemRollContext {
    ItemRollContext {
        has_nearby_racer: false,
        position: RacePositionBand::Middle,
    }
}
