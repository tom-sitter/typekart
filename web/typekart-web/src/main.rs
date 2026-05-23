mod fixtures;

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use fixtures::{
    GalleryFrame, GalleryScenario, LobbyFrame, ResultsFrame, SCENARIOS, color_class,
    minimap_position, scenario_frame,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::{SinkExt, StreamExt, select};
use gloo_net::websocket::{Message, futures::WebSocket};
use gloo_timers::future::{IntervalStream, TimeoutFuture};
use leptos::prelude::*;
use rand::thread_rng;
use typekart::game::{
    bonus::{BonusChoiceStatus, BonusState, claim_bonus_choice},
    item_effects::{
        AttackDirection, RaceImpactCueKind, RaceItemCueKind, RaceItemCuePlacement,
        RaceItemEffectState, activate_item_pickup, advance_mushrooms,
        player_has_active_mushroom_effect, player_is_stunned,
    },
    items::{ItemPickup, ItemRegistry, ItemRollContext, RacePositionBand},
    player::PlayerState,
    race::{
        PlayerColorId, RaceLifecycleStatus, RaceParticipant, RacePlayer,
        RacePlayerId, RaceState, build_race_result_rows as build_shared_race_result_rows,
        RaceRuntimeState,
    },
    track::{Track, WordList},
    typing::{KeyAction, first_typo_index},
};
use typekart_protocol::{
    AiDifficultySnapshot, AssignedColor, AttackDirectionSnapshot, BonusChoiceSnapshotStatus,
    BonusPointSnapshot, ClientMessage, ClientSequence, ImpactCueSnapshot, ImpactCueSnapshotKind,
    ItemCuePlacementSnapshot, ItemCueSnapshot, ItemCueSnapshotKind, LobbyPlayer, ModConfigSnapshot,
    NetworkRacePhase, PlayerId, PlayerKind, PlayerSnapshot, ProtocolKey, RaceDeltaSnapshot,
    RaceResultStatus, RaceSnapshot, RelayClientMessage, RelayServerMessage, RoomCode,
    ServerMessage,
};
use wasm_bindgen_futures::spawn_local;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const WEB_TRACK_WORDS_BEHIND: usize = 3;
const WEB_TRACK_VISIBLE_WORDS: usize = 10;
const BROWSER_HOST_TRACK_WORD_COUNT: usize = 16;
const BROWSER_HOST_AI_TICK_MS: u32 = 250;
const BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
enum BrowserOutboundMessage {
    Client {
        player_id: PlayerId,
        message: ClientMessage,
    },
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BrowserBonusAttempt {
    point_index: usize,
    choice_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserSessionKind {
    Joiner,
    Host,
}

#[derive(Debug, Clone, Copy)]
struct BrowserSessionSignals {
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
    set_relay_player_id: WriteSignal<Option<PlayerId>>,
    set_game_player_id: WriteSignal<Option<PlayerId>>,
}

#[derive(Debug, Clone, Copy)]
struct BrowserHostSignals {
    session: BrowserSessionSignals,
    set_room_code: WriteSignal<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected { message: String },
    Closed { reason: String },
    Failed { message: String },
}

impl ConnectionState {
    fn is_active(&self) -> bool {
        matches!(self, Self::Connecting | Self::Connected { .. })
    }

    fn label(&self) -> String {
        match self {
            Self::Disconnected => "Not connected".to_string(),
            Self::Connecting => "Connecting...".to_string(),
            Self::Connected { message } => message.clone(),
            Self::Closed { reason } => format!("Room closed: {reason}"),
            Self::Failed { message } => message.clone(),
        }
    }
}

#[derive(Clone)]
struct BrowserCommandSink {
    outbound: Option<UnboundedSender<BrowserOutboundMessage>>,
    relay_player_id: Option<PlayerId>,
    set_connection: WriteSignal<ConnectionState>,
}

impl BrowserCommandSink {
    fn send(&self, message: ClientMessage, success_status: &'static str) {
        send_browser_client_message(
            self.outbound.clone(),
            self.relay_player_id,
            message,
            success_status,
            self.set_connection,
        );
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let (selected, set_selected) = signal(0usize);
    let (unicode_icons, set_unicode_icons) = signal(true);
    let (mode, set_mode) = signal(AppMode::Gallery);
    let current_scenario = move || SCENARIOS[selected.get()];

    view! {
        <main class="shell">
            <header class="app-header">
                <div>
                    <p class="eyebrow">"TypeKart Web"</p>
                    <h1>"Renderer gallery"</h1>
                </div>
                <div class="actions" aria-label="Game setup placeholders">
                    <button
                        type="button"
                        class:selected=move || mode.get() == AppMode::Gallery
                        on:click=move |_| set_mode.set(AppMode::Gallery)
                    >
                        "Gallery"
                    </button>
                    <button
                        type="button"
                        class:selected=move || mode.get() == AppMode::Join
                        on:click=move |_| set_mode.set(AppMode::Join)
                    >
                        "Join room"
                    </button>
                </div>
            </header>

            {move || match mode.get() {
                AppMode::Gallery => view! {
                    <GalleryControls
                        selected=selected
                        set_selected=set_selected
                        current_scenario=current_scenario
                        unicode_icons=unicode_icons
                        set_unicode_icons=set_unicode_icons
                    />
                    <GalleryFrameView
                        scenario=current_scenario
                        unicode_icons=unicode_icons
                    />
                }.into_any(),
                AppMode::Join => view! {
                    <JoinRoomPanel unicode_icons=unicode_icons />
                }.into_any(),
            }}
        </main>
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Gallery,
    Join,
}

#[component]
fn GalleryControls(
    selected: ReadSignal<usize>,
    set_selected: WriteSignal<usize>,
    current_scenario: impl Fn() -> GalleryScenario + Copy + Send + Sync + 'static,
    unicode_icons: ReadSignal<bool>,
    set_unicode_icons: WriteSignal<bool>,
) -> impl IntoView {
    view! {
        <section class="panel gallery-layout">
            <nav class="scenario-list" aria-label="Gallery scenarios">
                <For
                    each=|| SCENARIOS.iter().copied().enumerate()
                    key=|(_, scenario)| scenario.slug
                    children={move |(index, scenario)| {
                        view! {
                            <button
                                type="button"
                                class:selected=move || selected.get() == index
                                on:click=move |_| set_selected.set(index)
                            >
                                {scenario.title}
                            </button>
                        }
                    }}
                />
            </nav>

            <div class="scenario-copy">
                <h2>{move || current_scenario().title}</h2>
                <p>{move || current_scenario().description}</p>
                <label class="toggle">
                    <input
                        type="checkbox"
                        checked=unicode_icons.get_untracked()
                        on:change=move |event| {
                            set_unicode_icons.set(event_target_checked(&event));
                        }
                    />
                    <span>"Unicode icons"</span>
                </label>
                <p class="note">{move || current_scenario().icon_mode_note}</p>
            </div>
        </section>
    }
}

#[component]
fn JoinRoomPanel(unicode_icons: ReadSignal<bool>) -> impl IntoView {
    let (relay_url, set_relay_url) = signal("wss://typekart-relay.fly.dev".to_string());
    let (room_code, set_room_code) = signal(String::new());
    let (name, set_name) = signal("web-player".to_string());
    let (connection, set_connection) = signal(ConnectionState::Disconnected);
    let (live_frame, set_live_frame) = signal(None::<GalleryFrame>);
    let (relay_player_id, set_relay_player_id) = signal(None::<PlayerId>);
    let (game_player_id, set_game_player_id) = signal(None::<PlayerId>);
    let (outbound, set_outbound) = signal(None::<UnboundedSender<BrowserOutboundMessage>>);
    let (input_sequence, set_input_sequence) = signal(0u64);

    let start_session = Rc::new(move |session_kind: BrowserSessionKind| {
        if connection.get_untracked().is_active() {
            return;
        }
        let relay = relay_url.get_untracked();
        let room = room_code.get_untracked();
        let name = name.get_untracked();
        let (outbound_tx, outbound_rx) = unbounded();
        set_outbound.set(Some(outbound_tx));
        set_relay_player_id.set(None);
        set_game_player_id.set(None);
        set_input_sequence.set(0);
        set_connection.set(ConnectionState::Connecting);
        set_live_frame.set(None);

        spawn_local(async move {
            let session_signals = BrowserSessionSignals {
                set_connection,
                set_live_frame,
                set_relay_player_id,
                set_game_player_id,
            };
            let result = match session_kind {
                BrowserSessionKind::Joiner => {
                    join_browser_room(relay, room, name, outbound_rx, session_signals).await
                }
                BrowserSessionKind::Host => {
                    let host_signals = BrowserHostSignals {
                        session: session_signals,
                        set_room_code,
                    };
                    host_browser_lobby(relay, name, outbound_rx, host_signals).await
                }
            };

            match result {
                Ok(()) => {
                    if connection.get_untracked().is_active() {
                        set_connection.set(ConnectionState::Disconnected);
                        set_live_frame.set(None);
                    }
                    set_outbound.set(None);
                    set_relay_player_id.set(None);
                    set_game_player_id.set(None);
                }
                Err(error) => {
                    set_connection.set(ConnectionState::Failed { message: error });
                    set_outbound.set(None);
                    set_live_frame.set(None);
                    set_relay_player_id.set(None);
                    set_game_player_id.set(None);
                }
            }
        });
    });
    let join = {
        let start_session = Rc::clone(&start_session);
        move |_| start_session(BrowserSessionKind::Joiner)
    };
    let create_room = {
        let start_session = Rc::clone(&start_session);
        move |_| start_session(BrowserSessionKind::Host)
    };

    let disconnect = move |_| {
        if let Some(outbound) = outbound.get_untracked() {
            let _ = outbound.unbounded_send(BrowserOutboundMessage::Disconnect);
        }
        set_outbound.set(None);
        set_live_frame.set(None);
        set_relay_player_id.set(None);
        set_game_player_id.set(None);
        set_connection.set(ConnectionState::Disconnected);
    };

    let send_key = move |key| {
        let sequence = input_sequence.get_untracked() + 1;
        set_input_sequence.set(sequence);
        send_browser_client_message(
            outbound.get_untracked(),
            relay_player_id.get_untracked(),
            ClientMessage::KeyInput {
                sequence: ClientSequence(sequence),
                key,
            },
            "Key sent",
            set_connection,
        );
    };

    let global_key_handler = move |event: leptos::ev::KeyboardEvent| {
        if !should_capture_global_gameplay_key(
            live_frame.get_untracked().as_ref(),
            game_player_id.get_untracked(),
        ) || browser_text_entry_is_active()
        {
            return;
        }

        if let Some(key) = keyboard_event_to_protocol_key(&event) {
            event.prevent_default();
            send_key(key);
        }
    };
    let global_key_listener = window_event_listener(leptos::ev::keydown, global_key_handler);
    on_cleanup(move || global_key_listener.remove());

    view! {
        <section class="panel join-panel">
            <div class="join-grid">
                <label>
                    <span>"Relay"</span>
                    <input
                        type="text"
                        prop:value=move || relay_url.get()
                        disabled=move || connection.get().is_active()
                        on:input=move |event| set_relay_url.set(event_target_value(&event))
                    />
                </label>
                <label>
                    <span>"Room"</span>
                    <input
                        type="text"
                        placeholder="rocket-salad-tiger"
                        prop:value=move || room_code.get()
                        disabled=move || connection.get().is_active()
                        on:input=move |event| set_room_code.set(event_target_value(&event))
                    />
                </label>
                <label>
                    <span>"Name"</span>
                    <input
                        type="text"
                        prop:value=move || name.get()
                        disabled=move || connection.get().is_active()
                        on:input=move |event| set_name.set(event_target_value(&event))
                    />
                </label>
                <button
                    type="button"
                    disabled=move || connection.get().is_active()
                    on:click=join
                >
                    "Join"
                </button>
                <button
                    type="button"
                    class="secondary"
                    disabled=move || connection.get().is_active()
                    on:click=create_room
                >
                    "Create room"
                </button>
            </div>
            <p class={move || connection_note_class(&connection.get())}>
                {move || connection.get().label()}
            </p>
            <div class="browser-controls">
                <button
                    type="button"
                    class="secondary"
                    hidden=move || !connection.get().is_active()
                    on:click=disconnect
                >
                    "Disconnect"
                </button>
                <button
                    type="button"
                    hidden=move || {
                        let frame = live_frame.get();
                        !browser_controls(frame.as_ref(), game_player_id.get()).show_ready
                    }
                    on:click=move |_| {
                        send_browser_client_message(
                            outbound.get_untracked(),
                            relay_player_id.get_untracked(),
                            ClientMessage::SetReady { ready: true },
                            "Ready sent",
                            set_connection,
                        );
                    }
                >
                    "Ready"
                </button>
                <button
                    type="button"
                    class="secondary"
                    hidden=move || {
                        let frame = live_frame.get();
                        !browser_controls(frame.as_ref(), game_player_id.get()).show_unready
                    }
                    on:click=move |_| {
                        send_browser_client_message(
                            outbound.get_untracked(),
                            relay_player_id.get_untracked(),
                            ClientMessage::SetReady { ready: false },
                            "Unready sent",
                            set_connection,
                        );
                    }
                >
                    "Unready"
                </button>
                <button
                    type="button"
                    class="secondary"
                    hidden=move || {
                        let frame = live_frame.get();
                        !browser_controls(frame.as_ref(), game_player_id.get()).show_start
                    }
                    on:click=move |_| {
                        send_browser_client_message(
                            outbound.get_untracked(),
                            relay_player_id.get_untracked(),
                            ClientMessage::StartCountdown,
                            "Start sent",
                            set_connection,
                        );
                    }
                >
                    "Start"
                </button>
                <button
                    type="button"
                    class="secondary"
                    hidden=move || {
                        let frame = live_frame.get();
                        !browser_controls(frame.as_ref(), game_player_id.get()).show_rematch_ready
                    }
                    on:click=move |_| {
                        send_browser_client_message(
                            outbound.get_untracked(),
                            relay_player_id.get_untracked(),
                            ClientMessage::SetReady { ready: true },
                            "Ready for rematch sent",
                            set_connection,
                        );
                    }
                >
                    "Ready for rematch"
                </button>
            </div>
        </section>

        {move || live_frame.get().map(|frame| {
            match frame {
                GalleryFrame::Lobby(snapshot) => view! {
                    <LobbyPanel snapshot=snapshot.clone() local_player_id=game_player_id.get() />
                    <BrowserLobbyManagement
                        snapshot=snapshot
                        local_player_id=game_player_id.get()
                        relay_player_id=relay_player_id.get()
                        outbound=outbound.get()
                        set_connection=set_connection
                    />
                }.into_any(),
                GalleryFrame::Race(snapshot) => {
                    view! {
                        <RacePanel
                            snapshot=snapshot
                            local_player_id=game_player_id.get()
                            unicode_icons=unicode_icons
                            on_key=send_key
                        />
                    }.into_any()
                }
                GalleryFrame::Results(snapshot) => view! { <ResultsPanel snapshot=snapshot /> }.into_any(),
            }
        })}
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BrowserControls {
    show_ready: bool,
    show_unready: bool,
    show_start: bool,
    show_rematch_ready: bool,
}

fn browser_controls(
    frame: Option<&GalleryFrame>,
    local_player_id: Option<PlayerId>,
) -> BrowserControls {
    match frame {
        Some(GalleryFrame::Lobby(lobby)) => {
            let Some(local_player_id) = local_player_id else {
                return BrowserControls::default();
            };
            let Some(local_player) = lobby
                .players
                .iter()
                .find(|player| player.id == local_player_id)
            else {
                return BrowserControls::default();
            };
            BrowserControls {
                show_ready: !local_player.ready,
                show_unready: local_player.ready,
                show_start: lobby.host_id == local_player_id && local_player.ready,
                show_rematch_ready: false,
            }
        }
        Some(GalleryFrame::Results(_)) => BrowserControls {
            show_start: local_player_id == Some(PlayerId(1)),
            show_rematch_ready: local_player_id.is_some(),
            ..BrowserControls::default()
        },
        Some(GalleryFrame::Race(_)) | None => BrowserControls::default(),
    }
}

fn should_capture_global_gameplay_key(
    frame: Option<&GalleryFrame>,
    local_player_id: Option<PlayerId>,
) -> bool {
    let Some(local_player_id) = local_player_id else {
        return false;
    };
    let Some(GalleryFrame::Race(snapshot)) = frame else {
        return false;
    };
    snapshot.phase == NetworkRacePhase::Racing
        && snapshot
            .players
            .iter()
            .any(|player| player.id == local_player_id && player.connected && !player.finished)
}

fn browser_text_entry_is_active() -> bool {
    let Some(active_element) = document().active_element() else {
        return false;
    };
    let tag_name = active_element.tag_name();

    tag_name.eq_ignore_ascii_case("input")
        || tag_name.eq_ignore_ascii_case("textarea")
        || tag_name.eq_ignore_ascii_case("select")
        || active_element
            .get_attribute("contenteditable")
            .is_some_and(|value| !value.eq_ignore_ascii_case("false"))
}

fn connection_note_class(connection: &ConnectionState) -> &'static str {
    match connection {
        ConnectionState::Closed { .. } | ConnectionState::Failed { .. } => "note error",
        _ => "note",
    }
}

fn send_browser_client_message(
    outbound: Option<UnboundedSender<BrowserOutboundMessage>>,
    player_id: Option<PlayerId>,
    message: ClientMessage,
    success_status: &'static str,
    set_connection: WriteSignal<ConnectionState>,
) {
    let Some(outbound) = outbound else {
        set_connection.set(ConnectionState::Failed {
            message: "Not connected to a room".to_string(),
        });
        return;
    };
    let Some(player_id) = player_id else {
        set_connection.set(ConnectionState::Connected {
            message: "Waiting for player assignment".to_string(),
        });
        return;
    };
    if outbound
        .unbounded_send(BrowserOutboundMessage::Client { player_id, message })
        .is_err()
    {
        set_connection.set(ConnectionState::Failed {
            message: "Connection writer is closed".to_string(),
        });
    } else {
        set_connection.set(ConnectionState::Connected {
            message: success_status.to_string(),
        });
    }
}

fn keyboard_event_to_protocol_key(event: &leptos::ev::KeyboardEvent) -> Option<ProtocolKey> {
    key_name_to_protocol_key(&event.key())
}

fn key_name_to_protocol_key(key: &str) -> Option<ProtocolKey> {
    match key {
        "Backspace" => Some(ProtocolKey::Backspace),
        " " | "Spacebar" => Some(ProtocolKey::Space),
        key if key.chars().count() == 1 => key
            .chars()
            .next()
            .filter(|ch| ch.is_ascii_alphabetic())
            .map(|ch| ProtocolKey::Char(ch.to_ascii_lowercase())),
        _ => None,
    }
}

async fn join_browser_room(
    relay_url: String,
    room_code: String,
    name: String,
    outbound: UnboundedReceiver<BrowserOutboundMessage>,
    signals: BrowserSessionSignals,
) -> Result<(), String> {
    let room = RoomCode::parse(&room_code).map_err(|error| error.to_string())?;
    let websocket_url = relay_join_url(&relay_url, &room);
    let websocket = WebSocket::open(&websocket_url)
        .map_err(|error| format!("failed to open relay websocket: {error:?}"))?;
    let (mut writer, reader) = websocket.split();
    let join = RelayClientMessage::JoinRoom {
        room: room.clone(),
        name,
        client_version: APP_VERSION.to_string(),
    };
    let encoded_join = serde_json::to_string(&join)
        .map_err(|error| format!("failed to encode join request: {error}"))?;
    writer
        .send(Message::Text(encoded_join))
        .await
        .map_err(|error| format!("failed to send join request: {error:?}"))?;
    signals.set_connection.set(ConnectionState::Connected {
        message: format!("Connected to relay, joining {}", room.display()),
    });

    let mut current_race: Option<RaceSnapshot> = None;
    let mut reader = reader.fuse();
    let mut outbound = outbound.fuse();
    loop {
        select! {
            message = reader.next() => {
                let Some(message) = message else {
                    break;
                };
                let Message::Text(text) =
                    message.map_err(|error| format!("failed to read relay message: {error:?}"))?
                else {
                    continue;
                };
                let relay_message = serde_json::from_str::<RelayServerMessage>(&text)
                    .map_err(|error| format!("failed to decode relay message: {error}"))?;
                let keep_running = handle_relay_message(
                    relay_message,
                    &mut current_race,
                    signals.set_connection,
                    signals.set_live_frame,
                    signals.set_relay_player_id,
                    signals.set_game_player_id,
                )?;
                if !keep_running {
                    break;
                }
            },
            outbound_message = outbound.next() => {
                let Some(outbound_message) = outbound_message else {
                    continue;
                };
                let disconnecting = matches!(outbound_message, BrowserOutboundMessage::Disconnect);
                let relay_message = match outbound_message {
                    BrowserOutboundMessage::Client { player_id, message } => {
                        RelayClientMessage::ClientToHost {
                            room: room.clone(),
                            player_id,
                            message: serde_json::to_value(&message)
                                .map_err(|error| format!("failed to encode client message: {error}"))?,
                        }
                    }
                    BrowserOutboundMessage::Disconnect => RelayClientMessage::LeaveRoom {
                        room: room.clone(),
                    },
                };
                let encoded = serde_json::to_string(&relay_message)
                    .map_err(|error| format!("failed to encode relay message: {error}"))?;
                writer
                    .send(Message::Text(encoded))
                    .await
                    .map_err(|error| format!("failed to send client message: {error:?}"))?;
                if disconnecting {
                    let _ = writer.close().await;
                    break;
                }
            },
        }
    }

    Ok(())
}

async fn host_browser_lobby(
    relay_url: String,
    host_name: String,
    outbound: UnboundedReceiver<BrowserOutboundMessage>,
    signals: BrowserHostSignals,
) -> Result<(), String> {
    let websocket = WebSocket::open(&relay_url)
        .map_err(|error| format!("failed to open relay websocket: {error:?}"))?;
    let (mut writer, reader) = websocket.split();
    let create = RelayClientMessage::CreateRoom {
        host_version: APP_VERSION.to_string(),
    };
    writer
        .send(Message::Text(serde_json::to_string(&create).map_err(
            |error| format!("failed to encode room create request: {error}"),
        )?))
        .await
        .map_err(|error| format!("failed to send room create request: {error:?}"))?;

    signals.session.set_connection.set(ConnectionState::Connected {
        message: "Creating room...".to_string(),
    });

    let mut state: Option<BrowserHostLobby> = None;
    let mut reader = reader.fuse();
    let mut outbound = outbound.fuse();
    let mut ai_ticks = IntervalStream::new(BROWSER_HOST_AI_TICK_MS).fuse();
    loop {
        select! {
            message = reader.next() => {
                let Some(message) = message else {
                    break;
                };
                let Message::Text(text) =
                    message.map_err(|error| format!("failed to read relay message: {error:?}"))?
                else {
                    continue;
                };
                let relay_message = serde_json::from_str::<RelayServerMessage>(&text)
                    .map_err(|error| format!("failed to decode relay message: {error}"))?;
                let keep_running = handle_browser_host_relay_message(
                    relay_message,
                    &mut state,
                    &host_name,
                    &mut writer,
                    signals,
                )
                .await?;
                if !keep_running {
                    break;
                }
            },
            outbound_message = outbound.next() => {
                let Some(outbound_message) = outbound_message else {
                    continue;
                };
                if matches!(outbound_message, BrowserOutboundMessage::Disconnect) {
                    if let Some(state) = &state {
                        let leave = RelayClientMessage::LeaveRoom {
                            room: state.room.clone(),
                        };
                        writer
                            .send(Message::Text(
                                serde_json::to_string(&leave)
                                    .map_err(|error| format!("failed to encode leave message: {error}"))?,
                            ))
                            .await
                            .map_err(|error| format!("failed to send leave message: {error:?}"))?;
                    }
                    let _ = writer.close().await;
                    break;
                }

                let Some(state) = state.as_mut() else {
                    continue;
                };
                if let BrowserOutboundMessage::Client { player_id, message } = outbound_message {
                    handle_browser_host_client_message(
                        state,
                        player_id,
                        message,
                        &mut writer,
                        signals.session.set_connection,
                        signals.session.set_live_frame,
                    )
                    .await?;
                }
            },
            _ = ai_ticks.next() => {
                let Some(state) = state.as_mut() else {
                    continue;
                };
                if apply_browser_host_ai_tick(state, BROWSER_HOST_AI_TICK_MS) {
                    publish_browser_host_state(state, &mut writer, signals.session.set_live_frame).await?;
                }
            },
        }
    }

    Ok(())
}

struct BrowserHostLobby {
    room: RoomCode,
    players: Vec<LobbyPlayer>,
    relay_players: HashMap<PlayerId, PlayerId>,
    next_player_id: u64,
    race_sequence: u64,
    next_track_words: Vec<String>,
    bonuses: BonusState,
    item_registry: ItemRegistry,
    active_race: Option<RaceSnapshot>,
    active_results: Option<ResultsFrame>,
    core_race: Option<RaceState>,
    runtime: RaceRuntimeState<PlayerId, BrowserBonusAttempt>,
    ai_char_budget: HashMap<PlayerId, f64>,
    ai_last_tick_ms: Option<f64>,
    events: Vec<String>,
    mod_config: ModConfigSnapshot,
}

impl BrowserHostLobby {
    fn new(room: RoomCode, host_name: String) -> Self {
        let next_track_words = browser_generate_track_words();
        let bonuses = browser_generate_bonus_state(&next_track_words);
        Self {
            room,
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: browser_lobby_name_or_default(&host_name, "host"),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }],
            relay_players: HashMap::new(),
            next_player_id: 2,
            race_sequence: 0,
            next_track_words,
            bonuses,
            item_registry: ItemRegistry::builtin(),
            active_race: None,
            active_results: None,
            core_race: None,
            runtime: RaceRuntimeState::new(),
            ai_char_budget: HashMap::new(),
            ai_last_tick_ms: None,
            events: vec!["host created room".to_string()],
            mod_config: browser_default_mod_config(),
        }
    }

    fn game_player_id_for_relay(&self, relay_player_id: PlayerId) -> Option<PlayerId> {
        if relay_player_id == PlayerId(1) {
            return Some(PlayerId(1));
        }
        self.relay_players.get(&relay_player_id).copied()
    }

    fn frame(&self) -> LobbyFrame {
        LobbyFrame {
            host_id: PlayerId(1),
            players: self.players.clone(),
            mod_config: self.mod_config.clone(),
            events: self.events.clone(),
        }
    }

    fn push_event(&mut self, event: impl Into<String>) {
        self.events.push(event.into());
        const MAX_EVENTS: usize = 8;
        if self.events.len() > MAX_EVENTS {
            self.events.drain(0..self.events.len() - MAX_EVENTS);
        }
    }

    fn next_race_sequence(&mut self) -> u64 {
        self.race_sequence += 1;
        self.race_sequence
    }
}

async fn handle_browser_host_relay_message(
    relay_message: RelayServerMessage,
    state: &mut Option<BrowserHostLobby>,
    host_name: &str,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    signals: BrowserHostSignals,
) -> Result<bool, String> {
    match relay_message {
        RelayServerMessage::RoomCreated { room } => {
            signals.set_room_code.set(room.display());
            signals.session.set_relay_player_id.set(Some(PlayerId(1)));
            signals.session.set_game_player_id.set(Some(PlayerId(1)));
            signals.session.set_connection.set(ConnectionState::Connected {
                message: format!("Hosting room {}", room.display()),
            });
            let lobby = BrowserHostLobby::new(room, host_name.to_string());
            *state = Some(lobby);
            if let Some(state) = state {
                publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
            }
        }
        RelayServerMessage::JoinForwarded {
            pending_player_id,
            name,
            ..
        } => {
            let Some(state) = state else {
                return Ok(true);
            };
            let assigned = add_browser_lobby_human(state, pending_player_id, &name);
            let welcome = ServerMessage::Welcome {
                player_id: assigned.id,
                assigned_color: assigned.color,
            };
            send_browser_host_direct(state, writer, pending_player_id, welcome).await?;
            state.push_event(format!("{} joined", assigned.name));
            publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
        }
        RelayServerMessage::ClientToHost {
            player_id, message, ..
        } => {
            let Some(state) = state else {
                return Ok(true);
            };
            let Some(game_player_id) = state.game_player_id_for_relay(player_id) else {
                state.push_event(format!("unknown relay player {} ignored", player_id.0));
                publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
                return Ok(true);
            };
            let message = serde_json::from_value::<ClientMessage>(message)
                .map_err(|error| format!("failed to decode client message: {error}"))?;
            handle_browser_host_client_message(
                state,
                game_player_id,
                message,
                writer,
                signals.session.set_connection,
                signals.session.set_live_frame,
            )
            .await?;
        }
        RelayServerMessage::ParticipantDisconnected { player_id, .. } => {
            if let Some(state) = state {
                let Some(game_player_id) = state.relay_players.remove(&player_id) else {
                    return Ok(true);
                };
                if let Some(player) = state
                    .players
                    .iter_mut()
                    .find(|player| player.id == game_player_id)
                {
                    player.connected = false;
                    let name = player.name.clone();
                    state.push_event(format!("{name} disconnected"));
                    publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
                }
            }
        }
        RelayServerMessage::Error { message } => {
            signals.session.set_connection.set(ConnectionState::Failed { message });
            signals.session.set_live_frame.set(None);
            return Ok(false);
        }
        RelayServerMessage::RoomClosed { reason } => {
            signals.session.set_connection.set(ConnectionState::Closed { reason });
            signals.session.set_live_frame.set(None);
            return Ok(false);
        }
        RelayServerMessage::HostToClient { .. } | RelayServerMessage::HostBroadcast { .. } => {}
    }
    Ok(true)
}

async fn handle_browser_host_client_message(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    message: ClientMessage,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    if matches!(message, ClientMessage::StartCountdown) && player_id == PlayerId(1) {
        run_browser_host_countdown(state, writer, set_connection, set_live_frame).await?;
        return Ok(());
    }

    process_browser_host_client_message(state, player_id, message, set_connection);
    publish_browser_host_state(state, writer, set_live_frame).await
}

fn process_browser_host_client_message(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    message: ClientMessage,
    set_connection: WriteSignal<ConnectionState>,
) {
    match message {
        ClientMessage::Rename { name } => {
            let name = name.trim();
            if name.is_empty() {
                return;
            }
            if let Some(index) = state
                .players
                .iter()
                .position(|player| player.id == player_id)
            {
                let unique_name = browser_unique_lobby_name(
                    state
                        .players
                        .iter()
                        .filter(|candidate| candidate.id != player_id),
                    name,
                );
                state.players[index].name = unique_name;
                let renamed = state.players[index].name.clone();
                state.push_event(format!("{renamed} renamed"));
            }
        }
        ClientMessage::SetReady { ready } => {
            if let Some(player) = state
                .players
                .iter_mut()
                .find(|player| player.id == player_id)
            {
                player.ready = ready;
                let name = player.name.clone();
                state.push_event(format!(
                    "{name} {}",
                    if ready { "ready" } else { "not ready" }
                ));
            }
        }
        ClientMessage::AddAi if player_id == PlayerId(1) => {
            if state.players.len() >= 6 {
                state.push_event("lobby is full");
                return;
            }
            let id = browser_next_lobby_player_id(state);
            let ai_number = state
                .players
                .iter()
                .filter(|player| player.kind == PlayerKind::Bot)
                .count()
                + 1;
            let name = browser_unique_lobby_name(state.players.iter(), &format!("ai-{ai_number}"));
            state.players.push(LobbyPlayer {
                id,
                name: name.clone(),
                kind: PlayerKind::Bot,
                color: browser_color_for_slot(state.players.len()),
                ready: true,
                connected: true,
                ai_difficulty: Some(AiDifficultySnapshot::Easy),
                ai_wpm: Some(browser_ai_wpm(AiDifficultySnapshot::Easy)),
            });
            state.push_event(format!("{name} added"));
        }
        ClientMessage::RemoveLobbyPlayer { player_id: target } if player_id == PlayerId(1) => {
            if target == PlayerId(1) {
                return;
            }
            if let Some(index) = state.players.iter().position(|player| player.id == target) {
                let removed = state.players.remove(index);
                state.push_event(format!("{} removed", removed.name));
            }
        }
        ClientMessage::SetAiDifficulty {
            player_id: target,
            difficulty,
        } if player_id == PlayerId(1) => {
            let mut changed = 0usize;
            for player in &mut state.players {
                if player.kind != PlayerKind::Bot {
                    continue;
                }
                if target.is_some() && target != Some(player.id) {
                    continue;
                }
                player.ai_difficulty = Some(difficulty);
                player.ai_wpm = Some(browser_ai_wpm(difficulty));
                changed += 1;
            }
            if changed > 0 {
                state.push_event(format!("updated {changed} AI racer difficulty"));
            }
        }
        ClientMessage::StartCountdown if player_id == PlayerId(1) => {}
        ClientMessage::Leave => {
            if let Some(player) = state
                .players
                .iter_mut()
                .find(|player| player.id == player_id)
            {
                player.connected = false;
                let name = player.name.clone();
                state.push_event(format!("{name} left"));
            }
        }
        ClientMessage::KeyInput { key, .. } => {
            apply_browser_host_race_key_input(state, player_id, key, set_connection);
        }
        ClientMessage::RestartRace if player_id == PlayerId(1) => {
            browser_return_host_to_lobby(state);
        }
        _ => {}
    }
}

fn browser_return_host_to_lobby(state: &mut BrowserHostLobby) {
    state.active_race = None;
    state.active_results = None;
    state.core_race = None;
    state.runtime.reset();
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = None;
    state.push_event("returned to lobby");
}

async fn publish_browser_host_state(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    if let Some(results) = &state.active_results {
        set_live_frame.set(Some(GalleryFrame::Results(results.clone())));
        return send_browser_host_broadcast(
            state,
            writer,
            ServerMessage::RaceResults {
                placements: results.placements.clone(),
                rows: results.rows.clone(),
            },
        )
        .await;
    }

    if let Some(snapshot) = &state.active_race {
        set_live_frame.set(Some(GalleryFrame::Race(snapshot.clone())));
        return send_browser_host_broadcast(
            state,
            writer,
            ServerMessage::RaceSnapshot(snapshot.clone()),
        )
        .await;
    }

    publish_browser_host_lobby(state, writer, set_live_frame).await
}

async fn publish_browser_host_lobby(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    let frame = state.frame();
    set_live_frame.set(Some(GalleryFrame::Lobby(frame.clone())));
    let message = ServerMessage::LobbySnapshot {
        players: frame.players,
        host_id: frame.host_id,
        mod_config: frame.mod_config,
        events: frame.events,
    };
    send_browser_host_broadcast(state, writer, message).await
}

async fn run_browser_host_countdown(
    state: &mut BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    if state.active_race.is_some() {
        state.push_event("race already started");
        publish_browser_host_state(state, writer, set_live_frame).await?;
        return Ok(());
    }

    let racers: Vec<LobbyPlayer> = state
        .players
        .iter()
        .filter(|player| player.connected && player.ready)
        .cloned()
        .collect();
    if racers.is_empty() {
        state.push_event("cannot start without ready racers");
        publish_browser_host_lobby(state, writer, set_live_frame).await?;
        return Ok(());
    }

    set_connection.set(ConnectionState::Connected {
        message: "Starting browser-hosted race shell".to_string(),
    });
    let race_track_words = state.next_track_words.clone();
    state.bonuses = browser_generate_bonus_state(&race_track_words);

    state.active_race = Some(browser_host_race_snapshot_with_track(
        state.next_race_sequence(),
        NetworkRacePhase::WaitingForHost,
        &state.mod_config,
        &racers,
        &race_track_words,
        &state.bonuses,
        vec!["browser host preparing race".to_string()],
    ));
    state.active_results = None;
    state.core_race = None;
    state.runtime.reset();
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = None;
    publish_browser_host_state(state, writer, set_live_frame).await?;

    for remaining_seconds in [3, 2, 1] {
        state.active_race = Some(browser_host_race_snapshot_with_track(
            state.next_race_sequence(),
            NetworkRacePhase::Countdown { remaining_seconds },
            &state.mod_config,
            &racers,
            &race_track_words,
            &state.bonuses,
            vec![format!("countdown {remaining_seconds}")],
        ));
        publish_browser_host_state(state, writer, set_live_frame).await?;
        TimeoutFuture::new(1000).await;
    }

    state.active_race = Some(browser_host_race_snapshot_with_track(
        state.next_race_sequence(),
        NetworkRacePhase::Racing,
        &state.mod_config,
        &racers,
        &race_track_words,
        &state.bonuses,
        vec!["browser-hosted race started".to_string()],
    ));
    state.active_results = None;
    state.core_race = Some(browser_host_core_race(&racers, race_track_words));
    state.next_track_words = browser_generate_track_words();
    state.runtime.reset();
    if let (Some(snapshot), Some(core_race)) = (&mut state.active_race, &state.core_race) {
        browser_sync_snapshot_from_core(snapshot, core_race, &state.players, &state.runtime.player_effects);
    }
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = Some(browser_now_ms());
    publish_browser_host_state(state, writer, set_live_frame).await
}

#[cfg(test)]
fn browser_host_race_snapshot(
    sequence: u64,
    phase: NetworkRacePhase,
    mod_config: &ModConfigSnapshot,
    racers: &[LobbyPlayer],
    events: Vec<String>,
) -> RaceSnapshot {
    browser_host_race_snapshot_with_track(
        sequence,
        phase,
        mod_config,
        racers,
        &browser_demo_track_words(),
        &browser_generate_bonus_state(&browser_demo_track_words()),
        events,
    )
}

fn browser_host_race_snapshot_with_track(
    sequence: u64,
    phase: NetworkRacePhase,
    mod_config: &ModConfigSnapshot,
    racers: &[LobbyPlayer],
    track_words: &[String],
    bonuses: &BonusState,
    events: Vec<String>,
) -> RaceSnapshot {
    RaceSnapshot {
        sequence,
        phase,
        mod_config: mod_config.clone(),
        track_words: track_words.to_vec(),
        bonuses: browser_bonus_snapshots(bonuses, Instant::now()),
        players: racers.iter().map(browser_host_player_snapshot).collect(),
        events,
    }
}

fn browser_host_player_snapshot(player: &LobbyPlayer) -> PlayerSnapshot {
    PlayerSnapshot {
        id: player.id,
        name: player.name.clone(),
        kind: player.kind,
        color: player.color,
        word_index: 0,
        input: String::new(),
        typo_index: None,
        word_overrides: Vec::new(),
        finished: false,
        connected: player.connected,
        shielded: false,
        focused: false,
        inked: false,
        boosted: false,
        stunned: false,
        impact_remaining_ms: 0,
        impact_cue: None,
        item_cue: None,
    }
}

fn browser_host_core_race(racers: &[LobbyPlayer], track_words: Vec<String>) -> RaceState {
    let now = Instant::now();
    let participants = racers.iter().map(|racer| RaceParticipant {
        id: RacePlayerId(racer.id.0),
        name: racer.name.clone(),
        color: browser_player_color_id(racer.color),
        connected: racer.connected,
    });
    RaceState::from_participants(Track::new(track_words), participants, now)
}

fn browser_ensure_core_race(state: &mut BrowserHostLobby) {
    if state.core_race.is_some() {
        return;
    }
    let Some(snapshot) = &state.active_race else {
        return;
    };
    state.core_race = Some(browser_core_race_from_snapshot(snapshot));
}

fn browser_core_race_from_snapshot(snapshot: &RaceSnapshot) -> RaceState {
    let now = Instant::now();
    RaceState {
        track: Track::new(snapshot.track_words.clone()),
        players: snapshot
            .players
            .iter()
            .map(|player| {
                let mut state = PlayerState::new(now);
                state.word_index = player.word_index;
                state.input = player.input.clone();
                state.typo_index = player.typo_index;
                state.word_overrides = player
                    .word_overrides
                    .iter()
                    .map(|override_word| (override_word.word_index, override_word.word.clone()))
                    .collect();
                if player.finished {
                    state.finished_at = Some(now);
                }

                RacePlayer {
                    id: RacePlayerId(player.id.0),
                    name: player.name.clone(),
                    color: browser_player_color_id(player.color),
                    state,
                    connected: player.connected,
                }
            })
            .collect(),
    }
}

fn apply_browser_host_race_key_input(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    key: ProtocolKey,
    set_connection: WriteSignal<ConnectionState>,
) {
    browser_ensure_core_race(state);
    let Some(race) = &state.active_race else {
        return;
    };
    if race.phase != NetworkRacePhase::Racing {
        return;
    }
    let Some(player_name) = race
        .players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.name.clone())
    else {
        return;
    };
    if race
        .players
        .iter()
        .find(|player| player.id == player_id)
        .is_some_and(|player| player.finished)
    {
        return;
    }
    if browser_player_input_is_paused(state, player_id, Instant::now()) {
        return;
    }

    let action = browser_protocol_key_to_action(key);
    if state.runtime.bonus_attempts.contains_key(&player_id) {
        let previous_event_count = state.events.len();
        apply_browser_bonus_typing_action(state, player_id, action);
        browser_sync_active_race_from_core(state);
        set_browser_race_input_event(state, &player_name, previous_event_count);
        set_connection.set(ConnectionState::Connected {
            message: format!("{player_name} typed"),
        });
        return;
    }

    if let KeyAction::Char(ch) = action
        && let Some(attempt) = browser_bonus_start(state, player_id, ch, Instant::now())
    {
        let previous_event_count = state.events.len();
        state.runtime.bonus_attempts.insert(player_id, attempt);
        apply_browser_bonus_char(state, player_id, ch);
        browser_sync_active_race_from_core(state);
        set_browser_race_input_event(state, &player_name, previous_event_count);
        set_connection.set(ConnectionState::Connected {
            message: format!("{player_name} typed"),
        });
        return;
    }

    {
        let Some(core_race) = &mut state.core_race else {
            return;
        };
        core_race.apply_key_input(RacePlayerId(player_id.0), action, Instant::now());
        state.runtime
        .lifecycle
            .update(core_race, Instant::now(), BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT);
    }
    browser_sync_active_race_from_core(state);
    if let Some(race) = &mut state.active_race {
        race.events = vec![format!("{player_name} typed")];
    }
    browser_update_race_status(state, Instant::now());
    set_connection.set(ConnectionState::Connected {
        message: format!("{player_name} typed"),
    });
}

fn browser_player_input_is_paused(
    state: &BrowserHostLobby,
    player_id: PlayerId,
    now: Instant,
) -> bool {
    if player_is_stunned(&state.runtime.player_effects, RacePlayerId(player_id.0), now) {
        return true;
    }

    state
        .core_race
        .as_ref()
        .and_then(|race| race.player(RacePlayerId(player_id.0)))
        .is_some_and(|player| player_has_active_mushroom_effect(player, now))
}

fn set_browser_race_input_event(
    state: &mut BrowserHostLobby,
    player_name: &str,
    previous_event_count: usize,
) {
    let event = state
        .events
        .get(previous_event_count..)
        .and_then(|events| events.last())
        .cloned()
        .unwrap_or_else(|| format!("{player_name} typed"));
    if let Some(race) = &mut state.active_race {
        race.events = vec![event];
    }
}

fn browser_sync_active_race_from_core(state: &mut BrowserHostLobby) {
    let (Some(snapshot), Some(core_race)) = (&mut state.active_race, &state.core_race) else {
        return;
    };
    browser_sync_snapshot_from_core(snapshot, core_race, &state.players, &state.runtime.player_effects);
    snapshot.bonuses = browser_bonus_snapshots(&state.bonuses, Instant::now());
    state.race_sequence += 1;
    snapshot.sequence = state.race_sequence;
}

fn apply_browser_bonus_typing_action(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    action: KeyAction,
) {
    match action {
        KeyAction::Char(ch) => apply_browser_bonus_char(state, player_id, ch),
        KeyAction::Backspace => {
            let Some(player) = state
                .core_race
                .as_mut()
                .and_then(|race| {
                    race.players
                        .iter_mut()
                        .find(|player| player.id == RacePlayerId(player_id.0))
                })
            else {
                state.runtime.bonus_attempts.remove(&player_id);
                return;
            };

            if player.state.input.pop().is_some() {
                player.state.stats.backspaces += 1;
            }
            let input = player.state.input.clone();
            let input_is_empty = input.is_empty();
            recalculate_browser_bonus_typo(state, player_id, &input);
            if input_is_empty {
                state.runtime.bonus_attempts.remove(&player_id);
            }
        }
        KeyAction::Space => {
            if browser_bonus_completed_without_typo(state, player_id) {
                claim_browser_bonus(state, player_id, Instant::now());
            } else {
                apply_browser_bonus_char(state, player_id, ' ');
            }
        }
    }
}

fn browser_bonus_start(
    state: &BrowserHostLobby,
    player_id: PlayerId,
    ch: char,
    now: Instant,
) -> Option<BrowserBonusAttempt> {
    let player = state.core_race.as_ref()?.player(RacePlayerId(player_id.0))?;
    if player.state.held_item.is_some()
        || player.state.has_active_shield(now)
        || player.state.has_active_focus(now)
        || player.state.typo_index.is_some()
        || !player.state.input.is_empty()
        || player.state.is_finished()
    {
        return None;
    }

    let (point_index, point) = state.bonuses.point_for_gap(player.state.word_index)?;
    if state.runtime
        .spent_bonus_gaps
        .get(&player_id)
        .is_some_and(|after_word_index| *after_word_index == point.after_word_index)
    {
        return None;
    }

    point
        .available_choice_starting_with(ch, now)
        .map(|(choice_index, _)| BrowserBonusAttempt {
            point_index,
            choice_index,
        })
}

fn apply_browser_bonus_char(state: &mut BrowserHostLobby, player_id: PlayerId, ch: char) {
    let Some(attempt) = state.runtime.bonus_attempts.get(&player_id).copied() else {
        return;
    };
    let Some(target) = browser_bonus_target(state, attempt).map(str::to_owned) else {
        state.runtime.bonus_attempts.remove(&player_id);
        return;
    };
    let Some(player) = state.core_race.as_mut().and_then(|race| {
        race.players
            .iter_mut()
            .find(|player| player.id == RacePlayerId(player_id.0))
    })
    else {
        state.runtime.bonus_attempts.remove(&player_id);
        return;
    };

    let previous_typo = player.state.typo_index;
    let input_index = player.state.input.chars().count();
    let is_correct = previous_typo.is_none() && target.chars().nth(input_index) == Some(ch);

    player.state.stats.typed_chars += 1;
    if is_correct {
        player.state.stats.correct_chars += 1;
    } else {
        player.state.stats.typo_chars += 1;
    }

    player.state.input.push(ch);
    player.state.typo_index = first_typo_index(&player.state.input, &target);
}

fn recalculate_browser_bonus_typo(state: &mut BrowserHostLobby, player_id: PlayerId, input: &str) {
    let Some(attempt) = state.runtime.bonus_attempts.get(&player_id).copied() else {
        return;
    };
    let target = browser_bonus_target(state, attempt).map(str::to_owned);
    let Some(player) = state.core_race.as_mut().and_then(|race| {
        race.players
            .iter_mut()
            .find(|player| player.id == RacePlayerId(player_id.0))
    })
    else {
        state.runtime.bonus_attempts.remove(&player_id);
        return;
    };

    player.state.typo_index = target
        .as_deref()
        .and_then(|target| first_typo_index(input, target));
}

fn browser_bonus_completed_without_typo(state: &BrowserHostLobby, player_id: PlayerId) -> bool {
    let Some(attempt) = state.runtime.bonus_attempts.get(&player_id).copied() else {
        return false;
    };
    let Some(target) = browser_bonus_target(state, attempt) else {
        return false;
    };
    let Some(player) = state
        .core_race
        .as_ref()
        .and_then(|race| race.player(RacePlayerId(player_id.0)))
    else {
        return false;
    };

    player.state.typo_index.is_none() && player.state.input == target
}

fn claim_browser_bonus(state: &mut BrowserHostLobby, player_id: PlayerId, now: Instant) {
    let Some(attempt) = state.runtime.bonus_attempts.remove(&player_id) else {
        return;
    };

    let after_word_index = state
        .bonuses
        .points
        .get(attempt.point_index)
        .map(|point| point.after_word_index);
    let item_context = browser_item_roll_context(state, player_id, 5);
    let item_registry = state.item_registry.clone();
    let mut rng = thread_rng();
    let pickup = claim_bonus_choice(
        &mut state.bonuses,
        attempt.point_index,
        attempt.choice_index,
        now,
        item_context,
        &item_registry,
        &mut rng,
    );

    if let Some(player) = state.core_race.as_mut().and_then(|race| {
        race.players
            .iter_mut()
            .find(|player| player.id == RacePlayerId(player_id.0))
    })
    {
        player.state.input.clear();
        player.state.typo_index = None;
    }

    if let Some(after_word_index) = after_word_index {
        state.runtime.spent_bonus_gaps.insert(player_id, after_word_index);
    }

    let name = state
        .players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.name.clone())
        .unwrap_or_else(|| format!("player {}", player_id.0));
    match pickup {
        Some(item) => {
            let item_name = browser_item_pickup_name(item);
            state.push_event(format!("{name} got {item_name}"));
            activate_browser_item_pickup(state, player_id, item, now);
        }
        None => state.push_event(format!("{name} missed the bonus")),
    }
}

fn activate_browser_item_pickup(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    item: ItemPickup,
    now: Instant,
) {
    let Some(core_race) = &mut state.core_race else {
        return;
    };
    let ai_players = state
        .players
        .iter()
        .filter(|player| player.kind == PlayerKind::Bot)
        .map(|player| RacePlayerId(player.id.0))
        .collect::<HashSet<_>>();
    let report = activate_item_pickup(
        core_race,
        &mut state.runtime.player_effects,
        &ai_players,
        &state.item_registry,
        RacePlayerId(player_id.0),
        item,
        now,
    );

    for interrupted in report.interrupted_players {
        state.runtime.bonus_attempts.remove(&PlayerId(interrupted.0));
    }
    for ai_id in report.reset_ai_players {
        state.ai_char_budget.insert(PlayerId(ai_id.0), 0.0);
    }
    for event in report.events {
        state.push_event(event);
    }
}

fn browser_item_roll_context(
    state: &BrowserHostLobby,
    player_id: PlayerId,
    max_distance_words: usize,
) -> ItemRollContext {
    ItemRollContext {
        has_nearby_racer: browser_player_has_nearby_racer(state, player_id, max_distance_words),
        position: browser_position_band(state, player_id),
    }
}

fn browser_player_has_nearby_racer(
    state: &BrowserHostLobby,
    player_id: PlayerId,
    max_distance_words: usize,
) -> bool {
    let Some(core_race) = &state.core_race else {
        return false;
    };
    let Some(player) = core_race.player(RacePlayerId(player_id.0)) else {
        return false;
    };

    core_race.players.iter().any(|other| {
        other.id != player.id
            && other.connected
            && !other.state.is_finished()
            && player.state.word_index.abs_diff(other.state.word_index) <= max_distance_words
    })
}

fn browser_position_band(state: &BrowserHostLobby, player_id: PlayerId) -> RacePositionBand {
    let Some(core_race) = &state.core_race else {
        return RacePositionBand::Middle;
    };
    let active_racers = core_race
        .players
        .iter()
        .filter(|player| player.connected && !player.state.is_finished())
        .collect::<Vec<_>>();
    if active_racers.len() <= 1 {
        return RacePositionBand::Middle;
    }

    let Some(player) = active_racers
        .iter()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return RacePositionBand::Middle;
    };
    let ahead = active_racers
        .iter()
        .filter(|other| other.state.word_index > player.state.word_index)
        .count();
    let behind = active_racers
        .iter()
        .filter(|other| other.state.word_index < player.state.word_index)
        .count();

    if ahead == 0 && behind > 0 {
        RacePositionBand::First
    } else if behind == 0 && ahead > 0 {
        RacePositionBand::Trailing
    } else {
        RacePositionBand::Middle
    }
}

fn browser_bonus_target(state: &BrowserHostLobby, attempt: BrowserBonusAttempt) -> Option<&str> {
    state
        .bonuses
        .points
        .get(attempt.point_index)?
        .choices
        .get(attempt.choice_index)
        .map(|choice| choice.word.as_str())
}

fn browser_item_pickup_name(item: ItemPickup) -> &'static str {
    match item {
        ItemPickup::Held(held_item) => held_item.name(),
        ItemPickup::Shield => "Shield",
    }
}

