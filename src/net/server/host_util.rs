//! Small network-host utility helpers.

use anyhow::{Result, bail};

use crate::{
    game::{lobby::LOBBY_COLOR_ROTATION, race::PlayerColorId},
    net::protocol::{AssignedColor, LobbyPlayer, PlayerId},
};

use super::print_server_line;

pub(super) fn validate_host_capacity(max_players: usize, ai_racer_count: usize) -> Result<()> {
    if max_players == 0 || max_players > LOBBY_COLOR_ROTATION.len() {
        bail!(
            "max players must be between 1 and {}",
            LOBBY_COLOR_ROTATION.len()
        );
    }
    if ai_racer_count >= max_players {
        bail!("ai racers must be less than max players so the host has a slot");
    }
    Ok(())
}

pub(super) fn print_lobby_snapshot(players: &[LobbyPlayer]) {
    print_server_line("Lobby:");
    for player in players {
        print_server_line(format!(
            "  {}: {} ({:?}){}{}{}",
            player.id.0,
            player.name,
            player.color,
            if player.ready { " ready" } else { "" },
            if player.connected {
                ""
            } else {
                " disconnected"
            },
            if player.id == PlayerId(1) {
                " host"
            } else {
                ""
            }
        ));
    }
}

impl From<AssignedColor> for PlayerColorId {
    fn from(value: AssignedColor) -> Self {
        match value {
            AssignedColor::Cyan => Self::Cyan,
            AssignedColor::Red => Self::Red,
            AssignedColor::Green => Self::Green,
            AssignedColor::Blue => Self::Blue,
            AssignedColor::Yellow => Self::Yellow,
            AssignedColor::Magenta => Self::Magenta,
        }
    }
}

impl From<PlayerColorId> for AssignedColor {
    fn from(value: PlayerColorId) -> Self {
        match value {
            PlayerColorId::Cyan => Self::Cyan,
            PlayerColorId::Red => Self::Red,
            PlayerColorId::Green => Self::Green,
            PlayerColorId::Blue => Self::Blue,
            PlayerColorId::Yellow => Self::Yellow,
            PlayerColorId::Magenta => Self::Magenta,
        }
    }
}
