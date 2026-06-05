use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier};

use crate::{
    game::{
        ai::AiDifficulty,
        bonus::{BonusChoice, BonusPoint, BonusState},
        effects::ActiveEffect,
        player::PlayerState,
        track::Track,
    },
    ui::render::{
        IconMode, WordRenderState, ai_color, bonus_column, build_track_window,
        is_bonus_point_claimable, minimap_line, player_list_height, racer_lines, result_rows,
        track_panel_height, track_word_line, visible_bonus_point, visible_word_char,
    },
    ui::session::{
        AiRacer, AttackDirection, ImpactCue, ImpactCueKind, ItemCue, ItemCueKind, RaceStatus,
    },
};
use typekart_protocol::NetworkRacePhase;

fn track(words: &[&str]) -> Track {
    Track::new(words.iter().map(|word| word.to_string()).collect())
}

#[test]
fn track_window_includes_current_word() {
    let track = track(&["one", "two", "three", "four"]);
    let window = build_track_window(&track, 2, 40);

    assert!(window.words.iter().any(|word| word.index == 2));
    assert_eq!(window.current_word().unwrap().word, "three");
}

#[test]
fn track_window_includes_upcoming_words_when_width_allows() {
    let track = track(&["one", "two", "three", "four"]);
    let window = build_track_window(&track, 1, 40);

    assert!(window.words.iter().any(|word| word.index == 2));
    assert!(window.words.iter().any(|word| word.index == 3));
}

#[test]
fn track_window_keeps_completed_words_behind_player_when_possible() {
    let track = track(&["one", "two", "three", "four", "five"]);
    let window = build_track_window(&track, 3, 40);

    assert_eq!(window.words.first().unwrap().index, 0);
    assert_eq!(window.words[0].state, WordRenderState::Completed);
}

#[test]
fn track_window_does_not_exceed_requested_width() {
    let track = track(&["one", "two", "three", "four"]);
    let window = build_track_window(&track, 0, 8);
    let end = window.words.last().map(|word| word.end_col).unwrap_or(0);

    assert!(end <= 8);
}

#[test]
fn visible_word_metadata_has_correct_start_columns() {
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);

    assert_eq!(window.words[0].start_col, 0);
    assert_eq!(window.words[1].start_col, 4);
    assert_eq!(window.words[2].start_col, 8);
}

#[test]
fn local_racer_marker_centers_under_current_word() {
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 1, 40);

    assert_eq!(
        window.racer_marker_start_for_player(&PlayerState::new(Instant::now()), 3),
        0
    );
}

#[test]
fn local_racer_marker_tracks_current_character() {
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 1, 40);
    let mut player = PlayerState::new(Instant::now());
    player.word_index = 1;
    player.input = "t".to_string();

    assert_eq!(window.racer_marker_start_for_player(&player, 3), 4);
}

#[test]
fn racer_lines_put_local_player_first_with_one_line_per_ai() {
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let mut player = PlayerState::new(Instant::now());
    player.input = "o".to_string();
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, Instant::now());
    ai.player.word_index = 1;
    let lines = racer_lines(
        &window,
        &player,
        &[ai],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        Instant::now(),
    );

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].spans[0].content.as_ref(), "█");
    assert_eq!(lines[0].spans[0].style.fg, Some(Color::Cyan));
    assert_eq!(lines[1].spans[3].content.as_ref(), "█");
    assert_eq!(lines[1].spans[3].style.fg, Some(Color::LightRed));
}

#[test]
fn local_racer_line_shows_countdown_next_to_marker() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 1, 40);
    let mut player = PlayerState::new(now);
    player.word_index = 1;
    let phase = NetworkRacePhase::Countdown {
        remaining_seconds: 3,
    };

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        None,
        phase,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines[0].spans[3].content.as_ref(), "█");
    assert_eq!(lines[0].spans[6].content.as_ref(), " ");
    assert_eq!(lines[0].spans[7].content.as_ref(), "3");
}

#[test]
fn layout_heights_fit_six_ai_racers() {
    assert_eq!(track_panel_height(6), 14);
    assert_eq!(player_list_height(6), 9);
}

#[test]
fn first_six_ai_racers_have_distinct_colors() {
    let colors = (1..=6).map(ai_color).collect::<Vec<_>>();

    for (index, color) in colors.iter().enumerate() {
        assert!(!colors[..index].contains(color));
    }
}

