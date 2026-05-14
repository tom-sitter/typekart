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
    game::{player::PlayerState, track::Track},
    ui::session::EventLog,
};

const WIDE_LAYOUT_MIN_WIDTH: u16 = 90;
const LOCAL_RACER_MARKER: &str = "███";
const LOCAL_RACER_MARKER_WIDTH: usize = 3;

pub struct TypingScreen<'a> {
    pub track: &'a Track,
    pub player: &'a PlayerState,
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

    pub fn racer_marker_start(&self) -> usize {
        let Some(current) = self.current_word() else {
            return 0;
        };
        let marker_center = current.start_col + current.word.chars().count() / 2;
        let marker_start = marker_center.saturating_sub(LOCAL_RACER_MARKER_WIDTH / 2);
        marker_start.min(self.width.saturating_sub(LOCAL_RACER_MARKER_WIDTH))
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
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(columns[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(columns[1]);

    render_track(frame, left[0], screen.track, screen.player);
    frame.render_widget(input_view(screen.track, screen.player), left[1]);
    frame.render_widget(finish_or_empty(screen.player), left[2]);
    frame.render_widget(stats_view(screen.player), right[0]);
    frame.render_widget(player_list(screen.track, screen.player), right[1]);
    frame.render_widget(event_feed(screen.events), right[2]);
}

fn render_narrow(frame: &mut Frame<'_>, area: Rect, screen: TypingScreen<'_>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(area);

    render_track(frame, rows[0], screen.track, screen.player);
    frame.render_widget(input_view(screen.track, screen.player), rows[1]);
    frame.render_widget(stats_view(screen.player), rows[2]);
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

fn render_track(frame: &mut Frame<'_>, area: Rect, track: &Track, player: &PlayerState) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let window = build_track_window(track, player.word_index, inner_width);
    frame.render_widget(track_view(&window), area);
}

fn track_view<'a>(window: &'a TrackWindow<'a>) -> Paragraph<'a> {
    let mut word_spans = Vec::new();
    for visible in &window.words {
        if visible.start_col > line_width(&word_spans) {
            word_spans.push(Span::raw(
                " ".repeat(visible.start_col - line_width(&word_spans)),
            ));
        }

        let style = match visible.state {
            WordRenderState::Completed => Style::default().fg(Color::DarkGray),
            WordRenderState::Current => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            WordRenderState::Upcoming => Style::default(),
        };
        word_spans.push(Span::styled(visible.word.to_string(), style));
    }

    let marker_start = window.racer_marker_start();
    let racer_line = Line::from(vec![
        Span::raw(" ".repeat(marker_start)),
        Span::styled(LOCAL_RACER_MARKER, Style::default().fg(Color::Cyan)),
    ]);

    Paragraph::new(vec![Line::from(word_spans), racer_line])
        .block(Block::default().title("Track").borders(Borders::ALL))
}

fn line_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
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

fn input_view<'a>(track: &'a Track, player: &PlayerState) -> Paragraph<'a> {
    let mut lines = vec![Line::from(vec![
        Span::styled("Target: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            track.current_word(player.word_index).unwrap_or(""),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    let mut input_spans = vec![Span::styled(
        "Input:  ",
        Style::default().fg(Color::DarkGray),
    )];
    let typo_index = player.typo_index;

    if player.input.is_empty() {
        input_spans.push(Span::styled("_", Style::default().fg(Color::DarkGray)));
    } else {
        for (index, ch) in player.input.chars().enumerate() {
            // Early Space is stored as a literal space so Backspace semantics
            // stay simple. Render it visibly so the player understands the typo.
            let display = if ch == ' ' {
                "␠".to_string()
            } else {
                ch.to_string()
            };
            let style = if typo_index.is_some_and(|typo| index >= typo) {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Green)
            };
            input_spans.push(Span::styled(display, style));
        }
        input_spans.push(Span::styled("_", Style::default().fg(Color::DarkGray)));
    }

    lines.push(Line::from(input_spans));

    Paragraph::new(lines).block(Block::default().title("Input").borders(Borders::ALL))
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
        Line::from("Press Esc or Ctrl-C to exit."),
    ];

    Paragraph::new(text).block(Block::default().title("Results").borders(Borders::ALL))
}

fn help_view<'a>() -> Paragraph<'a> {
    Paragraph::new(
        "Press Space between words. The final word finishes immediately. Backspace fixes typos. Esc quits.",
    )
    .block(Block::default().borders(Borders::TOP))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::{
        game::{player::PlayerState, track::Track},
        ui::render::{WordRenderState, build_track_window},
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

        assert_eq!(window.racer_marker_start(), 4);
    }

    #[test]
    fn local_racer_marker_clamps_at_left_edge() {
        let track = track(&["a", "two"]);
        let window = build_track_window(&track, 0, 40);

        assert_eq!(window.racer_marker_start(), 0);
    }

    #[test]
    fn local_racer_marker_clamps_at_right_edge() {
        let track = track(&["abcde"]);
        let window = build_track_window(&track, 0, 2);

        assert_eq!(window.racer_marker_start(), 0);
    }

    #[test]
    fn player_state_type_still_compiles_for_renderer_tests() {
        let player = PlayerState::new(Instant::now());

        assert_eq!(player.word_index, 0);
    }
}
