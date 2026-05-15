//! Item types and item-specific helper rules.

use rand::{Rng, seq::SliceRandom};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldItem {
    Mushroom,
    Banana,
}

impl HeldItem {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mushroom => "Mushroom",
            Self::Banana => "Banana",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemPickup {
    Held(HeldItem),
    Shield,
}

impl ItemPickup {
    pub fn name(self) -> &'static str {
        match self {
            Self::Held(item) => item.name(),
            Self::Shield => "Shield",
        }
    }
}

pub fn roll_item(rng: &mut impl Rng) -> ItemPickup {
    [
        ItemPickup::Held(HeldItem::Mushroom),
        ItemPickup::Held(HeldItem::Banana),
        ItemPickup::Shield,
    ]
    .choose(rng)
    .copied()
    .expect("item table is non-empty")
}

pub fn roll_item_with_proximity(rng: &mut impl Rng, has_nearby_racer: bool) -> ItemPickup {
    if has_nearby_racer {
        [
            ItemPickup::Held(HeldItem::Mushroom),
            ItemPickup::Held(HeldItem::Banana),
            ItemPickup::Shield,
            ItemPickup::Shield,
            ItemPickup::Shield,
        ]
        .choose(rng)
        .copied()
        .expect("item table is non-empty")
    } else {
        roll_item(rng)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemUse {
    Normal,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetDirection {
    Behind,
    Ahead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RacerPosition {
    pub id: usize,
    pub word_index: usize,
}

pub fn banana_direction(item_use: ItemUse) -> TargetDirection {
    match item_use {
        ItemUse::Normal => TargetDirection::Behind,
        ItemUse::Modified => TargetDirection::Ahead,
    }
}

pub fn select_banana_target(
    current_word_index: usize,
    racers: &[RacerPosition],
    direction: TargetDirection,
    max_distance_words: usize,
) -> Option<RacerPosition> {
    racers
        .iter()
        .copied()
        .filter(|racer| match direction {
            TargetDirection::Behind => racer.word_index < current_word_index,
            TargetDirection::Ahead => racer.word_index > current_word_index,
        })
        .filter(|racer| current_word_index.abs_diff(racer.word_index) <= max_distance_words)
        .min_by_key(|racer| current_word_index.abs_diff(racer.word_index))
}

#[cfg(test)]
mod tests {
    use rand::{SeedableRng, rngs::StdRng};

    use super::{
        ItemPickup, RacerPosition, TargetDirection, roll_item_with_proximity, select_banana_target,
    };

    #[test]
    fn banana_targets_nearest_racer_behind_in_range() {
        let racers = [
            RacerPosition {
                id: 1,
                word_index: 4,
            },
            RacerPosition {
                id: 2,
                word_index: 8,
            },
        ];

        let target = select_banana_target(10, &racers, TargetDirection::Behind, 10);

        assert_eq!(target.unwrap().id, 2);
    }

    #[test]
    fn banana_targets_nearest_racer_ahead_in_range() {
        let racers = [
            RacerPosition {
                id: 1,
                word_index: 14,
            },
            RacerPosition {
                id: 2,
                word_index: 17,
            },
        ];

        let target = select_banana_target(10, &racers, TargetDirection::Ahead, 10);

        assert_eq!(target.unwrap().id, 1);
    }

    #[test]
    fn banana_ignores_racers_out_of_range() {
        let racers = [RacerPosition {
            id: 1,
            word_index: 25,
        }];

        let target = select_banana_target(10, &racers, TargetDirection::Ahead, 10);

        assert_eq!(target, None);
    }

    #[test]
    fn proximity_roll_can_return_shield() {
        let mut rng = StdRng::seed_from_u64(2);
        let mut saw_shield = false;

        for _ in 0..20 {
            if roll_item_with_proximity(&mut rng, true) == ItemPickup::Shield {
                saw_shield = true;
                break;
            }
        }

        assert!(saw_shield);
    }
}
