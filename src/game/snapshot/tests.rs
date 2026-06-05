use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use super::{
    build_bonus_snapshots, build_placement_snapshots, build_player_snapshots,
    build_race_result_snapshots,
};
use crate::game::{
    bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::Track,
};

#[test]
fn bonus_snapshot_reports_expired_cooldowns_as_available() {
    let now = std::time::Instant::now();
    let bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice {
                    word: "dash".to_string(),
                    status: BonusChoiceStatus::Cooldown {
                        until: now - Duration::from_secs(1),
                    },
                },
                BonusChoice::available("spin"),
                BonusChoice::available("zoom"),
            ],
        )],
        Vec::new(),
    );

    let snapshot = build_bonus_snapshots(&bonuses, now);

    assert!(matches!(
        snapshot[0].choices[0].status,
        typekart_protocol::BonusChoiceSnapshotStatus::Available
    ));
}

#[test]
fn player_snapshot_contains_core_typing_state() {
    let now = std::time::Instant::now();
    let mut race = RaceState::new(Track::new(vec!["one".to_string(), "two".to_string()]));
    race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);
    race.players[0].state.word_index = 1;
    race.players[0].state.input = "tw".to_string();

    let snapshot = build_player_snapshots(&race, &HashMap::new(), now, |_| {
        typekart_protocol::PlayerKind::Human
    });

    assert_eq!(snapshot[0].id, typekart_protocol::PlayerId(1));
    assert_eq!(snapshot[0].word_index, 1);
    assert_eq!(snapshot[0].input, "tw");
    assert_eq!(snapshot[0].color, typekart_protocol::AssignedColor::Cyan);
}

#[test]
fn race_result_snapshots_convert_shared_rows_to_protocol_rows() {
    let now = Instant::now();
    let mut race = RaceState::new(Track::new(vec!["go".to_string()]));
    race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
    race.players[0].state.finished_at = Some(now);

    let rows = build_race_result_snapshots(&race, &[RacePlayerId(1)], now);
    let placements = build_placement_snapshots(&[RacePlayerId(1)]);

    assert_eq!(placements, vec![typekart_protocol::PlayerId(1)]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].player_id, typekart_protocol::PlayerId(1));
    assert_eq!(
        rows[0].status,
        typekart_protocol::RaceResultStatus::Finished
    );
}
