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

const STANDARD_ITEM_TABLE: [ItemPickup; 6] = [
    ItemPickup::Held(HeldItem::Mushroom),
    ItemPickup::Held(HeldItem::Banana),
    ItemPickup::Held(HeldItem::Mushroom),
    ItemPickup::Held(HeldItem::Banana),
    ItemPickup::Held(HeldItem::Mushroom),
    ItemPickup::Shield,
];

const NEARBY_RACER_ITEM_TABLE: [ItemPickup; 10] = [
    ItemPickup::Held(HeldItem::Mushroom),
    ItemPickup::Held(HeldItem::Banana),
    ItemPickup::Held(HeldItem::Mushroom),
    ItemPickup::Held(HeldItem::Banana),
    ItemPickup::Held(HeldItem::Mushroom),
    ItemPickup::Held(HeldItem::Banana),
    ItemPickup::Held(HeldItem::Mushroom),
    ItemPickup::Shield,
    ItemPickup::Shield,
    ItemPickup::Shield,
];

pub fn roll_item(rng: &mut impl Rng) -> ItemPickup {
    STANDARD_ITEM_TABLE
        .choose(rng)
        .copied()
        .expect("item table is non-empty")
}

pub fn roll_item_with_proximity(rng: &mut impl Rng, has_nearby_racer: bool) -> ItemPickup {
    if has_nearby_racer {
        NEARBY_RACER_ITEM_TABLE
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
pub struct RacerPosition {
    pub id: usize,
    pub word_index: usize,
}

/// Selects the closest valid Banana target, regardless of whether that racer
/// is ahead, behind, or exactly overlapping the user.
pub fn select_nearest_banana_target(
    current_word_index: usize,
    racers: &[RacerPosition],
    max_distance_words: usize,
) -> Option<RacerPosition> {
    racers
        .iter()
        .copied()
        .filter(|racer| current_word_index.abs_diff(racer.word_index) <= max_distance_words)
        .min_by_key(|racer| (current_word_index.abs_diff(racer.word_index), racer.id))
}

#[cfg(test)]
mod tests {
    use rand::{SeedableRng, rngs::StdRng};

    use super::{
        ItemPickup, NEARBY_RACER_ITEM_TABLE, RacerPosition, STANDARD_ITEM_TABLE,
        roll_item_with_proximity, select_nearest_banana_target,
    };

    #[test]
    fn banana_targets_nearest_racer_on_either_side_in_range() {
        let racers = [
            RacerPosition {
                id: 1,
                word_index: 8,
            },
            RacerPosition {
                id: 2,
                word_index: 11,
            },
        ];

        let target = select_nearest_banana_target(10, &racers, 10);

        assert_eq!(target.unwrap().id, 2);
    }

    #[test]
    fn banana_can_target_racer_on_same_word() {
        let racers = [
            RacerPosition {
                id: 1,
                word_index: 10,
            },
            RacerPosition {
                id: 2,
                word_index: 11,
            },
        ];

        let target = select_nearest_banana_target(10, &racers, 10);

        assert_eq!(target.unwrap().id, 1);
    }

    #[test]
    fn banana_ignores_racers_out_of_range() {
        let racers = [RacerPosition {
            id: 1,
            word_index: 25,
        }];

        let target = select_nearest_banana_target(10, &racers, 10);

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

    #[test]
    fn standard_item_table_has_reduced_shield_probability() {
        let shield_count = STANDARD_ITEM_TABLE
            .iter()
            .filter(|item| **item == ItemPickup::Shield)
            .count();

        assert_eq!(shield_count, 1);
        assert_eq!(STANDARD_ITEM_TABLE.len(), 6);
    }

    #[test]
    fn nearby_racer_item_table_keeps_reduced_shield_bias() {
        let shield_count = NEARBY_RACER_ITEM_TABLE
            .iter()
            .filter(|item| **item == ItemPickup::Shield)
            .count();

        assert_eq!(shield_count, 3);
        assert_eq!(NEARBY_RACER_ITEM_TABLE.len(), 10);
    }
}
