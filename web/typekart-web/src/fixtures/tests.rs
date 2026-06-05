    use super::{
        GalleryFrame, SCENARIOS, masked_word, minimap_position, scenario_frame, scenario_slugs,
    };
    use typekart_protocol::{
        BonusChoiceSnapshotStatus, ImpactCueSnapshotKind, ItemCueSnapshotKind, NetworkRacePhase,
        RaceResultStatus, ServerMessage, encode_server_message,
    };

    #[test]
    fn gallery_covers_all_major_web_states() {
        let slugs = scenario_slugs();

        for expected in [
            "lobby",
            "countdown",
            "banana-impact",
            "mushroom-boost",
            "shield-focus",
            "cyclone-impact",
            "fog",
            "finish-sprint",
            "results",
        ] {
            assert!(slugs.contains(&expected), "missing scenario {expected}");
        }
    }

    #[test]
    fn item_scenarios_cover_current_items_and_impacts() {
        let all_players = SCENARIOS
            .iter()
            .filter_map(|scenario| match scenario_frame(*scenario) {
                GalleryFrame::Race(snapshot) => Some(snapshot.players),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();

        assert!(all_players.iter().any(|player| player.shielded));
        assert!(all_players.iter().any(|player| player.focused));
        assert!(all_players.iter().any(|player| player.fogged));
        assert!(all_players.iter().any(|player| player.boosted));
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.item_cue.as_ref())
                .any(|cue| matches!(cue.kind, ItemCueSnapshotKind::Banana { .. }))
        );
        assert!(all_players.iter().any(|player| player.boosted));
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.item_cue.as_ref())
                .any(|cue| matches!(cue.kind, ItemCueSnapshotKind::Cyclone { .. }))
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.item_cue.as_ref())
                .any(|cue| cue.kind == ItemCueSnapshotKind::Fog)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.impact_cue.as_ref())
                .any(|impact| impact.kind == ImpactCueSnapshotKind::Banana)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.impact_cue.as_ref())
                .any(|impact| impact.kind == ImpactCueSnapshotKind::Cyclone)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.impact_cue.as_ref())
                .any(|impact| impact.kind == ImpactCueSnapshotKind::Fog)
        );
    }

    #[test]
    fn consumed_bonus_and_results_states_are_represented() {
        assert!(SCENARIOS.iter().any(|scenario| {
            match scenario_frame(*scenario) {
                GalleryFrame::Race(snapshot) => snapshot
                    .bonuses
                    .iter()
                    .flat_map(|bonus| &bonus.choices)
                    .any(|choice| {
                        matches!(choice.status, BonusChoiceSnapshotStatus::Cooldown { .. })
                    }),
                _ => false,
            }
        }));
        assert!(SCENARIOS.iter().any(|scenario| {
            match scenario_frame(*scenario) {
                GalleryFrame::Results(results) => results
                    .rows
                    .iter()
                    .any(|row| row.status == RaceResultStatus::TimedOut),
                _ => false,
            }
        }));
    }

    #[test]
    fn minimap_position_pins_finish_to_end() {
        let finish = SCENARIOS
            .iter()
            .find_map(|scenario| match scenario_frame(*scenario) {
                GalleryFrame::Race(snapshot) if snapshot.phase == NetworkRacePhase::Finished => {
                    Some(snapshot)
                }
                _ => None,
            })
            .unwrap();
        let finished = &finish.players[0];

        assert_eq!(minimap_position(finished, finish.track_words.len()), 100);
    }

    #[test]
    fn fog_masks_only_future_words() {
        let fogged = match scenario_frame(SCENARIOS[6]) {
            GalleryFrame::Race(snapshot) => snapshot.players[1].clone(),
            _ => unreachable!(),
        };

        assert_eq!(masked_word(&fogged, fogged.word_index, "cyclone"), "cyclone");
        assert_eq!(masked_word(&fogged, fogged.word_index + 1, "maple"), "█████");
    }

    #[test]
    fn gallery_frames_are_serializable_as_protocol_messages() {
        for scenario in SCENARIOS {
            match scenario_frame(*scenario) {
                GalleryFrame::Lobby(lobby) => {
                    let message = ServerMessage::LobbySnapshot {
                        players: lobby.players,
                        host_id: lobby.host_id,
                        mod_config: lobby.mod_config,
                        events: lobby.events,
                    };
                    encode_server_message(&message).unwrap();
                }
                GalleryFrame::Race(snapshot) => {
                    encode_server_message(&ServerMessage::RaceSnapshot(snapshot)).unwrap();
                }
                GalleryFrame::Results(results) => {
                    let message = ServerMessage::RaceResults {
                        placements: results.placements,
                        rows: results.rows,
                    };
                    encode_server_message(&message).unwrap();
                }
            }
        }
    }
