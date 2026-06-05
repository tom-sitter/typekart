use std::io::Cursor;

use super::{read_client_message, read_server_message, write_client_message, write_server_message};
use crate::net::protocol::{
    AssignedColor, ClientMessage, ClientSequence, PlayerId, ProtocolKey, ServerMessage,
};

#[test]
fn client_messages_round_trip_over_json_line_transport() {
    let message = ClientMessage::KeyInput {
        sequence: ClientSequence(7),
        key: ProtocolKey::Char('x'),
    };
    let mut bytes = Vec::new();

    write_client_message(&mut bytes, &message).unwrap();
    let decoded = read_client_message(&mut Cursor::new(bytes)).unwrap();

    assert_eq!(decoded, Some(message));
}

#[test]
fn server_messages_round_trip_over_json_line_transport() {
    let message = ServerMessage::Welcome {
        player_id: PlayerId(2),
        assigned_color: AssignedColor::Red,
    };
    let mut bytes = Vec::new();

    write_server_message(&mut bytes, &message).unwrap();
    let decoded = read_server_message(&mut Cursor::new(bytes)).unwrap();

    assert_eq!(decoded, Some(message));
}

#[test]
fn reading_empty_transport_returns_none() {
    let decoded = read_server_message(&mut Cursor::new(Vec::<u8>::new())).unwrap();

    assert_eq!(decoded, None);
}
