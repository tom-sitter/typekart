//! Terminal lifecycle and input loop.
//!
//! This module owns raw mode, alternate-screen setup, key polling, and cleanup.
//! It converts `crossterm` key events into game-level `KeyAction` values.

use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    game::{player::PlayerState, track::Track, typing::KeyAction},
    ui::render::{TypingScreen, render},
    ui::session::LocalSession,
};

type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

pub fn run_typing_session(track: Track, player: PlayerState) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, LocalSession::new(track, player));
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<AppTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    Ok(terminal)
}

fn restore_terminal(terminal: &mut AppTerminal) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn run_loop(terminal: &mut AppTerminal, mut session: LocalSession) -> Result<()> {
    loop {
        terminal.draw(|frame| {
            render(
                frame,
                TypingScreen {
                    track: &session.track,
                    player: &session.player,
                    events: &session.events,
                },
            );
        })?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        // Crossterm can report resize, mouse, focus, and paste events. Milestone
        // 1 only cares about key presses, so non-key events are ignored.
        let Event::Key(key_event) = event::read()? else {
            continue;
        };

        if key_event.kind != KeyEventKind::Press {
            continue;
        }

        if should_quit(key_event) {
            return Ok(());
        }

        if let Some(action) = key_action(key_event) {
            session.apply_action(action, Instant::now());
        }
    }
}

fn should_quit(key_event: KeyEvent) -> bool {
    key_event.code == KeyCode::Esc
        || (key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn key_action(key_event: KeyEvent) -> Option<KeyAction> {
    match key_event.code {
        KeyCode::Char(' ') => Some(KeyAction::Space),
        KeyCode::Char(ch)
            if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
        {
            Some(KeyAction::Char(ch))
        }
        KeyCode::Backspace => Some(KeyAction::Backspace),
        _ => None,
    }
}