fn apply_browser_host_ai_tick(state: &mut BrowserHostLobby, tick_ms: u32) -> bool {
    let elapsed_ms = browser_host_ai_elapsed_ms(state, tick_ms);
    if elapsed_ms <= 0.0 {
        return false;
    }
    browser_ensure_core_race(state);
    let mut changed = false;
    if let Some(core_race) = &mut state.core_race {
        for interrupted in advance_mushrooms(core_race, Instant::now()) {
            state.runtime.bonus_attempts.remove(&PlayerId(interrupted.0));
            changed = true;
        }
    }

    let ai_wpm_by_id: HashMap<PlayerId, u32> = state
        .players
        .iter()
        .filter(|player| player.kind == PlayerKind::Bot)
        .map(|player| {
            (
                player.id,
                player
                    .ai_wpm
                    .unwrap_or_else(|| browser_ai_wpm(AiDifficultySnapshot::Easy)),
            )
        })
        .collect();

    {
        let Some(snapshot) = &mut state.active_race else {
            return false;
        };
        if snapshot.phase != NetworkRacePhase::Racing {
            return false;
        }
        let Some(core_race) = &mut state.core_race else {
            return false;
        };
        for player in &mut core_race.players {
            player.state.expire_effects(Instant::now());
        }

        for player in core_race
            .players
            .iter_mut()
            .filter(|player| {
                state
                    .players
                    .iter()
                    .find(|lobby_player| lobby_player.id == PlayerId(player.id.0))
                    .is_some_and(|lobby_player| lobby_player.kind == PlayerKind::Bot)
            })
        {
            if player.state.is_finished()
                || player_is_stunned(&state.runtime.player_effects, player.id, Instant::now())
                || player_has_active_mushroom_effect(player, Instant::now())
            {
                continue;
            }
            let base_wpm = ai_wpm_by_id
                .get(&PlayerId(player.id.0))
                .copied()
                .unwrap_or_else(|| browser_ai_wpm(AiDifficultySnapshot::Easy));
            let wpm = browser_effective_ai_wpm(base_wpm, player, &state.item_registry);
            let budget = state.ai_char_budget.entry(PlayerId(player.id.0)).or_default();
            *budget += browser_ai_chars_for_elapsed_ms(wpm, elapsed_ms);

            while *budget >= 1.0 && !player.state.is_finished() {
                *budget -= 1.0;
                changed |= advance_browser_host_ai_char(player, &core_race.track);
            }
        }

        if changed {
            state.runtime.lifecycle.update(
                core_race,
                Instant::now(),
                BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT,
            );
            browser_sync_snapshot_from_core(
                snapshot,
                core_race,
                &state.players,
                &state.runtime.player_effects,
            );
            state.race_sequence += 1;
            snapshot.sequence = state.race_sequence;
            snapshot.events = vec!["AI racers advanced".to_string()];
        }
    }

    let race_status_changed = browser_update_race_status(state, Instant::now());
    changed || race_status_changed
}

