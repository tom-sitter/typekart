//! Shared host-session helpers.
//!
//! A host session is the authoritative game loop regardless of whether the
//! adapter is local terminal play, LAN multiplayer, or a browser-hosted race.
//! This module contains browser-safe pieces of that authority and leaves
//! sockets, rendering, timers, and UI side effects to the adapters.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    time::{Duration, Instant},
};

use rand::Rng;
use typekart_protocol::{LobbyPlayer, NetworkRacePhase, PlayerId, RaceResultRow};

use super::{
    ai_driver::{AiDriverAdvance, AiDriverConfig, AiDriverState, advance_ai_driver},
    bonus::BonusState,
    bonus_flow::{
        BonusAttempt, BonusClaimRoll, BonusFlowEvent, BonusFlowState, apply_bonus_key,
        claim_random_available_bonus,
    },
    input_rules::player_input_is_paused_or_finished,
    item_effects::{ItemActivationReport, RaceItemEffectState, activate_item_pickup},
    items::{ItemPickup, ItemRegistry, ItemRollContext},
    lobby::{lobby_players_to_participants, ready_connected_participants},
    race::{RaceLifecycleState, RaceParticipant, RacePlayerId, RaceState},
    race_flow::{RaceFlowOutcome, advance_race_flow},
    snapshot::{build_placement_snapshots, build_race_result_snapshots},
    track::{Track, WordList},
    typing::{KeyAction, TypingEvent},
};

pub use super::host_events::HostEvent;

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
    pub finish_event: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRaceTickOutcome {
    pub lifecycle: HostRaceLifecycleOutcome,
    pub tick: HostRaceTickOutcome,
    pub expired_effect_players: Vec<RacePlayerId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPlayerKeyOutcome {
    pub handled: bool,
    pub typing_events: Vec<TypingEvent>,
    pub bonus_events: Vec<BonusFlowEvent>,
}

pub struct HostPlayerKeyState<'a, PlayerKey> {
    pub race: &'a mut RaceState,
    pub bonuses: &'a mut BonusState,
    pub bonus_attempts: &'a mut HashMap<PlayerKey, BonusAttempt>,
    pub spent_bonus_gaps: &'a mut HashMap<PlayerKey, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPlayerKeyInput<PlayerKey> {
    pub player_key: PlayerKey,
    pub race_player_id: RacePlayerId,
    pub action: KeyAction,
    pub now: Instant,
}

pub struct HostItemPickupState<'a> {
    pub race: &'a mut RaceState,
    pub effects: &'a mut HashMap<RacePlayerId, RaceItemEffectState>,
    pub ai_players: &'a HashSet<RacePlayerId>,
    pub item_registry: &'a ItemRegistry,
}

