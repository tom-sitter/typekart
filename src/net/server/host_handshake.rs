//! Network-host join handshake adapter.
//!
//! The main server accept loop owns socket lifetime. This module owns the
//! first-message protocol contract: clients must send `Hello`, and accepted
//! joiners receive `Welcome`.

use std::{io::BufReader, net::TcpStream};

use anyhow::{Context, Result, bail};

use crate::net::{
    protocol::{AssignedColor, ClientMessage, PlayerId, ServerMessage},
    transport::{read_client_message, write_server_message},
};

use super::send_server_message;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct JoinHello {
    pub(super) name: String,
    pub(super) client_version: String,
}

pub(super) fn read_join_hello(stream: &TcpStream) -> Result<JoinHello> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .context("failed to clone client stream for reading")?,
    );
    let Some(message) = read_client_message(&mut reader).context("failed to read client hello")?
    else {
        bail!("client disconnected before hello");
    };
    let ClientMessage::Hello {
        name,
        client_version,
    } = message
    else {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Expected hello message".to_string(),
            },
        )?;
        bail!("client sent non-hello first message");
    };

    if name.trim().is_empty() {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Name cannot be empty".to_string(),
            },
        )?;
        bail!("client sent empty name");
    }

    if client_version.trim().is_empty() {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Client version cannot be empty".to_string(),
            },
        )?;
        bail!("client sent empty version");
    }

    Ok(JoinHello {
        name: name.trim().to_string(),
        client_version: client_version.trim().to_string(),
    })
}

pub(super) fn welcome_joiner(
    stream: &TcpStream,
    player_id: PlayerId,
    assigned_color: AssignedColor,
) -> Result<TcpStream> {
    let mut write_stream = stream
        .try_clone()
        .context("failed to clone client stream for writing")?;
    write_server_message(
        &mut write_stream,
        &ServerMessage::Welcome {
            player_id,
            assigned_color,
        },
    )?;

    Ok(write_stream)
}
