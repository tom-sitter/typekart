    use super::*;
    use leptos::prelude::signal;
    use typekart::game::bonus::{BonusChoice, BonusPoint, BonusState};
    use typekart::game::items::{HeldItem, ItemPickup};
    use typekart::game::race::RacePlayerId;
    use typekart_protocol::RaceResultStatus;


#[test]
fn browser_host_lobby_starts_with_ready_host() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let lobby = BrowserHostLobby::new(room, "web-host".to_string());

    assert_eq!(lobby.players.len(), 1);
    assert_eq!(lobby.players[0].id, PlayerId(1));
    assert_eq!(lobby.players[0].name, "web-host");
    assert_eq!(lobby.players[0].color, AssignedColor::Cyan);
    assert!(lobby.players[0].ready);
}

#[test]
fn browser_host_assigns_joiner_from_relay_pending_id() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());

    let player = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");

    assert_eq!(player.id, PlayerId(2));
    assert_eq!(player.name, "laura");
    assert_eq!(player.color, AssignedColor::Red);
    assert!(!player.ready);
    assert_eq!(lobby.next_player_id, 3);
    assert_eq!(
        lobby.game_player_id_for_relay(PlayerId(4)),
        Some(PlayerId(2))
    );
}

#[test]
fn browser_host_joiner_id_does_not_collide_with_existing_ai() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby.players.push(LobbyPlayer {
        id: PlayerId(2),
        name: "ai-1".to_string(),
        kind: PlayerKind::Bot,
        color: AssignedColor::Red,
        ready: true,
        connected: true,
        ai_difficulty: Some(AiDifficultySnapshot::Easy),
        ai_wpm: Some(browser_ai_wpm(AiDifficultySnapshot::Easy)),
    });

    let player = add_browser_lobby_human(&mut lobby, PlayerId(2), "laura");

    assert_eq!(lobby.players[1].name, "ai-1");
    assert_eq!(player.id, PlayerId(3));
    assert_eq!(
        lobby.game_player_id_for_relay(PlayerId(2)),
        Some(PlayerId(3))
    );
}

#[test]
fn browser_lobby_names_are_deduped() {
    let existing = [
        LobbyPlayer {
            id: PlayerId(1),
            name: "tom".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Cyan,
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        },
        LobbyPlayer {
            id: PlayerId(2),
            name: "tom2".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Red,
            ready: false,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        },
    ];

    assert_eq!(unique_lobby_name(existing.iter(), "tom"), "tom3");
}

#[test]
fn browser_ai_wpm_tracks_difficulty() {
    assert!(
        browser_ai_wpm(AiDifficultySnapshot::Hard) > browser_ai_wpm(AiDifficultySnapshot::Easy)
    );
}

#[test]
fn browser_generated_track_uses_shared_track_length() {
    let words = browser_generate_track_words();

    assert_eq!(words.len(), BROWSER_HOST_TRACK_WORD_COUNT);
    assert!(words.iter().all(|word| !word.is_empty()));
}

#[test]
fn browser_generated_track_includes_bonus_snapshots() {
    let words = browser_generate_track_words();
    let bonuses = browser_generate_bonus_state(&words);
    let snapshots = build_bonus_snapshots(&bonuses, std::time::Instant::now());

    assert!(!snapshots.is_empty());
    assert!(snapshots.iter().all(|point| point.choices.len() == 3));
    assert!(
        snapshots
            .iter()
            .all(|point| point.after_word_index < words.len() - 1)
    );
}

#[test]
fn browser_host_race_snapshot_uses_lobby_racers() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];

    let snapshot = browser_host_race_snapshot(
        7,
        NetworkRacePhase::Countdown {
            remaining_seconds: 3,
        },
        &lobby.mod_config,
        &racers,
        vec!["countdown 3".to_string()],
    );

    assert_eq!(snapshot.sequence, 7);
    assert_eq!(
        snapshot.phase,
        NetworkRacePhase::Countdown {
            remaining_seconds: 3
        }
    );
    assert_eq!(snapshot.players.len(), 2);
    assert_eq!(snapshot.players[0].id, PlayerId(1));
    assert_eq!(snapshot.players[1].id, PlayerId(2));
    assert_eq!(
        snapshot.track_words.first().map(String::as_str),
        Some("spark")
    );
    assert!(snapshot.bonuses.is_empty());
}

#[test]
fn browser_host_race_key_input_advances_words() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby
        .item_registry
        .items
        .retain(|item| item.id.as_str() == "banana");
    let racers = vec![lobby.players[0].clone()];
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        browser_demo_track_words(),
        Vec::new(),
    );
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for ch in "spark".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }
    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Space,
        set_connection,
    );

    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.word_index, 1);
    assert_eq!(player.input, "");
    assert_eq!(player.typo_index, None);
}

