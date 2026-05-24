//! Shared race lifecycle helpers.
//!
//! This module keeps finish/timeout/rematch policy out of host adapters. Hosts
//! still decide how to log, broadcast, and render lifecycle changes.

use std::time::{Duration, Instant};

use super::race::{
    RaceLifecycleState, RaceLifecycleStatus, RaceLifecycleUpdate, RaceRuntimeState, RaceState,
};

pub fn update_race_flow(
    lifecycle: &mut RaceLifecycleState,
    race: &RaceState,
    now: Instant,
    post_first_finish_timeout: Duration,
) -> RaceLifecycleUpdate {
    lifecycle.update(race, now, post_first_finish_timeout)
}

pub fn race_flow_is_finished(update: &RaceLifecycleUpdate) -> bool {
    matches!(update.status, RaceLifecycleStatus::Finished { .. })
}

pub fn reset_race_runtime<PlayerId, BonusAttempt>(
    runtime: &mut RaceRuntimeState<PlayerId, BonusAttempt>,
) {
    runtime.reset();
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{race_flow_is_finished, reset_race_runtime, update_race_flow};
    use crate::game::{
        race::{PlayerColorId, RaceLifecycleState, RacePlayerId, RaceRuntimeState, RaceState},
        track::Track,
    };

    #[test]
    fn race_flow_reports_finished_update() {
        let now = Instant::now();
        let mut race = RaceState::new(Track::new(vec!["go".to_string()]));
        race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
        race.players[0].state.finished_at = Some(now);
        let mut lifecycle = RaceLifecycleState::new();

        let update = update_race_flow(&mut lifecycle, &race, now, Duration::from_secs(30));

        assert!(race_flow_is_finished(&update));
        assert_eq!(lifecycle.placements, vec![RacePlayerId(1)]);
    }

    #[test]
    fn reset_race_runtime_clears_shared_runtime_state() {
        let now = Instant::now();
        let mut runtime = RaceRuntimeState::<RacePlayerId, usize>::new();
        runtime.lifecycle.first_finished_at = Some(now);
        runtime.bonus_attempts.insert(RacePlayerId(1), 0);
        runtime.spent_bonus_gaps.insert(RacePlayerId(1), 3);

        reset_race_runtime(&mut runtime);

        assert_eq!(runtime.lifecycle.first_finished_at, None);
        assert!(runtime.bonus_attempts.is_empty());
        assert!(runtime.spent_bonus_gaps.is_empty());
    }
}
