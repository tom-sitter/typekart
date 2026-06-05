use super::{
    AssignedColor, NetworkMarkerPosition, NetworkTrackWindow, NetworkViewState, PlayerId,
    PlayerSnapshot, display_word_number, enter_sets_ready, host_cancel_key, join_rejection_message,
    lifecycle_command_message, network_bonus_column, network_bonus_lines, network_help_lines,
    network_minimap_column, network_racer_label, network_racer_line, network_track_word_line,
    phase_accepts_typed_commands, primary_command_help, rename_prefill, space_starts_countdown,
    starts_rename_mode, stream_index_for_word_char, visible_network_bonus_point,
};
use crate::net::protocol::{
    BonusChoiceSnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot, ClientMessage,
    ImpactCueSnapshot, ImpactCueSnapshotKind, ModConfigSnapshot, NetworkRacePhase, PlayerKind,
    RaceSnapshot,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};

#[test]
fn network_track_window_keeps_current_word_visible() {
    let words = words(["zero", "one", "two", "three", "four"]);
    let window = NetworkTrackWindow::new(&words, 3, 13);

    assert!(window.words.iter().any(|word| word.index == 3));
    assert!(window.words.iter().all(|word| word.end_col <= window.width));
}

#[test]
fn network_track_window_keeps_three_completed_words_behind_player() {
    let words = words([
        "zero", "one", "two", "three", "four", "five", "six", "seven",
    ]);
    let window = NetworkTrackWindow::new(&words, 5, 80);

    assert_eq!(window.words.first().map(|word| word.index), Some(2));
}

#[test]
fn network_marker_tracks_current_character_position() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 1, 20);
    let player = player(PlayerId(1), 1, "tw", None, false);

    assert_eq!(window.column_for_player(&player), 6);
}

#[test]
fn network_marker_pins_to_first_typo() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 1, 20);
    let player = player(PlayerId(1), 1, "txxx", Some(1), false);

    assert_eq!(window.column_for_player(&player), 5);
}

#[test]
fn network_marker_uses_finished_edge_marker_when_finish_is_offscreen() {
    let words = words(["one", "two", "three", "four"]);
    let window = NetworkTrackWindow::new(&words, 1, 7);
    let player = player(PlayerId(2), 3, "", None, true);

    assert_eq!(
        window.marker_for_player(&player),
        NetworkMarkerPosition::FinishedAhead
    );
}

#[test]
fn network_minimap_pins_finished_player_to_finish_edge() {
    let player = player(PlayerId(1), 2, "", None, true);

    assert_eq!(network_minimap_column(4, 10, &player), 11);
}

#[test]
fn network_stream_index_counts_spaces_between_words() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 20);
    let target = window.words.iter().find(|word| word.index == 1).unwrap();

    assert_eq!(stream_index_for_word_char(&window, 0, target, 0), Some(4));
}

#[test]
fn network_word_line_renders_space_position_typos_red() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 20);
    let player = player(PlayerId(1), 0, "onex", Some(3), false);

    let line = network_track_word_line(&window, Some(&player));

    assert_eq!(line.spans[3].content.as_ref(), "x");
    assert_eq!(line.spans[3].style.fg, Some(Color::Red));
    assert!(line.spans[3].style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn network_word_line_tints_focused_track_words() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 20);
    let mut player = player(PlayerId(1), 0, "", None, false);
    player.focused = true;

    let line = network_track_word_line(&window, Some(&player));

    assert_eq!(line.spans[0].style.fg, Some(Color::LightMagenta));
    assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(line.spans[4].style.fg, Some(Color::LightMagenta));
}

#[test]
fn network_bonus_point_is_visible_when_gap_words_are_visible() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 20);
    let snapshot = snapshot_with_bonus(0);

    let point = visible_network_bonus_point(&window, &snapshot).unwrap();

    assert_eq!(point.after_word_index, 0);
}

#[test]
fn network_bonus_column_aligns_between_gap_words() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 20);

    assert_eq!(network_bonus_column(&window, 0, 4), 1);
}

#[test]
fn network_bonus_words_are_magenta_only_when_local_player_can_claim_them() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 20);
    let snapshot = snapshot_with_bonus(0);

    let before_gap = player(PlayerId(1), 0, "", None, false);
    let claimable = player(PlayerId(1), 1, "", None, false);
    let bonus_attempt = player(PlayerId(1), 1, "d", None, false);
    let next_word_started = player(PlayerId(1), 1, "t", None, false);
    let after_gap = player(PlayerId(1), 2, "", None, false);

    assert_eq!(
        network_bonus_lines(&window, &snapshot, Some(&claimable))[0].spans[1]
            .style
            .fg,
        Some(Color::Magenta)
    );
    assert_eq!(
        network_bonus_lines(&window, &snapshot, Some(&bonus_attempt))[0].spans[1]
            .style
            .fg,
        Some(Color::Magenta)
    );
    assert_eq!(
        network_bonus_lines(&window, &snapshot, Some(&before_gap))[0].spans[1]
            .style
            .fg,
        Some(Color::DarkGray)
    );
    assert_eq!(
        network_bonus_lines(&window, &snapshot, Some(&next_word_started))[0].spans[1]
            .style
            .fg,
        Some(Color::DarkGray)
    );
    assert_eq!(
        network_bonus_lines(&window, &snapshot, Some(&after_gap))[0].spans[1]
            .style
            .fg,
        Some(Color::DarkGray)
    );
}

