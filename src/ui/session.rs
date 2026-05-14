//! Local terminal session state.
//!
//! Multiplayer will eventually have server snapshots and remote players. For
//! Milestone 2, this type gives the single-player terminal loop the same rough
//! shape: game state plus display-facing event history.

use std::{collections::VecDeque, time::Instant};

use crate::game::{
    player::PlayerState,
    track::Track,
    typing::{KeyAction, TypingEvent, apply_key},
};

#[derive(Debug)]
pub struct LocalSession {
    pub track: Track,
    pub player: PlayerState,
    pub events: EventLog,
}

impl LocalSession {
    pub fn new(track: Track, player: PlayerState) -> Self {
        let mut events = EventLog::new(6);
        events.push("Race started");

        Self {
            track,
            player,
            events,
        }
    }

    pub fn apply_action(&mut self, action: KeyAction, now: Instant) {
        let previous_word_index = self.player.word_index;
        let previous_word = self
            .track
            .current_word(previous_word_index)
            .map(str::to_owned);
        let events = apply_key(&mut self.player, &self.track, action, now);

        for event in events {
            if let Some(message) = event_message(event, previous_word.as_deref()) {
                self.events.push(message);
            }
        }
    }
}

fn event_message(event: TypingEvent, previous_word: Option<&str>) -> Option<String> {
    match event {
        TypingEvent::InputChanged => None,
        TypingEvent::WordCompleted => previous_word.map(|word| format!("Completed {word}")),
        TypingEvent::RaceFinished => Some("Race finished".to_string()),
        TypingEvent::TypoStarted { .. } => Some("Typo started".to_string()),
        TypingEvent::TypoCleared => Some("Typo cleared".to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct EventLog {
    entries: VecDeque<String>,
    capacity: usize,
}

impl EventLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, message: impl Into<String>) {
        if self.capacity == 0 {
            return;
        }

        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(message.into());
    }

    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::{
        game::{player::PlayerState, track::Track, typing::KeyAction},
        ui::session::{EventLog, LocalSession},
    };

    fn track(words: &[&str]) -> Track {
        Track::new(words.iter().map(|word| word.to_string()).collect())
    }

    #[test]
    fn event_log_keeps_only_capacity_entries() {
        let mut log = EventLog::new(2);

        log.push("one");
        log.push("two");
        log.push("three");

        assert_eq!(log.entries().collect::<Vec<_>>(), vec!["two", "three"]);
    }

    #[test]
    fn local_session_logs_meaningful_typing_events() {
        let track = track(&["fox", "road"]);
        let player = PlayerState::new(Instant::now());
        let mut session = LocalSession::new(track, player);

        session.apply_action(KeyAction::Char('f'), Instant::now());
        session.apply_action(KeyAction::Char('a'), Instant::now());
        session.apply_action(KeyAction::Backspace, Instant::now());
        session.apply_action(KeyAction::Char('o'), Instant::now());
        session.apply_action(KeyAction::Char('x'), Instant::now());
        session.apply_action(KeyAction::Space, Instant::now());

        let entries = session.events.entries().collect::<Vec<_>>();
        assert!(entries.contains(&"Race started"));
        assert!(entries.contains(&"Typo started"));
        assert!(entries.contains(&"Typo cleared"));
        assert!(entries.contains(&"Completed fox"));
    }
}