fn browser_update_race_status(state: &mut BrowserHostLobby, now: Instant) -> bool {
    if state.active_results.is_some() {
        return false;
    }
    let Some(core_race) = &state.core_race else {
        return false;
    };

    let update = state.runtime
        .lifecycle
        .update(core_race, now, BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT);

    if !matches!(update.status, RaceLifecycleStatus::Finished { .. }) {
        return false;
    }

    browser_finish_race(state);
    true
}

fn browser_finish_race(state: &mut BrowserHostLobby) {
    let Some(core_race) = &state.core_race else {
        return;
    };
    let rows = build_shared_race_result_rows(core_race, &state.runtime.lifecycle.placements, Instant::now())
        .into_iter()
        .map(|row| typekart_protocol::RaceResultRow {
            placement: row.placement,
            player_id: PlayerId(row.player_id.0),
            name: row.name,
            color: browser_assigned_color(row.color),
            status: match row.status {
                typekart::game::race::RaceResultStatus::Finished => RaceResultStatus::Finished,
                typekart::game::race::RaceResultStatus::TimedOut => RaceResultStatus::TimedOut,
                typekart::game::race::RaceResultStatus::Disconnected => {
                    RaceResultStatus::Disconnected
                }
            },
            progress_words: row.progress_words,
            track_words: row.track_words,
            wpm: row.wpm,
            accuracy_percent: row.accuracy_percent,
            typo_chars: row.typo_chars,
            backspaces: row.backspaces,
        })
        .collect();
    if let Some(snapshot) = &mut state.active_race {
        snapshot.phase = NetworkRacePhase::Finished;
        snapshot.events = vec!["Race finished".to_string()];
    }
    state.active_results = Some(ResultsFrame {
        placements: browser_protocol_placements(&state.runtime.lifecycle.placements),
        rows,
        events: vec!["Race finished".to_string()],
    });
    state.active_race = None;
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = None;
}

