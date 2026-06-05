use std::time::Instant;

use super::{CoreHost, CoreHostCommand, CoreHostEvent};
use crate::game::{
    race::{PlayerColorId, RacePlayerId},
    track::Track,
    typing::KeyAction,
};

fn track(words: &[&str]) -> Track {
    Track::new(words.iter().map(|word| word.to_string()).collect())
}

#[test]
fn core_host_can_drive_race_without_terminal_or_network() {
    let now = Instant::now();
    let player_id = RacePlayerId(1);
    let mut host = CoreHost::new(track(&["go"]));

    let events = host.tick(
        now,
        [
            CoreHostCommand::AddPlayer {
                id: player_id,
                name: "host".to_string(),
                color: PlayerColorId::Cyan,
            },
            CoreHostCommand::KeyInput {
                id: player_id,
                action: KeyAction::Char('g'),
            },
            CoreHostCommand::KeyInput {
                id: player_id,
                action: KeyAction::Char('o'),
            },
        ],
    );

    assert_eq!(
        events,
        [
            CoreHostEvent::PlayerAdded { id: player_id },
            CoreHostEvent::InputChanged { id: player_id },
            CoreHostEvent::InputChanged { id: player_id },
            CoreHostEvent::WordCompleted { id: player_id },
            CoreHostEvent::RaceFinished { id: player_id },
        ]
    );
    assert!(host.race().player(player_id).unwrap().state.is_finished());
}

#[test]
fn core_host_reports_missing_players() {
    let now = Instant::now();
    let missing = RacePlayerId(99);
    let mut host = CoreHost::new(track(&["go"]));

    let events = host.tick(
        now,
        [CoreHostCommand::KeyInput {
            id: missing,
            action: KeyAction::Char('g'),
        }],
    );

    assert_eq!(events, [CoreHostEvent::PlayerMissing { id: missing }]);
}
