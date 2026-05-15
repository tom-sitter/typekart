//! Ratatui rendering for the local typing prototype.
//!
//! Rendering is intentionally a read-only view over game state. If the display
//! needs a value, it should derive it from `Track` and `PlayerState` rather than
//! mutating either one.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    game::{
        bonus::{BonusChoiceStatus, BonusState},
        effects::AttackWarning,
        player::PlayerState,
        track::Track,
    },
    ui::session::{BonusAttempt, EventLog},
};

const WIDE_LAYOUT_MIN_WIDTH: u16 = 90;
const LOCAL_RACER_MARKER: &str = "███";
const SHIELDED_RACER_MARKER: &str = "[███]";

pub struct TypingScreen<'a> {
    pub track: &'a Track,
    pub player: &'a PlayerState,
    pub bonuses: &'a BonusState,
    pub bonus_attempt: Option<BonusAttempt>,
    pub attack_warning: Option<AttackWarning>,
    pub events: &'a EventLog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordRenderState {
    Completed,
    Current,
    Upcoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleWord<'a> {
    pub index: usize,
    pub word: &'a str,
    pub start_col: usize,
    pub end_col: usize,
    pub state: WordRenderState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackWindow<'a> {
    pub words: Vec<VisibleWord<'a>>,
    pub width: usize,
}

impl<'a> TrackWindow<'a> {
    pub fn current_word(&self) -> Option<&VisibleWord<'a>> {
        self.words
            .iter()
            .find(|word| word.state == WordRenderState::Current)
    }

    pub fn racer_marker_start_for_player(
        &self,
        player: &PlayerState,
        marker_width: usize,
    ) -> usize {
        let marker_center = self.player_track_column(player);
        let marker_start = marker_center.saturating_sub(marker_width / 2);
        marker_start.min(self.width.saturating_sub(marker_width))
    }

    fn player_track_column(&self, player: &PlayerState) -> usize {
        let target_stream_index = player
            .typo_index
            .unwrap_or_else(|| player.input.chars().count());

        self.column_for_stream_index(player.word_index, target_stream_index)
            .or_else(|| self.current_word().map(|word| word.start_col))
            .unwrap_or(0)
    }

    fn column_for_stream_index(
        &self,
        current_word_index: usize,
        target_stream_index: usize,
    ) -> Option<usize> {
        let mut stream_index = 0;
        let mut previous_visible_word_index = None;

        for visible in self
            .words
            .iter()
            .filter(|word| word.index >= current_word_index)
        {
            if previous_visible_word_index.is_some() {
                if target_stream_index == stream_index {
                    return Some(visible.start_col.saturating_sub(1));
                }
                stream_index += 1;
            }

            let word_width = visible.word.chars().count();
            if target_stream_index < stream_index + word_width {
                return Some(visible.start_col + target_stream_index - stream_index);
            }

            stream_index += word_width;
            previous_visible_word_index = Some(visible.index);
        }

        self.words
            .last()
            .map(|word| word.end_col.min(self.width.saturating_sub(1)))
    }
}

pub fn render(frame: &mut Frame<'_>, screen: TypingScreen<'_>) {
    let area = frame.size();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    frame.render_widget(header(screen.track, screen.player), root[0]);
    frame.render_widget(help_view(), root[2]);

    if area.width >= WIDE_LAYOUT_MIN_WIDTH {
        render_wide(frame, root[1], screen);
    } else {
        render_narrow(frame, root[1], screen);
    }
}

fn render_wide(frame: &mut Frame<'_>, area: Rect, screen: TypingScreen<'_>) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(0)])
        .split(columns[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(columns[1]);

    render_track(
        frame,
        left[0],
        screen.track,
        screen.player,
        screen.bonuses,
        screen.bonus_attempt,
    );
    frame.render_widget(finish_or_empty(screen.player), left[1]);
    frame.render_widget(stats_view(screen.player), right[0]);
    frame.render_widget(item_view(screen.player, screen.attack_warning), right[1]);
    frame.render_widget(player_list(screen.track, screen.player), right[2]);
    frame.render_widget(event_feed(screen.events), right[3]);
}

fn render_narrow(frame: &mut Frame<'_>, area: Rect, screen: TypingScreen<'_>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(area);

    render_track(
        frame,
        rows[0],
        screen.track,
        screen.player,
        screen.bonuses,
        screen.bonus_attempt,
    );
    frame.render_widget(stats_view(screen.player), rows[1]);
    frame.render_widget(item_view(screen.player, screen.attack_warning), rows[2]);
    frame.render_widget(player_list(screen.track, screen.player), rows[3]);
    frame.render_widget(results_or_events(screen.player, screen.events), rows[4]);
}

fn header<'a>(track: &Track, player: &PlayerState) -> Paragraph<'a> {
    let title = if player.is_finished() {
        "TypeKart - Finished"
    } else {
        "TypeKart"
    };
    let now = player.finished_at.unwrap_or_else(std::time::Instant::now);
    let progress = format!("{}/{} words", player.stats.completed_words, track.len());
    let wpm = format!(
        "{:.0} WPM",
        player.stats.words_per_minute(player.started_at, now)
    );

    Paragraph::new(Line::from(vec![
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("    "),
        Span::raw(progress),
        Span::raw("    "),
        Span::raw(wpm),
    ]))
    .block(Block::default().borders(Borders::BOTTOM))
}

fn render_track(
    frame: &mut Frame<'_>,
    area: Rect,
    track: &Track,
    player: &PlayerState,
    bonuses: &BonusState,
    bonus_attempt: Option<BonusAttempt>,
) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let window = build_track_window(track, player.word_index, inner_width);
    frame.render_widget(track_view(&window, player, bonuses, bonus_attempt), area);
}

