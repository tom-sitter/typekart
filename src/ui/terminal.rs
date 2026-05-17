//! Terminal lifecycle and input loop.
//!
//! This module owns raw mode, alternate-screen setup, key polling, and cleanup.
//! It converts `crossterm` key events into game-level `KeyAction` values.

use std::{
    fs,
    io::{self, Stdout},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    game::{
        ai::AiDifficulty,
        items::ItemRegistry,
        mods::ActiveModConfig,
        player::PlayerState,
        track::{Track, WordList},
        typing::KeyAction,
    },
    ui::render::{IconMode, TypingScreen, render},
    ui::session::{LocalAction, LocalSession, RunLog},
};

type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

pub fn run_typing_session(
    track: Track,
    player: PlayerState,
    word_list: WordList,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    item_registry: ItemRegistry,
    active_mod_config: ActiveModConfig,
    icon_mode: IconMode,
    debug_log: Option<PathBuf>,
) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_loop(
        &mut terminal,
        LocalSession::new(
            track,
            player,
            word_list,
            ai_racer_count,
            ai_difficulty,
            item_registry,
            active_mod_config,
        ),
        icon_mode,
    );
    restore_terminal(&mut terminal)?;
    let session = result?;
    if let Some(path) = debug_log {
        write_debug_log(path, &session.run_log)?;
    }

    Ok(())
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

fn run_loop(
    terminal: &mut AppTerminal,
    mut session: LocalSession,
    icon_mode: IconMode,
) -> Result<LocalSession> {
    loop {
        terminal.draw(|frame| {
            render(
                frame,
                TypingScreen {
                    track: &session.track,
                    player: &session.player,
                    bonuses: &session.bonuses,
                    bonus_attempt: session.bonus_attempt,
                    player_impact_cue: session.player_impact_cue,
                    player_item_cue: session.player_item_cue.clone(),
                    race_status: session.race_status,
                    race_phase: session.race_phase,
                    icon_mode,
                    ai_racers: &session.ai_racers,
                    events: &session.events,
                },
            );
        })?;

        session.tick(Instant::now());

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
            return Ok(session);
        }

        if let Some(action) = local_action(key_event) {
            session.apply_action(action, Instant::now());
        }
    }
}

fn write_debug_log(path: PathBuf, run_log: &RunLog) -> Result<()> {
    let contents = run_log.entries().collect::<Vec<_>>().join("\n");
    fs::write(&path, format!("{contents}\n"))
        .with_context(|| format!("failed to write debug log to {}", path.display()))
}

fn should_quit(key_event: KeyEvent) -> bool {
    key_event.code == KeyCode::Esc
        || (key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn local_action(key_event: KeyEvent) -> Option<LocalAction> {
    match key_event.code {
        KeyCode::Enter if key_event.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(LocalAction::ActivateModifiedItem)
        }
        KeyCode::Enter => Some(LocalAction::ActivateItem),
        KeyCode::Char('r') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LocalAction::Restart)
        }
        KeyCode::Char('k') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LocalAction::ActivateModifiedItem)
        }
        KeyCode::Char(' ') => Some(LocalAction::Typing(KeyAction::Space)),
        KeyCode::Char(ch)
            if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
        {
            Some(LocalAction::Typing(KeyAction::Char(ch)))
        }
        KeyCode::Backspace => Some(LocalAction::Typing(KeyAction::Backspace)),
        _ => None,
    }
}
