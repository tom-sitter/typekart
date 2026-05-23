mod fixtures;
mod host;
mod render;
mod session;

use std::rc::Rc;

use fixtures::{GalleryFrame, GalleryScenario, LobbyFrame, ResultsFrame, SCENARIOS};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::{SinkExt, StreamExt, select};
use gloo_net::websocket::{Message, futures::WebSocket};
use leptos::prelude::*;
use typekart_protocol::{
    ClientMessage, ClientSequence, PlayerId, RaceDeltaSnapshot, RaceSnapshot, RelayClientMessage,
    RelayServerMessage, RoomCode, ServerMessage,
};
use wasm_bindgen_futures::spawn_local;

use host::host_browser_lobby;
use render::{BrowserLobbyManagement, GalleryFrameView, LobbyPanel, RacePanel, ResultsPanel};
use session::{
    BrowserHostSignals, BrowserOutboundMessage, BrowserSessionKind, BrowserSessionSignals,
    ConnectionState, browser_controls, browser_text_entry_is_active, connection_note_class,
    keyboard_event_to_protocol_key, send_browser_client_message, should_capture_global_gameplay_key,
};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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
        relay_join_url,
    };
    use crate::fixtures::{GalleryFrame, SCENARIOS, scenario_frame};
    use crate::render::{
        build_track_window, marker_position, ordered_players_for_local_perspective,
    };
    use crate::session::{
        browser_controls, key_name_to_protocol_key, should_capture_global_gameplay_key,
    };
    use typekart_protocol::{
        NetworkRacePhase, PlayerId, ProtocolKey, RoomCode,
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


    fn scenario_race_with_finished_local() -> typekart_protocol::RaceSnapshot {
        let GalleryFrame::Race(mut racing) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        racing.players[0].finished = true;
        racing
    }
}