fn track_view<'a>(
    window: &'a TrackWindow<'a>,
    player: &PlayerState,
    bonuses: &'a BonusState,
    bonus_attempt: Option<BonusAttempt>,
) -> Paragraph<'a> {
    let word_line = track_word_line(window, player);

    let now = std::time::Instant::now();
    let marker = if player.has_active_shield(now) {
        SHIELDED_RACER_MARKER
    } else {
        LOCAL_RACER_MARKER
    };
    let marker_start = window.racer_marker_start_for_player(player, marker.chars().count());
    let racer_line = Line::from(vec![
        Span::raw(" ".repeat(marker_start)),
        Span::styled(marker, Style::default().fg(Color::Cyan)),
    ]);

    let mut lines = bonus_lines(window, player, bonuses, bonus_attempt, now);
    lines.push(word_line);
    lines.push(racer_line);

    Paragraph::new(lines).block(Block::default().title("Track").borders(Borders::ALL))
}

fn bonus_lines<'a>(
    window: &TrackWindow<'_>,
    player: &PlayerState,
    bonuses: &'a BonusState,
    bonus_attempt: Option<BonusAttempt>,
    now: std::time::Instant,
) -> Vec<Line<'a>> {
    let Some((point_index, point)) = visible_bonus_point(window, bonuses) else {
        return vec![Line::from(""), Line::from(""), Line::from("")];
    };

    let claimable = is_bonus_point_claimable(player, bonuses, point_index);
    let unavailable = !claimable
        || player.held_item.is_some()
        || player.typo_index.is_some()
        || player.has_active_shield(now);
    point
        .choices
        .iter()
        .enumerate()
        .map(|(choice_index, choice)| {
            let text = match choice.status {
                BonusChoiceStatus::Cooldown { until } if until > now => "---".to_string(),
                _ => choice.word.clone(),
            };
            let mut spans = vec![Span::raw(" ".repeat(bonus_column(
                window,
                point.after_word_index,
                text.chars().count(),
            )))];
            let is_current_attempt = bonus_attempt.is_some_and(|attempt| {
                attempt.point_index == point_index && attempt.choice_index == choice_index
            });
            let style = match choice.status {
                BonusChoiceStatus::Cooldown { until } if until > now => {
                    Style::default().fg(Color::DarkGray)
                }
                _ if unavailable => Style::default().fg(Color::DarkGray),
                _ if is_current_attempt => Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default().fg(Color::Magenta),
            };
            spans.push(Span::styled(text, style));
            Line::from(spans)
        })
        .collect()
}

fn is_bonus_point_claimable(
    player: &PlayerState,
    bonuses: &BonusState,
    point_index: usize,
) -> bool {
    bonuses
        .point_for_gap(player.word_index)
        .is_some_and(|(claimable_point_index, _)| claimable_point_index == point_index)
}

fn visible_bonus_point<'a>(
    window: &TrackWindow<'_>,
    bonuses: &'a BonusState,
) -> Option<(usize, &'a crate::game::bonus::BonusPoint)> {
    let first_visible = window.words.first()?.index;
    let last_visible = window.words.last()?.index;

    bonuses
        .points
        .iter()
        .enumerate()
        .filter(|(_, point)| point.after_word_index >= first_visible)
        .filter(|(_, point)| point.after_word_index.saturating_add(1) <= last_visible)
        .min_by_key(|(_, point)| point.after_word_index)
}

fn bonus_column(window: &TrackWindow<'_>, after_word_index: usize, word_width: usize) -> usize {
    let Some(before_word) = window
        .words
        .iter()
        .find(|word| word.index == after_word_index)
    else {
        return 0;
    };
    let Some(after_word) = window
        .words
        .iter()
        .find(|word| word.index == after_word_index + 1)
    else {
        return 0;
    };

    // Bonus points belong to the gap after `after_word_index`. Centering the
    // rendered word on that gap keeps it visually pinned between the two track
    // words even when the bonus word is wider than the gap itself.
    let gap_center = (before_word.end_col + after_word.start_col) / 2;
    gap_center
        .saturating_sub(word_width / 2)
        .min(window.width.saturating_sub(word_width))
}