fn browser_protocol_placements(placements: &[RacePlayerId]) -> Vec<PlayerId> {
    placements
        .iter()
        .map(|player_id| PlayerId(player_id.0))
        .collect()
}

fn browser_host_ai_elapsed_ms(state: &mut BrowserHostLobby, tick_ms: u32) -> f64 {
    let Some(last_tick_ms) = state.ai_last_tick_ms else {
        return f64::from(tick_ms);
    };

    let now_ms = browser_now_ms();
    let elapsed_ms = now_ms - last_tick_ms;
    let minimum_real_tick_ms = f64::from(tick_ms) * 0.5;
    if elapsed_ms < minimum_real_tick_ms {
        return 0.0;
    }

    state.ai_last_tick_ms = Some(now_ms);
    elapsed_ms.min(1000.0)
}

fn browser_ai_chars_for_elapsed_ms(wpm: u32, elapsed_ms: f64) -> f64 {
    f64::from(wpm) * 5.0 * elapsed_ms / 60_000.0
}

fn browser_effective_ai_wpm(
    base_wpm: u32,
    player: &RacePlayer,
    item_registry: &ItemRegistry,
) -> u32 {
    let now = Instant::now();
    let focused_wpm = if player.state.has_active_focus(now) {
        base_wpm.saturating_add(item_registry.focus_effect().ai_wpm_boost)
    } else {
        base_wpm
    };
    if player.state.is_inked_at(now) {
        (u64::from(focused_wpm)
            * u64::from(item_registry.squid_ink_effect().ai_wpm_multiplier_percent)
            / 100) as u32
    } else {
        focused_wpm
    }
}

fn advance_browser_host_ai_char(
    player: &mut typekart::game::race::RacePlayer,
    track: &Track,
) -> bool {
    let Some(key) = browser_host_next_ai_key(&player.state, track) else {
        return false;
    };
    typekart::game::typing::apply_key(&mut player.state, track, key, Instant::now());
    true
}

