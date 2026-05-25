//! Shared host-session helpers.
//!
//! A host session is the authoritative game loop regardless of whether the
//! adapter is local terminal play, LAN multiplayer, or a browser-hosted race.
//! This module contains browser-safe pieces of that authority and leaves
//! sockets, rendering, timers, and UI side effects to the adapters.

use std::time::{Duration, Instant};

use typekart_protocol::{LobbyPlayer, NetworkRacePhase};

use super::{
    bonus::BonusState,
    lobby::{lobby_players_to_participants, ready_connected_participants},
    race::{RaceLifecycleState, RaceParticipant, RaceState},
    race_flow::{RaceFlowOutcome, advance_race_flow},
    track::{Track, WordList},
};

#[derive(Debug, Clone)]
pub struct PreparedHostRace {
    pub race: RaceState,
    pub bonuses: BonusState,
    pub participants: Vec<RaceParticipant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountdownRacePreparation {
    PrepareRace,
    UseCurrentRace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountdownStartRejection {
    RaceAlreadyActive,
    NoConnectedRacers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountdownAdvanceRejection {
    NotCountingDown,
    NoConnectedRacers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnToLobbyDecision {
    Ignore,
    CancelRace,
    ReturnFromResults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnToLobbyOutcome {
    pub decision: ReturnToLobbyDecision,
    pub event: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRaceTickAction {
    Ignore,
    BroadcastDelta,
    BroadcastResults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRaceTickOutcome {
    pub action: HostRaceTickAction,
    pub race_changed: bool,
    pub bonus_choices_refreshed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRaceLifecycleOutcome {
    pub phase: NetworkRacePhase,
    pub flow: RaceFlowOutcome,
}

impl PreparedHostRace {
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }
}

pub fn prepare_race_from_lobby(
    lobby_players: &[LobbyPlayer],
    track: Track,
    word_list: &WordList,
    now: Instant,
) -> PreparedHostRace {
    let participants = ready_connected_participants(lobby_players);
    prepare_race_from_participants(participants, track, word_list, now)
}

pub fn prepare_race_from_selected_lobby_players(
    selected_players: &[LobbyPlayer],
    track: Track,
    word_list: &WordList,
    now: Instant,
) -> PreparedHostRace {
    let participants = lobby_players_to_participants(selected_players);
    prepare_race_from_participants(participants, track, word_list, now)
}

pub fn prepare_race_from_participants(
    participants: Vec<RaceParticipant>,
    track: Track,
    word_list: &WordList,
    now: Instant,
) -> PreparedHostRace {
    let bonuses = BonusState::generate(&track, word_list);
    let race = RaceState::from_participants(track, participants.clone(), now);

    PreparedHostRace {
        race,
        bonuses,
        participants,
    }
}

pub fn connected_racer_count(race: &RaceState) -> usize {
    race.players
        .iter()
        .filter(|player| player.connected)
        .count()
}

pub fn has_connected_active_racer(race: &RaceState) -> bool {
    race.players
        .iter()
        .any(|player| player.connected && !player.state.is_finished())
}

pub fn countdown_start_plan(
    phase: NetworkRacePhase,
) -> Result<CountdownRacePreparation, CountdownStartRejection> {
    match phase {
        NetworkRacePhase::Lobby | NetworkRacePhase::Finished => {
            Ok(CountdownRacePreparation::PrepareRace)
        }
        NetworkRacePhase::WaitingForHost => Ok(CountdownRacePreparation::UseCurrentRace),
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => {
            Err(CountdownStartRejection::RaceAlreadyActive)
        }
    }
}

pub fn begin_countdown_phase(
    connected_racer_count: usize,
) -> Result<NetworkRacePhase, CountdownStartRejection> {
    if connected_racer_count == 0 {
        return Err(CountdownStartRejection::NoConnectedRacers);
    }

    Ok(NetworkRacePhase::Countdown {
        remaining_seconds: 3,
    })
}

pub fn countdown_tick_phase(remaining_seconds: u8) -> NetworkRacePhase {
    NetworkRacePhase::Countdown { remaining_seconds }
}

pub fn countdown_should_cancel(race: &RaceState) -> bool {
    !has_connected_active_racer(race)
}

pub fn advance_countdown_to_racing(
    phase: NetworkRacePhase,
    has_connected_active_racer: bool,
) -> Result<NetworkRacePhase, CountdownAdvanceRejection> {
    if !matches!(phase, NetworkRacePhase::Countdown { .. }) {
        return Err(CountdownAdvanceRejection::NotCountingDown);
    }
    if !has_connected_active_racer {
        return Err(CountdownAdvanceRejection::NoConnectedRacers);
    }

    Ok(NetworkRacePhase::Racing)
}

pub fn return_to_lobby_decision(phase: NetworkRacePhase) -> ReturnToLobbyDecision {
    match phase {
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => {
            ReturnToLobbyDecision::CancelRace
        }
        NetworkRacePhase::Finished => ReturnToLobbyDecision::ReturnFromResults,
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => ReturnToLobbyDecision::Ignore,
    }
}

pub fn return_to_lobby_outcome(phase: NetworkRacePhase) -> Option<ReturnToLobbyOutcome> {
    match return_to_lobby_decision(phase) {
        ReturnToLobbyDecision::Ignore => None,
        ReturnToLobbyDecision::CancelRace => Some(ReturnToLobbyOutcome {
            decision: ReturnToLobbyDecision::CancelRace,
            event: "Race cancelled",
        }),
        ReturnToLobbyDecision::ReturnFromResults => Some(ReturnToLobbyOutcome {
            decision: ReturnToLobbyDecision::ReturnFromResults,
            event: "Returned to lobby",
        }),
    }
}

pub fn host_race_tick_outcome(
    phase_after_tick: NetworkRacePhase,
    race_changed: bool,
    bonus_choices_refreshed: usize,
) -> HostRaceTickOutcome {
    let action = match phase_after_tick {
        NetworkRacePhase::Finished => HostRaceTickAction::BroadcastResults,
        NetworkRacePhase::Racing if race_changed || bonus_choices_refreshed > 0 => {
            HostRaceTickAction::BroadcastDelta
        }
        NetworkRacePhase::Racing => HostRaceTickAction::Ignore,
        NetworkRacePhase::Lobby
        | NetworkRacePhase::WaitingForHost
        | NetworkRacePhase::Countdown { .. } => HostRaceTickAction::Ignore,
    };

    HostRaceTickOutcome {
        action,
        race_changed,
        bonus_choices_refreshed,
    }
}

pub fn advance_host_race_lifecycle(
    lifecycle: &mut RaceLifecycleState,
    race: &RaceState,
    phase: NetworkRacePhase,
    now: Instant,
    post_first_finish_timeout: Duration,
) -> HostRaceLifecycleOutcome {
    if phase != NetworkRacePhase::Racing {
        return HostRaceLifecycleOutcome {
            phase,
            flow: RaceFlowOutcome {
                newly_finished: Vec::new(),
                finished: None,
            },
        };
    }

    let flow = advance_race_flow(lifecycle, race, now, post_first_finish_timeout);
    let phase = if flow.finished.is_some() {
        NetworkRacePhase::Finished
    } else {
        NetworkRacePhase::Racing
    };

    HostRaceLifecycleOutcome { phase, flow }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use typekart_protocol::{
        AiDifficultySnapshot, AssignedColor, LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind,
    };

    use super::prepare_race_from_selected_lobby_players;
    use super::{
        CountdownAdvanceRejection, CountdownRacePreparation, CountdownStartRejection,
        HostRaceTickAction, ReturnToLobbyDecision, advance_countdown_to_racing,
        advance_host_race_lifecycle, begin_countdown_phase, connected_racer_count,
        countdown_should_cancel, countdown_start_plan, countdown_tick_phase,
        has_connected_active_racer, host_race_tick_outcome, prepare_race_from_lobby,
        return_to_lobby_decision, return_to_lobby_outcome,
    };
    use crate::game::{
        race::{RaceLifecycleState, RacePlayerId},
        track::{Track, WordList},
    };

    fn lobby_player(
        id: u64,
        name: &str,
        ready: bool,
        connected: bool,
        kind: PlayerKind,
    ) -> LobbyPlayer {
        LobbyPlayer {
            id: PlayerId(id),
            name: name.to_string(),
            color: AssignedColor::Cyan,
            ready,
            connected,
            kind,
            ai_difficulty: match kind {
                PlayerKind::Bot => Some(AiDifficultySnapshot::Easy),
                PlayerKind::Human => None,
            },
            ai_wpm: None,
        }
    }

    #[test]
    fn prepare_race_from_lobby_selects_ready_connected_players() {
        let now = Instant::now();
        let players = vec![
            lobby_player(1, "host", true, true, PlayerKind::Human),
            lobby_player(2, "waiting", false, true, PlayerKind::Human),
            lobby_player(3, "offline", true, false, PlayerKind::Human),
            lobby_player(4, "ai-1", true, true, PlayerKind::Bot),
        ];

        let prepared = prepare_race_from_lobby(
            &players,
            Track::new(vec!["go".to_string(), "fast".to_string()]),
            &WordList::from_static("go\nfast\nbonus"),
            now,
        );

        assert_eq!(prepared.participant_count(), 2);
        assert_eq!(
            prepared
                .race
                .players
                .iter()
                .map(|player| player.name.as_str())
                .collect::<Vec<_>>(),
            ["host", "ai-1"]
        );
        assert_eq!(prepared.bonuses.points.len(), 0);
    }

    #[test]
    fn connected_active_racer_excludes_finished_players() {
        let now = Instant::now();
        let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
        let mut prepared = prepare_race_from_lobby(
            &players,
            Track::new(vec!["go".to_string()]),
            &WordList::from_static("go\nbonus\nword"),
            now,
        );

        assert_eq!(connected_racer_count(&prepared.race), 1);
        assert!(has_connected_active_racer(&prepared.race));

        prepared.race.players[0].state.finished_at = Some(now);

        assert!(!has_connected_active_racer(&prepared.race));
    }

    #[test]
    fn prepare_race_from_selected_players_keeps_unready_selected_racers() {
        let now = Instant::now();
        let players = vec![
            lobby_player(1, "host", false, true, PlayerKind::Human),
            lobby_player(2, "guest", false, true, PlayerKind::Human),
        ];

        let prepared = prepare_race_from_selected_lobby_players(
            &players,
            Track::new(vec!["go".to_string()]),
            &WordList::from_static("go\nbonus\nword"),
            now,
        );

        assert_eq!(prepared.participant_count(), 2);
    }

    #[test]
    fn countdown_start_policy_identifies_when_race_preparation_is_needed() {
        assert_eq!(
            countdown_start_plan(NetworkRacePhase::Lobby),
            Ok(CountdownRacePreparation::PrepareRace)
        );
        assert_eq!(
            countdown_start_plan(NetworkRacePhase::Finished),
            Ok(CountdownRacePreparation::PrepareRace)
        );
        assert_eq!(
            countdown_start_plan(NetworkRacePhase::WaitingForHost),
            Ok(CountdownRacePreparation::UseCurrentRace)
        );
        assert_eq!(
            countdown_start_plan(NetworkRacePhase::Racing),
            Err(CountdownStartRejection::RaceAlreadyActive)
        );
    }

    #[test]
    fn countdown_begin_requires_connected_racers() {
        assert_eq!(
            begin_countdown_phase(0),
            Err(CountdownStartRejection::NoConnectedRacers)
        );
        assert_eq!(
            begin_countdown_phase(1),
            Ok(NetworkRacePhase::Countdown {
                remaining_seconds: 3
            })
        );
    }

    #[test]
    fn countdown_advance_policy_requires_countdown_and_active_racer() {
        assert_eq!(
            countdown_tick_phase(2),
            NetworkRacePhase::Countdown {
                remaining_seconds: 2
            }
        );
        assert_eq!(
            advance_countdown_to_racing(NetworkRacePhase::Racing, true),
            Err(CountdownAdvanceRejection::NotCountingDown)
        );
        assert_eq!(
            advance_countdown_to_racing(
                NetworkRacePhase::Countdown {
                    remaining_seconds: 1
                },
                false,
            ),
            Err(CountdownAdvanceRejection::NoConnectedRacers)
        );
        assert_eq!(
            advance_countdown_to_racing(
                NetworkRacePhase::Countdown {
                    remaining_seconds: 1
                },
                true,
            ),
            Ok(NetworkRacePhase::Racing)
        );
    }

    #[test]
    fn return_to_lobby_policy_distinguishes_cancel_and_results_return() {
        assert_eq!(
            return_to_lobby_decision(NetworkRacePhase::Racing),
            ReturnToLobbyDecision::CancelRace
        );
        assert_eq!(
            return_to_lobby_decision(NetworkRacePhase::Finished),
            ReturnToLobbyDecision::ReturnFromResults
        );
        assert_eq!(
            return_to_lobby_decision(NetworkRacePhase::Lobby),
            ReturnToLobbyDecision::Ignore
        );
        assert_eq!(
            return_to_lobby_outcome(NetworkRacePhase::Racing).map(|outcome| outcome.event),
            Some("Race cancelled")
        );
        assert_eq!(return_to_lobby_outcome(NetworkRacePhase::Lobby), None);
    }

    #[test]
    fn countdown_cancel_policy_uses_connected_active_racers() {
        let now = Instant::now();
        let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
        let mut prepared = prepare_race_from_lobby(
            &players,
            Track::new(vec!["go".to_string()]),
            &WordList::from_static("go\nbonus\nword"),
            now,
        );

        assert!(!countdown_should_cancel(&prepared.race));

        prepared.race.players[0].connected = false;

        assert!(countdown_should_cancel(&prepared.race));
    }

    #[test]
    fn race_tick_policy_selects_adapter_action() {
        assert_eq!(
            host_race_tick_outcome(NetworkRacePhase::Racing, false, 0).action,
            HostRaceTickAction::Ignore
        );
        assert_eq!(
            host_race_tick_outcome(NetworkRacePhase::Racing, true, 0).action,
            HostRaceTickAction::BroadcastDelta
        );
        assert_eq!(
            host_race_tick_outcome(NetworkRacePhase::Racing, false, 2).action,
            HostRaceTickAction::BroadcastDelta
        );
        assert_eq!(
            host_race_tick_outcome(NetworkRacePhase::Finished, false, 0).action,
            HostRaceTickAction::BroadcastResults
        );
    }

    #[test]
    fn host_race_lifecycle_advances_only_while_racing() {
        let now = Instant::now();
        let players = vec![lobby_player(1, "host", true, true, PlayerKind::Human)];
        let mut prepared = prepare_race_from_lobby(
            &players,
            Track::new(vec!["go".to_string()]),
            &WordList::from_static("go\nbonus\nword"),
            now,
        );
        prepared.race.players[0].state.finished_at = Some(now);
        let mut lifecycle = RaceLifecycleState::new();

        let waiting = advance_host_race_lifecycle(
            &mut lifecycle,
            &prepared.race,
            NetworkRacePhase::WaitingForHost,
            now,
            Duration::from_secs(30),
        );

        assert_eq!(waiting.phase, NetworkRacePhase::WaitingForHost);
        assert!(waiting.flow.finished.is_none());
        assert!(lifecycle.placements.is_empty());

        let racing = advance_host_race_lifecycle(
            &mut lifecycle,
            &prepared.race,
            NetworkRacePhase::Racing,
            now,
            Duration::from_secs(30),
        );

        assert_eq!(racing.phase, NetworkRacePhase::Finished);
        assert_eq!(racing.flow.newly_finished[0].player_id, RacePlayerId(1));
        assert!(racing.flow.finished.is_some());
    }
}
