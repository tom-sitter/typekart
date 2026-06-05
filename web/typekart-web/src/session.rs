use futures_channel::mpsc::UnboundedSender;
use leptos::prelude::*;
use typekart_protocol::{ClientMessage, PlayerId, ProtocolKey};

use crate::fixtures::GalleryFrame;

#[derive(Debug, Clone)]
pub(crate) enum BrowserOutboundMessage {
    Client {
        player_id: PlayerId,
        message: ClientMessage,
    },
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserSessionKind {
    Joiner,
    Host,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BrowserSessionSignals {
    pub(crate) set_connection: WriteSignal<ConnectionState>,
    pub(crate) set_live_frame: WriteSignal<Option<GalleryFrame>>,
    pub(crate) set_relay_player_id: WriteSignal<Option<PlayerId>>,
    pub(crate) set_game_player_id: WriteSignal<Option<PlayerId>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BrowserHostSignals {
    pub(crate) session: BrowserSessionSignals,
    pub(crate) set_room_code: WriteSignal<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    Disconnected,
    Connecting,
    Connected { message: String },
    Closed { reason: String },
    Failed { message: String },
}

impl ConnectionState {
    pub(crate) fn is_active(&self) -> bool {
        matches!(self, Self::Connecting | Self::Connected { .. })
    }

    pub(crate) fn label(&self) -> String {
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
pub(crate) struct BrowserCommandSink {
    pub(crate) outbound: Option<UnboundedSender<BrowserOutboundMessage>>,
    pub(crate) relay_player_id: Option<PlayerId>,
    pub(crate) set_connection: WriteSignal<ConnectionState>,
}

impl BrowserCommandSink {
    pub(crate) fn send(&self, message: ClientMessage, success_status: &'static str) {
        send_browser_client_message(
            self.outbound.clone(),
            self.relay_player_id,
            message,
            success_status,
            self.set_connection,
        );
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BrowserControls {
    pub(crate) show_ready: bool,
    pub(crate) show_unready: bool,
    pub(crate) show_start: bool,
    pub(crate) show_rematch_ready: bool,
}

pub(crate) fn browser_controls(
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

pub(crate) fn should_capture_global_gameplay_key(
    frame: Option<&GalleryFrame>,
    local_player_id: Option<PlayerId>,
) -> bool {
    let Some(local_player_id) = local_player_id else {
        return false;
    };
    let Some(GalleryFrame::Race(snapshot)) = frame else {
        return false;
    };
    snapshot.phase == typekart_protocol::NetworkRacePhase::Racing
        && snapshot
            .players
            .iter()
            .any(|player| player.id == local_player_id && player.connected && !player.finished)
}

pub(crate) fn browser_text_entry_is_active() -> bool {
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

pub(crate) fn connection_note_class(connection: &ConnectionState) -> &'static str {
    match connection {
        ConnectionState::Closed { .. } | ConnectionState::Failed { .. } => "note error",
        _ => "note",
    }
}

pub(crate) fn send_browser_client_message(
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

pub(crate) fn keyboard_event_to_protocol_key(
    event: &leptos::ev::KeyboardEvent,
) -> Option<ProtocolKey> {
    key_name_to_protocol_key(&event.key())
}

pub(crate) fn key_name_to_protocol_key(key: &str) -> Option<ProtocolKey> {
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

#[cfg(test)]
mod tests;