#[test]
fn minimap_line_shows_local_and_ai_markers() {
    let now = Instant::now();
    let mut player = PlayerState::new(now);
    player.word_index = 1;
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
    ai.player.word_index = 3;

    let line = minimap_line(5, 20, &player, &[ai]);

    assert_eq!(line.spans[5].content.as_ref(), "|");
    assert_eq!(line.spans[9].content.as_ref(), "@");
    assert_eq!(line.spans[9].style.fg, Some(Color::Cyan));
    assert_eq!(line.spans[16].content.as_ref(), "1");
    assert_eq!(line.spans[16].style.fg, Some(Color::LightRed));
    assert_eq!(line.spans[19].content.as_ref(), "|");
}

#[test]
fn minimap_line_pins_finished_racer_to_finish_edge() {
    let now = Instant::now();
    let player = PlayerState::new(now);
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
    ai.player.finished_at = Some(now);

    let line = minimap_line(5, 20, &player, &[ai]);

    assert_eq!(line.spans[19].content.as_ref(), "1");
}

#[test]
fn minimap_line_prefers_local_marker_on_overlap() {
    let now = Instant::now();
    let player = PlayerState::new(now);
    let ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);

    let line = minimap_line(5, 20, &player, &[ai]);

    assert_eq!(line.spans[6].content.as_ref(), "@");
}

#[test]
fn minimap_line_shows_focus_for_ai_overlap() {
    let now = Instant::now();
    let player = PlayerState::new(now);
    let mut ai_1 = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
    ai_1.player.word_index = 2;
    let mut ai_2 = AiRacer::new(2, AiDifficulty::Easy, 35.0, now);
    ai_2.player.word_index = 2;

    let line = minimap_line(5, 20, &player, &[ai_1, ai_2]);

    assert_eq!(line.spans[13].content.as_ref(), "*");
    assert_eq!(line.spans[13].style.fg, Some(Color::White));
}

#[test]
fn racer_lines_include_all_six_ai_racers() {
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let player = PlayerState::new(Instant::now());
    let now = Instant::now();
    let ai_racers = (1..=6)
        .map(|id| AiRacer::new(id, AiDifficulty::Easy, 35.0, now))
        .collect::<Vec<_>>();

    let lines = racer_lines(
        &window,
        &player,
        &ai_racers,
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines.len(), 7);
}

#[test]
fn ai_racer_marker_shows_left_indicator_when_behind_visible_window() {
    let track = track(&["one", "two", "three", "four", "five", "six"]);
    let window = build_track_window(&track, 5, 40);
    let player = PlayerState::new(Instant::now());
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, Instant::now());
    ai.player.word_index = 1;

    let lines = racer_lines(
        &window,
        &player,
        &[ai],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        Instant::now(),
    );

    assert_eq!(lines[1].spans[0].content.as_ref(), "<");
    assert_eq!(lines[1].spans[0].style.fg, Some(Color::LightRed));
}

#[test]
fn ai_racer_marker_shows_right_indicator_when_ahead_of_visible_window() {
    let track = track(&["one", "two", "three", "four", "five", "six"]);
    let window = build_track_window(&track, 0, 7);
    let player = PlayerState::new(Instant::now());
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, Instant::now());
    ai.player.word_index = 3;

    let lines = racer_lines(
        &window,
        &player,
        &[ai],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        Instant::now(),
    );

    assert_eq!(lines[1].spans[6].content.as_ref(), ">");
    assert_eq!(lines[1].spans[6].style.fg, Some(Color::LightRed));
}

#[test]
fn finished_ai_racer_keeps_kart_marker_at_finish_line() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 2, 40);
    let player = PlayerState::new(now);
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
    ai.player.word_index = 3;
    ai.player.finished_at = Some(now);

    let lines = racer_lines(
        &window,
        &player,
        &[ai],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert!(
        lines[1].spans.iter().any(|span| {
            span.content.as_ref() == "█" && span.style.fg == Some(Color::LightRed)
        })
    );
    assert!(
        !lines[1]
            .spans
            .iter()
            .any(|span| { span.content.as_ref() == ">" && span.style.fg == Some(Color::LightRed) })
    );
}

