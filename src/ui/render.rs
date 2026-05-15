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
        player::PlayerState,
        track::Track,
    },
    ui::session::{AiRacer, BonusAttempt, EventLog, RaceStatus},
};

const WIDE_LAYOUT_MIN_WIDTH: u16 = 90;
const TRACK_PANEL_HEIGHT: u16 = 10;
const LOCAL_RACER_MARKER: &str = "███";
const SHIELDED_RACER_MARKER: &str = "[███]";
const BOOST_MARKER_SUFFIX: &str = ">>>";

pub struct TypingScreen<'a> {
    pub track: &'a Track,
    pub player: &'a PlayerState,
    pub bonuses: &'a BonusState,
    pub bonus_attempt: Option<BonusAttempt>,
    pub player_impact_until: Option<std::time::Instant>,
    pub race_status: RaceStatus,
    pub ai_racers: &'a [AiRacer],
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

    pub fn racer_marker_for_visible_player(
        &self,
        player: &PlayerState,
        marker_width: usize,
    ) -> Option<VisibleRacerMarker> {
        let first_visible = self.words.first()?.index;
        let last_visible = self.words.last()?.index;

        if player.word_index < first_visible {
            return Some(VisibleRacerMarker::Behind);
        }

        if player.is_finished() {
            return Some(VisibleRacerMarker::Visible {
                start: self.racer_marker_start_for_player(player, marker_width),
            });
        }

        if player.word_index > last_visible {
            return Some(VisibleRacerMarker::Ahead);
        }

        Some(VisibleRacerMarker::Visible {
            start: self.racer_marker_start_for_player(player, marker_width),
        })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibleRacerMarker {
    Visible { start: usize },
    Behind,
    Ahead,
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
        .constraints([Constraint::Length(TRACK_PANEL_HEIGHT), Constraint::Min(0)])
        .split(columns[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
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
        screen.ai_racers,
        screen.player_impact_until,
    );
    frame.render_widget(
        finish_or_empty(screen.player, screen.ai_racers, screen.race_status),
        left[1],
    );
    frame.render_widget(stats_view(screen.player), right[0]);
    frame.render_widget(
        player_list(screen.track, screen.player, screen.ai_racers),
        right[1],
    );
    frame.render_widget(event_feed(screen.events), right[2]);
}

fn render_narrow(frame: &mut Frame<'_>, area: Rect, screen: TypingScreen<'_>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TRACK_PANEL_HEIGHT),
            Constraint::Length(4),
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
        screen.ai_racers,
        screen.player_impact_until,
    );
    frame.render_widget(stats_view(screen.player), rows[1]);
    frame.render_widget(
        player_list(screen.track, screen.player, screen.ai_racers),
        rows[2],
    );
    frame.render_widget(
        results_or_events(
            screen.player,
            screen.ai_racers,
            screen.race_status,
            screen.events,
        ),
        rows[3],
    );
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
    ai_racers: &[AiRacer],
    player_impact_until: Option<std::time::Instant>,
) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let window = build_track_window(track, player.word_index, inner_width);
    frame.render_widget(
        track_view(
            &window,
            player,
            bonuses,
            bonus_attempt,
            ai_racers,
            player_impact_until,
        ),
        area,
    );
}

fn track_view<'a>(
    window: &'a TrackWindow<'a>,
    player: &PlayerState,
    bonuses: &'a BonusState,
    bonus_attempt: Option<BonusAttempt>,
    ai_racers: &[AiRacer],
    player_impact_until: Option<std::time::Instant>,
) -> Paragraph<'a> {
    let word_line = track_word_line(window, player);

    let now = std::time::Instant::now();
    let racer_lines = racer_lines(window, player, ai_racers, player_impact_until, now);

    let mut lines = bonus_lines(window, player, bonuses, bonus_attempt, now);
    lines.push(word_line);
    lines.extend(racer_lines);

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

fn racer_lines(
    window: &TrackWindow<'_>,
    player: &PlayerState,
    ai_racers: &[AiRacer],
    player_impact_until: Option<std::time::Instant>,
    now: std::time::Instant,
) -> Vec<Line<'static>> {
    let mut lines = vec![racer_line_for_player(
        window,
        player,
        now,
        Color::Cyan,
        player_impact_until,
        RacerVisibility::Always,
    )];
    lines.extend(ai_racers.iter().map(|ai| {
        racer_line_for_player(
            window,
            &ai.player,
            now,
            ai_color(ai.id),
            ai.impact_until,
            RacerVisibility::OnlyWhenCurrentWordVisible,
        )
    }));
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RacerVisibility {
    Always,
    OnlyWhenCurrentWordVisible,
}