pub struct HostAiBonusClaimState<'a, PlayerKey> {
    pub race: &'a mut RaceState,
    pub bonuses: &'a mut BonusState,
    pub bonus_attempts: &'a mut HashMap<PlayerKey, BonusAttempt>,
    pub spent_bonus_gaps: &'a mut HashMap<PlayerKey, usize>,
    pub effects: &'a mut HashMap<RacePlayerId, RaceItemEffectState>,
    pub ai_players: &'a HashSet<RacePlayerId>,
    pub item_registry: &'a ItemRegistry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostItemPickupInput {
    pub player_id: RacePlayerId,
    pub item: ItemPickup,
    pub now: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostAiTickInput {
    pub player_id: RacePlayerId,
    pub config: AiDriverConfig,
    pub now: Instant,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAiBonusClaimInput<PlayerKey> {
    pub player_key: PlayerKey,
    pub player_id: RacePlayerId,
    pub player_name: String,
    pub item_context: ItemRollContext,
    pub now: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBonusClaimInput {
    pub player_id: RacePlayerId,
    pub player_name: String,
    pub pickup: Option<ItemPickup>,
    pub now: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostItemAftermath {
    pub interrupted_players: Vec<RacePlayerId>,
    pub reset_ai_players: Vec<RacePlayerId>,
    pub events: Vec<HostEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAftermathAction {
    ClearBonusAttempt(RacePlayerId),
    ResetAiDriver(RacePlayerId),
    EmitEvent(HostEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBonusClaimAftermath {
    pub pickup: Option<ItemPickup>,
    pub aftermath: HostItemAftermath,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostRaceResults {
    pub placements: Vec<PlayerId>,
    pub rows: Vec<RaceResultRow>,
    pub events: Vec<HostEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartRaceOutcome {
    pub phase: NetworkRacePhase,
    pub event: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountdownCancelOutcome {
    pub phase: NetworkRacePhase,
    pub event: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrepareWaitingRaceOutcome {
    pub phase: NetworkRacePhase,
    pub reset_runtime: bool,
    pub clear_results: bool,
    pub clear_events: bool,
    pub reset_ai_timing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartActiveRaceRuntimeOutcome {
    pub reset_runtime: bool,
    pub clear_ai_timing: bool,
    pub set_ai_timing_now: bool,
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

pub fn prepare_waiting_race_outcome() -> PrepareWaitingRaceOutcome {
    PrepareWaitingRaceOutcome {
        phase: NetworkRacePhase::WaitingForHost,
        reset_runtime: true,
        clear_results: true,
        clear_events: true,
        reset_ai_timing: true,
    }
}

pub fn start_active_race_runtime_outcome() -> StartActiveRaceRuntimeOutcome {
    StartActiveRaceRuntimeOutcome {
        reset_runtime: true,
        clear_ai_timing: true,
        set_ai_timing_now: true,
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

pub fn cancel_countdown_outcome() -> CountdownCancelOutcome {
    CountdownCancelOutcome {
        phase: NetworkRacePhase::WaitingForHost,
        event: "Countdown cancelled",
    }
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

pub fn start_race_from_countdown(
    phase: NetworkRacePhase,
    has_connected_active_racer: bool,
) -> Result<StartRaceOutcome, CountdownAdvanceRejection> {
    let phase = advance_countdown_to_racing(phase, has_connected_active_racer)?;
    Ok(StartRaceOutcome {
        phase,
        event: "Race started",
    })
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
            finish_event: None,
        };
    }

    let flow = advance_race_flow(lifecycle, race, now, post_first_finish_timeout);
    let (phase, finish_event) = if flow.finished.is_some() {
        (NetworkRacePhase::Finished, Some("Race finished"))
    } else {
        (NetworkRacePhase::Racing, None)
    };

    HostRaceLifecycleOutcome {
        phase,
        flow,
        finish_event,
    }
}

pub fn advance_active_race_tick(
    lifecycle: &mut RaceLifecycleState,
    race: &mut RaceState,
    bonuses: &mut BonusState,
    phase: NetworkRacePhase,
    now: Instant,
    post_first_finish_timeout: Duration,
    race_changed: bool,
) -> ActiveRaceTickOutcome {
    if phase != NetworkRacePhase::Racing {
        let lifecycle =
            advance_host_race_lifecycle(lifecycle, race, phase, now, post_first_finish_timeout);
        return ActiveRaceTickOutcome {
            tick: host_race_tick_outcome(lifecycle.phase, false, 0),
            lifecycle,
            expired_effect_players: Vec::new(),
        };
    }

    let bonus_choices_refreshed = bonuses.expire_cooldowns(&race.track, now);
    let expired_effect_players = race
        .players
        .iter_mut()
        .filter_map(|player| (player.state.expire_effects(now) > 0).then_some(player.id))
        .collect::<Vec<_>>();
    let lifecycle =
        advance_host_race_lifecycle(lifecycle, race, phase, now, post_first_finish_timeout);
    let tick = host_race_tick_outcome(
        lifecycle.phase,
        race_changed || !expired_effect_players.is_empty(),
        bonus_choices_refreshed,
    );

    ActiveRaceTickOutcome {
        lifecycle,
        tick,
        expired_effect_players,
    }
}

pub fn apply_host_player_key<PlayerKey, R>(
    state: &mut HostPlayerKeyState<'_, PlayerKey>,
    input: HostPlayerKeyInput<PlayerKey>,
    roll: BonusClaimRoll<'_, R>,
) -> HostPlayerKeyOutcome
where
    PlayerKey: Copy + Eq + Hash,
    R: Rng,
{
    let bonus_outcome = apply_bonus_key(
        &mut BonusFlowState {
            race: state.race,
            bonuses: state.bonuses,
            bonus_attempts: state.bonus_attempts,
            spent_bonus_gaps: state.spent_bonus_gaps,
        },
        input.player_key,
        input.race_player_id,
        input.action,
        input.now,
        roll,
    );
    if bonus_outcome.handled {
        return HostPlayerKeyOutcome {
            handled: true,
            typing_events: Vec::new(),
            bonus_events: bonus_outcome.events,
        };
    }

    let Some(typing_events) =
        state
            .race
            .apply_key_input(input.race_player_id, input.action, input.now)
    else {
        return HostPlayerKeyOutcome {
            handled: false,
            typing_events: Vec::new(),
            bonus_events: Vec::new(),
        };
    };

    HostPlayerKeyOutcome {
        handled: true,
        typing_events,
        bonus_events: Vec::new(),
    }
}

pub fn apply_host_item_pickup(
    state: &mut HostItemPickupState<'_>,
    input: HostItemPickupInput,
) -> ItemActivationReport {
    activate_item_pickup(
        state.race,
        state.effects,
        state.ai_players,
        state.item_registry,
        input.player_id,
        input.item,
        input.now,
    )
}

pub fn advance_host_ai_racer_tick(
    race: &mut RaceState,
    player_effects: &HashMap<RacePlayerId, RaceItemEffectState>,
    driver: &mut AiDriverState,
    input: HostAiTickInput,
) -> AiDriverAdvance {
    if player_input_is_paused_or_finished(race, player_effects, input.player_id, input.now) {
        return AiDriverAdvance {
            typed_actions: Vec::new(),
            typing_events: Vec::new(),
        };
    }

    advance_ai_driver(
        race,
        input.player_id,
        driver,
        input.config,
        input.now,
        input.elapsed,
    )
}

pub fn try_host_ai_bonus_claim<PlayerKey, R>(
    state: &mut HostAiBonusClaimState<'_, PlayerKey>,
    input: HostAiBonusClaimInput<PlayerKey>,
    rng: &mut R,
) -> Option<HostBonusClaimAftermath>
where
    PlayerKey: Copy + Eq + Hash,
    R: Rng,
{
    if player_input_is_paused_or_finished(state.race, state.effects, input.player_id, input.now)
        || state
            .effects
            .get(&input.player_id)
            .and_then(|effects| effects.item_cue.as_ref())
            .is_some_and(|cue| cue.until > input.now)
    {
        return None;
    }

    let outcome = claim_random_available_bonus(
        &mut BonusFlowState {
            race: state.race,
            bonuses: state.bonuses,
            bonus_attempts: state.bonus_attempts,
            spent_bonus_gaps: state.spent_bonus_gaps,
        },
        input.player_key,
        input.player_id,
        input.now,
        BonusClaimRoll {
            item_context: input.item_context,
            item_registry: state.item_registry,
            rng,
        },
    )?;

    Some(apply_host_bonus_claim(
        &mut HostItemPickupState {
            race: state.race,
            effects: state.effects,
            ai_players: state.ai_players,
            item_registry: state.item_registry,
        },
        HostBonusClaimInput {
            player_id: input.player_id,
            player_name: input.player_name,
            pickup: outcome.pickup,
            now: input.now,
        },
    ))
}

pub fn host_item_aftermath_actions(report: ItemActivationReport) -> HostItemAftermath {
    HostItemAftermath {
        interrupted_players: report.interrupted_players,
        reset_ai_players: report.reset_ai_players,
        events: report.events,
    }
}

pub fn host_aftermath_adapter_actions(aftermath: HostItemAftermath) -> Vec<HostAftermathAction> {
    aftermath
        .interrupted_players
        .into_iter()
        .map(HostAftermathAction::ClearBonusAttempt)
        .chain(
            aftermath
                .reset_ai_players
                .into_iter()
                .map(HostAftermathAction::ResetAiDriver),
        )
        .chain(
            aftermath
                .events
                .into_iter()
                .map(HostAftermathAction::EmitEvent),
        )
        .collect()
}

pub fn apply_host_bonus_claim(
    state: &mut HostItemPickupState<'_>,
    input: HostBonusClaimInput,
) -> HostBonusClaimAftermath {
    let mut aftermath = HostItemAftermath {
        interrupted_players: Vec::new(),
        reset_ai_players: Vec::new(),
        events: Vec::new(),
    };

    match input.pickup {
        Some(item) => {
            aftermath.events.push(HostEvent::ItemPickedUp {
                player_id: input.player_id,
                player_name: input.player_name,
                item,
            });
            let report = apply_host_item_pickup(
                state,
                HostItemPickupInput {
                    player_id: input.player_id,
                    item,
                    now: input.now,
                },
            );
            let item_aftermath = host_item_aftermath_actions(report);
            aftermath
                .interrupted_players
                .extend(item_aftermath.interrupted_players);
            aftermath
                .reset_ai_players
                .extend(item_aftermath.reset_ai_players);
            aftermath.events.extend(item_aftermath.events);
        }
        None => aftermath.events.push(HostEvent::BonusMissed {
            player_id: input.player_id,
            player_name: input.player_name,
        }),
    }

    HostBonusClaimAftermath {
        pickup: input.pickup,
        aftermath,
    }
}

pub fn finalize_host_race_results(
    race: &RaceState,
    placements: &[RacePlayerId],
    now: Instant,
) -> HostRaceResults {
    HostRaceResults {
        placements: build_placement_snapshots(placements),
        rows: build_race_result_snapshots(race, placements, now),
        events: vec![HostEvent::RaceFinished],
    }
}

#[cfg(test)]
mod tests;