fn browser_host_next_ai_key(player: &PlayerState, track: &Track) -> Option<KeyAction> {
    if player.is_finished() {
        return None;
    }
    let target = track.current_word(player.word_index)?;
    let input_len = player.input.chars().count();
    target
        .chars()
        .nth(input_len)
        .map(KeyAction::Char)
        .or(Some(KeyAction::Space))
}

fn browser_sync_snapshot_from_core(
    snapshot: &mut RaceSnapshot,
    core_race: &RaceState,
    lobby_players: &[LobbyPlayer],
    player_effects: &HashMap<RacePlayerId, RaceItemEffectState>,
) {
    let now = Instant::now();
    snapshot.track_words = core_race.track.words.clone();
    for snapshot_player in &mut snapshot.players {
        let Some(core_player) = core_race
            .players
            .iter()
            .find(|player| player.id == RacePlayerId(snapshot_player.id.0))
        else {
            continue;
        };
        snapshot_player.name = core_player.name.clone();
        snapshot_player.color = browser_assigned_color(core_player.color);
        snapshot_player.word_index = core_player
            .state
            .word_index
            .min(core_race.track.len().saturating_sub(1));
        snapshot_player.input = core_player.state.input.clone();
        snapshot_player.typo_index = core_player.state.typo_index;
        snapshot_player.word_overrides = core_player
            .state
            .word_overrides
            .iter()
            .map(|(word_index, word)| typekart_protocol::WordOverrideSnapshot {
                word_index: *word_index,
                word: word.clone(),
            })
            .collect();
        snapshot_player.finished = core_player.state.is_finished();
        snapshot_player.connected = core_player.connected;
        snapshot_player.shielded = core_player.state.has_active_shield(now);
        snapshot_player.focused = core_player.state.has_active_focus(now);
        snapshot_player.inked = core_player.state.is_inked_at(now);
        snapshot_player.boosted = player_has_active_mushroom_effect(core_player, now);
        let effects = player_effects
            .get(&core_player.id)
            .cloned()
            .unwrap_or_default();
        snapshot_player.stunned = effects
            .stunned_until
            .is_some_and(|until| until > now);
        snapshot_player.impact_remaining_ms =
            browser_remaining_ms(effects.impact_cue.map(|cue| cue.until), now);
        snapshot_player.impact_cue = browser_impact_cue_snapshot(effects.impact_cue, now);
        snapshot_player.item_cue = browser_item_cue_snapshot(effects.item_cue, now);
        if let Some(lobby_player) = lobby_players
            .iter()
            .find(|lobby_player| lobby_player.id == snapshot_player.id)
        {
            snapshot_player.kind = lobby_player.kind;
        }
    }
}

fn browser_remaining_ms(until: Option<Instant>, now: Instant) -> u64 {
    until
        .filter(|until| *until > now)
        .map(|until| until.saturating_duration_since(now).as_millis() as u64)
        .unwrap_or(0)
}

fn browser_impact_cue_snapshot(
    cue: Option<typekart::game::item_effects::RaceImpactCue>,
    now: Instant,
) -> Option<ImpactCueSnapshot> {
    let cue = cue.filter(|cue| cue.until > now)?;
    Some(ImpactCueSnapshot {
        kind: match cue.kind {
            RaceImpactCueKind::Banana => ImpactCueSnapshotKind::Banana,
            RaceImpactCueKind::Cyclone => ImpactCueSnapshotKind::Cyclone,
            RaceImpactCueKind::SquidInk => ImpactCueSnapshotKind::SquidInk,
            RaceImpactCueKind::ShieldBlock => ImpactCueSnapshotKind::ShieldBlock,
        },
        remaining_ms: cue.until.saturating_duration_since(now).as_millis() as u64,
    })
}

fn browser_item_cue_snapshot(
    cue: Option<typekart::game::item_effects::RaceItemCue>,
    now: Instant,
) -> Option<ItemCueSnapshot> {
    let cue = cue.filter(|cue| cue.until > now)?;
    Some(ItemCueSnapshot {
        kind: match cue.kind {
            RaceItemCueKind::Banana { direction } => ItemCueSnapshotKind::Banana {
                direction: browser_attack_direction(direction),
            },
            RaceItemCueKind::Cyclone { direction } => ItemCueSnapshotKind::Cyclone {
                direction: browser_attack_direction(direction),
            },
            RaceItemCueKind::SquidInk => ItemCueSnapshotKind::SquidInk,
        },
        ascii_label: cue.ascii_label,
        unicode_label: cue.unicode_label,
        placement: match cue.placement {
            RaceItemCuePlacement::Before => ItemCuePlacementSnapshot::Before,
            RaceItemCuePlacement::After => ItemCuePlacementSnapshot::After,
        },
        remaining_ms: cue.until.saturating_duration_since(now).as_millis() as u64,
    })
}

fn browser_attack_direction(direction: AttackDirection) -> AttackDirectionSnapshot {
    match direction {
        AttackDirection::Ahead => AttackDirectionSnapshot::Ahead,
        AttackDirection::Behind => AttackDirectionSnapshot::Behind,
        AttackDirection::Overlap => AttackDirectionSnapshot::Overlap,
    }
}

fn browser_protocol_key_to_action(key: ProtocolKey) -> KeyAction {
    match key {
        ProtocolKey::Char(ch) => KeyAction::Char(ch),
        ProtocolKey::Space => KeyAction::Space,
        ProtocolKey::Backspace => KeyAction::Backspace,
    }
}

fn browser_player_color_id(color: AssignedColor) -> PlayerColorId {
    match color {
        AssignedColor::Cyan => PlayerColorId::Cyan,
        AssignedColor::Red => PlayerColorId::Red,
        AssignedColor::Green => PlayerColorId::Green,
        AssignedColor::Blue => PlayerColorId::Blue,
        AssignedColor::Yellow => PlayerColorId::Yellow,
        AssignedColor::Magenta => PlayerColorId::Magenta,
    }
}