fn racer_line_for_player(
    window: &TrackWindow<'_>,
    player: &PlayerState,
    now: std::time::Instant,
    color: Color,
    impact_until: Option<std::time::Instant>,
    visibility: RacerVisibility,
) -> Line<'static> {
    let mut cells = vec![TrackCell::default(); window.width];
    let marker = racer_marker(player, now);
    let visible_marker = match visibility {
        RacerVisibility::Always => Some(VisibleRacerMarker::Visible {
            start: window.racer_marker_start_for_player(player, marker.chars().count()),
        }),
        RacerVisibility::OnlyWhenCurrentWordVisible => {
            window.racer_marker_for_visible_player(player, marker.chars().count())
        }
    };

    if let Some(visible_marker) = visible_marker {
        let (marker_start, marker) = match visible_marker {
            VisibleRacerMarker::Visible { start } => (start, marker),
            VisibleRacerMarker::Behind => (0, edge_racer_marker('<', player, now)),
            VisibleRacerMarker::Ahead => (
                window
                    .width
                    .saturating_sub(edge_racer_marker('>', player, now).chars().count()),
                edge_racer_marker('>', player, now),
            ),
        };
        write_marker(
            &mut cells,
            marker_start,
            marker.as_str(),
            marker_style(color, impact_until, now),
        );
    }

    Line::from(
        cells
            .into_iter()
            .map(|cell| Span::styled(cell.ch.to_string(), cell.style))
            .collect::<Vec<_>>(),
    )
}

fn edge_racer_marker(direction: char, player: &PlayerState, now: std::time::Instant) -> String {
    let mut marker = if player.has_active_shield(now) {
        format!("[{direction}]")
    } else {
        direction.to_string()
    };

    if has_active_mushroom(player) {
        marker.push_str(BOOST_MARKER_SUFFIX);
    }

    marker
}

fn racer_marker(player: &PlayerState, now: std::time::Instant) -> String {
    let mut marker = if player.has_active_shield(now) {
        SHIELDED_RACER_MARKER.to_string()
    } else {
        LOCAL_RACER_MARKER.to_string()
    };

    if has_active_mushroom(player) {
        marker.push_str(BOOST_MARKER_SUFFIX);
    }

    marker
}

fn has_active_mushroom(player: &PlayerState) -> bool {
    player.active_effects.iter().any(|effect| {
        matches!(
            effect,
            crate::game::effects::ActiveEffect::Mushroom {
                remaining_words,
                ..
            } if *remaining_words > 0
        )
    })
}

fn write_marker(cells: &mut [TrackCell], start: usize, marker: &str, style: Style) {
    for (offset, ch) in marker.chars().enumerate() {
        if let Some(cell) = cells.get_mut(start + offset) {
            *cell = TrackCell { ch, style };
        }
    }
}

fn marker_style(
    color: Color,
    impact_until: Option<std::time::Instant>,
    now: std::time::Instant,
) -> Style {
    let base = Style::default().fg(color).add_modifier(Modifier::BOLD);
    if impact_blink_visible(impact_until, now) {
        base.bg(Color::Yellow).fg(Color::Black)
    } else {
        base
    }
}

fn impact_blink_visible(impact_until: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    let Some(until) = impact_until else {
        return false;
    };
    if until <= now {
        return false;
    }

    let remaining_ms = until.saturating_duration_since(now).as_millis();
    (remaining_ms / 150) % 2 == 0
}

