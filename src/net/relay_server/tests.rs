use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use super::{
    ConnectionRole, RelayLimits, RelayShared, allow_message_from_ip, allow_room_create_from_ip,
    allow_room_join_from_ip, cleanup_connection, cleanup_stale_rooms, handle_relay_message,
    idle_sweep_interval, spawn_idle_room_sweeper,
};
use crate::net::{
    protocol::PlayerId,
    relay::{RelayClientMessage, RelayServerMessage},
};

fn relay_shared() -> Arc<RelayShared> {
    Arc::new(RelayShared {
        rooms: Default::default(),
        rates: Default::default(),
        redis_routing: None,
    })
}

fn localhost() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

#[test]
fn relay_creates_room_and_forwards_join_to_host() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);

    let Some((room, ConnectionRole::Host)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    assert!(matches!(
        host_rx.recv().unwrap(),
        RelayServerMessage::RoomCreated { .. }
    ));

    let joined = handle_relay_message(
        RelayClientMessage::JoinRoom {
            room: room.clone(),
            name: "joiner".to_string(),
            client_version: "test".to_string(),
        },
        &state,
        &joiner_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap();

    assert_eq!(
        joined,
        Some((room.clone(), ConnectionRole::Participant(PlayerId(2))))
    );
    assert_eq!(
        host_rx.recv().unwrap(),
        RelayServerMessage::JoinForwarded {
            room,
            pending_player_id: PlayerId(2),
            name: "joiner".to_string(),
            client_version: "test".to_string(),
        }
    );
}

#[test]
fn relay_tracks_connection_limits_by_ip() {
    let state = relay_shared();
    let limits = RelayLimits {
        max_connections: 2,
        max_connections_per_ip: 1,
        ..RelayLimits::default()
    };

    let first = super::try_register_connection(&state, localhost(), &limits).unwrap();
    assert!(super::try_register_connection(&state, localhost(), &limits).is_err());
    drop(first);
    assert!(super::try_register_connection(&state, localhost(), &limits).is_ok());
}

#[test]
fn relay_rate_limits_room_creation_by_ip() {
    let state = relay_shared();
    let limits = RelayLimits {
        max_room_creates_per_minute_per_ip: 1,
        ..RelayLimits::default()
    };

    assert!(allow_room_create_from_ip(&state, localhost(), &limits));
    assert!(!allow_room_create_from_ip(&state, localhost(), &limits));
}

#[test]
fn relay_rate_limits_joins_by_ip() {
    let state = relay_shared();
    let limits = RelayLimits {
        max_room_joins_per_minute_per_ip: 1,
        ..RelayLimits::default()
    };

    assert!(allow_room_join_from_ip(&state, localhost(), &limits));
    assert!(!allow_room_join_from_ip(&state, localhost(), &limits));
}

#[test]
fn relay_rate_limits_messages_by_ip() {
    let state = relay_shared();
    let limits = RelayLimits {
        max_messages_per_second_per_ip: 1,
        ..RelayLimits::default()
    };

    assert!(allow_message_from_ip(&state, localhost(), &limits));
    assert!(!allow_message_from_ip(&state, localhost(), &limits));
}

#[test]
fn relay_routes_client_messages_to_host() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);
    let Some((room, _)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();
    handle_relay_message(
        RelayClientMessage::JoinRoom {
            room: room.clone(),
            name: "joiner".to_string(),
            client_version: "test".to_string(),
        },
        &state,
        &joiner_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap();
    let _ = host_rx.recv().unwrap();

    let message = serde_json::json!({
        "type": "future_client_command",
        "payload": { "sequence": 7 }
    });
    handle_relay_message(
        RelayClientMessage::ClientToHost {
            room: room.clone(),
            player_id: PlayerId(2),
            message: message.clone(),
        },
        &state,
        &joiner_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap();

    assert_eq!(
        host_rx.recv().unwrap(),
        RelayServerMessage::ClientToHost {
            room,
            player_id: PlayerId(2),
            message,
        }
    );
}

#[test]
fn relay_rejects_join_when_client_version_differs_from_host() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let (joiner_tx, joiner_rx) = mpsc::sync_channel(32);

    let Some((room, ConnectionRole::Host)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "1.2.3".to_string(),
        },
        &state,
        &host_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();

    let joined = handle_relay_message(
        RelayClientMessage::JoinRoom {
            room,
            name: "joiner".to_string(),
            client_version: "1.2.4".to_string(),
        },
        &state,
        &joiner_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap();

    assert_eq!(joined, None);
    assert!(matches!(
        joiner_rx.recv().unwrap(),
        RelayServerMessage::Error { message } if message.contains("Version mismatch")
            && message.contains("room is running TypeKart 1.2.3")
            && message.contains("you are running TypeKart 1.2.4")
            && message.contains("1.2.3")
            && message.contains("1.2.4")
    ));
    assert!(host_rx.try_recv().is_err());
}

#[test]
fn relay_broadcasts_host_messages_once_per_participant() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let (joiner_tx, joiner_rx) = mpsc::sync_channel(32);
    let Some((room, _)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();
    handle_relay_message(
        RelayClientMessage::JoinRoom {
            room: room.clone(),
            name: "joiner".to_string(),
            client_version: "test".to_string(),
        },
        &state,
        &joiner_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap();

    let message = serde_json::json!({
        "type": "future_server_command",
        "payload": { "message": "test" }
    });
    handle_relay_message(
        RelayClientMessage::HostBroadcast {
            room: room.clone(),
            message: message.clone(),
        },
        &state,
        &host_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap();

    assert_eq!(
        joiner_rx.try_recv().unwrap(),
        RelayServerMessage::HostBroadcast { room, message }
    );
    assert!(joiner_rx.try_recv().is_err());
}

#[test]
fn relay_closes_room_when_host_disconnects() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let (joiner_tx, joiner_rx) = mpsc::sync_channel(32);
    let Some((room, _)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();
    handle_relay_message(
        RelayClientMessage::JoinRoom {
            room: room.clone(),
            name: "joiner".to_string(),
            client_version: "test".to_string(),
        },
        &state,
        &joiner_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap();

    cleanup_connection(&state, &room, ConnectionRole::Host);

    assert!(matches!(
        joiner_rx.recv().unwrap(),
        RelayServerMessage::RoomClosed { .. }
    ));
    assert!(!state.rooms.lock().unwrap().rooms.contains_key(&room));
}

#[test]
fn relay_notifies_host_when_participant_disconnects() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);
    let Some((room, _)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();
    let Some((_, ConnectionRole::Participant(player_id))) = handle_relay_message(
        RelayClientMessage::JoinRoom {
            room: room.clone(),
            name: "joiner".to_string(),
            client_version: "test".to_string(),
        },
        &state,
        &joiner_tx,
        None,
        &RelayLimits::default(),
    )
    .unwrap() else {
        panic!("joiner should enter room");
    };
    let _ = host_rx.recv().unwrap();

    cleanup_connection(&state, &room, ConnectionRole::Participant(player_id));

    assert!(matches!(
        host_rx.recv().unwrap(),
        RelayServerMessage::ParticipantDisconnected {
            room: disconnected_room,
            player_id: disconnected_player,
        } if disconnected_room == room && disconnected_player == player_id
    ));
    assert!(
        !state
            .rooms
            .lock()
            .unwrap()
            .rooms
            .get(&room)
            .unwrap()
            .participants
            .contains_key(&player_id)
    );
}

#[test]
fn relay_rejects_room_creation_when_room_limit_is_reached() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let limits = RelayLimits {
        max_rooms: 1,
        ..RelayLimits::default()
    };

    let Some((_, ConnectionRole::Host)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &limits,
    )
    .unwrap() else {
        panic!("first host should create a room");
    };
    let _ = host_rx.recv().unwrap();

    assert_eq!(
        handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &limits,
        )
        .unwrap(),
        None
    );
    assert!(matches!(
        host_rx.recv().unwrap(),
        RelayServerMessage::Error { message } if message.contains("Relay is full")
    ));
}

#[test]
fn relay_rejects_join_when_room_is_full() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);
    let (extra_tx, extra_rx) = mpsc::sync_channel(32);
    let limits = RelayLimits {
        max_participants_per_room: 1,
        ..RelayLimits::default()
    };

    let Some((room, _)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &limits,
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();
    handle_relay_message(
        RelayClientMessage::JoinRoom {
            room: room.clone(),
            name: "joiner".to_string(),
            client_version: "test".to_string(),
        },
        &state,
        &joiner_tx,
        None,
        &limits,
    )
    .unwrap();
    let _ = host_rx.recv().unwrap();

    assert_eq!(
        handle_relay_message(
            RelayClientMessage::JoinRoom {
                room,
                name: "extra".to_string(),
                client_version: "test".to_string(),
            },
            &state,
            &extra_tx,
            None,
            &limits,
        )
        .unwrap(),
        None
    );
    assert!(matches!(
        extra_rx.recv().unwrap(),
        RelayServerMessage::Error { message } if message.contains("is full")
    ));
}

#[test]
fn relay_cleans_up_idle_rooms() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let limits = RelayLimits::default();
    let Some((room, _)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &limits,
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();

    state
        .rooms
        .lock()
        .unwrap()
        .rooms
        .get_mut(&room)
        .unwrap()
        .last_activity = Instant::now() - Duration::from_secs(5);

    cleanup_stale_rooms(&state, Duration::from_secs(1));

    assert!(!state.rooms.lock().unwrap().rooms.contains_key(&room));
}

#[test]
fn idle_sweep_interval_is_bounded() {
    assert_eq!(
        idle_sweep_interval(Duration::from_secs(1)),
        Duration::from_secs(1)
    );
    assert_eq!(
        idle_sweep_interval(Duration::from_secs(80)),
        Duration::from_secs(20)
    );
    assert_eq!(
        idle_sweep_interval(Duration::from_secs(1000)),
        Duration::from_secs(60)
    );
}

#[test]
fn health_check_request_matches_only_plain_health_http() {
    assert!(super::is_health_check_request(
        "GET /healthz HTTP/1.1\r\nHost: relay\r\n\r\n"
    ));
    assert!(super::is_health_check_request(
        "HEAD /healthz HTTP/1.1\r\nHost: relay\r\n\r\n"
    ));
    assert!(!super::is_health_check_request(
        "GET / HTTP/1.1\r\nHost: relay\r\n\r\n"
    ));
    assert!(!super::is_health_check_request(
        "GET /healthz HTTP/1.1\r\nHost: relay\r\nUpgrade: websocket\r\n\r\n"
    ));
}

#[test]
fn websocket_room_query_extracts_join_routing_room() {
    let room = super::websocket_room_query(
        "GET /?typekart_room=rocket-salad-tiger HTTP/1.1\r\nHost: relay\r\n\r\n",
    )
    .expect("room query should parse");

    assert_eq!(room.display(), "rocket-salad-tiger");
    assert!(super::websocket_room_query("GET / HTTP/1.1\r\nHost: relay\r\n\r\n").is_none());
}

#[test]
fn idle_room_sweeper_removes_stale_rooms_without_new_messages() {
    let state = relay_shared();
    let (host_tx, host_rx) = mpsc::sync_channel(32);
    let limits = RelayLimits::default();
    let Some((room, _)) = handle_relay_message(
        RelayClientMessage::CreateRoom {
            host_version: "test".to_string(),
        },
        &state,
        &host_tx,
        None,
        &limits,
    )
    .unwrap() else {
        panic!("host should create a room");
    };
    let _ = host_rx.recv().unwrap();

    state
        .rooms
        .lock()
        .unwrap()
        .rooms
        .get_mut(&room)
        .unwrap()
        .last_activity = Instant::now() - Duration::from_secs(10);

    spawn_idle_room_sweeper(Arc::clone(&state), Duration::from_millis(1));
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if !state.rooms.lock().unwrap().rooms.contains_key(&room) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("idle sweeper did not remove stale room");
}
