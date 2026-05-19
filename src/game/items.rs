//! Item types and item-specific helper rules.

use std::{fs, path::Path};

use anyhow::{Result, bail};
use rand::Rng;
use serde::Deserialize;

use super::mods::ContentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldItem {
    Mushroom,
    Banana,
    Focus,
    Cyclone,
    SquidInk,
}

impl HeldItem {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mushroom => "Mushroom",
            Self::Banana => "Banana",
            Self::Focus => "Focus",
            Self::Cyclone => "Cyclone",
            Self::SquidInk => "Squid Ink",
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
    pub context_weights: ItemContextWeights,
    pub effect: ItemEffectConfig,
    pub display: ItemDisplayConfig,
    pub standard_weight: u32,
    pub nearby_racer_weight: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemEffectConfig {
    pub mushroom: Option<MushroomEffectConfig>,
    pub banana: Option<BananaEffectConfig>,
    pub shield: Option<ShieldEffectConfig>,
    pub focus: Option<FocusEffectConfig>,
    pub cyclone: Option<CycloneEffectConfig>,
    pub squid_ink: Option<SquidInkEffectConfig>,
}

impl ItemEffectConfig {
    fn for_pickup(pickup: ItemPickup) -> Self {
        match pickup {
            ItemPickup::Held(HeldItem::Mushroom) => Self {
                mushroom: Some(MushroomEffectConfig::default()),
                banana: None,
                shield: None,
                focus: None,
                cyclone: None,
                squid_ink: None,
            },
            ItemPickup::Held(HeldItem::Banana) => Self {
                mushroom: None,
                banana: Some(BananaEffectConfig::default()),
                shield: None,
                focus: None,
                cyclone: None,
                squid_ink: None,
            },
            ItemPickup::Held(HeldItem::Focus) => Self {
                mushroom: None,
                banana: None,
                shield: None,
                focus: Some(FocusEffectConfig::default()),
                cyclone: None,
                squid_ink: None,
            },
            ItemPickup::Held(HeldItem::Cyclone) => Self {
                mushroom: None,
                banana: None,
                shield: None,
                focus: None,
                cyclone: Some(CycloneEffectConfig::default()),
                squid_ink: None,
            },
            ItemPickup::Held(HeldItem::SquidInk) => Self {
                mushroom: None,
                banana: None,
                shield: None,
                focus: None,
                cyclone: None,
                squid_ink: Some(SquidInkEffectConfig::default()),
            },
            ItemPickup::Shield => Self {
                mushroom: None,
                banana: None,
                shield: Some(ShieldEffectConfig::default()),
                focus: None,
                cyclone: None,
                squid_ink: None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct MushroomEffectConfig {
    pub boost_words: usize,
    pub wpm: u32,
}

impl Default for MushroomEffectConfig {
    fn default() -> Self {
        Self {
            boost_words: 3,
            wpm: 180,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct BananaEffectConfig {
    pub range_words: usize,
    pub stun_ms: u64,
    pub impact_blink_ms: u64,
    pub cue_ms: u64,
}

impl Default for BananaEffectConfig {
    fn default() -> Self {
        Self {
            range_words: 10,
            stun_ms: 2_000,
            impact_blink_ms: 1_200,
            cue_ms: 1_500,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ShieldEffectConfig {
    pub duration_ms: u64,
}

impl Default for ShieldEffectConfig {
    fn default() -> Self {
        Self { duration_ms: 5_000 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct FocusEffectConfig {
    pub duration_ms: u64,
    pub ai_wpm_boost: u32,
}

impl Default for FocusEffectConfig {
    fn default() -> Self {
        Self {
            duration_ms: 10_000,
            ai_wpm_boost: 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct CycloneEffectConfig {
    pub affected_words: usize,
    pub stun_ms: u64,
}

impl Default for CycloneEffectConfig {
    fn default() -> Self {
        Self {
            affected_words: 1,
            stun_ms: 2_500,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct SquidInkEffectConfig {
    pub range_words: usize,
    pub duration_ms: u64,
    pub impact_blink_ms: u64,
    pub cue_ms: u64,
    pub ai_wpm_multiplier_percent: u32,
}

impl Default for SquidInkEffectConfig {
    fn default() -> Self {
        Self {
            range_words: 5,
            duration_ms: 5_000,
            impact_blink_ms: 1_200,
            cue_ms: 1_500,
            ai_wpm_multiplier_percent: 70,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDisplayConfig {
    pub banana: Option<BananaDisplayConfig>,
}

impl ItemDisplayConfig {
    fn for_pickup(pickup: ItemPickup) -> Self {
        match pickup {
            ItemPickup::Held(HeldItem::Banana) => Self {
                banana: Some(BananaDisplayConfig::default()),
            },
            ItemPickup::Held(HeldItem::Mushroom)
            | ItemPickup::Held(HeldItem::Focus)
            | ItemPickup::Held(HeldItem::Cyclone)
            | ItemPickup::Held(HeldItem::SquidInk)
            | ItemPickup::Shield => Self { banana: None },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BananaDisplayConfig {
    pub ascii_ahead: String,
    pub ascii_behind: String,
    pub ascii_overlap: String,
    pub unicode_ahead: String,
    pub unicode_behind: String,
    pub unicode_overlap: String,
}

impl Default for BananaDisplayConfig {
    fn default() -> Self {
        Self {
            ascii_ahead: " ))>>".to_string(),
            ascii_behind: "((<< ".to_string(),
            ascii_overlap: " ))<>".to_string(),
            unicode_ahead: " 🍌 >>".to_string(),
            unicode_behind: "<< 🍌 ".to_string(),
            unicode_overlap: " 🍌 <>".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemContextWeights {
    pub standard: PositionWeights,
    pub nearby_racer: PositionWeights,
}

impl ItemContextWeights {
    #[cfg(test)]
    fn from_flat(standard_weight: u32, nearby_racer_weight: u32) -> Self {
        Self {
            standard: PositionWeights::flat(standard_weight),
            nearby_racer: PositionWeights::flat(nearby_racer_weight),
        }
    }

    fn has_positive_weight(self) -> bool {
        self.standard.has_positive_weight() || self.nearby_racer.has_positive_weight()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct PositionWeights {
    pub first: u32,
    pub middle: u32,
    pub trailing: u32,
}

impl PositionWeights {
    fn flat(weight: u32) -> Self {
        Self {
            first: weight,
            middle: weight,
            trailing: weight,
        }
    }

    fn has_positive_weight(self) -> bool {
        self.first > 0 || self.middle > 0 || self.trailing > 0
    }
}

impl ItemDefinition {
    #[cfg(test)]
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
            context_weights: ItemContextWeights::from_flat(standard_weight, nearby_racer_weight),
            effect: ItemEffectConfig::for_pickup(pickup),
            display: ItemDisplayConfig::for_pickup(pickup),
            standard_weight,
            nearby_racer_weight,
            enabled: true,
        }
    }

    fn built_in_with_context(
        id: &'static str,
        name: &'static str,
        pickup: ItemPickup,
        activation: ItemActivation,
        standard_weight: u32,
        nearby_racer_weight: u32,
        context_weights: ItemContextWeights,
    ) -> Self {
        Self {
            id: ContentId::builtin(id),
            name: name.to_string(),
            pickup,
            activation,
            context_weights,
            effect: ItemEffectConfig::for_pickup(pickup),
            display: ItemDisplayConfig::for_pickup(pickup),
            standard_weight,
            nearby_racer_weight,
            enabled: true,
        }
    }

    fn weight(&self, context: ItemRollContext) -> u32 {
        let weights = if context.has_nearby_racer {
            self.context_weights.nearby_racer
        } else {
            self.context_weights.standard
        };

        match context.position {
            RacePositionBand::First => weights.first,
            RacePositionBand::Middle => weights.middle,
            RacePositionBand::Trailing => weights.trailing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RacePositionBand {
    First,
    Middle,
    Trailing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemRollContext {
    pub has_nearby_racer: bool,
    pub position: RacePositionBand,
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
            .any(|item| item.enabled && item.context_weights.has_positive_weight())
        {
            bail!(
                "item registry must contain at least one enabled item with a positive context weight"
            );
        }

        Ok(Self { items })
    }

    pub fn builtin() -> Self {
        Self::new(vec![
            ItemDefinition::built_in_with_context(
                "mushroom",
                "Mushroom",
                ItemPickup::Held(HeldItem::Mushroom),
                ItemActivation::Held,
                3,
                4,
                ItemContextWeights {
                    standard: PositionWeights {
                        first: 3,
                        middle: 3,
                        trailing: 6,
                    },
                    nearby_racer: PositionWeights {
                        first: 4,
                        middle: 4,
                        trailing: 8,
                    },
                },
            ),
            ItemDefinition::built_in_with_context(
                "banana",
                "Banana",
                ItemPickup::Held(HeldItem::Banana),
                ItemActivation::Held,
                2,
                3,
                ItemContextWeights {
                    standard: PositionWeights {
                        first: 1,
                        middle: 2,
                        trailing: 3,
                    },
                    nearby_racer: PositionWeights {
                        first: 2,
                        middle: 3,
                        trailing: 5,
                    },
                },
            ),
            ItemDefinition::built_in_with_context(
                "shield",
                "Shield",
                ItemPickup::Shield,
                ItemActivation::Immediate,
                1,
                3,
                ItemContextWeights {
                    standard: PositionWeights {
                        first: 1,
                        middle: 1,
                        trailing: 1,
                    },
                    nearby_racer: PositionWeights {
                        first: 3,
                        middle: 3,
                        trailing: 2,
                    },
                },
            ),
            ItemDefinition::built_in_with_context(
                "focus",
                "Focus",
                ItemPickup::Held(HeldItem::Focus),
                ItemActivation::Held,
                1,
                2,
                ItemContextWeights {
                    standard: PositionWeights {
                        first: 1,
                        middle: 2,
                        trailing: 3,
                    },
                    nearby_racer: PositionWeights {
                        first: 1,
                        middle: 2,
                        trailing: 4,
                    },
                },
            ),
            ItemDefinition::built_in_with_context(
                "cyclone",
                "Cyclone",
                ItemPickup::Held(HeldItem::Cyclone),
                ItemActivation::Held,
                1,
                2,
                ItemContextWeights {
                    standard: PositionWeights {
                        first: 0,
                        middle: 1,
                        trailing: 3,
                    },
                    nearby_racer: PositionWeights {
                        first: 0,
                        middle: 2,
                        trailing: 4,
                    },
                },
            ),
            ItemDefinition::built_in_with_context(
                "squid_ink",
                "Squid Ink",
                ItemPickup::Held(HeldItem::SquidInk),
                ItemActivation::Held,
                2,
                5,
                ItemContextWeights {
                    standard: PositionWeights {
                        first: 1,
                        middle: 2,
                        trailing: 2,
                    },
                    nearby_racer: PositionWeights {
                        first: 4,
                        middle: 6,
                        trailing: 5,
                    },
                },
            ),
        ])
        .expect("built-in item registry is valid")
    }

    /// Load a host-provided item pack from JSON.
    ///
    /// This external format intentionally tunes built-in items only: names,
    /// enabled flags, roll weights, selected effect parameters, and display
    /// labels. Adding entirely new item effects needs the shared item engine
    /// first, because the current game still resolves Mushroom, Banana, and
    /// Shield, Focus, and Cyclone through concrete Rust handlers.
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
                target.context_weights.standard = PositionWeights::flat(weight);
            }

            if let Some(weight) = override_item.nearby_racer_weight {
                target.nearby_racer_weight = weight;
                target.context_weights.nearby_racer = PositionWeights::flat(weight);
            }

            if let Some(context_weights) = override_item.context_weights {
                if let Some(standard) = context_weights.standard {
                    target.context_weights.standard = standard;
                    target.standard_weight = standard.middle;
                }
                if let Some(nearby_racer) = context_weights.nearby_racer {
                    target.context_weights.nearby_racer = nearby_racer;
                    target.nearby_racer_weight = nearby_racer.middle;
                }
            }

            if let Some(effect) = override_item.effect {
                if target.effect.mushroom.is_some()
                    && (effect.boost_words.is_some() || effect.wpm.is_some())
                {
                    let mut mushroom = target.effect.mushroom.unwrap_or_default();
                    if let Some(boost_words) = effect.boost_words {
                        mushroom.boost_words = boost_words;
                    }
                    if let Some(wpm) = effect.wpm {
                        mushroom.wpm = wpm;
                    }
                    if mushroom.boost_words == 0 {
                        bail!(
                            "item '{}' mushroom boost_words must be greater than zero",
                            override_item.id
                        );
                    }
                    if mushroom.wpm == 0 {
                        bail!(
                            "item '{}' mushroom wpm must be greater than zero",
                            override_item.id
                        );
                    }
                    target.effect.mushroom = Some(mushroom);
                } else if effect.boost_words.is_some() || effect.wpm.is_some() {
                    bail!(
                        "item '{}' does not support mushroom effect config",
                        override_item.id
                    );
                }

                if target.effect.banana.is_some()
                    && (effect.range_words.is_some()
                        || effect.stun_ms.is_some()
                        || effect.impact_blink_ms.is_some()
                        || effect.cue_ms.is_some())
                {
                    let mut banana = target.effect.banana.unwrap_or_default();
                    if let Some(range_words) = effect.range_words {
                        banana.range_words = range_words;
                    }
                    if let Some(stun_ms) = effect.stun_ms {
                        banana.stun_ms = stun_ms;
                    }
                    if let Some(impact_blink_ms) = effect.impact_blink_ms {
                        banana.impact_blink_ms = impact_blink_ms;
                    }
                    if let Some(cue_ms) = effect.cue_ms {
                        banana.cue_ms = cue_ms;
                    }
                    if banana.range_words == 0 {
                        bail!(
                            "item '{}' banana range_words must be greater than zero",
                            override_item.id
                        );
                    }
                    target.effect.banana = Some(banana);
                } else if effect.range_words.is_some()
                    || effect.stun_ms.is_some()
                    || effect.impact_blink_ms.is_some()
                    || effect.cue_ms.is_some()
                {
                    bail!(
                        "item '{}' does not support banana effect config",
                        override_item.id
                    );
                }

                if target.effect.shield.is_some() && effect.duration_ms.is_some() {
                    let mut shield = target.effect.shield.unwrap_or_default();
                    if let Some(duration_ms) = effect.duration_ms {
                        shield.duration_ms = duration_ms;
                    }
                    if shield.duration_ms == 0 {
                        bail!(
                            "item '{}' shield duration_ms must be greater than zero",
                            override_item.id
                        );
                    }
                    target.effect.shield = Some(shield);
                } else if effect.duration_ms.is_some() {
                    if target.effect.focus.is_some() {
                        let mut focus = target.effect.focus.unwrap_or_default();
                        focus.duration_ms = effect.duration_ms.unwrap();
                        if focus.duration_ms == 0 {
                            bail!(
                                "item '{}' focus duration_ms must be greater than zero",
                                override_item.id
                            );
                        }
                        target.effect.focus = Some(focus);
                    } else {
                        bail!(
                            "item '{}' does not support duration_ms effect config",
                            override_item.id
                        );
                    }
                }

                if target.effect.focus.is_some() && effect.focus_ai_wpm_boost.is_some() {
                    let mut focus = target.effect.focus.unwrap_or_default();
                    if let Some(boost) = effect.focus_ai_wpm_boost {
                        focus.ai_wpm_boost = boost;
                    }
                    target.effect.focus = Some(focus);
                } else if effect.focus_ai_wpm_boost.is_some() {
                    bail!(
                        "item '{}' does not support focus effect config",
                        override_item.id
                    );
                }

                if target.effect.cyclone.is_some()
                    && (effect.affected_words.is_some() || effect.cyclone_stun_ms.is_some())
                {
                    let mut cyclone = target.effect.cyclone.unwrap_or_default();
                    if let Some(affected_words) = effect.affected_words {
                        cyclone.affected_words = affected_words;
                    }
                    if let Some(stun_ms) = effect.cyclone_stun_ms {
                        cyclone.stun_ms = stun_ms;
                    }
                    if cyclone.affected_words == 0 {
                        bail!(
                            "item '{}' cyclone affected_words must be greater than zero",
                            override_item.id
                        );
                    }
                    if cyclone.stun_ms == 0 {
                        bail!(
                            "item '{}' cyclone stun_ms must be greater than zero",
                            override_item.id
                        );
                    }
                    target.effect.cyclone = Some(cyclone);
                } else if effect.affected_words.is_some() || effect.cyclone_stun_ms.is_some() {
                    bail!(
                        "item '{}' does not support cyclone effect config",
                        override_item.id
                    );
                }

                if target.effect.squid_ink.is_some()
                    && (effect.ink_range_words.is_some()
                        || effect.ink_duration_ms.is_some()
                        || effect.ink_impact_blink_ms.is_some()
                        || effect.ink_cue_ms.is_some()
                        || effect.ink_ai_wpm_multiplier_percent.is_some())
                {
                    let mut squid_ink = target.effect.squid_ink.unwrap_or_default();
                    if let Some(range_words) = effect.ink_range_words {
                        squid_ink.range_words = range_words;
                    }
                    if let Some(duration_ms) = effect.ink_duration_ms {
                        squid_ink.duration_ms = duration_ms;
                    }
                    if let Some(impact_blink_ms) = effect.ink_impact_blink_ms {
                        squid_ink.impact_blink_ms = impact_blink_ms;
                    }
                    if let Some(cue_ms) = effect.ink_cue_ms {
                        squid_ink.cue_ms = cue_ms;
                    }
                    if let Some(multiplier) = effect.ink_ai_wpm_multiplier_percent {
                        squid_ink.ai_wpm_multiplier_percent = multiplier;
                    }
                    if squid_ink.range_words == 0 {
                        bail!(
                            "item '{}' squid ink range_words must be greater than zero",
                            override_item.id
                        );
                    }
                    if squid_ink.duration_ms == 0 {
                        bail!(
                            "item '{}' squid ink duration_ms must be greater than zero",
                            override_item.id
                        );
                    }
                    if squid_ink.ai_wpm_multiplier_percent == 0
                        || squid_ink.ai_wpm_multiplier_percent > 100
                    {
                        bail!(
                            "item '{}' squid ink ai_wpm_multiplier_percent must be between 1 and 100",
                            override_item.id
                        );
                    }
                    target.effect.squid_ink = Some(squid_ink);
                } else if effect.ink_range_words.is_some()
                    || effect.ink_duration_ms.is_some()
                    || effect.ink_impact_blink_ms.is_some()
                    || effect.ink_cue_ms.is_some()
                    || effect.ink_ai_wpm_multiplier_percent.is_some()
                {
                    bail!(
                        "item '{}' does not support squid ink effect config",
                        override_item.id
                    );
                }
            }

            if let Some(display) = override_item.display {
                if target.display.banana.is_some()
                    && (display.ascii_ahead.is_some()
                        || display.ascii_behind.is_some()
                        || display.ascii_overlap.is_some()
                        || display.unicode_ahead.is_some()
                        || display.unicode_behind.is_some()
                        || display.unicode_overlap.is_some())
                {
                    let mut banana = target.display.banana.clone().unwrap_or_default();
                    if let Some(label) = display.ascii_ahead {
                        banana.ascii_ahead = label;
                    }
                    if let Some(label) = display.ascii_behind {
                        banana.ascii_behind = label;
                    }
                    if let Some(label) = display.ascii_overlap {
                        banana.ascii_overlap = label;
                    }
                    if let Some(label) = display.unicode_ahead {
                        banana.unicode_ahead = label;
                    }
                    if let Some(label) = display.unicode_behind {
                        banana.unicode_behind = label;
                    }
                    if let Some(label) = display.unicode_overlap {
                        banana.unicode_overlap = label;
                    }
                    target.display.banana = Some(banana);
                } else if display.ascii_ahead.is_some()
                    || display.ascii_behind.is_some()
                    || display.ascii_overlap.is_some()
                    || display.unicode_ahead.is_some()
                    || display.unicode_behind.is_some()
                    || display.unicode_overlap.is_some()
                {
                    bail!(
                        "item '{}' does not support banana display config",
                        override_item.id
                    );
                }
            }
        }

        Self::new(registry.items)
    }

    pub fn mushroom_effect(&self) -> MushroomEffectConfig {
        self.items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Mushroom))
            .and_then(|item| item.effect.mushroom)
            .unwrap_or_default()
    }

    pub fn banana_effect(&self) -> BananaEffectConfig {
        self.items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Banana))
            .and_then(|item| item.effect.banana)
            .unwrap_or_default()
    }

    pub fn banana_display(&self) -> BananaDisplayConfig {
        self.items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Banana))
            .and_then(|item| item.display.banana.clone())
            .unwrap_or_default()
    }

    pub fn shield_effect(&self) -> ShieldEffectConfig {
        self.items
            .iter()
            .find(|item| item.pickup == ItemPickup::Shield)
            .and_then(|item| item.effect.shield)
            .unwrap_or_default()
    }

    pub fn focus_effect(&self) -> FocusEffectConfig {
        self.items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Focus))
            .and_then(|item| item.effect.focus)
            .unwrap_or_default()
    }

    pub fn cyclone_effect(&self) -> CycloneEffectConfig {
        self.items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Cyclone))
            .and_then(|item| item.effect.cyclone)
            .unwrap_or_default()
    }

    pub fn squid_ink_effect(&self) -> SquidInkEffectConfig {
        self.items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::SquidInk))
            .and_then(|item| item.effect.squid_ink)
            .unwrap_or_default()
    }

    pub fn roll_pickup(&self, rng: &mut impl Rng, context: ItemRollContext) -> Option<ItemPickup> {
        let candidates = self
            .items
            .iter()
            .filter(|item| item.enabled)
            .filter_map(|item| {
                let weight = item.weight(context);
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
        .roll_pickup(
            rng,
            ItemRollContext {
                has_nearby_racer,
                position: RacePositionBand::Middle,
            },
        )
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
    context_weights: Option<ItemPackContextWeights>,
    effect: Option<ItemPackEffectConfig>,
    display: Option<ItemPackDisplayConfig>,
}

#[derive(Debug, Deserialize)]
struct ItemPackContextWeights {
    standard: Option<PositionWeights>,
    nearby_racer: Option<PositionWeights>,
}

#[derive(Debug, Deserialize)]
struct ItemPackEffectConfig {
    boost_words: Option<usize>,
    wpm: Option<u32>,
    range_words: Option<usize>,
    stun_ms: Option<u64>,
    impact_blink_ms: Option<u64>,
    cue_ms: Option<u64>,
    duration_ms: Option<u64>,
    focus_ai_wpm_boost: Option<u32>,
    affected_words: Option<usize>,
    cyclone_stun_ms: Option<u64>,
    ink_range_words: Option<usize>,
    ink_duration_ms: Option<u64>,
    ink_impact_blink_ms: Option<u64>,
    ink_cue_ms: Option<u64>,
    ink_ai_wpm_multiplier_percent: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ItemPackDisplayConfig {
    ascii_ahead: Option<String>,
    ascii_behind: Option<String>,
    ascii_overlap: Option<String>,
    unicode_ahead: Option<String>,
    unicode_behind: Option<String>,
    unicode_overlap: Option<String>,
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
        HeldItem, ItemActivation, ItemDefinition, ItemPackConfig, ItemPackItem, ItemPickup,
        ItemRegistry, ItemRollContext, RacePositionBand, RacerPosition, roll_item_with_proximity,
        select_nearest_banana_target,
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
        assert_eq!(total_weight, 10);
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
        assert_eq!(total_weight, 19);
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
                context_weights: None,
                effect: None,
                display: None,
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
                    ItemRollContext {
                        has_nearby_racer: false,
                        position: RacePositionBand::Middle,
                    },
                ),
                Some(ItemPickup::Shield)
            );
        }
    }

    #[test]
    fn first_place_rolls_reduce_banana_weight() {
        let registry = ItemRegistry::builtin();
        let banana = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Banana))
            .unwrap();

        assert!(
            banana.weight(ItemRollContext {
                has_nearby_racer: false,
                position: RacePositionBand::First,
            }) < banana.weight(ItemRollContext {
                has_nearby_racer: false,
                position: RacePositionBand::Middle,
            })
        );
    }

    #[test]
    fn trailing_rolls_increase_mushroom_weight() {
        let registry = ItemRegistry::builtin();
        let mushroom = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Mushroom))
            .unwrap();

        assert!(
            mushroom.weight(ItemRollContext {
                has_nearby_racer: false,
                position: RacePositionBand::Trailing,
            }) > mushroom.weight(ItemRollContext {
                has_nearby_racer: false,
                position: RacePositionBand::Middle,
            })
        );
    }

    #[test]
    fn nearby_context_still_increases_shield_weight() {
        let registry = ItemRegistry::builtin();
        let shield = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Shield)
            .unwrap();

        assert!(
            shield.weight(ItemRollContext {
                has_nearby_racer: true,
                position: RacePositionBand::Middle,
            }) > shield.weight(ItemRollContext {
                has_nearby_racer: false,
                position: RacePositionBand::Middle,
            })
        );
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
                context_weights: None,
                effect: None,
                display: None,
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
        assert_eq!(
            mushroom.weight(ItemRollContext {
                has_nearby_racer: false,
                position: RacePositionBand::Trailing,
            }),
            10
        );
        assert_eq!(
            mushroom.weight(ItemRollContext {
                has_nearby_racer: true,
                position: RacePositionBand::Trailing,
            }),
            12
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn item_pack_file_loads_context_weight_overrides() {
        let path = std::env::temp_dir().join(format!(
            "typekart-context-item-pack-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{
                "items": [
                    {
                        "id": "banana",
                        "context_weights": {
                            "standard": { "first": 9, "middle": 8, "trailing": 7 },
                            "nearby_racer": { "first": 6, "middle": 5, "trailing": 4 }
                        }
                    }
                ]
            }"#,
        )
        .unwrap();

        let registry = ItemRegistry::load_json_file(&path).unwrap();
        let banana = registry
            .items
            .iter()
            .find(|item| item.pickup == ItemPickup::Held(HeldItem::Banana))
            .unwrap();

        assert_eq!(
            banana.weight(ItemRollContext {
                has_nearby_racer: false,
                position: RacePositionBand::First,
            }),
            9
        );
        assert_eq!(
            banana.weight(ItemRollContext {
                has_nearby_racer: true,
                position: RacePositionBand::Trailing,
            }),
            4
        );
        assert_eq!(banana.standard_weight, 8);
        assert_eq!(banana.nearby_racer_weight, 5);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn item_pack_file_loads_effect_tuning() {
        let path = std::env::temp_dir().join(format!(
            "typekart-effect-item-pack-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{
                "items": [
                    {
                        "id": "mushroom",
                        "effect": { "boost_words": 4, "wpm": 240 }
                    },
                    {
                        "id": "banana",
                        "effect": {
                            "range_words": 6,
                            "stun_ms": 1500,
                            "impact_blink_ms": 900,
                            "cue_ms": 700
                        }
                    },
                    {
                        "id": "shield",
                        "effect": { "duration_ms": 3000 }
                    },
                    {
                        "id": "focus",
                        "effect": {
                            "duration_ms": 7500,
                            "focus_ai_wpm_boost": 15
                        }
                    },
                    {
                        "id": "cyclone",
                        "effect": {
                            "affected_words": 2,
                            "cyclone_stun_ms": 3000
                        }
                    },
                    {
                        "id": "squid_ink",
                        "effect": {
                            "ink_range_words": 7,
                            "ink_duration_ms": 2500,
                            "ink_impact_blink_ms": 800,
                            "ink_cue_ms": 600,
                            "ink_ai_wpm_multiplier_percent": 55
                        }
                    }
                ]
            }"#,
        )
        .unwrap();

        let registry = ItemRegistry::load_json_file(&path).unwrap();

        assert_eq!(registry.mushroom_effect().boost_words, 4);
        assert_eq!(registry.mushroom_effect().wpm, 240);
        assert_eq!(registry.banana_effect().range_words, 6);
        assert_eq!(registry.banana_effect().stun_ms, 1500);
        assert_eq!(registry.banana_effect().impact_blink_ms, 900);
        assert_eq!(registry.banana_effect().cue_ms, 700);
        assert_eq!(registry.shield_effect().duration_ms, 3000);
        assert_eq!(registry.focus_effect().duration_ms, 7500);
        assert_eq!(registry.focus_effect().ai_wpm_boost, 15);
        assert_eq!(registry.cyclone_effect().affected_words, 2);
        assert_eq!(registry.cyclone_effect().stun_ms, 3000);
        assert_eq!(registry.squid_ink_effect().range_words, 7);
        assert_eq!(registry.squid_ink_effect().duration_ms, 2500);
        assert_eq!(registry.squid_ink_effect().impact_blink_ms, 800);
        assert_eq!(registry.squid_ink_effect().cue_ms, 600);
        assert_eq!(registry.squid_ink_effect().ai_wpm_multiplier_percent, 55);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn item_pack_file_loads_banana_display_tuning() {
        let path = std::env::temp_dir().join(format!(
            "typekart-display-item-pack-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{
                "items": [
                    {
                        "id": "banana",
                        "display": {
                            "ascii_ahead": "BA>",
                            "ascii_behind": "<BA",
                            "unicode_ahead": "🍌>",
                            "unicode_behind": "<🍌"
                        }
                    }
                ]
            }"#,
        )
        .unwrap();

        let registry = ItemRegistry::load_json_file(&path).unwrap();
        let display = registry.banana_display();

        assert_eq!(display.ascii_ahead, "BA>");
        assert_eq!(display.ascii_behind, "<BA");
        assert_eq!(display.ascii_overlap, " ))<>");
        assert_eq!(display.unicode_ahead, "🍌>");
        assert_eq!(display.unicode_behind, "<🍌");
        assert_eq!(display.unicode_overlap, " 🍌 <>");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shipped_classic_item_template_loads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("mods")
            .join("items")
            .join("classic.json");

        let registry = ItemRegistry::load_json_file(path).unwrap();

        assert_eq!(registry.items.len(), ItemRegistry::builtin().items.len());
        assert_eq!(registry.focus_effect().duration_ms, 10_000);
        assert_eq!(registry.focus_effect().ai_wpm_boost, 10);
        assert!(registry.cyclone_effect().stun_ms > registry.banana_effect().stun_ms);
        assert_eq!(registry.squid_ink_effect().range_words, 5);
    }
}
