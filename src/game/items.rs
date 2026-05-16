//! Item types and item-specific helper rules.

use anyhow::{Result, bail};
use rand::Rng;

use super::mods::ContentId;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemActivation {
    Immediate,
    Held,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDefinition {
    pub id: ContentId,
    pub name: String,
    pub pickup: ItemPickup,
    pub activation: ItemActivation,
    pub standard_weight: u32,
    pub nearby_racer_weight: u32,
    pub enabled: bool,
}

impl ItemDefinition {
    pub fn built_in(
        id: &'static str,
        name: &'static str,
        pickup: ItemPickup,
        activation: ItemActivation,
        standard_weight: u32,
        nearby_racer_weight: u32,
    ) -> Self {
        Self {
            id: ContentId::builtin(id),
            name: name.to_string(),
            pickup,
            activation,
            standard_weight,
            nearby_racer_weight,
            enabled: true,
        }
    }

    fn weight(&self, has_nearby_racer: bool) -> u32 {
        if has_nearby_racer {
            self.nearby_racer_weight
        } else {
            self.standard_weight
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemRollContext {
    pub has_nearby_racer: bool,
}

#[derive(Debug, Clone)]
pub struct ItemRegistry {
    pub items: Vec<ItemDefinition>,
}

impl ItemRegistry {
    pub fn new(items: Vec<ItemDefinition>) -> Result<Self> {
        if items.is_empty() {
            bail!("item registry must contain at least one item");
        }

        for (index, item) in items.iter().enumerate() {
            if items
                .iter()
                .skip(index + 1)
                .any(|other| other.id == item.id)
            {
                bail!("duplicate item id '{}'", item.id.as_str());
            }
        }

        if !items
            .iter()
            .any(|item| item.enabled && item.standard_weight > 0)
        {
            bail!("item registry must contain at least one standard item with a positive weight");
        }

        Ok(Self { items })
    }

    pub fn builtin() -> Self {
        Self::new(vec![
            ItemDefinition::built_in(
                "mushroom",
                "Mushroom",
                ItemPickup::Held(HeldItem::Mushroom),
                ItemActivation::Held,
                3,
                4,
            ),
            ItemDefinition::built_in(
                "banana",
                "Banana",
                ItemPickup::Held(HeldItem::Banana),
                ItemActivation::Held,
                2,
                3,
            ),
            ItemDefinition::built_in(
                "shield",
                "Shield",
                ItemPickup::Shield,
                ItemActivation::Immediate,
                1,
                3,
            ),
        ])
        .expect("built-in item registry is valid")
    }

    pub fn roll_pickup(&self, rng: &mut impl Rng, context: ItemRollContext) -> Option<ItemPickup> {
        let candidates = self
            .items
            .iter()
            .filter(|item| item.enabled)
            .filter_map(|item| {
                let weight = item.weight(context.has_nearby_racer);
                (weight > 0).then_some((item, weight))
            })
            .collect::<Vec<_>>();
        let total_weight = candidates.iter().map(|(_, weight)| *weight).sum::<u32>();

        if total_weight == 0 {
            return None;
        }

        let mut roll = rng.gen_range(0..total_weight);
        for (item, weight) in candidates {
            if roll < weight {
                return Some(item.pickup);
            }
            roll -= weight;
        }

        None
    }
}

pub fn roll_item_with_proximity(rng: &mut impl Rng, has_nearby_racer: bool) -> ItemPickup {
    ItemRegistry::builtin()
        .roll_pickup(rng, ItemRollContext { has_nearby_racer })
        .expect("built-in item registry has rollable items")
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
        HeldItem, ItemActivation, ItemDefinition, ItemPickup, ItemRegistry, RacerPosition,
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
        let registry = ItemRegistry::builtin();
        let shield = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Shield)
            .unwrap();
        let total_weight = registry
            .items
            .iter()
            .map(|item| item.standard_weight)
            .sum::<u32>();

        assert_eq!(shield.standard_weight, 1);
        assert_eq!(total_weight, 6);
    }

    #[test]
    fn nearby_racer_item_table_keeps_reduced_shield_bias() {
        let registry = ItemRegistry::builtin();
        let shield = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Shield)
            .unwrap();
        let total_weight = registry
            .items
            .iter()
            .map(|item| item.nearby_racer_weight)
            .sum::<u32>();

        assert_eq!(shield.nearby_racer_weight, 3);
        assert_eq!(total_weight, 10);
    }

    #[test]
    fn registry_rejects_duplicate_item_ids() {
        let result = ItemRegistry::new(vec![
            ItemDefinition::built_in(
                "banana",
                "Banana",
                ItemPickup::Held(HeldItem::Banana),
                ItemActivation::Held,
                1,
                1,
            ),
            ItemDefinition::built_in(
                "banana",
                "Banana 2",
                ItemPickup::Held(HeldItem::Banana),
                ItemActivation::Held,
                1,
                1,
            ),
        ]);

        assert!(result.is_err());
    }
}
