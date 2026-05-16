//! Item types and item-specific helper rules.

use std::{fs, path::Path};

use anyhow::{bail, Result};
use rand::Rng;
use serde::Deserialize;

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

    /// Load a host-provided item pack from JSON.
    ///
    /// This first external format intentionally tunes the built-in items only:
    /// weights, names, and enabled flags. Adding entirely new item effects needs
    /// the shared item engine first, because the current game still resolves
    /// Mushroom, Banana, and Shield through concrete Rust handlers.
    pub fn load_json_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)?;
        let config: ItemPackConfig = serde_json::from_str(&contents)?;
        Self::from_pack_config(config)
    }

    fn from_pack_config(config: ItemPackConfig) -> Result<Self> {
        let mut registry = Self::builtin();

        for override_item in config.items {
            let target = registry
                .items
                .iter_mut()
                .find(|item| item.id.as_str() == override_item.id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown item id '{}'; this modding slice can tune built-in items only",
                        override_item.id
                    )
                })?;

            if let Some(name) = override_item.name {
                if name.trim().is_empty() {
                    bail!("item '{}' cannot have an empty name", override_item.id);
                }
                target.name = name;
            }

            if let Some(enabled) = override_item.enabled {
                target.enabled = enabled;
            }

            if let Some(weight) = override_item.standard_weight {
                target.standard_weight = weight;
            }

            if let Some(weight) = override_item.nearby_racer_weight {
                target.nearby_racer_weight = weight;
            }
        }

        Self::new(registry.items)
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

#[cfg(test)]
pub fn roll_item_with_proximity(rng: &mut impl Rng, has_nearby_racer: bool) -> ItemPickup {
    ItemRegistry::builtin()
        .roll_pickup(rng, ItemRollContext { has_nearby_racer })
        .expect("built-in item registry has rollable items")
}

#[derive(Debug, Deserialize)]
struct ItemPackConfig {
    items: Vec<ItemPackItem>,
}

#[derive(Debug, Deserialize)]
struct ItemPackItem {
    id: String,
    name: Option<String>,
    enabled: Option<bool>,
    standard_weight: Option<u32>,
    nearby_racer_weight: Option<u32>,
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
    use rand::{rngs::StdRng, SeedableRng};

    use super::{
        roll_item_with_proximity, select_nearest_banana_target, HeldItem, ItemActivation,
        ItemDefinition, ItemPackConfig, ItemPackItem, ItemPickup, ItemRegistry, RacerPosition,
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

    #[test]
    fn item_pack_can_disable_a_builtin_item() {
        let config = ItemPackConfig {
            items: vec![ItemPackItem {
                id: "banana".to_string(),
                name: None,
                enabled: Some(false),
                standard_weight: None,
                nearby_racer_weight: None,
            }],
        };

        let registry = ItemRegistry::from_pack_config(config).unwrap();
        let banana = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Banana))
            .unwrap();

        assert!(!banana.enabled);
    }

    #[test]
    fn item_rolls_ignore_disabled_items() {
        let registry = ItemRegistry::new(vec![
            ItemDefinition {
                enabled: false,
                ..ItemDefinition::built_in(
                    "banana",
                    "Banana",
                    ItemPickup::Held(HeldItem::Banana),
                    ItemActivation::Held,
                    100,
                    100,
                )
            },
            ItemDefinition::built_in(
                "shield",
                "Shield",
                ItemPickup::Shield,
                ItemActivation::Immediate,
                1,
                1,
            ),
        ])
        .unwrap();
        let mut rng = StdRng::seed_from_u64(1);

        for _ in 0..10 {
            assert_eq!(
                registry.roll_pickup(
                    &mut rng,
                    super::ItemRollContext {
                        has_nearby_racer: false
                    }
                ),
                Some(ItemPickup::Shield)
            );
        }
    }

    #[test]
    fn item_pack_rejects_unknown_item_ids_until_new_effects_are_supported() {
        let config = ItemPackConfig {
            items: vec![ItemPackItem {
                id: "lightning".to_string(),
                name: None,
                enabled: None,
                standard_weight: Some(1),
                nearby_racer_weight: Some(1),
            }],
        };

        assert!(ItemRegistry::from_pack_config(config).is_err());
    }

    #[test]
    fn item_pack_file_loads_weight_overrides() {
        let path =
            std::env::temp_dir().join(format!("typekart-item-pack-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"{
                "items": [
                    {
                        "id": "mushroom",
                        "standard_weight": 10,
                        "nearby_racer_weight": 12
                    }
                ]
            }"#,
        )
        .unwrap();

        let registry = ItemRegistry::load_json_file(&path).unwrap();
        let mushroom = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Mushroom))
            .unwrap();

        assert_eq!(mushroom.standard_weight, 10);
        assert_eq!(mushroom.nearby_racer_weight, 12);

        let _ = std::fs::remove_file(path);
    }
}