pub fn build_track_window(track: &Track, current_index: usize, width: usize) -> TrackWindow<'_> {
    if width == 0 || track.words.is_empty() {
        return TrackWindow {
            words: Vec::new(),
            width,
        };
    }

    let safe_current = current_index.min(track.words.len().saturating_sub(1));
    let start_index = safe_current.saturating_sub(3);
    let mut words = Vec::new();
    let mut col = 0;

    for index in start_index..track.words.len() {
        let word = track.words[index].as_str();
        let word_width = word.chars().count();
        let leading_space = usize::from(!words.is_empty());
        let next_end = col + leading_space + word_width;

        if !words.is_empty() && next_end > width {
            break;
        }

        if words.is_empty() && word_width > width {
            let truncated = &word[..width.min(word.len())];
            words.push(VisibleWord {
                index,
                word: truncated,
                start_col: 0,
                end_col: truncated.chars().count(),
                state: word_state(index, safe_current),
            });
            break;
        }

        col += leading_space;
        words.push(VisibleWord {
            index,
            word,
            start_col: col,
            end_col: col + word_width,
            state: word_state(index, safe_current),
        });
        col += word_width;
    }

    TrackWindow { words, width }
}

fn word_state(index: usize, current_index: usize) -> WordRenderState {
    if index < current_index {
        WordRenderState::Completed
    } else if index == current_index {
        WordRenderState::Current
    } else {
        WordRenderState::Upcoming
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackCell {
    ch: char,
    style: Style,
}

impl Default for TrackCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            style: Style::default(),
        }
    }
}

fn track_word_line(window: &TrackWindow<'_>, player: &PlayerState) -> Line<'static> {
    let mut cells = vec![TrackCell::default(); window.width];

    for visible in &window.words {
        for (offset, ch) in visible.word.chars().enumerate() {
            let column = visible.start_col + offset;
            if let Some(cell) = cells.get_mut(column) {
                *cell = TrackCell {
                    ch,
                    style: base_word_style(visible.state, player),
                };
            }
        }
    }

    overlay_player_input(&mut cells, window, player);

    Line::from(
        cells
            .into_iter()
            .map(|cell| Span::styled(cell.ch.to_string(), cell.style))
            .collect::<Vec<_>>(),
    )
}

fn base_word_style(state: WordRenderState, player: &PlayerState) -> Style {
    match state {
        WordRenderState::Completed => Style::default().fg(Color::DarkGray),
        WordRenderState::Upcoming => Style::default(),
        WordRenderState::Current if player.is_finished() => Style::default().fg(Color::DarkGray),
        WordRenderState::Current => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    }
}

fn overlay_player_input(cells: &mut [TrackCell], window: &TrackWindow<'_>, player: &PlayerState) {
    if player.is_finished() {
        return;
    }

    for (stream_index, typed_ch) in player.input.chars().enumerate() {
        let Some(column) = window.column_for_stream_index(player.word_index, stream_index) else {
            break;
        };
        let Some(cell) = cells.get_mut(column) else {
            break;
        };

        cell.ch = display_input_char(typed_ch);
        cell.style = typed_char_style(stream_index, player.typo_index);
    }

    if player.typo_index.is_none() {
        let cursor_index = player.input.chars().count();
        if let Some(column) = window.column_for_stream_index(player.word_index, cursor_index) {
            if let Some(cell) = cells.get_mut(column) {
                cell.style = cursor_style();
            }
        }
    }
}

fn display_input_char(ch: char) -> char {
    if ch == ' ' { '␠' } else { ch }
}

fn typed_char_style(stream_index: usize, typo_index: Option<usize>) -> Style {
    if typo_index.is_some_and(|typo| stream_index >= typo) {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    }
}

fn cursor_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn stats_view<'a>(player: &PlayerState) -> Paragraph<'a> {
    let now = player.finished_at.unwrap_or_else(std::time::Instant::now);
    let wpm = player.stats.words_per_minute(player.started_at, now);
    let stats = vec![
        Line::from(format!("Accuracy {:.0}%", player.stats.accuracy())),
        Line::from(format!("WPM {:.0}", wpm)),
        Line::from(format!("Backspaces {}", player.stats.backspaces)),
        Line::from(format!("Typos {}", player.stats.typo_chars)),
    ];

    Paragraph::new(stats).block(Block::default().title("Stats").borders(Borders::ALL))
}

fn item_view<'a>(player: &PlayerState, attack_warning: Option<AttackWarning>) -> Paragraph<'a> {
    let now = std::time::Instant::now();
    let held = player.held_item.map(|item| item.name()).unwrap_or("None");
    let shield = player
        .active_effects
        .iter()
        .find_map(|effect| match effect {
            crate::game::effects::ActiveEffect::Shield { until } if *until > now => Some(format!(
                "Shield {:.1}s",
                until.saturating_duration_since(now).as_secs_f64()
            )),
            _ => None,
        })
        .unwrap_or_else(|| "Shield inactive".to_string());
    let warning = attack_warning
        .map(|warning| {
            format!(
                "Warning {:.1}s",
                warning
                    .resolves_at
                    .saturating_duration_since(now)
                    .as_secs_f64()
            )
        })
        .unwrap_or_else(|| "Warning none".to_string());

    Paragraph::new(vec![
        Line::from(format!("Held: {held}")),
        Line::from(shield),
        Line::from(warning),
    ])
    .block(Block::default().title("Item").borders(Borders::ALL))
}

fn player_list<'a>(track: &Track, player: &PlayerState) -> Paragraph<'a> {
    let placement = if player.is_finished() { "1." } else { "1." };
    let line = format!(
        "{placement} you     {}/{} words",
        player.stats.completed_words,
        track.len()
    );

    Paragraph::new(line).block(Block::default().title("Players").borders(Borders::ALL))
}

fn event_feed<'a>(events: &'a EventLog) -> Paragraph<'a> {
    let lines = events.entries().map(Line::from).collect::<Vec<_>>();
    Paragraph::new(lines).block(Block::default().title("Events").borders(Borders::ALL))
}

