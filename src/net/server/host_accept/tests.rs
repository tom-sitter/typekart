use super::joiner_needs_active_race_snapshot;
use crate::net::protocol::NetworkRacePhase;

#[test]
fn active_race_screen_phases_send_full_snapshot_to_new_joiners() {
    assert!(!joiner_needs_active_race_snapshot(NetworkRacePhase::Lobby));
    assert!(!joiner_needs_active_race_snapshot(
        NetworkRacePhase::WaitingForHost
    ));
    assert!(joiner_needs_active_race_snapshot(
        NetworkRacePhase::Countdown {
            remaining_seconds: 3
        }
    ));
    assert!(joiner_needs_active_race_snapshot(NetworkRacePhase::Racing));
    assert!(joiner_needs_active_race_snapshot(
        NetworkRacePhase::Finished
    ));
}