fn ai_color(id: usize) -> Color {
    match id % 3 {
        1 => Color::LightRed,
        2 => Color::LightGreen,
        _ => Color::LightBlue,
    }
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

fn player_list<'a>(track: &Track, player: &PlayerState, ai_racers: &[AiRacer]) -> Paragraph<'a> {
    let mut standings = Vec::with_capacity(ai_racers.len() + 1);
    standings.push(PlayerListRow {
        name: "you".to_string(),
        completed_words: player.stats.completed_words,
        input_chars: player.input.chars().count(),
        finished: player.is_finished(),
    });
    standings.extend(ai_racers.iter().map(|ai| PlayerListRow {
        name: format!(
            "{} ({} {:.0})",
            ai.name,
            ai.difficulty.name(),
            ai.words_per_minute
        ),
        completed_words: ai.player.stats.completed_words,
        input_chars: ai.player.input.chars().count(),
        finished: ai.player.is_finished(),
    }));
    standings.sort_by(|a, b| {
        b.finished
            .cmp(&a.finished)
            .then_with(|| b.completed_words.cmp(&a.completed_words))
            .then_with(|| b.input_chars.cmp(&a.input_chars))
    });

    let lines = standings
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            Line::from(format!(
                "{}. {:<12} {}/{} words",
                index + 1,
                row.name,
                row.completed_words,
                track.len()
            ))
        })
        .collect::<Vec<_>>();

    Paragraph::new(lines).block(Block::default().title("Players").borders(Borders::ALL))
}

struct PlayerListRow {
    name: String,
    completed_words: usize,
    input_chars: usize,
    finished: bool,
}

fn event_feed<'a>(events: &'a EventLog) -> Paragraph<'a> {
    let lines = events.entries().map(Line::from).collect::<Vec<_>>();
    Paragraph::new(lines).block(Block::default().title("Events").borders(Borders::ALL))
}

fn results_or_events<'a>(
    player: &PlayerState,
    ai_racers: &[AiRacer],
    race_status: RaceStatus,
    events: &'a EventLog,
) -> Paragraph<'a> {
    if race_status.is_ended() {
        results_view(player, ai_racers, race_status)
    } else {
        event_feed(events)
    }
}

fn finish_or_empty<'a>(
    player: &PlayerState,
    ai_racers: &[AiRacer],
    race_status: RaceStatus,
) -> Paragraph<'a> {
    if race_status.is_ended() {
        results_view(player, ai_racers, race_status)
    } else {
        Paragraph::new("").block(Block::default().borders(Borders::ALL))
    }
}

fn results_view<'a>(
    player: &PlayerState,
    ai_racers: &[AiRacer],
    race_status: RaceStatus,
) -> Paragraph<'a> {
    let mut text = vec![Line::from(Span::styled(
        "Race results",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))];
    text.extend(result_rows(player, ai_racers, race_status));
    text.push(Line::from(
        "Press Ctrl-R to restart, Esc or Ctrl-C to exit.",
    ));

    Paragraph::new(text).block(Block::default().title("Results").borders(Borders::ALL))
}

fn result_rows<'a>(
    player: &PlayerState,
    ai_racers: &[AiRacer],
    race_status: RaceStatus,
) -> Vec<Line<'a>> {
    let mut rows = race_result_rows(player, ai_racers, race_status);
    rows.sort_by(|a, b| {
        a.rank_key()
            .cmp(&b.rank_key())
            .then_with(|| b.completed_words.cmp(&a.completed_words))
            .then_with(|| b.input_chars.cmp(&a.input_chars))
    });

    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            Line::from(format!(
                "{}. {:<12} {:>5} {:>7} {:>4.0} WPM",
                index + 1,
                row.name,
                row.time_label(),
                row.status_label(),
                row.wpm
            ))
        })
        .collect()
}

fn race_result_rows(
    player: &PlayerState,
    ai_racers: &[AiRacer],
    race_status: RaceStatus,
) -> Vec<RaceResultRow> {
    let mut rows = vec![RaceResultRow::new("you".to_string(), player, race_status)];
    rows.extend(
        ai_racers
            .iter()
            .map(|ai| RaceResultRow::new(ai.name.clone(), &ai.player, race_status)),
    );
    rows
}

struct RaceResultRow {
    name: String,
    finished_at: Option<std::time::Instant>,
    ended_at: Option<std::time::Instant>,
    started_at: std::time::Instant,
    completed_words: usize,
    input_chars: usize,
    wpm: f64,
}

impl RaceResultRow {
    fn new(name: String, player: &PlayerState, race_status: RaceStatus) -> Self {
        let end_for_wpm = player
            .finished_at
            .or(race_status.ended_at)
            .unwrap_or_else(std::time::Instant::now);
        Self {
            name,
            finished_at: player.finished_at,
            ended_at: race_status.ended_at,
            started_at: player.started_at,
            completed_words: player.stats.completed_words,
            input_chars: player.input.chars().count(),
            wpm: player
                .stats
                .words_per_minute(player.started_at, end_for_wpm),
        }
    }