#[test]
fn finished_ai_racer_shows_finished_edge_marker_when_finish_line_is_offscreen() {
    let now = Instant::now();
    let track = track(&["one", "two", "three", "four", "five", "six"]);
    let window = build_track_window(&track, 0, 12);
    let player = PlayerState::new(now);
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
    ai.player.word_index = 6;
    ai.player.finished_at = Some(now);

    let lines = racer_lines(
        &window,
        &player,
        &[ai],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines[1].spans[10].content.as_ref(), ">");
    assert_eq!(lines[1].spans[11].content.as_ref(), "!");
    assert_eq!(lines[1].spans[10].style.fg, Some(Color::LightRed));
    assert!(
        !lines[1].spans.iter().any(|span| {
            span.content.as_ref() == "█" && span.style.fg == Some(Color::LightRed)
        })
    );
}

#[test]
fn offscreen_ai_indicator_can_show_status_effect_marker() {
    let now = Instant::now();
    let track = track(&["one", "two", "three", "four", "five", "six"]);
    let window = build_track_window(&track, 5, 40);
    let player = PlayerState::new(now);
    let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, now);
    ai.player.word_index = 1;
    ai.player.active_effects.push(ActiveEffect::Shield {
        until: now + std::time::Duration::from_secs(1),
    });

    let lines = racer_lines(
        &window,
        &player,
        &[ai],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines[1].spans[0].content.as_ref(), "[");
    assert_eq!(lines[1].spans[1].content.as_ref(), "<");
    assert_eq!(lines[1].spans[2].content.as_ref(), "]");
}

#[test]
fn racer_line_shows_mushroom_boost_prefix() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let mut player = PlayerState::new(now);
    player.active_effects.push(ActiveEffect::Mushroom {
        remaining_words: 2,
        next_step_at: now,
        step_interval: std::time::Duration::from_millis(400),
    });

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines[0].spans[0].content.as_ref(), ">");
    assert_eq!(lines[0].spans[1].content.as_ref(), ">");
    assert_eq!(lines[0].spans[2].content.as_ref(), ">");
    assert_eq!(lines[0].spans[3].content.as_ref(), "█");
    assert_eq!(lines[0].spans[4].content.as_ref(), "█");
    assert_eq!(lines[0].spans[5].content.as_ref(), "█");
}

#[test]
fn racer_line_shows_unicode_mushroom_boost_prefix() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let mut player = PlayerState::new(now);
    player.active_effects.push(ActiveEffect::Mushroom {
        remaining_words: 2,
        next_step_at: now,
        step_interval: std::time::Duration::from_millis(400),
    });

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Unicode,
        now,
    );

    assert_eq!(lines[0].spans[0].content.as_ref(), ">");
    assert_eq!(lines[0].spans[1].content.as_ref(), ">");
    assert_eq!(lines[0].spans[2].content.as_ref(), "🍄");
    assert_eq!(lines[0].spans[3].content.as_ref(), "█");
}

#[test]
fn racer_line_uses_unicode_shield_marker_when_enabled() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let mut player = PlayerState::new(now);
    player.active_effects.push(ActiveEffect::Shield {
        until: now + std::time::Duration::from_secs(1),
    });

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Unicode,
        now,
    );

    assert_eq!(lines[0].spans[0].content.as_ref(), "█");
    assert_eq!(lines[0].spans[1].content.as_ref(), "🛡");
    assert_eq!(lines[0].spans[2].content.as_ref(), " ");
}

#[test]
fn racer_line_uses_unicode_focus_marker_when_enabled() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let mut player = PlayerState::new(now);
    player.active_effects.push(ActiveEffect::Focus {
        until: now + std::time::Duration::from_secs(1),
    });

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        None,
        NetworkRacePhase::Racing,
        IconMode::Unicode,
        now,
    );

    assert_eq!(lines[0].spans[0].content.as_ref(), "█");
    assert_eq!(lines[0].spans[1].content.as_ref(), "★");
    assert_eq!(lines[0].spans[2].content.as_ref(), "█");
}