fn browser_assigned_color(color: PlayerColorId) -> AssignedColor {
    match color {
        PlayerColorId::Cyan => AssignedColor::Cyan,
        PlayerColorId::Red => AssignedColor::Red,
        PlayerColorId::Green => AssignedColor::Green,
        PlayerColorId::Blue => AssignedColor::Blue,
        PlayerColorId::Yellow => AssignedColor::Yellow,
        PlayerColorId::Magenta => AssignedColor::Magenta,
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_now_ms() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;

    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as f64
}

fn browser_generate_track_words() -> Vec<String> {
    Track::generate(
        &WordList::from_static(BROWSER_HOST_WORDS),
        BROWSER_HOST_TRACK_WORD_COUNT,
    )
    .map(|track| track.words)
    .unwrap_or_else(|_| browser_demo_track_words())
}

fn browser_generate_bonus_state(track_words: &[String]) -> BonusState {
    BonusState::generate(
        &Track::new(track_words.to_vec()),
        &WordList::from_static(BROWSER_HOST_WORDS),
    )
}

fn browser_bonus_snapshots(bonuses: &BonusState, now: Instant) -> Vec<BonusPointSnapshot> {
    bonuses
        .points
        .iter()
        .map(|point| BonusPointSnapshot {
            after_word_index: point.after_word_index,
            choices: point
                .choices
                .iter()
                .map(|choice| typekart_protocol::BonusChoiceSnapshot {
                    word: choice.word.clone(),
                    status: match choice.status {
                        BonusChoiceStatus::Available => BonusChoiceSnapshotStatus::Available,
                        BonusChoiceStatus::Cooldown { until } if until <= now => {
                            BonusChoiceSnapshotStatus::Available
                        }
                        BonusChoiceStatus::Cooldown { until } => {
                            BonusChoiceSnapshotStatus::Cooldown {
                                remaining_ms: until.saturating_duration_since(now).as_millis()
                                    as u64,
                            }
                        }
                    },
                })
                .collect(),
        })
        .collect()
}

fn browser_demo_track_words() -> Vec<String> {
    [
        "spark", "river", "focus", "cyclone", "maple", "harbor", "pixel", "finish",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

const BROWSER_HOST_WORDS: &str = "\
spark
river
focus
cyclone
maple
harbor
pixel
finish
rocket
salad
tiger
ember
frost
shadow
quiet
water
lemon
grape
panda
racer
ultra
crisp
vivid
storm
marker
typing
boost
shield
banana
mushroom
";

async fn send_browser_host_direct(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    player_id: PlayerId,
    message: ServerMessage,
) -> Result<(), String> {
    let relay_message = RelayClientMessage::HostToClient {
        room: state.room.clone(),
        player_id,
        message: serde_json::to_value(message)
            .map_err(|error| format!("failed to encode host direct message: {error}"))?,
    };
    writer
        .send(Message::Text(
            serde_json::to_string(&relay_message)
                .map_err(|error| format!("failed to encode relay message: {error}"))?,
        ))
        .await
        .map_err(|error| format!("failed to send host direct message: {error:?}"))
}

async fn send_browser_host_broadcast(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: ServerMessage,
) -> Result<(), String> {
    let relay_message = RelayClientMessage::HostBroadcast {
        room: state.room.clone(),
        message: serde_json::to_value(message)
            .map_err(|error| format!("failed to encode host broadcast message: {error}"))?,
    };
    writer
        .send(Message::Text(
            serde_json::to_string(&relay_message)
                .map_err(|error| format!("failed to encode relay message: {error}"))?,
        ))
        .await
        .map_err(|error| format!("failed to send host broadcast message: {error:?}"))
}

fn add_browser_lobby_human(
    state: &mut BrowserHostLobby,
    relay_player_id: PlayerId,
    name: &str,
) -> LobbyPlayer {
    let name = browser_unique_lobby_name(
        state.players.iter(),
        &browser_lobby_name_or_default(name, "player"),
    );
    let player_id = browser_next_lobby_player_id(state);
    let player = LobbyPlayer {
        id: player_id,
        name,
        kind: PlayerKind::Human,
        color: browser_color_for_slot(state.players.len()),
        ready: false,
        connected: true,
        ai_difficulty: None,
        ai_wpm: None,
    };
    state.players.push(player.clone());
    state.next_player_id = state.next_player_id.max(player_id.0 + 1);
    state.relay_players.insert(relay_player_id, player_id);
    player
}

fn browser_next_lobby_player_id(state: &mut BrowserHostLobby) -> PlayerId {
    while state
        .players
        .iter()
        .any(|player| player.id == PlayerId(state.next_player_id))
    {
        state.next_player_id += 1;
    }
    let player_id = PlayerId(state.next_player_id);
    state.next_player_id += 1;
    player_id
}

fn browser_lobby_name_or_default(name: &str, fallback: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn browser_unique_lobby_name<'a>(
    players: impl Iterator<Item = &'a LobbyPlayer>,
    requested: &str,
) -> String {
    let base = browser_lobby_name_or_default(requested, "player");
    let existing: Vec<&str> = players.map(|player| player.name.as_str()).collect();
    if !existing.iter().any(|name| *name == base) {
        return base;
    }
    for suffix in 2..100 {
        let candidate = format!("{base}{suffix}");
        if !existing.iter().any(|name| *name == candidate) {
            return candidate;
        }
    }
    base
}

fn browser_color_for_slot(slot: usize) -> AssignedColor {
    const COLORS: [AssignedColor; 6] = [
        AssignedColor::Cyan,
        AssignedColor::Red,
        AssignedColor::Green,
        AssignedColor::Blue,
        AssignedColor::Yellow,
        AssignedColor::Magenta,
    ];
    COLORS[slot % COLORS.len()]
}

fn browser_ai_wpm(difficulty: AiDifficultySnapshot) -> u32 {
    match difficulty {
        AiDifficultySnapshot::Easy => 45,
        AiDifficultySnapshot::Hard => 85,
    }
}

fn browser_default_mod_config() -> ModConfigSnapshot {
    ModConfigSnapshot {
        word_set_id: "classic".to_string(),
        word_set_name: "Classic".to_string(),
        word_set_hash: "0000000000000001".to_string(),
        item_pack_name: "classic".to_string(),
        item_registry_hash: "0000000000000002".to_string(),
        combined_hash: "a598dc2b".to_string(),
    }
}

fn handle_relay_message(
    relay_message: RelayServerMessage,
    current_race: &mut Option<RaceSnapshot>,
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
    set_relay_player_id: WriteSignal<Option<PlayerId>>,
    set_game_player_id: WriteSignal<Option<PlayerId>>,
) -> Result<bool, String> {
    match relay_message {
        RelayServerMessage::HostToClient {
            player_id, message, ..
        } => {
            let server_message = serde_json::from_value::<ServerMessage>(message)
                .map_err(|error| format!("failed to decode host message: {error}"))?;
            handle_server_message(
                server_message,
                current_race,
                set_connection,
                set_live_frame,
                set_relay_player_id,
                set_game_player_id,
                Some(player_id),
            );
        }
        RelayServerMessage::HostBroadcast { message, .. } => {
            let server_message = serde_json::from_value::<ServerMessage>(message)
                .map_err(|error| format!("failed to decode host message: {error}"))?;
            handle_server_message(
                server_message,
                current_race,
                set_connection,
                set_live_frame,
                set_relay_player_id,
                set_game_player_id,
                None,
            );
        }
        RelayServerMessage::Error { message } => {
            set_connection.set(ConnectionState::Failed { message });
            set_live_frame.set(None);
            return Ok(false);
        }
        RelayServerMessage::RoomClosed { reason } => {
            set_connection.set(ConnectionState::Closed { reason });
            set_live_frame.set(None);
            set_relay_player_id.set(None);
            set_game_player_id.set(None);
            return Ok(false);
        }
        RelayServerMessage::ParticipantDisconnected { player_id, .. } => {
            set_connection.set(ConnectionState::Connected {
                message: format!("Participant {} disconnected", player_id.0),
            });
        }
        RelayServerMessage::RoomCreated { .. }
        | RelayServerMessage::JoinForwarded { .. }
        | RelayServerMessage::ClientToHost { .. } => {}
    }
    Ok(true)
}

fn handle_server_message(
    message: ServerMessage,
    current_race: &mut Option<RaceSnapshot>,
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
    set_relay_player_id: WriteSignal<Option<PlayerId>>,
    set_game_player_id: WriteSignal<Option<PlayerId>>,
    relay_player_id: Option<PlayerId>,
) {
    match message {
        ServerMessage::Welcome {
            player_id,
            assigned_color,
        } => {
            let outbound_player_id = relay_player_id.unwrap_or(player_id);
            set_relay_player_id.set(Some(outbound_player_id));
            set_game_player_id.set(Some(player_id));
            set_connection.set(ConnectionState::Connected {
                message: format!("Joined as player {} ({assigned_color:?})", player_id.0),
            });
        }
        ServerMessage::LobbySnapshot {
            players,
            host_id,
            mod_config,
            events,
        } => {
            set_live_frame.set(Some(GalleryFrame::Lobby(LobbyFrame {
                host_id,
                players,
                mod_config,
                events,
            })));
        }
        ServerMessage::RaceSnapshot(snapshot) => {
            *current_race = Some(snapshot.clone());
            set_live_frame.set(Some(GalleryFrame::Race(snapshot)));
        }
        ServerMessage::RaceDelta(delta) => {
            if let Some(snapshot) = apply_delta(current_race.take(), delta) {
                *current_race = Some(snapshot.clone());
                set_live_frame.set(Some(GalleryFrame::Race(snapshot)));
            } else {
                set_connection.set(ConnectionState::Failed {
                    message: "Received race delta before full race snapshot".to_string(),
                });
            }
        }
        ServerMessage::RaceEvent { message } => {
            set_connection.set(ConnectionState::Connected { message });
        }
        ServerMessage::RaceResults { placements, rows } => {
            set_live_frame.set(Some(GalleryFrame::Results(ResultsFrame {
                placements,
                rows,
                events: Vec::new(),
            })));
        }
        ServerMessage::Error { message } => {
            set_connection.set(ConnectionState::Failed {
                message: format!("Host error: {message}"),
            });
        }
    }
}

fn apply_delta(
    current_race: Option<RaceSnapshot>,
    delta: RaceDeltaSnapshot,
) -> Option<RaceSnapshot> {
    let mut snapshot = current_race?;
    snapshot.sequence = delta.sequence;
    snapshot.phase = delta.phase;
    snapshot.bonuses = delta.bonuses;
    snapshot.players = delta.players;
    snapshot.events = delta.events;
    Some(snapshot)
}

fn relay_join_url(relay: &str, room: &RoomCode) -> String {
    let base = if relay_has_path_or_query(relay) {
        relay.to_string()
    } else {
        format!("{relay}/")
    };
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}typekart_room={}", room.as_str())
}

fn relay_has_path_or_query(relay: &str) -> bool {
    relay.contains('?')
        || relay
            .split_once("://")
            .is_none_or(|(_, rest)| rest.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::{
        BROWSER_HOST_AI_TICK_MS, BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT,
        BROWSER_HOST_TRACK_WORD_COUNT, BrowserHostLobby, add_browser_lobby_human,
        apply_browser_host_ai_tick, apply_browser_host_race_key_input, browser_ai_wpm,
        browser_bonus_snapshots, browser_controls, browser_generate_bonus_state,
        browser_ensure_core_race, browser_generate_track_words, browser_host_race_snapshot,
        browser_sync_active_race_from_core, browser_unique_lobby_name, browser_update_race_status,
        build_track_window, key_name_to_protocol_key, marker_position,
        activate_browser_item_pickup, ordered_players_for_local_perspective,
        process_browser_host_client_message, relay_join_url, should_capture_global_gameplay_key,
    };
    use crate::fixtures::{GalleryFrame, SCENARIOS, scenario_frame};
    use leptos::prelude::signal;
    use typekart::game::bonus::{BonusChoice, BonusPoint, BonusState};
    use typekart::game::items::{HeldItem, ItemPickup};
    use typekart::game::race::RacePlayerId;
    use typekart_protocol::{
        AiDifficultySnapshot, AssignedColor, BonusChoiceSnapshotStatus, LobbyPlayer,
        NetworkRacePhase, PlayerId, PlayerKind, ProtocolKey, RaceResultStatus, RoomCode,
    };

    #[test]
    fn relay_join_url_adds_room_query_to_plain_relay_url() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();

        assert_eq!(
            relay_join_url("ws://127.0.0.1:8080", &room),
            "ws://127.0.0.1:8080/?typekart_room=rocket-salad-tiger"
        );
    }

    #[test]
    fn relay_join_url_preserves_existing_path_and_query() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();

        assert_eq!(
            relay_join_url("wss://relay.example/ws?debug=true", &room),
            "wss://relay.example/ws?debug=true&typekart_room=rocket-salad-tiger"
        );
    }

    #[test]
    fn track_window_keeps_anchor_with_context() {
        let GalleryFrame::Race(snapshot) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        let anchor = snapshot.players.first().unwrap();
        let window = build_track_window(&snapshot, Some(anchor));

        assert!(window.start_word <= anchor.word_index);
        assert!(window.end_word > anchor.word_index);
        assert!(window.words.len() <= 10);
    }

    #[test]
    fn marker_position_tracks_current_character() {
        let GalleryFrame::Race(snapshot) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        assert_eq!(snapshot.phase, NetworkRacePhase::Racing);
        let player = snapshot.players.first().unwrap();
        let window = build_track_window(&snapshot, Some(player));
        let word = window
            .words
            .iter()
            .find(|word| word.index == player.word_index)
            .unwrap();

        assert_eq!(
            marker_position(player, &window).column,
            word.start_ch + player.input.chars().count()
        );
    }

    #[test]
    fn local_player_is_rendered_first() {
        let GalleryFrame::Race(snapshot) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        let local_player_id = snapshot.players[1].id;

        let ordered =
            ordered_players_for_local_perspective(&snapshot.players, Some(local_player_id));

        assert_eq!(ordered[0].id, local_player_id);
        assert_eq!(ordered.len(), snapshot.players.len());
    }

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

    #[test]
    fn browser_host_lobby_starts_with_ready_host() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let lobby = BrowserHostLobby::new(room, "web-host".to_string());

        assert_eq!(lobby.players.len(), 1);
        assert_eq!(lobby.players[0].id, PlayerId(1));
        assert_eq!(lobby.players[0].name, "web-host");
        assert_eq!(lobby.players[0].color, AssignedColor::Cyan);
        assert!(lobby.players[0].ready);
    }

    #[test]
    fn browser_host_assigns_joiner_from_relay_pending_id() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());

        let player = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");

        assert_eq!(player.id, PlayerId(2));
        assert_eq!(player.name, "laura");
        assert_eq!(player.color, AssignedColor::Red);
        assert!(!player.ready);
        assert_eq!(lobby.next_player_id, 3);
        assert_eq!(
            lobby.game_player_id_for_relay(PlayerId(4)),
            Some(PlayerId(2))
        );
    }

    #[test]
    fn browser_host_joiner_id_does_not_collide_with_existing_ai() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        lobby.players.push(LobbyPlayer {
            id: PlayerId(2),
            name: "ai-1".to_string(),
            kind: PlayerKind::Bot,
            color: AssignedColor::Red,
            ready: true,
            connected: true,
            ai_difficulty: Some(AiDifficultySnapshot::Easy),
            ai_wpm: Some(browser_ai_wpm(AiDifficultySnapshot::Easy)),
        });

        let player = add_browser_lobby_human(&mut lobby, PlayerId(2), "laura");

        assert_eq!(lobby.players[1].name, "ai-1");
        assert_eq!(player.id, PlayerId(3));
        assert_eq!(
            lobby.game_player_id_for_relay(PlayerId(2)),
            Some(PlayerId(3))
        );
    }

    #[test]
    fn browser_lobby_names_are_deduped() {
        let existing = [
            LobbyPlayer {
                id: PlayerId(1),
                name: "tom".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
            LobbyPlayer {
                id: PlayerId(2),
                name: "tom2".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                ready: false,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
        ];

        assert_eq!(browser_unique_lobby_name(existing.iter(), "tom"), "tom3");
    }

    #[test]
    fn browser_ai_wpm_tracks_difficulty() {
        assert!(
            browser_ai_wpm(AiDifficultySnapshot::Hard) > browser_ai_wpm(AiDifficultySnapshot::Easy)
        );
    }

    #[test]
    fn browser_generated_track_uses_shared_track_length() {
        let words = browser_generate_track_words();

        assert_eq!(words.len(), BROWSER_HOST_TRACK_WORD_COUNT);
        assert!(words.iter().all(|word| !word.is_empty()));
    }

    #[test]
    fn browser_generated_track_includes_bonus_snapshots() {
        let words = browser_generate_track_words();
        let bonuses = browser_generate_bonus_state(&words);
        let snapshots = browser_bonus_snapshots(&bonuses, std::time::Instant::now());

        assert!(!snapshots.is_empty());
        assert!(snapshots.iter().all(|point| point.choices.len() == 3));
        assert!(
            snapshots
                .iter()
                .all(|point| point.after_word_index < words.len() - 1)
        );
    }

    #[test]
    fn browser_host_race_snapshot_uses_lobby_racers() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
        let racers = vec![lobby.players[0].clone(), joiner];

        let snapshot = browser_host_race_snapshot(
            7,
            NetworkRacePhase::Countdown {
                remaining_seconds: 3,
            },
            &lobby.mod_config,
            &racers,
            vec!["countdown 3".to_string()],
        );

        assert_eq!(snapshot.sequence, 7);
        assert_eq!(
            snapshot.phase,
            NetworkRacePhase::Countdown {
                remaining_seconds: 3
            }
        );
        assert_eq!(snapshot.players.len(), 2);
        assert_eq!(snapshot.players[0].id, PlayerId(1));
        assert_eq!(snapshot.players[1].id, PlayerId(2));
        assert_eq!(
            snapshot.track_words.first().map(String::as_str),
            Some("spark")
        );
        assert!(snapshot.bonuses.is_empty());
    }

    #[test]
    fn browser_host_race_key_input_advances_words() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let racers = vec![lobby.players[0].clone()];
        lobby.active_race = Some(browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        ));
        let (_connection, set_connection) = signal(super::ConnectionState::Disconnected);

        for ch in "spark".chars() {
            apply_browser_host_race_key_input(
                &mut lobby,
                PlayerId(1),
                ProtocolKey::Char(ch),
                set_connection,
            );
        }
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Space,
            set_connection,
        );

        let player = &lobby.active_race.as_ref().unwrap().players[0];
        assert_eq!(player.word_index, 1);
        assert_eq!(player.input, "");
        assert_eq!(player.typo_index, None);
    }

    #[test]
    fn browser_host_race_key_input_finishes_final_word_without_space() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let racers = vec![lobby.players[0].clone()];
        let mut snapshot = browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        );
        snapshot.track_words = vec!["go".to_string()];
        lobby.active_race = Some(snapshot);
        let (_connection, set_connection) = signal(super::ConnectionState::Disconnected);

        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char('g'),
            set_connection,
        );
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char('o'),
            set_connection,
        );

        let results = lobby.active_results.as_ref().unwrap();
        assert_eq!(results.placements, vec![PlayerId(1)]);
        assert_eq!(results.rows[0].player_id, PlayerId(1));
        assert_eq!(results.rows[0].progress_words, 1);
    }

    #[test]
    fn browser_host_race_results_rank_racers_by_finish_order() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
        let racers = vec![lobby.players[0].clone(), joiner];
        let mut snapshot = browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        );
        snapshot.track_words = vec!["go".to_string()];
        lobby.active_race = Some(snapshot);
        let (_connection, set_connection) = signal(super::ConnectionState::Disconnected);

        for player_id in [PlayerId(2), PlayerId(1)] {
            apply_browser_host_race_key_input(
                &mut lobby,
                player_id,
                ProtocolKey::Char('g'),
                set_connection,
            );
            apply_browser_host_race_key_input(
                &mut lobby,
                player_id,
                ProtocolKey::Char('o'),
                set_connection,
            );
        }

        let results = lobby.active_results.as_ref().unwrap();
        assert_eq!(results.placements, vec![PlayerId(2), PlayerId(1)]);
        assert_eq!(results.rows.len(), 2);
        assert_eq!(results.rows[0].player_id, PlayerId(2));
        assert_eq!(results.rows[0].placement, 1);
        assert_eq!(results.rows[0].progress_words, 1);
        assert_eq!(results.rows[1].player_id, PlayerId(1));
        assert_eq!(results.rows[1].placement, 2);
        assert!(lobby.active_race.is_none());
    }

    #[test]
    fn browser_host_race_results_timeout_places_unfinished_racers_by_progress() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
        let racers = vec![lobby.players[0].clone(), joiner];
        let mut snapshot = browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        );
        snapshot.track_words = vec!["go".to_string(), "fast".to_string()];
        lobby.active_race = Some(snapshot);
        let (_connection, set_connection) = signal(super::ConnectionState::Disconnected);

        for ch in "gof".chars() {
            apply_browser_host_race_key_input(
                &mut lobby,
                PlayerId(2),
                ProtocolKey::Char(ch),
                set_connection,
            );
            if ch == 'o' {
                apply_browser_host_race_key_input(
                    &mut lobby,
                    PlayerId(2),
                    ProtocolKey::Space,
                    set_connection,
                );
            }
        }
        for ch in "go".chars() {
            apply_browser_host_race_key_input(
                &mut lobby,
                PlayerId(1),
                ProtocolKey::Char(ch),
                set_connection,
            );
        }
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Space,
            set_connection,
        );
        for ch in "fast".chars() {
            apply_browser_host_race_key_input(
                &mut lobby,
                PlayerId(1),
                ProtocolKey::Char(ch),
                set_connection,
            );
        }

        let first_finished_at = lobby.runtime.lifecycle.first_finished_at.unwrap();
        assert!(browser_update_race_status(
            &mut lobby,
            first_finished_at + BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT
        ));

        let results = lobby.active_results.as_ref().unwrap();
        assert_eq!(results.placements, vec![PlayerId(1), PlayerId(2)]);
        assert_eq!(results.rows[0].status, RaceResultStatus::Finished);
        assert_eq!(results.rows[1].status, RaceResultStatus::TimedOut);
        assert_eq!(results.rows[1].progress_words, 1);
    }

    #[test]
    fn browser_host_restart_command_returns_results_to_lobby() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        lobby.active_results = Some(crate::fixtures::ResultsFrame {
            placements: vec![PlayerId(1)],
            rows: Vec::new(),
            events: vec!["Race finished".to_string()],
        });
        lobby.active_race = Some(browser_host_race_snapshot(
            1,
            NetworkRacePhase::Finished,
            &lobby.mod_config,
            &lobby.players.clone(),
            Vec::new(),
        ));
        lobby.core_race = Some(super::browser_host_core_race(
            &lobby.players.clone(),
            super::browser_demo_track_words(),
        ));
        lobby.runtime.lifecycle.placements = vec![RacePlayerId(1)];

        process_browser_host_client_message(
            &mut lobby,
            PlayerId(1),
            typekart_protocol::ClientMessage::RestartRace,
            signal(super::ConnectionState::Disconnected).1,
        );

        assert!(lobby.active_results.is_none());
        assert!(lobby.active_race.is_none());
        assert!(lobby.core_race.is_none());
        assert!(lobby.runtime.lifecycle.placements.is_empty());
        assert!(lobby.events.iter().any(|event| event == "returned to lobby"));
    }

    #[test]
    fn browser_host_race_key_input_marks_and_clears_typos() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let racers = vec![lobby.players[0].clone()];
        lobby.active_race = Some(browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        ));
        let (_connection, set_connection) = signal(super::ConnectionState::Disconnected);

        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char('x'),
            set_connection,
        );
        assert_eq!(
            lobby.active_race.as_ref().unwrap().players[0].typo_index,
            Some(0)
        );

        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Backspace,
            set_connection,
        );
        let player = &lobby.active_race.as_ref().unwrap().players[0];
        assert_eq!(player.input, "");
        assert_eq!(player.typo_index, None);
    }

    #[test]
    fn browser_host_bonus_word_claims_choice_after_space() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let racers = vec![lobby.players[0].clone()];
        let mut snapshot = browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        );
        snapshot.track_words = vec!["one".to_string(), "two".to_string()];
        snapshot.players[0].word_index = 1;
        lobby.active_race = Some(snapshot);
        lobby.bonuses = BonusState::with_points(
            vec![BonusPoint::new(
                0,
                [
                    BonusChoice::available("dash"),
                    BonusChoice::available("drift"),
                    BonusChoice::available("turbo"),
                ],
            )],
            vec!["dash".to_string(), "drift".to_string(), "turbo".to_string()],
        );
        let (_connection, set_connection) = signal(super::ConnectionState::Disconnected);

        for ch in "dash".chars() {
            apply_browser_host_race_key_input(
                &mut lobby,
                PlayerId(1),
                ProtocolKey::Char(ch),
                set_connection,
            );
        }

        let player = &lobby.active_race.as_ref().unwrap().players[0];
        assert_eq!(player.word_index, 1);
        assert_eq!(player.input, "dash");
        assert!(lobby.runtime.bonus_attempts.contains_key(&PlayerId(1)));
        assert!(matches!(
            lobby.active_race.as_ref().unwrap().bonuses[0].choices[0].status,
            BonusChoiceSnapshotStatus::Available
        ));

        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Space,
            set_connection,
        );

        let player = &lobby.active_race.as_ref().unwrap().players[0];
        assert_eq!(player.word_index, 1);
        assert_eq!(player.input, "");
        assert_eq!(player.typo_index, None);
        assert!(!lobby.runtime.bonus_attempts.contains_key(&PlayerId(1)));
        assert_eq!(lobby.runtime.spent_bonus_gaps.get(&PlayerId(1)), Some(&0));
        assert!(matches!(
            lobby.active_race.as_ref().unwrap().bonuses[0].choices[0].status,
            BonusChoiceSnapshotStatus::Cooldown { .. }
        ));
        assert!(lobby.events.iter().any(|event| event.contains("got")));
        assert!(
            !lobby.active_race.as_ref().unwrap().events[0].contains("typed"),
            "bonus pickup or item event should be visible in the race feed"
        );
    }

    #[test]
    fn browser_host_banana_activation_resets_target_and_renders_impact() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
        let racers = vec![lobby.players[0].clone(), joiner];
        let mut snapshot = browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        );
        snapshot.track_words = vec!["one".to_string(), "two".to_string()];
        snapshot.players[0].word_index = 0;
        snapshot.players[1].word_index = 1;
        snapshot.players[1].input = "twx".to_string();
        snapshot.players[1].typo_index = Some(2);
        lobby.active_race = Some(snapshot);
        browser_ensure_core_race(&mut lobby);

        activate_browser_item_pickup(
            &mut lobby,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Banana),
            std::time::Instant::now(),
        );
        browser_sync_active_race_from_core(&mut lobby);

        let target = lobby
            .active_race
            .as_ref()
            .unwrap()
            .players
            .iter()
            .find(|player| player.id == PlayerId(2))
            .unwrap();
        assert_eq!(target.input, "");
        assert_eq!(target.typo_index, None);
        assert!(target.impact_cue.is_some());
        assert!(!target.stunned);
    }

    #[test]
    fn browser_host_ai_tick_advances_bot_racers() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        process_browser_host_client_message(
            &mut lobby,
            PlayerId(1),
            typekart_protocol::ClientMessage::AddAi,
            signal(super::ConnectionState::Disconnected).1,
        );
        let racers = lobby.players.clone();
        lobby.active_race = Some(browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        ));

        assert!(apply_browser_host_ai_tick(&mut lobby, 1_000));

        let ai = lobby
            .active_race
            .as_ref()
            .unwrap()
            .players
            .iter()
            .find(|player| player.kind == PlayerKind::Bot)
            .unwrap();
        assert!(!ai.input.is_empty() || ai.word_index > 0);
    }

    #[test]
    fn browser_host_ai_tick_finishes_bot_with_enough_budget() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        process_browser_host_client_message(
            &mut lobby,
            PlayerId(1),
            typekart_protocol::ClientMessage::AddAi,
            signal(super::ConnectionState::Disconnected).1,
        );
        let ai_id = lobby
            .players
            .iter()
            .find(|player| player.kind == PlayerKind::Bot)
            .unwrap()
            .id;
        lobby
            .players
            .iter_mut()
            .find(|player| player.id == ai_id)
            .unwrap()
            .ai_wpm = Some(1000);
        let racers = lobby.players.clone();
        lobby.active_race = Some(browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        ));

        assert!(apply_browser_host_ai_tick(&mut lobby, 60_000));

        let ai = lobby
            .active_race
            .as_ref()
            .unwrap()
            .players
            .iter()
            .find(|player| player.id == ai_id)
            .unwrap();
        assert!(ai.finished);
    }

    #[test]
    fn browser_host_ai_tick_ignores_queued_countdown_ticks() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let mut lobby = BrowserHostLobby::new(room, "host".to_string());
        process_browser_host_client_message(
            &mut lobby,
            PlayerId(1),
            typekart_protocol::ClientMessage::AddAi,
            signal(super::ConnectionState::Disconnected).1,
        );
        let racers = lobby.players.clone();
        lobby.active_race = Some(browser_host_race_snapshot(
            1,
            NetworkRacePhase::Racing,
            &lobby.mod_config,
            &racers,
            Vec::new(),
        ));
        lobby.ai_last_tick_ms = Some(super::browser_now_ms());

        assert!(!apply_browser_host_ai_tick(
            &mut lobby,
            BROWSER_HOST_AI_TICK_MS
        ));

        let ai = lobby
            .active_race
            .as_ref()
            .unwrap()
            .players
            .iter()
            .find(|player| player.kind == PlayerKind::Bot)
            .unwrap();
        assert_eq!(ai.word_index, 0);
        assert_eq!(ai.input, "");
    }

    fn scenario_race_with_finished_local() -> typekart_protocol::RaceSnapshot {
        let GalleryFrame::Race(mut racing) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        racing.players[0].finished = true;
        racing
    }
}

