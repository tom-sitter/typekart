use super::{
    LobbyAiDifficultyOutcome, LobbyPolicyError, add_ai_lobby_player, first_available_player_id,
    new_human_lobby_player, ready_connected_participants, remove_lobby_player, rename_lobby_player,
    set_lobby_ai_difficulty, set_lobby_ready, unique_lobby_name,
};
use typekart_protocol::{AiDifficultySnapshot, AssignedColor, NetworkRacePhase, PlayerId};

#[test]
fn unique_lobby_name_appends_suffix_case_insensitively() {
    let players = [
        new_human_lobby_player(PlayerId(1), "tom", AssignedColor::Cyan),
        new_human_lobby_player(PlayerId(2), "Tom2", AssignedColor::Red),
    ];

    assert_eq!(unique_lobby_name(players.iter(), "tom"), "tom3");
    assert_eq!(unique_lobby_name(players.iter(), "alex"), "alex");
}

#[test]
fn first_available_player_id_skips_existing_ids() {
    let players = vec![
        new_human_lobby_player(PlayerId(1), "host", AssignedColor::Cyan),
        new_human_lobby_player(PlayerId(3), "joiner", AssignedColor::Red),
    ];

    assert_eq!(first_available_player_id(&players, 2), PlayerId(2));
    assert_eq!(first_available_player_id(&players, 3), PlayerId(4));
}

#[test]
fn ready_connected_participants_excludes_unready_players() {
    let mut players = vec![
        new_human_lobby_player(PlayerId(1), "host", AssignedColor::Cyan),
        new_human_lobby_player(PlayerId(2), "joiner", AssignedColor::Red),
    ];
    players[1].ready = false;

    let participants = ready_connected_participants(&players);

    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].id.0, 1);
}

#[test]
fn rename_lobby_player_dedupes_and_rejects_racing_phase() {
    let mut players = vec![
        new_human_lobby_player(PlayerId(1), "host", AssignedColor::Cyan),
        new_human_lobby_player(PlayerId(2), "tom", AssignedColor::Red),
    ];

    let outcome =
        rename_lobby_player(&mut players, NetworkRacePhase::Lobby, PlayerId(2), "host").unwrap();

    assert_eq!(outcome.previous_name, "tom");
    assert_eq!(outcome.new_name, "host2");
    assert_eq!(players[1].name, "host2");
    assert_eq!(
        rename_lobby_player(&mut players, NetworkRacePhase::Racing, PlayerId(2), "alex"),
        Err(LobbyPolicyError::RenameUnavailable)
    );
}

#[test]
fn add_and_remove_ai_lobby_player_enforces_roster_policy() {
    let mut players = vec![new_human_lobby_player(
        PlayerId(1),
        "host",
        AssignedColor::Cyan,
    )];

    let added = add_ai_lobby_player(
        &mut players,
        NetworkRacePhase::Lobby,
        2,
        AiDifficultySnapshot::Easy,
        45,
    )
    .unwrap();

    assert_eq!(added.player.name, "ai-1");
    assert_eq!(players.len(), 2);
    assert_eq!(
        add_ai_lobby_player(
            &mut players,
            NetworkRacePhase::Lobby,
            2,
            AiDifficultySnapshot::Easy,
            45,
        ),
        Err(LobbyPolicyError::LobbyFull)
    );

    let removed =
        remove_lobby_player(&mut players, NetworkRacePhase::Lobby, added.player.id).unwrap();

    assert_eq!(removed.player.name, "ai-1");
    assert_eq!(
        remove_lobby_player(&mut players, NetworkRacePhase::Lobby, PlayerId(1)),
        Err(LobbyPolicyError::HostCannotBeRemoved)
    );
}

#[test]
fn ready_and_ai_difficulty_updates_are_shared_policy() {
    let mut players = vec![
        new_human_lobby_player(PlayerId(1), "host", AssignedColor::Cyan),
        new_human_lobby_player(PlayerId(2), "tom", AssignedColor::Red),
    ];
    add_ai_lobby_player(
        &mut players,
        NetworkRacePhase::Lobby,
        3,
        AiDifficultySnapshot::Easy,
        45,
    )
    .unwrap();

    let ready = set_lobby_ready(&mut players, PlayerId(2), true).unwrap();
    assert_eq!(ready.name, "tom");
    assert!(players[1].ready);

    let difficulty = set_lobby_ai_difficulty(
        &mut players,
        NetworkRacePhase::Lobby,
        Some(PlayerId(3)),
        AiDifficultySnapshot::Hard,
        85,
    )
    .unwrap();

    assert_eq!(
        difficulty,
        LobbyAiDifficultyOutcome::PlayerChanged {
            player_id: PlayerId(3),
            name: "ai-1".to_string(),
            difficulty: AiDifficultySnapshot::Hard,
            words_per_minute: 85,
        }
    );
    assert_eq!(players[2].ai_difficulty, Some(AiDifficultySnapshot::Hard));
}