#[test]
fn display_word_number_clamps_finished_player_to_track_length() {
    let player = player(PlayerId(1), 3, "", None, true);

    assert_eq!(display_word_number(&player, 3), 3);
}

#[test]
fn network_local_racer_label_shows_countdown() {
    assert_eq!(
        network_racer_label(
            true,
            NetworkRacePhase::Countdown {
                remaining_seconds: 3
            }
        ),
        Some(" 3".to_string())
    );
    assert_eq!(
        network_racer_label(
            false,
            NetworkRacePhase::Countdown {
                remaining_seconds: 3
            }
        ),
        None
    );
}

#[test]
fn waiting_snapshot_returns_network_view_to_lobby() {
    let mut state = NetworkViewState::new(PlayerId(1), crate::ui::render::IconMode::Ascii, None);
    let mut snapshot = snapshot_with_bonus(0);
    state.apply_race_snapshot(snapshot.clone());
    assert!(state.race_snapshot.is_some());

    snapshot.phase = NetworkRacePhase::WaitingForHost;
    state.apply_race_snapshot(snapshot);

    assert!(state.race_snapshot.is_none());
    assert!(state.placements.is_empty());
    assert!(state.result_rows.is_empty());
}

#[test]
fn lobby_observer_can_hold_race_snapshot_without_being_racer() {
    let mut state = NetworkViewState::new(PlayerId(9), crate::ui::render::IconMode::Ascii, None);

    state.apply_race_snapshot(snapshot_with_bonus(0));

    assert!(state.race_snapshot.is_some());
    assert!(!state.is_local_player_in_current_race());
}

#[test]
fn lifecycle_commands_are_phase_aware() {
    assert_eq!(
        lifecycle_command_message("ready", true, NetworkRacePhase::WaitingForHost),
        Some(ClientMessage::SetReady { ready: true })
    );
    assert_eq!(
        lifecycle_command_message("ready", true, NetworkRacePhase::Racing),
        None
    );
    assert_eq!(
        lifecycle_command_message("lobby", true, NetworkRacePhase::Finished),
        Some(ClientMessage::RestartRace)
    );
    assert_eq!(
        lifecycle_command_message("rename speedy", false, NetworkRacePhase::WaitingForHost),
        None
    );
    assert_eq!(
        lifecycle_command_message("lobby", false, NetworkRacePhase::Finished),
        None
    );
}

#[test]
fn n_starts_rename_mode_only_from_lobby_without_typed_command() {
    let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);

    assert!(starts_rename_mode(
        NetworkRacePhase::WaitingForHost,
        "",
        key
    ));
    assert!(!starts_rename_mode(
        NetworkRacePhase::WaitingForHost,
        "ready",
        key
    ));
    assert!(!starts_rename_mode(NetworkRacePhase::Racing, "", key));
}

#[test]
fn rename_prefill_starts_blank_for_default_anonymous_names() {
    assert_eq!(rename_prefill(Some("anonymous")), "");
    assert_eq!(rename_prefill(Some("anonymous2")), "");
    assert_eq!(rename_prefill(Some("anonymous42")), "");
    assert_eq!(rename_prefill(Some("anonymous-racer")), "anonymous-racer");
    assert_eq!(rename_prefill(Some("tom")), "tom");
}

#[test]
fn ctrl_r_is_host_only_cancel_during_active_network_races() {
    let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);

    assert!(host_cancel_key(
        true,
        NetworkRacePhase::Countdown {
            remaining_seconds: 2
        },
        key
    ));
    assert!(host_cancel_key(true, NetworkRacePhase::Racing, key));
    assert!(host_cancel_key(true, NetworkRacePhase::Finished, key));
    assert!(!host_cancel_key(false, NetworkRacePhase::Racing, key));
    assert!(!host_cancel_key(
        true,
        NetworkRacePhase::WaitingForHost,
        key
    ));
}