#[test]
fn racer_line_shows_unicode_banana_attack_direction_cue() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let player = PlayerState::new(now);
    let cue = ItemCue::new(
        ItemCueKind::Banana {
            direction: AttackDirection::Ahead,
        },
        now,
    );

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        Some(cue),
        NetworkRacePhase::Racing,
        IconMode::Unicode,
        now,
    );

    assert_eq!(lines[0].spans[3].content.as_ref(), " ");
    assert_eq!(lines[0].spans[4].content.as_ref(), "🍌");
    assert_eq!(lines[0].spans[6].content.as_ref(), ">");
    assert_eq!(lines[0].spans[7].content.as_ref(), ">");
}

#[test]
fn racer_line_shows_unicode_cyclone_attack_direction_cue() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let player = PlayerState::new(now);
    let cue = ItemCue::new(
        ItemCueKind::Cyclone {
            direction: AttackDirection::Ahead,
        },
        now,
    );

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        Some(cue),
        NetworkRacePhase::Racing,
        IconMode::Unicode,
        now,
    );

    assert_eq!(lines[0].spans[3].content.as_ref(), " ");
    assert_eq!(lines[0].spans[4].content.as_ref(), "🌀");
    assert_eq!(lines[0].spans[6].content.as_ref(), ">");
    assert_eq!(lines[0].spans[7].content.as_ref(), ">");
}

#[test]
fn racer_line_shows_ascii_banana_attack_direction_cue() {
    let now = Instant::now();
    let track = track(&["one", "two", "three", "four"]);
    let window = build_track_window(&track, 2, 40);
    let mut player = PlayerState::new(now);
    player.word_index = 2;
    let cue = ItemCue::new(
        ItemCueKind::Banana {
            direction: AttackDirection::Behind,
        },
        now,
    );

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        Some(cue),
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines[0].spans[2].content.as_ref(), "(");
    assert_eq!(lines[0].spans[3].content.as_ref(), "(");
    assert_eq!(lines[0].spans[4].content.as_ref(), "<");
    assert_eq!(lines[0].spans[5].content.as_ref(), "<");
    assert_eq!(lines[0].spans[6].content.as_ref(), " ");
    assert_eq!(lines[0].spans[7].content.as_ref(), "█");
}

#[test]
fn racer_line_shows_overlap_banana_attack_direction_cue() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let player = PlayerState::new(now);
    let cue = ItemCue::new(
        ItemCueKind::Banana {
            direction: AttackDirection::Overlap,
        },
        now,
    );

    let lines = racer_lines(
        &window,
        &player,
        &[],
        None,
        Some(cue),
        NetworkRacePhase::Racing,
        IconMode::Unicode,
        now,
    );

    assert_eq!(lines[0].spans[3].content.as_ref(), " ");
    assert_eq!(lines[0].spans[4].content.as_ref(), "🍌");
    assert_eq!(lines[0].spans[6].content.as_ref(), "<");
    assert_eq!(lines[0].spans[7].content.as_ref(), ">");
}

#[test]
fn racer_line_blinks_when_impacted() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let player = PlayerState::new(now);

    let lines = racer_lines(
        &window,
        &player,
        &[],
        Some(ImpactCue {
            kind: ImpactCueKind::Banana,
            until: now + std::time::Duration::from_millis(300),
        }),
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines[0].spans[0].style.bg, Some(Color::Yellow));
    assert_eq!(lines[0].spans[0].content.as_ref(), "[");
    assert_eq!(lines[0].spans[1].content.as_ref(), "B");
    assert_eq!(lines[0].spans[2].content.as_ref(), "]");
}

#[test]
fn racer_line_blinks_blue_when_hit_by_cyclone() {
    let now = Instant::now();
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);
    let player = PlayerState::new(now);

    let lines = racer_lines(
        &window,
        &player,
        &[],
        Some(ImpactCue {
            kind: ImpactCueKind::Cyclone,
            until: now + std::time::Duration::from_millis(300),
        }),
        None,
        NetworkRacePhase::Racing,
        IconMode::Ascii,
        now,
    );

    assert_eq!(lines[0].spans[0].style.bg, Some(Color::Blue));
    assert_eq!(lines[0].spans[0].content.as_ref(), "[");
    assert_eq!(lines[0].spans[1].content.as_ref(), "C");
    assert_eq!(lines[0].spans[2].content.as_ref(), "]");
}

