mod fixtures;

use fixtures::{
    GalleryFrame, GalleryScenario, LobbyFrame, ResultsFrame, SCENARIOS, color_class,
    minimap_position, scenario_frame,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::{SinkExt, StreamExt, select};
use gloo_net::websocket::{Message, futures::WebSocket};
use leptos::prelude::*;
use typekart_protocol::{
    AiDifficultySnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot, ClientMessage,
    ClientSequence, ImpactCueSnapshot, ImpactCueSnapshotKind, ItemCuePlacementSnapshot,
    ItemCueSnapshot, LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind, PlayerSnapshot,
    ProtocolKey, RaceDeltaSnapshot, RaceResultStatus, RaceSnapshot, RelayClientMessage,
    RelayServerMessage, RoomCode, ServerMessage,
};
use wasm_bindgen_futures::spawn_local;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const WEB_TRACK_WORDS_BEHIND: usize = 3;
const WEB_TRACK_VISIBLE_WORDS: usize = 10;

#[derive(Debug, Clone)]
enum BrowserOutboundMessage {
    Client {
        player_id: PlayerId,
        message: ClientMessage,
    },
    Disconnect,
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

    let join = move |_| {
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
            match observe_room(
                relay,
                room,
                name,
                outbound_rx,
                set_connection,
                set_live_frame,
                set_relay_player_id,
                set_game_player_id,
            )
            .await
            {
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

async fn observe_room(
    relay_url: String,
    room_code: String,
    name: String,
    outbound: UnboundedReceiver<BrowserOutboundMessage>,
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
    set_relay_player_id: WriteSignal<Option<PlayerId>>,
    set_game_player_id: WriteSignal<Option<PlayerId>>,
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
    set_connection.set(ConnectionState::Connected {
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
                    set_connection,
                    set_live_frame,
                    set_relay_player_id,
                    set_game_player_id,
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
        browser_controls, build_track_window, key_name_to_protocol_key, marker_position,
        ordered_players_for_local_perspective, relay_join_url, should_capture_global_gameplay_key,
    };
    use crate::fixtures::{GalleryFrame, SCENARIOS, scenario_frame};
    use typekart_protocol::{NetworkRacePhase, ProtocolKey, RoomCode};

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

        let controls = browser_controls(Some(&frame), Some(typekart_protocol::PlayerId(2)));

        assert!(controls.show_rematch_ready);
        assert!(!controls.show_ready);
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
