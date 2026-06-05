//! Shared host-visible events.
//!
//! These events describe authoritative game outcomes without tying the game
//! rules to a specific UI, socket protocol, or log format. Adapters can keep
//! using [`HostEvent::message`] for simple event feeds while richer renderers
//! can inspect the structured variants directly.

use super::{
    items::{HeldItem, ItemPickup},
    race::RacePlayerId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    PlayerFinished {
        placement: usize,
        name: String,
    },
    RaceFinished,
    ItemPickedUp {
        player_id: RacePlayerId,
        player_name: String,
        item: ItemPickup,
    },
    BonusMissed {
        player_id: RacePlayerId,
        player_name: String,
    },
    ItemMissed {
        player_id: RacePlayerId,
        player_name: String,
        item: HeldItem,
    },
    ItemHit {
        attacker_id: RacePlayerId,
        attacker_name: String,
        target_id: RacePlayerId,
        target_name: String,
        item: HeldItem,
    },
    ItemBlocked {
        target_id: RacePlayerId,
        target_name: String,
        item: HeldItem,
    },
    FogHit {
        attacker_id: RacePlayerId,
        attacker_name: String,
        hit_count: usize,
    },
}

impl HostEvent {
    pub fn message(&self) -> String {
        match self {
            Self::PlayerFinished { placement, name } => format!("{placement}. {name} finished"),
            Self::RaceFinished => "Race finished".to_string(),
            Self::ItemPickedUp {
                player_name, item, ..
            } => format!("{player_name} got {}", item_pickup_name(*item)),
            Self::BonusMissed { player_name, .. } => {
                format!("{player_name} missed the bonus")
            }
            Self::ItemMissed {
                player_name, item, ..
            } => format!("{player_name} missed {}", item.name()),
            Self::ItemHit {
                attacker_name,
                target_name,
                item,
                ..
            } => match item {
                HeldItem::Banana => format!("{attacker_name} hit {target_name}"),
                _ => format!("{attacker_name} hit {target_name} with {}", item.name()),
            },
            Self::ItemBlocked {
                target_name, item, ..
            } => format!("{target_name} blocked {}", item.name()),
            Self::FogHit {
                attacker_name,
                hit_count,
                ..
            } => format!("{attacker_name} fogged {hit_count} racer(s)"),
        }
    }
}

pub fn push_capped_event(events: &mut Vec<String>, event: impl Into<String>, capacity: usize) {
    if capacity == 0 {
        return;
    }

    events.push(event.into());
    if events.len() > capacity {
        events.drain(0..events.len() - capacity);
    }
}

fn item_pickup_name(item: ItemPickup) -> &'static str {
    match item {
        ItemPickup::Held(held_item) => held_item.name(),
        ItemPickup::Shield => "Shield",
    }
}

#[cfg(test)]
mod tests {
    use super::push_capped_event;

    #[test]
    fn capped_events_keep_recent_entries() {
        let mut events = Vec::new();

        push_capped_event(&mut events, "one", 2);
        push_capped_event(&mut events, "two", 2);
        push_capped_event(&mut events, "three", 2);

        assert_eq!(events, vec!["two".to_string(), "three".to_string()]);
    }

    #[test]
    fn capped_events_ignore_zero_capacity() {
        let mut events = Vec::new();

        push_capped_event(&mut events, "one", 0);

        assert!(events.is_empty());
    }
}
