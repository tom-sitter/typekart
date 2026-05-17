//! Transport helpers for moving protocol messages over byte streams.
//!
//! The current LAN implementation uses newline-delimited JSON over TCP. Keeping
//! that framing here gives future transports, such as WebSockets, a clear place
//! to adapt framing without changing the game protocol types.

use std::io::{BufRead, Write};

use anyhow::{Context, Result};

use super::protocol::{
    ClientMessage, ServerMessage, decode_client_message, decode_server_message,
    encode_client_message, encode_server_message,
};

pub fn write_client_message(writer: &mut impl Write, message: &ClientMessage) -> Result<()> {
    let encoded = encode_client_message(message).context("failed to encode client message")?;
    writeln!(writer, "{encoded}").context("failed to write client message")?;
    writer.flush().context("failed to flush client message")
}

pub fn write_server_message(writer: &mut impl Write, message: &ServerMessage) -> Result<()> {
    let encoded = encode_server_message(message).context("failed to encode server message")?;
    writeln!(writer, "{encoded}").context("failed to write server message")?;
    writer.flush().context("failed to flush server message")
}

pub fn read_client_message(reader: &mut impl BufRead) -> Result<Option<ClientMessage>> {
    let Some(line) = read_json_line(reader)? else {
        return Ok(None);
    };
    decode_client_message(line.trim_end())
        .context("failed to decode client message")
        .map(Some)
}

pub fn read_server_message(reader: &mut impl BufRead) -> Result<Option<ServerMessage>> {
    let Some(line) = read_json_line(reader)? else {
        return Ok(None);
    };
    decode_server_message(line.trim_end())
        .context("failed to decode server message")
        .map(Some)
}

fn read_json_line(reader: &mut impl BufRead) -> Result<Option<String>> {
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .context("failed to read framed message")?;
    if bytes == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        read_client_message, read_server_message, write_client_message, write_server_message,
    };
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
}