    fn rank_key(&self) -> (usize, std::time::Instant) {
        match self.finished_at {
            Some(finished_at) => (0, finished_at),
            None => (1, self.ended_at.unwrap_or_else(std::time::Instant::now)),
        }
    }

    fn time_label(&self) -> String {
        self.finished_at
            .map(|finished_at| {
                format!(
                    "{:.1}s",
                    finished_at
                        .saturating_duration_since(self.started_at)
                        .as_secs_f64()
                )
            })
            .unwrap_or_else(|| "--".to_string())
    }

    fn status_label(&self) -> &'static str {
        if self.finished_at.is_some() {
            "done"
        } else {
            "timeout"
        }
    }
}

fn help_view<'a>() -> Paragraph<'a> {
    Paragraph::new("Space between words. Backspace fixes typos. Ctrl-R restarts. Esc quits.")
        .block(Block::default().borders(Borders::TOP))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::style::Color;

    use crate::{
        game::{
            ai::AiDifficulty,
            bonus::{BonusChoice, BonusPoint, BonusState},
            effects::ActiveEffect,
            player::PlayerState,
            track::Track,
        },
        ui::render::{
            WordRenderState, bonus_column, build_track_window, is_bonus_point_claimable,
            racer_lines, result_rows, track_word_line, visible_bonus_point,
        },
        ui::session::{AiRacer, RaceStatus},
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
    fn racer_lines_put_local_player_first_with_one_line_per_ai() {
        let track = track(&["one", "two", "three"]);
        let window = build_track_window(&track, 0, 40);
        let mut player = PlayerState::new(Instant::now());
        player.input = "o".to_string();
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, Instant::now());
        ai.player.word_index = 1;
        let lines = racer_lines(&window, &player, &[ai], None, Instant::now());

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "█");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(lines[1].spans[3].content.as_ref(), "█");
        assert_eq!(lines[1].spans[3].style.fg, Some(Color::LightRed));
    }

    #[test]
    fn ai_racer_marker_shows_left_indicator_when_behind_visible_window() {
        let track = track(&["one", "two", "three", "four", "five", "six"]);
        let window = build_track_window(&track, 5, 40);
        let player = PlayerState::new(Instant::now());
        let mut ai = AiRacer::new(1, AiDifficulty::Easy, 35.0, Instant::now());
        ai.player.word_index = 1;

        let lines = racer_lines(&window, &player, &[ai], None, Instant::now());

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

        let lines = racer_lines(&window, &player, &[ai], None, Instant::now());

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

        let lines = racer_lines(&window, &player, &[ai], None, now);

        assert!(lines[1].spans.iter().any(|span| {
            span.content.as_ref() == "█" && span.style.fg == Some(Color::LightRed)
        }));
        assert!(!lines[1].spans.iter().any(|span| {
            span.content.as_ref() == ">" && span.style.fg == Some(Color::LightRed)
        }));
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

        let lines = racer_lines(&window, &player, &[ai], None, now);

        assert_eq!(lines[1].spans[0].content.as_ref(), "[");
        assert_eq!(lines[1].spans[1].content.as_ref(), "<");
        assert_eq!(lines[1].spans[2].content.as_ref(), "]");
    }

    #[test]
    fn racer_line_shows_mushroom_boost_suffix() {
        let now = Instant::now();
        let track = track(&["one", "two", "three"]);
        let window = build_track_window(&track, 0, 40);
        let mut player = PlayerState::new(now);
        player.active_effects.push(ActiveEffect::Mushroom {
            remaining_words: 2,
            next_step_at: now,
            step_interval: std::time::Duration::from_millis(400),
        });

        let lines = racer_lines(&window, &player, &[], None, now);

        assert_eq!(lines[0].spans[0].content.as_ref(), "█");
        assert_eq!(lines[0].spans[1].content.as_ref(), "█");
        assert_eq!(lines[0].spans[2].content.as_ref(), "█");
        assert_eq!(lines[0].spans[3].content.as_ref(), ">");
        assert_eq!(lines[0].spans[4].content.as_ref(), ">");
        assert_eq!(lines[0].spans[5].content.as_ref(), ">");
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
            Some(now + std::time::Duration::from_millis(300)),
            now,
        );

        assert_eq!(lines[0].spans[0].style.bg, Some(Color::Yellow));
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
