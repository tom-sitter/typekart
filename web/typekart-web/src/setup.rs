use std::rc::Rc;

use futures_channel::mpsc::{UnboundedSender, unbounded};
use leptos::prelude::*;
use typekart_protocol::{ClientMessage, ClientSequence, PlayerId};
use wasm_bindgen_futures::spawn_local;

use crate::client::join_browser_room;
use crate::fixtures::GalleryFrame;
use crate::host::host_browser_lobby;
use crate::render::{BrowserLobbyManagement, LobbyPanel, RacePanel, ResultsPanel};
use crate::session::{
    BrowserHostSignals, BrowserOutboundMessage, BrowserSessionKind, BrowserSessionSignals,
    ConnectionState, browser_controls, browser_text_entry_is_active, connection_note_class,
    keyboard_event_to_protocol_key, send_browser_client_message, should_capture_global_gameplay_key,
};

#[component]
pub(crate) fn JoinRoomPanel(unicode_icons: ReadSignal<bool>) -> impl IntoView {
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