#[test]
fn browser_host_race_key_input_finishes_final_word_without_space() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let racers = vec![lobby.players[0].clone()];
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        vec!["go".to_string()],
        Vec::new(),
    );
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Char('g'),
        set_connection,
    );
    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Char('o'),
        set_connection,
    );

    let results = lobby.active_results.as_ref().unwrap();
    assert_eq!(results.placements, vec![PlayerId(1)]);
    assert_eq!(results.rows[0].player_id, PlayerId(1));
    assert_eq!(results.rows[0].progress_words, 1);
}

#[test]
fn browser_host_race_results_rank_racers_by_finish_order() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        vec!["go".to_string()],
        Vec::new(),
    );
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for player_id in [PlayerId(2), PlayerId(1)] {
        apply_browser_host_race_key_input(
            &mut lobby,
            player_id,
            ProtocolKey::Char('g'),
            set_connection,
        );
        apply_browser_host_race_key_input(
            &mut lobby,
            player_id,
            ProtocolKey::Char('o'),
            set_connection,
        );
    }

    let results = lobby.active_results.as_ref().unwrap();
    assert_eq!(results.placements, vec![PlayerId(2), PlayerId(1)]);
    assert_eq!(results.rows.len(), 2);
    assert_eq!(results.rows[0].player_id, PlayerId(2));
    assert_eq!(results.rows[0].placement, 1);
    assert_eq!(results.rows[0].progress_words, 1);
    assert_eq!(results.rows[1].player_id, PlayerId(1));
    assert_eq!(results.rows[1].placement, 2);
    assert!(lobby.active_race.is_none());
}

#[test]
fn browser_host_race_results_timeout_places_unfinished_racers_by_progress() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        vec!["go".to_string(), "fast".to_string()],
        Vec::new(),
    );
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for ch in "gof".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(2),
            ProtocolKey::Char(ch),
            set_connection,
        );
        if ch == 'o' {
            apply_browser_host_race_key_input(
                &mut lobby,
                PlayerId(2),
                ProtocolKey::Space,
                set_connection,
            );
        }
    }
    for ch in "go".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }
    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Space,
        set_connection,
    );
    for ch in "fast".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }

    let first_finished_at = lobby.runtime.lifecycle.first_finished_at.unwrap();
    assert!(browser_update_race_status(
        &mut lobby,
        first_finished_at + BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT
    ));

    let results = lobby.active_results.as_ref().unwrap();
    assert_eq!(results.placements, vec![PlayerId(1), PlayerId(2)]);
    assert_eq!(results.rows[0].status, RaceResultStatus::Finished);
    assert_eq!(results.rows[1].status, RaceResultStatus::TimedOut);
    assert_eq!(results.rows[1].progress_words, 1);
}

#[test]
fn browser_host_restart_command_returns_results_to_lobby() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby.active_results = Some(crate::fixtures::ResultsFrame {
        placements: vec![PlayerId(1)],
        rows: Vec::new(),
        events: vec!["Race finished".to_string()],
    });
    lobby.active_race = Some(browser_host_race_snapshot(
        1,
        NetworkRacePhase::Finished,
        &lobby.mod_config,
        &lobby.players.clone(),
        Vec::new(),
    ));
    lobby.core_race = Some(browser_host_core_race(
        &lobby.players.clone(),
        browser_demo_track_words(),
    ));
    lobby.runtime.lifecycle.placements = vec![RacePlayerId(1)];

    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::RestartRace,
        signal(ConnectionState::Disconnected).1,
    );

    assert!(lobby.active_results.is_none());
    assert!(lobby.active_race.is_none());
    assert!(lobby.core_race.is_none());
    assert!(lobby.runtime.lifecycle.placements.is_empty());
    assert!(lobby.events.iter().any(|event| event == "Returned to lobby"));
}

#[test]
fn browser_host_race_key_input_marks_and_clears_typos() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let racers = vec![lobby.players[0].clone()];
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        browser_demo_track_words(),
        Vec::new(),
    );
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Char('x'),
        set_connection,
    );
    assert_eq!(
        lobby.active_race.as_ref().unwrap().players[0].typo_index,
        Some(0)
    );

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Backspace,
        set_connection,
    );
    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.input, "");
    assert_eq!(player.typo_index, None);
}