fn results_or_events<'a>(player: &PlayerState, events: &'a EventLog) -> Paragraph<'a> {
    if player.is_finished() {
        results_view(player)
    } else {
        event_feed(events)
    }
}

fn finish_or_empty<'a>(player: &PlayerState) -> Paragraph<'a> {
    if player.is_finished() {
        results_view(player)
    } else {
        Paragraph::new("").block(Block::default().borders(Borders::ALL))
    }
}

fn results_view<'a>(player: &PlayerState) -> Paragraph<'a> {
    let finished_at = player.finished_at.unwrap_or_else(std::time::Instant::now);
    let elapsed = finished_at.saturating_duration_since(player.started_at);
    let wpm = player
        .stats
        .words_per_minute(player.started_at, finished_at);
    let text = vec![
        Line::from(Span::styled(
            "Race complete",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("Time: {:.1}s", elapsed.as_secs_f64())),
        Line::from(format!("WPM: {:.0}", wpm)),
        Line::from(format!("Accuracy: {:.0}%", player.stats.accuracy())),
        Line::from("Press Ctrl-R to restart, Esc or Ctrl-C to exit."),
    ];

    Paragraph::new(text).block(Block::default().title("Results").borders(Borders::ALL))
}

fn help_view<'a>() -> Paragraph<'a> {
    Paragraph::new(
        "Space between words. Backspace fixes typos. Enter uses item. Ctrl-R restarts. Esc quits.",
    )
    .block(Block::default().borders(Borders::TOP))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::style::Color;

    use crate::{
        game::{
            bonus::{BonusChoice, BonusPoint, BonusState},
            player::PlayerState,
            track::Track,
        },
        ui::render::{
            WordRenderState, bonus_column, build_track_window, is_bonus_point_claimable,
            track_word_line, visible_bonus_point,
        },
    };

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
    fn current_word_spans_show_typed_prefix_and_cursor_on_track() {
        let track = track(&["fox", "road"]);
        let window = build_track_window(&track, 0, 40);
        let mut player = PlayerState::new(Instant::now());
        player.input = "fo".to_string();

        let line = track_word_line(&window, &player);
        let spans = line.spans;

        assert_eq!(spans[0].content.as_ref(), "f");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        assert_eq!(spans[1].content.as_ref(), "o");
        assert_eq!(spans[1].style.fg, Some(Color::Green));
        assert_eq!(spans[2].content.as_ref(), "x");
        assert_eq!(spans[2].style.bg, Some(Color::Yellow));
    }

    #[test]
    fn current_word_spans_render_typos_red_on_track() {
        let track = track(&["fox", "road"]);
        let window = build_track_window(&track, 0, 40);
        let mut player = PlayerState::new(Instant::now());
        player.input = "fa".to_string();
        player.typo_index = Some(1);

        let line = track_word_line(&window, &player);
        let spans = line.spans;

        assert_eq!(spans[0].content.as_ref(), "f");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        assert_eq!(spans[1].content.as_ref(), "a");
        assert_eq!(spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn typo_overflow_renders_across_following_words() {
        let track = track(&["fox", "road"]);
        let window = build_track_window(&track, 0, 40);
        let mut player = PlayerState::new(Instant::now());
        player.input = "fa road".to_string();
        player.typo_index = Some(1);

        let line = track_word_line(&window, &player);
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
}