#[test]
fn local_racer_marker_clamps_at_left_edge() {
    let track = track(&["a", "two"]);
    let window = build_track_window(&track, 0, 40);

    assert_eq!(
        window.racer_marker_start_for_player(&PlayerState::new(Instant::now()), 3),
        0
    );
}

#[test]
fn local_racer_marker_clamps_at_right_edge() {
    let track = track(&["abcde"]);
    let window = build_track_window(&track, 0, 2);

    assert_eq!(
        window.racer_marker_start_for_player(&PlayerState::new(Instant::now()), 5),
        0
    );
}

#[test]
fn player_state_type_still_compiles_for_renderer_tests() {
    let player = PlayerState::new(Instant::now());

    assert_eq!(player.word_index, 0);
}

#[test]
fn result_rows_rank_all_racers_by_finish_time_then_timeout_progress() {
    let started_at = Instant::now();
    let mut player = PlayerState::new(started_at);
    player.finished_at = Some(started_at + std::time::Duration::from_secs(12));
    player.stats.completed_words = 2;

    let mut ai_winner = AiRacer::new(1, AiDifficulty::Easy, 35.0, started_at);
    ai_winner.player.finished_at = Some(started_at + std::time::Duration::from_secs(10));
    ai_winner.player.stats.completed_words = 2;

    let mut ai_timeout = AiRacer::new(2, AiDifficulty::Easy, 35.0, started_at);
    ai_timeout.player.stats.completed_words = 1;

    let rows = result_rows(
        &player,
        &[ai_timeout, ai_winner],
        RaceStatus {
            first_finished_at: Some(started_at + std::time::Duration::from_secs(10)),
            ended_at: Some(started_at + std::time::Duration::from_secs(25)),
        },
    );

    assert!(rows[0].spans[0].content.contains("ai-1"));
    assert!(rows[1].spans[0].content.contains("you"));
    assert!(rows[2].spans[0].content.contains("ai-2"));
    assert!(rows[2].spans[0].content.contains("timeout"));
}

#[test]
fn current_word_spans_show_typed_prefix_and_cursor_on_track() {
    let track = track(&["fox", "road"]);
    let window = build_track_window(&track, 0, 40);
    let now = Instant::now();
    let mut player = PlayerState::new(now);
    player.input = "fo".to_string();

    let line = track_word_line(&window, &player, NetworkRacePhase::Racing, now);
    let spans = line.spans;

    assert_eq!(spans[0].content.as_ref(), "f");
    assert_eq!(spans[0].style.fg, Some(Color::Green));
    assert_eq!(spans[1].content.as_ref(), "o");
    assert_eq!(spans[1].style.fg, Some(Color::Green));
    assert_eq!(spans[2].content.as_ref(), "x");
    assert_eq!(spans[2].style.bg, Some(Color::Yellow));
}

#[test]
fn track_words_are_grey_before_race_begins() {
    let track = track(&["fox", "road"]);
    let window = build_track_window(&track, 0, 40);
    let now = Instant::now();
    let mut player = PlayerState::new(now);
    player.input = "fo".to_string();

    let line = track_word_line(&window, &player, NetworkRacePhase::WaitingForHost, now);
    let spans = line.spans;

    assert_eq!(spans[0].content.as_ref(), "f");
    assert_eq!(spans[0].style.fg, Some(Color::DarkGray));
    assert_eq!(spans[0].style.bg, None);
    assert_eq!(spans[1].content.as_ref(), "o");
    assert_eq!(spans[1].style.fg, Some(Color::DarkGray));
    assert_eq!(spans[2].content.as_ref(), "x");
    assert_eq!(spans[2].style.fg, Some(Color::DarkGray));
}

#[test]
fn focus_effect_tints_current_and_upcoming_track_words() {
    let track = track(&["fox", "road"]);
    let window = build_track_window(&track, 0, 40);
    let now = Instant::now();
    let mut player = PlayerState::new(now);
    player.active_effects.push(ActiveEffect::Focus {
        until: now + Duration::from_secs(5),
    });

    let line = track_word_line(&window, &player, NetworkRacePhase::Racing, now);
    let spans = line.spans;

    assert_eq!(spans[0].style.fg, Some(Color::Black));
    assert_eq!(spans[0].style.bg, Some(Color::Yellow));
    assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(spans[4].style.fg, Some(Color::LightMagenta));
}