#[test]
fn browser_host_bonus_word_claims_choice_after_space() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby
        .item_registry
        .items
        .retain(|item| item.id.as_str() == "shield");
    let racers = vec![lobby.players[0].clone()];
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        vec!["one".to_string(), "two".to_string()],
        Vec::new(),
    );
    lobby.core_race.as_mut().unwrap().players[0].state.word_index = 1;
    lobby.bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice::available("dash"),
                BonusChoice::available("drift"),
                BonusChoice::available("turbo"),
            ],
        )],
        vec!["dash".to_string(), "drift".to_string(), "turbo".to_string()],
    );
    browser_sync_active_race_from_core(&mut lobby);
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for ch in "dash".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }

    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.word_index, 1);
    assert_eq!(player.input, "dash");
    assert!(lobby.runtime.bonus_attempts.contains_key(&PlayerId(1)));
    assert!(matches!(
        lobby.active_race.as_ref().unwrap().bonuses[0].choices[0].status,
        typekart_protocol::BonusChoiceSnapshotStatus::Available
    ));

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Space,
        set_connection,
    );

    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.word_index, 1);
    assert_eq!(player.input, "");
    assert_eq!(player.typo_index, None);
    assert!(!lobby.runtime.bonus_attempts.contains_key(&PlayerId(1)));
    assert_eq!(lobby.runtime.spent_bonus_gaps.get(&PlayerId(1)), Some(&0));
    assert!(matches!(
        lobby.active_race.as_ref().unwrap().bonuses[0].choices[0].status,
        typekart_protocol::BonusChoiceSnapshotStatus::Cooldown { .. }
    ));
    assert!(lobby.events.iter().any(|event| event.contains("got")));
    assert!(
        !lobby.active_race.as_ref().unwrap().events[0].contains("typed"),
        "bonus pickup or item event should be visible in the race feed"
    );
}

#[test]
fn browser_host_banana_activation_resets_target_and_renders_impact() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        vec!["one".to_string(), "two".to_string()],
        Vec::new(),
    );
    let core_race = lobby.core_race.as_mut().unwrap();
    core_race.players[0].state.word_index = 0;
    core_race.players[1].state.word_index = 1;
    core_race.players[1].state.input = "twx".to_string();
    core_race.players[1].state.typo_index = Some(2);
    browser_sync_active_race_from_core(&mut lobby);

    activate_browser_item_pickup(
        &mut lobby,
        PlayerId(1),
        ItemPickup::Held(HeldItem::Banana),
        std::time::Instant::now(),
    );
    browser_sync_active_race_from_core(&mut lobby);

    let target = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.id == PlayerId(2))
        .unwrap();
    assert_eq!(target.input, "");
    assert_eq!(target.typo_index, None);
    assert!(target.impact_cue.is_some());
    assert!(!target.stunned);
}

#[test]
fn browser_host_ai_tick_advances_bot_racers() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::AddAi,
        signal(ConnectionState::Disconnected).1,
    );
    let racers = lobby.players.clone();
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        browser_demo_track_words(),
        Vec::new(),
    );

    assert!(apply_browser_host_ai_tick(&mut lobby, 1_000));

    let ai = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.kind == PlayerKind::Bot)
        .unwrap();
    assert!(!ai.input.is_empty() || ai.word_index > 0);
}

#[test]
fn browser_host_ai_tick_can_claim_bonus_pickup() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::AddAi,
        signal(ConnectionState::Disconnected).1,
    );
    let ai_id = lobby
        .players
        .iter()
        .find(|player| player.kind == PlayerKind::Bot)
        .unwrap()
        .id;
    let racers = lobby.players.clone();
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        vec!["one".to_string(), "two".to_string(), "three".to_string()],
        Vec::new(),
    );
    lobby
        .core_race
        .as_mut()
        .unwrap()
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(ai_id.0))
        .unwrap()
        .state
        .word_index = 1;
    lobby.bonuses = BonusState::with_points(
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
    browser_sync_active_race_from_core(&mut lobby);

    assert!(apply_browser_host_ai_tick(&mut lobby, 1_000));

    assert_eq!(lobby.runtime.spent_bonus_gaps.get(&ai_id), Some(&0));
    assert!(lobby.events.iter().any(|event| event.starts_with("ai-1 got ")));
}

#[test]
fn browser_host_ai_tick_finishes_bot_with_enough_budget() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::AddAi,
        signal(ConnectionState::Disconnected).1,
    );
    let ai_id = lobby
        .players
        .iter()
        .find(|player| player.kind == PlayerKind::Bot)
        .unwrap()
        .id;
    lobby
        .players
        .iter_mut()
        .find(|player| player.id == ai_id)
        .unwrap()
        .ai_wpm = Some(1000);
    let racers = lobby.players.clone();
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        browser_demo_track_words(),
        Vec::new(),
    );

    assert!(apply_browser_host_ai_tick(&mut lobby, 60_000));

    let ai = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.id == ai_id)
        .unwrap();
    assert!(ai.finished);
}

#[test]
fn browser_host_ai_tick_ignores_queued_countdown_ticks() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::AddAi,
        signal(ConnectionState::Disconnected).1,
    );
    let racers = lobby.players.clone();
    seed_browser_host_active_race(
        &mut lobby,
        NetworkRacePhase::Racing,
        &racers,
        browser_demo_track_words(),
        Vec::new(),
    );
    lobby.ai_last_tick_ms = Some(browser_now_ms());

    assert!(!apply_browser_host_ai_tick(
        &mut lobby,
        BROWSER_HOST_AI_TICK_MS
    ));

    let ai = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.kind == PlayerKind::Bot)
        .unwrap();
    assert_eq!(ai.word_index, 0);
    assert_eq!(ai.input, "");
}
