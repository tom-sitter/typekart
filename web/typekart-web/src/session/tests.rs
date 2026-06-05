    use super::{browser_controls, key_name_to_protocol_key, should_capture_global_gameplay_key};
    use crate::fixtures::{GalleryFrame, SCENARIOS, scenario_frame};
    use typekart_protocol::{PlayerId, ProtocolKey};

    #[test]
    fn keyboard_mapping_preserves_typing_controls() {
        assert_eq!(key_name_to_protocol_key("a"), Some(ProtocolKey::Char('a')));
        assert_eq!(key_name_to_protocol_key("A"), Some(ProtocolKey::Char('a')));
        assert_eq!(key_name_to_protocol_key(" "), Some(ProtocolKey::Space));
        assert_eq!(
            key_name_to_protocol_key("Backspace"),
            Some(ProtocolKey::Backspace)
        );
        assert_eq!(key_name_to_protocol_key("Enter"), None);
    }

    #[test]
    fn browser_controls_show_ready_for_unready_lobby_joiner() {
        let GalleryFrame::Lobby(mut lobby) = scenario_frame(SCENARIOS[0]) else {
            unreachable!();
        };
        let local_player_id = lobby.players[1].id;
        lobby.players[1].ready = false;
        let frame = GalleryFrame::Lobby(lobby);

        let controls = browser_controls(Some(&frame), Some(local_player_id));

        assert!(controls.show_ready);
        assert!(!controls.show_unready);
        assert!(!controls.show_start);
    }

    #[test]
    fn browser_controls_show_start_only_for_ready_lobby_host() {
        let GalleryFrame::Lobby(lobby) = scenario_frame(SCENARIOS[0]) else {
            unreachable!();
        };
        let host_id = lobby.host_id;
        let joiner_id = lobby.players[1].id;
        let frame = GalleryFrame::Lobby(lobby);

        let host_controls = browser_controls(Some(&frame), Some(host_id));
        let joiner_controls = browser_controls(Some(&frame), Some(joiner_id));

        assert!(host_controls.show_unready);
        assert!(host_controls.show_start);
        assert!(joiner_controls.show_unready);
        assert!(!joiner_controls.show_start);
    }

    #[test]
    fn browser_controls_offer_rematch_ready_after_results() {
        let GalleryFrame::Results(results) = scenario_frame(SCENARIOS[8]) else {
            unreachable!();
        };
        let frame = GalleryFrame::Results(results);

        let controls = browser_controls(Some(&frame), Some(PlayerId(1)));

        assert!(controls.show_rematch_ready);
        assert!(!controls.show_ready);
        assert!(controls.show_start);
    }

    #[test]
    fn browser_controls_hide_result_start_for_joiners() {
        let GalleryFrame::Results(results) = scenario_frame(SCENARIOS[8]) else {
            unreachable!();
        };
        let frame = GalleryFrame::Results(results);

        let controls = browser_controls(Some(&frame), Some(PlayerId(2)));

        assert!(controls.show_rematch_ready);
        assert!(!controls.show_start);
    }

    #[test]
    fn global_gameplay_keys_capture_only_during_active_local_race() {
        let GalleryFrame::Race(countdown) = scenario_frame(SCENARIOS[1]) else {
            unreachable!();
        };
        let GalleryFrame::Race(racing) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        let local_player_id = racing.players[0].id;

        assert!(!should_capture_global_gameplay_key(
            Some(&GalleryFrame::Race(countdown)),
            Some(local_player_id)
        ));
        assert!(should_capture_global_gameplay_key(
            Some(&GalleryFrame::Race(racing)),
            Some(local_player_id)
        ));
        assert!(!should_capture_global_gameplay_key(
            Some(&GalleryFrame::Race(scenario_race_with_finished_local())),
            Some(local_player_id)
        ));
        assert!(!should_capture_global_gameplay_key(
            None,
            Some(local_player_id)
        ));
    }

    fn scenario_race_with_finished_local() -> typekart_protocol::RaceSnapshot {
        let GalleryFrame::Race(mut racing) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        racing.players[0].finished = true;
        racing
    }