#[component]
fn GalleryFrameView(
    scenario: impl Fn() -> GalleryScenario + Copy + Send + Sync + 'static,
    unicode_icons: ReadSignal<bool>,
) -> impl IntoView {
    move || match scenario_frame(scenario()) {
        GalleryFrame::Lobby(snapshot) => view! {
            <LobbyPanel snapshot=snapshot local_player_id=None />
        }
        .into_any(),
        GalleryFrame::Race(snapshot) => view! {
                <RacePanel
                    snapshot=snapshot
                    local_player_id=None
                    unicode_icons=unicode_icons
                    on_key=|_key| {}
                />
        }
        .into_any(),
        GalleryFrame::Results(snapshot) => view! { <ResultsPanel snapshot=snapshot /> }.into_any(),
    }
}

#[component]
fn LobbyPanel(snapshot: LobbyFrame, local_player_id: Option<PlayerId>) -> impl IntoView {
    view! {
        <section class="panel lobby-panel" aria-label="Lobby preview">
            <div class="phase">"Lobby"</div>
            <div class="mod-strip">
                <span>"Words " {snapshot.mod_config.word_set_name}</span>
                <span>"Items " {snapshot.mod_config.item_pack_name}</span>
                <span>"Mod " {snapshot.mod_config.combined_hash}</span>
            </div>
            <div class="lobby-list">
                <For
                    each={move || snapshot.players.clone()}
                    key=|player: &LobbyPlayer| player.id
                    children={move |player| {
                        view! {
                            <div class="lobby-player" class:self-lane=Some(player.id) == local_player_id>
                                <span class={format!("marker {}", color_class(player.color))}>{"●"}</span>
                                <span>{player.name}</span>
                                <span>{kind_label(player.kind)}</span>
                                <span>{if player.id == snapshot.host_id { "host" } else { "" }}</span>
                                <span>{if player.ready { "ready" } else { "not ready" }}</span>
                                <span>{if player.connected { "" } else { "offline" }}</span>
                            </div>
                        }
                    }}
                />
            </div>
            <EventFeed events=snapshot.events />
        </section>
    }
}

#[component]
fn BrowserLobbyManagement(
    snapshot: LobbyFrame,
    local_player_id: Option<PlayerId>,
    relay_player_id: Option<PlayerId>,
    outbound: Option<UnboundedSender<BrowserOutboundMessage>>,
    set_connection: WriteSignal<ConnectionState>,
) -> impl IntoView {
    let (rename_value, set_rename_value) = signal(String::new());
    let local_player = local_player_id
        .and_then(|id| snapshot.players.iter().find(|player| player.id == id))
        .cloned();
    let is_host = local_player_id == Some(snapshot.host_id);
    let can_send = relay_player_id.is_some() && outbound.is_some();
    let management_rows = snapshot.players.clone();
    let command_sink = BrowserCommandSink {
        outbound,
        relay_player_id,
        set_connection,
    };

    view! {
        <section class="panel lobby-management" aria-label="Lobby management">
            <div class="phase">"Lobby controls"</div>
            <div class="rename-row">
                <label>
                    <span>"Display name"</span>
                    <input
                        type="text"
                        placeholder={local_player.as_ref().map(|player| player.name.clone()).unwrap_or_else(|| "name".to_string())}
                        prop:value=move || rename_value.get()
                        disabled=move || !can_send
                        on:input=move |event| set_rename_value.set(event_target_value(&event))
                    />
                </label>
                <button
                    type="button"
                    disabled=move || !can_send || rename_value.get().trim().is_empty()
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| {
                        let name = rename_value.get_untracked().trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        command_sink.send(ClientMessage::Rename { name }, "Rename sent");
                        set_rename_value.set(String::new());
                    }
                    }
                >
                    "Rename"
                </button>
            </div>

            <div class="host-controls" hidden=move || !is_host>
                <button
                    type="button"
                    disabled=move || !can_send
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| command_sink.send(ClientMessage::AddAi, "Add AI sent")
                    }
                >
                    "Add AI"
                </button>
                <button
                    type="button"
                    class="secondary"
                    disabled=move || !can_send
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| {
                        command_sink.send(
                            ClientMessage::SetAiDifficulty {
                                player_id: None,
                                difficulty: AiDifficultySnapshot::Easy,
                            },
                            "Set AI difficulty sent",
                        )
                    }
                    }
                >
                    "All AI Easy"
                </button>
                <button
                    type="button"
                    class="secondary"
                    disabled=move || !can_send
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| {
                        command_sink.send(
                            ClientMessage::SetAiDifficulty {
                                player_id: None,
                                difficulty: AiDifficultySnapshot::Hard,
                            },
                            "Set AI difficulty sent",
                        )
                    }
                    }
                >
                    "All AI Hard"
                </button>
            </div>

            <div class="management-list" hidden=move || !is_host>
                <For
                    each=move || management_rows.clone()
                    key=|player: &LobbyPlayer| player.id
                    children={move |player| {
                        let target_id = player.id;
                        let is_self = Some(target_id) == local_player_id;
                        let is_bot = player.kind == PlayerKind::Bot;
                        let easy_sink = command_sink.clone();
                        let hard_sink = command_sink.clone();
                        let remove_sink = command_sink.clone();
                        view! {
                            <div class="management-player">
                                <span class={format!("marker {}", color_class(player.color))}>{"●"}</span>
                                <span>{player.name.clone()}</span>
                                <span>{kind_label(player.kind)}</span>
                                <span>{ai_detail_label(&player)}</span>
                                <button
                                    type="button"
                                    class="secondary"
                                    hidden=move || !is_bot
                                    disabled=move || !can_send
                                    on:click=move |_| {
                                        easy_sink.send(
                                            ClientMessage::SetAiDifficulty {
                                                player_id: Some(target_id),
                                                difficulty: AiDifficultySnapshot::Easy,
                                            },
                                            "Set AI difficulty sent",
                                        )
                                    }
                                >
                                    "Easy"
                                </button>
                                <button
                                    type="button"
                                    class="secondary"
                                    hidden=move || !is_bot
                                    disabled=move || !can_send
                                    on:click=move |_| {
                                        hard_sink.send(
                                            ClientMessage::SetAiDifficulty {
                                                player_id: Some(target_id),
                                                difficulty: AiDifficultySnapshot::Hard,
                                            },
                                            "Set AI difficulty sent",
                                        )
                                    }
                                >
                                    "Hard"
                                </button>
                                <button
                                    type="button"
                                    class="secondary danger"
                                    hidden=move || is_self
                                    disabled=move || !can_send
                                    on:click=move |_| {
                                        remove_sink.send(
                                            ClientMessage::RemoveLobbyPlayer { player_id: target_id },
                                            "Remove player sent",
                                        )
                                    }
                                >
                                    {if is_bot { "Remove" } else { "Kick" }}
                                </button>
                            </div>
                        }
                    }}
                />
            </div>
        </section>
    }
}

#[component]
fn RacePanel(
    snapshot: RaceSnapshot,
    local_player_id: Option<PlayerId>,
    unicode_icons: ReadSignal<bool>,
    on_key: impl Fn(ProtocolKey) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let players = snapshot.players.clone();
    let local_player = local_player_id
        .and_then(|id| players.iter().find(|player| player.id == id).cloned())
        .or_else(|| players.first().cloned());
    let local_player_id = local_player.as_ref().map(|player| player.id);
    let display_players = ordered_players_for_local_perspective(&players, local_player_id);
    let track_window = build_track_window(&snapshot, local_player.as_ref());
    let track_width_ch = track_window.width_ch;
    let lane_window = track_window.clone();
    let minimap_snapshot = snapshot.clone();
    let events = snapshot.events.clone();

    view! {
        <section
            class="panel track-panel"
            aria-label="Race preview"
            tabindex="0"
            on:keydown=move |event| {
                if let Some(key) = keyboard_event_to_protocol_key(&event) {
                    event.prevent_default();
                    event.stop_propagation();
                    on_key(key);
                }
            }
        >
            <div class="phase">{phase_label(snapshot.phase)} " · seq " {snapshot.sequence}</div>
            <div class="mod-strip">
                <span>{snapshot.mod_config.word_set_name.clone()}</span>
                <span>{snapshot.mod_config.item_pack_name.clone()}</span>
                <span>{snapshot.mod_config.combined_hash.clone()}</span>
            </div>
            <div
                class="race-window"
                style={format!("--track-ch: {};", track_width_ch)}
            >
                <BonusStack bonuses=snapshot.bonuses.clone() window=track_window.clone() local_player=local_player.clone() />
                <TrackWords snapshot=snapshot.clone() window=track_window.clone() local_player=local_player.clone() />
                <For
                    each={move || display_players.clone()}
                    key=|player: &PlayerSnapshot| player.id
                    children={move |player| {
                        view! {
                            <RacerLane
                                player=player.clone()
                                window=lane_window.clone()
                                local=Some(player.id) == local_player_id
                                unicode_icons=unicode_icons
                            />
                        }
                    }}
                />
            </div>
            <Minimap snapshot=minimap_snapshot />
            <EventFeed events=events />
        </section>
    }
}