#[test]
fn lifecycle_help_hides_irrelevant_commands() {
    assert!(primary_command_help(true, NetworkRacePhase::WaitingForHost).contains("Space"));
    assert!(primary_command_help(false, NetworkRacePhase::WaitingForHost).contains("Enter"));
    assert!(!primary_command_help(true, NetworkRacePhase::Finished).contains("ready"));
    let finished_host_help = format!("{:?}", network_help_lines(true, NetworkRacePhase::Finished));
    assert!(finished_host_help.contains("lobby"));
    assert_eq!(
        primary_command_help(false, NetworkRacePhase::Finished),
        "    ? help | quit"
    );
    assert!(!phase_accepts_typed_commands(
        true,
        NetworkRacePhase::Countdown {
            remaining_seconds: 2
        }
    ));
    assert!(space_starts_countdown(true, NetworkRacePhase::Finished));
    assert!(phase_accepts_typed_commands(
        false,
        NetworkRacePhase::Finished
    ));
    assert!(enter_sets_ready(true, NetworkRacePhase::WaitingForHost, ""));
    assert!(!enter_sets_ready(
        true,
        NetworkRacePhase::WaitingForHost,
        "quit"
    ));
    assert!(!enter_sets_ready(
        false,
        NetworkRacePhase::WaitingForHost,
        ""
    ));
}

#[test]
fn join_rejection_messages_add_user_guidance() {
    assert_eq!(
        join_rejection_message("Name 'tom' is already in use"),
        "Name 'tom' is already in use. Choose a different --name."
    );
    assert!(
        join_rejection_message("Lobby is full: 6/6 connected players")
            .contains("ask the host to remove a racer")
    );
    assert!(
        join_rejection_message("Room vivid-grape-lemon was not found")
            .contains("make sure the host is still online")
    );
    assert!(
            join_rejection_message(
                "Version mismatch: this room is running TypeKart 0.1.0, but you are running TypeKart 0.2.0. Install or launch the same TypeKart version as the room host."
            )
            .contains("room is running TypeKart 0.1.0")
        );
}

#[test]
fn network_racer_marker_shows_banana_impact_icon() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 40);
    let mut racer = player(PlayerId(1), 0, "", None, false);
    racer.impact_cue = Some(ImpactCueSnapshot {
        kind: ImpactCueSnapshotKind::Banana,
        remaining_ms: 500,
    });

    let line = network_racer_line(
        &window,
        &racer,
        true,
        NetworkRacePhase::Racing,
        super::IconMode::Unicode,
    );

    assert_eq!(line.spans[0].content.as_ref(), "█");
    assert_eq!(line.spans[1].content.as_ref(), "🍌");
}

#[test]
fn network_racer_marker_shows_cyclone_impact_icon() {
    let words = words(["one", "two", "three"]);
    let window = NetworkTrackWindow::new(&words, 0, 40);
    let mut racer = player(PlayerId(1), 0, "", None, false);
    racer.impact_cue = Some(ImpactCueSnapshot {
        kind: ImpactCueSnapshotKind::Cyclone,
        remaining_ms: 500,
    });

    let line = network_racer_line(
        &window,
        &racer,
        true,
        NetworkRacePhase::Racing,
        super::IconMode::Unicode,
    );

    assert_eq!(line.spans[0].content.as_ref(), "█");
    assert_eq!(line.spans[1].content.as_ref(), "🌀");
}

fn words<const N: usize>(words: [&str; N]) -> Vec<String> {
    words.into_iter().map(str::to_string).collect()
}

fn player(
    id: PlayerId,
    word_index: usize,
    input: &str,
    typo_index: Option<usize>,
    finished: bool,
) -> PlayerSnapshot {
    PlayerSnapshot {
        id,
        name: format!("player-{}", id.0),
        kind: PlayerKind::Human,
        color: AssignedColor::Cyan,
        word_index,
        input: input.to_string(),
        typo_index,
        word_overrides: Vec::new(),
        finished,
        connected: true,
        shielded: false,
        focused: false,
        fogged: false,
        boosted: false,
        stunned: false,
        impact_remaining_ms: 0,
        impact_cue: None,
        item_cue: None,
    }
}

fn snapshot_with_bonus(after_word_index: usize) -> RaceSnapshot {
    RaceSnapshot {
        sequence: 1,
        phase: NetworkRacePhase::Racing,
        mod_config: test_mod_config(),
        track_words: words(["one", "two", "three"]),
        bonuses: vec![BonusPointSnapshot {
            after_word_index,
            choices: vec![BonusChoiceSnapshot {
                word: "dash".to_string(),
                status: BonusChoiceSnapshotStatus::Available,
            }],
        }],
        players: Vec::new(),
        events: Vec::new(),
    }
}

fn test_mod_config() -> ModConfigSnapshot {
    ModConfigSnapshot {
        word_set_id: "classic".to_string(),
        word_set_name: "Classic".to_string(),
        word_set_hash: "0000000000000001".to_string(),
        item_pack_name: "classic".to_string(),
        item_registry_hash: "0000000000000002".to_string(),
        combined_hash: "0000000000000003".to_string(),
    }
}