#[test]
fn current_word_spans_render_typos_red_on_track() {
    let track = track(&["fox", "road"]);
    let window = build_track_window(&track, 0, 40);
    let now = Instant::now();
    let mut player = PlayerState::new(now);
    player.input = "fa".to_string();
    player.typo_index = Some(1);

    let line = track_word_line(&window, &player, NetworkRacePhase::Racing, now);
    let spans = line.spans;

    assert_eq!(spans[0].content.as_ref(), "f");
    assert_eq!(spans[0].style.fg, Some(Color::Green));
    assert_eq!(spans[1].content.as_ref(), "a");
    assert_eq!(spans[1].style.fg, Some(Color::Red));
}

#[test]
fn fog_reveals_current_word_and_masks_future_words_until_expired() {
    let now = Instant::now();
    let mut player = PlayerState::new(now);
    player.word_index = 1;
    player.fogged_word_index = Some(0);
    player.fogged_until = Some(now + Duration::from_secs(5));

    assert_eq!(visible_word_char(&player, 1, 'r', now), 'r');
    assert_eq!(visible_word_char(&player, 2, 'l', now), '█');
    assert_eq!(
        visible_word_char(&player, 2, 'l', now + Duration::from_secs(5)),
        'l'
    );
}

#[test]
fn typo_overflow_renders_across_following_words() {
    let track = track(&["fox", "road"]);
    let window = build_track_window(&track, 0, 40);
    let now = Instant::now();
    let mut player = PlayerState::new(now);
    player.input = "fa road".to_string();
    player.typo_index = Some(1);

    let line = track_word_line(&window, &player, NetworkRacePhase::Racing, now);
    let spans = line.spans;

    assert_eq!(spans[1].content.as_ref(), "a");
    assert_eq!(spans[1].style.fg, Some(Color::Red));
    assert_eq!(spans[2].content.as_ref(), "␠");
    assert_eq!(spans[2].style.fg, Some(Color::Red));
    assert_eq!(spans[3].content.as_ref(), "r");
    assert_eq!(spans[3].style.fg, Some(Color::Red));
    assert_eq!(spans[6].content.as_ref(), "d");
    assert_eq!(spans[6].style.fg, Some(Color::Red));
}

#[test]
fn racer_marker_pins_to_first_typo() {
    let track = track(&["fox", "road"]);
    let window = build_track_window(&track, 0, 40);
    let mut player = PlayerState::new(Instant::now());
    player.input = "fa road".to_string();
    player.typo_index = Some(1);

    assert_eq!(window.racer_marker_start_for_player(&player, 3), 0);
}

#[test]
fn visible_bonus_point_can_be_ahead_of_player() {
    let track = track(&["one", "two", "three", "four"]);
    let window = build_track_window(&track, 0, 40);
    let bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            1,
            [
                BonusChoice::available("drift"),
                BonusChoice::available("spark"),
                BonusChoice::available("turbo"),
            ],
        )],
        vec![],
    );

    let (point_index, point) = visible_bonus_point(&window, &bonuses).unwrap();

    assert_eq!(point_index, 0);
    assert_eq!(point.after_word_index, 1);
}

#[test]
fn bonus_column_aligns_after_preceding_word() {
    let track = track(&["one", "two", "three"]);
    let window = build_track_window(&track, 0, 40);

    assert_eq!(bonus_column(&window, 1, "spark".len()), 5);
}

#[test]
fn visible_bonus_point_requires_both_gap_words() {
    let track = track(&["one", "two"]);
    let window = build_track_window(&track, 0, 4);
    let bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice::available("drift"),
                BonusChoice::available("spark"),
                BonusChoice::available("turbo"),
            ],
        )],
        vec![],
    );

    assert!(visible_bonus_point(&window, &bonuses).is_none());
}

#[test]
fn visible_bonus_point_is_not_claimable_until_player_reaches_gap() {
    let mut player = PlayerState::new(Instant::now());
    let bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            1,
            [
                BonusChoice::available("drift"),
                BonusChoice::available("spark"),
                BonusChoice::available("turbo"),
            ],
        )],
        vec![],
    );

    player.word_index = 0;
    assert!(!is_bonus_point_claimable(&player, &bonuses, 0));

    player.word_index = 2;
    assert!(is_bonus_point_claimable(&player, &bonuses, 0));
}