fn ordered_players_for_local_perspective(
    players: &[PlayerSnapshot],
    local_player_id: Option<PlayerId>,
) -> Vec<PlayerSnapshot> {
    let Some(local_player_id) = local_player_id else {
        return players.to_vec();
    };
    let mut ordered = Vec::with_capacity(players.len());
    if let Some(local_player) = players.iter().find(|player| player.id == local_player_id) {
        ordered.push(local_player.clone());
    }
    ordered.extend(
        players
            .iter()
            .filter(|player| player.id != local_player_id)
            .cloned(),
    );
    ordered
}

#[component]
fn BonusStack(
    bonuses: Vec<BonusPointSnapshot>,
    window: TrackWindow,
    local_player: Option<PlayerSnapshot>,
) -> impl IntoView {
    view! {
        <div class="bonus-layer">
            <For
                each={move || visible_bonus_columns(&bonuses, &window)}
                key=|(bonus, _)| bonus.after_word_index
                children={move |(bonus, column)| {
                    let point = bonus.clone();
                    let choices = bonus.choices.clone();
                    let local_player = local_player.clone();
                    view! {
                        <div class="bonus-stack" style={format!("left: {column}ch")}>
                            <For
                                each=move || choices.clone()
                                key=|choice| choice.word.clone()
                                children={move |choice| {
                                    let available_to_local = local_player.as_ref().is_some_and(|player| {
                                        bonus_choice_available_to_player(player, &point, &choice)
                                    });
                                    view! {
                                        <span class:cooldown=move || !available_to_local>
                                            {choice.word}
                                            {cooldown_label(choice.status)}
                                        </span>
                                    }
                                }}
                            />
                        </div>
                    }
                }}
            />
        </div>
    }
}

fn bonus_choice_available_to_player(
    player: &PlayerSnapshot,
    point: &BonusPointSnapshot,
    choice: &typekart_protocol::BonusChoiceSnapshot,
) -> bool {
    matches!(choice.status, BonusChoiceSnapshotStatus::Available)
        && player.word_index == point.after_word_index.saturating_add(1)
        && !player.finished
        && !player.shielded
        && !player.focused
        && !player.boosted
        && !player.stunned
        && player.typo_index.is_none()
        && (player.input.is_empty() || choice.word.starts_with(&player.input))
}

#[component]
fn TrackWords(
    snapshot: RaceSnapshot,
    window: TrackWindow,
    local_player: Option<PlayerSnapshot>,
) -> impl IntoView {
    view! {
        <div class="track-text">
            <For
                each={move || window.words.clone()}
                key=|word| word.index
                children={move |word| {
                    let rendered = render_word_segments(&snapshot, local_player.as_ref(), &word);
                    view! {
                        <span class="track-word">
                            <For
                                each=move || rendered.clone().into_iter().enumerate()
                                key=|(index, _)| *index
                                children={|(_index, segment)| {
                                    view! {
                                        <span class=segment.class>
                                            {segment.text}
                                        </span>
                                    }
                                }}
                            />
                        </span>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn RacerLane(
    player: PlayerSnapshot,
    window: TrackWindow,
    local: bool,
    unicode_icons: ReadSignal<bool>,
) -> impl IntoView {
    let use_unicode_icons = unicode_icons.get();
    let cue_before = player
        .item_cue
        .as_ref()
        .filter(|cue| cue.placement == ItemCuePlacementSnapshot::Before)
        .map(|cue| cue_label(cue, use_unicode_icons).to_string());
    let cue_after = player
        .item_cue
        .as_ref()
        .filter(|cue| cue.placement == ItemCuePlacementSnapshot::After)
        .map(|cue| cue_label(cue, use_unicode_icons).to_string());
    let marker = marker_position(&player, &window);
    let offscreen = marker.offscreen_class();

    view! {
        <div
            class="lane"
            class:self-lane=local
            class:stunned=player.stunned
            class:disconnected=!player.connected
        >
            <span class="lane-name">{player.name.clone()}</span>
            <span class="lane-kind">{kind_label(player.kind)}</span>
            <span class="lane-track">
                <span
                    class={format!("lane-marker-wrap {offscreen}")}
                    style={format!("left: {}ch", marker.column)}
                >
                    {cue_before}
                    <span class={format!("marker {}", color_class(player.color))}>
                        {effect_prefix(&player, use_unicode_icons)}
                        {marker.glyph}
                        {impact_label(player.impact_cue, use_unicode_icons)}
                    </span>
                    {cue_after}
                </span>
            </span>
            <span class="lane-status">{status_label(&player)}</span>
        </div>
    }
}

#[component]
fn Minimap(snapshot: RaceSnapshot) -> impl IntoView {
    let players = snapshot.players.clone();
    let word_count = snapshot.track_words.len();

    view! {
        <div class="minimap" aria-label="Race minimap">
            <div class="minimap-line"></div>
            <For
                each={move || players.clone()}
                key=|player: &PlayerSnapshot| player.id
                children={move |player| {
                    let left = minimap_position(&player, word_count);
                    view! {
                        <span
                            class={format!("minimap-dot {}", color_class(player.color))}
                            style={format!("left: {left}%")}
                            title={player.name}
                        ></span>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn ResultsPanel(snapshot: ResultsFrame) -> impl IntoView {
    view! {
        <section class="panel results-panel" aria-label="Results preview">
            <div class="phase">"Results"</div>
            <div class="result-grid">
                <For
                    each={move || snapshot.rows.clone()}
                    key=|row| row.placement
                    children={|row| {
                        view! {
                            <div class="result-row">
                                <span>{row.placement}</span>
                                <span class={format!("marker {}", color_class(row.color))}>{"●"}</span>
                                <span>{row.name}</span>
                                <span>{result_status_label(row.status)}</span>
                                <span>{row.progress_words} "/" {row.track_words}</span>
                                <span>{row.wpm} " wpm"</span>
                                <span>{row.accuracy_percent} "%"</span>
                            </div>
                        }
                    }}
                />
            </div>
            <EventFeed events=snapshot.events />
        </section>
    }
}

#[component]
fn EventFeed(events: Vec<String>) -> impl IntoView {
    view! {
        <div class="events">
            <For
                each={move || events.clone()}
                key=|event| event.clone()
                children={|event| view! { <span>{event}</span> }}
            />
        </div>
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackWindow {
    start_word: usize,
    end_word: usize,
    width_ch: usize,
    words: Vec<VisibleWord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleWord {
    index: usize,
    start_ch: usize,
    end_ch: usize,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WordSegment {
    text: String,
    class: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OffscreenSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkerPosition {
    column: usize,
    glyph: &'static str,
    offscreen: Option<OffscreenSide>,
}

impl MarkerPosition {
    fn offscreen_class(self) -> &'static str {
        match self.offscreen {
            Some(OffscreenSide::Left) => "offscreen-left",
            Some(OffscreenSide::Right) => "offscreen-right",
            None => "",
        }
    }
}

fn build_track_window(snapshot: &RaceSnapshot, anchor: Option<&PlayerSnapshot>) -> TrackWindow {
    let word_count = snapshot.track_words.len();
    if word_count == 0 {
        return TrackWindow {
            start_word: 0,
            end_word: 0,
            width_ch: 1,
            words: Vec::new(),
        };
    }

    let anchor_word = anchor
        .map(|player| player.word_index.min(word_count.saturating_sub(1)))
        .unwrap_or(0);
    let mut start_word = anchor_word.saturating_sub(WEB_TRACK_WORDS_BEHIND);
    let mut end_word = (start_word + WEB_TRACK_VISIBLE_WORDS).min(word_count);
    if end_word - start_word < WEB_TRACK_VISIBLE_WORDS {
        start_word = end_word.saturating_sub(WEB_TRACK_VISIBLE_WORDS);
    }
    end_word = end_word.max(start_word + 1).min(word_count);

    let mut cursor = 0usize;
    let mut words = Vec::new();
    for index in start_word..end_word {
        if index > start_word {
            cursor += 1;
        }
        let text = word_for_player(snapshot, anchor, index);
        let start_ch = cursor;
        cursor += text.chars().count();
        words.push(VisibleWord {
            index,
            start_ch,
            end_ch: cursor,
            text,
        });
    }

    TrackWindow {
        start_word,
        end_word,
        width_ch: cursor.max(1),
        words,
    }
}

fn visible_bonus_columns(
    bonuses: &[BonusPointSnapshot],
    window: &TrackWindow,
) -> Vec<(BonusPointSnapshot, usize)> {
    bonuses
        .iter()
        .filter_map(|bonus| {
            let word = window
                .words
                .iter()
                .find(|word| word.index == bonus.after_word_index)?;
            if bonus.after_word_index + 1 >= window.end_word {
                return None;
            }
            Some((bonus.clone(), word.end_ch + 1))
        })
        .collect()
}

fn render_word_segments(
    snapshot: &RaceSnapshot,
    local_player: Option<&PlayerSnapshot>,
    word: &VisibleWord,
) -> Vec<WordSegment> {
    let Some(player) = local_player else {
        return vec![WordSegment {
            text: word.text.clone(),
            class: "",
        }];
    };

    if word.index != player.word_index || player.input.is_empty() {
        return vec![WordSegment {
            text: word.text.clone(),
            class: word_state_class(snapshot.phase),
        }];
    }

    let typo_index = player.typo_index.unwrap_or(usize::MAX);
    let mut segments = Vec::new();
    for (index, ch) in word.text.chars().enumerate() {
        let class = if index >= typo_index {
            "typed typo"
        } else if index < player.input.chars().count() {
            "typed"
        } else if index == player.input.chars().count() {
            "cursor"
        } else {
            word_state_class(snapshot.phase)
        };
        segments.push(WordSegment {
            text: ch.to_string(),
            class,
        });
    }
    segments
}

fn word_state_class(phase: NetworkRacePhase) -> &'static str {
    match phase {
        NetworkRacePhase::Lobby
        | NetworkRacePhase::WaitingForHost
        | NetworkRacePhase::Countdown { .. } => "pending",
        NetworkRacePhase::Racing | NetworkRacePhase::Finished => "",
    }
}

fn marker_position(player: &PlayerSnapshot, window: &TrackWindow) -> MarkerPosition {
    if window.words.is_empty() {
        return MarkerPosition {
            column: 0,
            glyph: "███",
            offscreen: None,
        };
    }

    if player.word_index < window.start_word {
        return MarkerPosition {
            column: 0,
            glyph: "<",
            offscreen: Some(OffscreenSide::Left),
        };
    }

    if player.word_index >= window.end_word {
        return MarkerPosition {
            column: window.width_ch,
            glyph: if player.finished { ">!" } else { ">" },
            offscreen: Some(OffscreenSide::Right),
        };
    }

    let word = window
        .words
        .iter()
        .find(|word| word.index == player.word_index)
        .expect("visible player word must exist in track window");
    let input_chars = player.input.chars().count();
    let typo_index = player.typo_index.unwrap_or(input_chars);
    let progress_chars = input_chars.min(typo_index).min(word.text.chars().count());
    let column = if player.finished {
        word.end_ch
    } else {
        word.start_ch + progress_chars
    };

    MarkerPosition {
        column,
        glyph: "███",
        offscreen: None,
    }
}

fn phase_label(phase: NetworkRacePhase) -> String {
    match phase {
        NetworkRacePhase::Lobby => "Lobby".to_string(),
        NetworkRacePhase::WaitingForHost => "Waiting for host".to_string(),
        NetworkRacePhase::Countdown { remaining_seconds } => {
            format!("Countdown: {remaining_seconds}")
        }
        NetworkRacePhase::Racing => "Racing".to_string(),
        NetworkRacePhase::Finished => "Finished".to_string(),
    }
}

fn word_for_player(
    snapshot: &RaceSnapshot,
    player: Option<&PlayerSnapshot>,
    index: usize,
) -> String {
    let Some(base_word) = snapshot.track_words.get(index) else {
        return String::new();
    };
    let Some(player) = player else {
        return base_word.clone();
    };
    if let Some(override_word) = player
        .word_overrides
        .iter()
        .find(|override_word| override_word.word_index == index)
    {
        return override_word.word.clone();
    }
    fixtures::masked_word(player, index, base_word)
}

fn effect_prefix(player: &PlayerSnapshot, unicode_icons: bool) -> &'static str {
    match unicode_icons {
        true if player.boosted => ">>🍄 ",
        false if player.boosted => ">>> ",
        true if player.shielded => "🛡 ",
        false if player.shielded => "[",
        true if player.focused => "⭐ ",
        false if player.focused => "* ",
        true if player.inked => "⬛ ",
        false if player.inked => "# ",
        _ => "",
    }
}

fn cue_label(cue: &ItemCueSnapshot, unicode_icons: bool) -> &str {
    if unicode_icons {
        &cue.unicode_label
    } else {
        &cue.ascii_label
    }
}

fn impact_label(impact: Option<ImpactCueSnapshot>, unicode_icons: bool) -> &'static str {
    impact
        .map(|impact| match (impact.kind, unicode_icons) {
            (ImpactCueSnapshotKind::Banana, true) => "🍌",
            (ImpactCueSnapshotKind::Banana, false) => "BAN",
            (ImpactCueSnapshotKind::Cyclone, true) => "🌀",
            (ImpactCueSnapshotKind::Cyclone, false) => "CYC",
            (ImpactCueSnapshotKind::SquidInk, true) => "⬛",
            (ImpactCueSnapshotKind::SquidInk, false) => "INK",
            (ImpactCueSnapshotKind::ShieldBlock, true) => "🛡",
            (ImpactCueSnapshotKind::ShieldBlock, false) => "BLK",
        })
        .unwrap_or("")
}

fn status_label(player: &PlayerSnapshot) -> &'static str {
    if player.finished {
        "finished"
    } else if player.stunned {
        "stunned"
    } else if player.inked {
        "inked"
    } else if !player.connected {
        "offline"
    } else {
        "word"
    }
}

fn kind_label(kind: PlayerKind) -> &'static str {
    match kind {
        PlayerKind::Human => "human",
        PlayerKind::Bot => "ai",
    }
}

fn ai_detail_label(player: &LobbyPlayer) -> String {
    match (player.ai_difficulty, player.ai_wpm) {
        (Some(difficulty), Some(wpm)) => format!("{difficulty:?} {wpm}wpm"),
        (Some(difficulty), None) => format!("{difficulty:?}"),
        (None, _) => String::new(),
    }
}

fn result_status_label(status: RaceResultStatus) -> &'static str {
    match status {
        RaceResultStatus::Finished => "finished",
        RaceResultStatus::TimedOut => "timed out",
        RaceResultStatus::Disconnected => "disconnected",
    }
}

fn cooldown_label(status: BonusChoiceSnapshotStatus) -> String {
    match status {
        BonusChoiceSnapshotStatus::Available => String::new(),
        BonusChoiceSnapshotStatus::Cooldown { remaining_ms } => format!(" ({remaining_ms}ms)"),
    }
}
