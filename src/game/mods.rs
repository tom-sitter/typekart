//! Shared helpers for moddable game content.
//!
//! The first moddable surfaces are items and word sets. This module keeps the
//! boring but important rules that both surfaces need: stable ids and small
//! pieces of metadata that can later be sent over the network or shown in a
//! lobby.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::{items::ItemRegistry, words::WordSetDefinition};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentId(String);

impl ContentId {
    /// Build a stable id for moddable content.
    ///
    /// We intentionally keep ids lowercase ASCII with `-` and `_` separators.
    /// That makes them safe for file names, CLI arguments, protocol metadata,
    /// and future config formats without needing separate escaping rules.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();

        if value.is_empty() {
            bail!("content id cannot be empty");
        }

        if !value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        {
            bail!("content id '{value}' must use lowercase ascii letters, digits, '-' or '_' only");
        }

        Ok(Self(value))
    }

    pub fn builtin(value: &'static str) -> Self {
        Self::new(value).expect("built-in content ids are valid")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentSource {
    BuiltIn,
    File { path: String },
}

impl ContentSource {
    pub fn label(&self) -> String {
        match self {
            Self::BuiltIn => "built-in".to_string(),
            Self::File { path } => path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentMetadata {
    pub id: ContentId,
    pub name: String,
    pub source: ContentSource,
}

impl ContentMetadata {
    pub fn built_in(id: &'static str, name: impl Into<String>) -> Self {
        Self {
            id: ContentId::builtin(id),
            name: name.into(),
            source: ContentSource::BuiltIn,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModHash(pub u64);

impl ModHash {
    pub fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveModConfig {
    pub word_set_id: String,
    pub word_set_name: String,
    pub word_set_source: String,
    pub word_set_hash: ModHash,
    pub item_pack_name: String,
    pub item_pack_source: String,
    pub item_registry_hash: ModHash,
    pub combined_hash: ModHash,
}

impl ActiveModConfig {
    pub fn new(
        word_set: &WordSetDefinition,
        item_registry: &ItemRegistry,
        item_pack_source: Option<String>,
    ) -> Self {
        let word_set_hash = hash_words(&word_set.words.words);
        let item_registry_hash = hash_item_registry(item_registry);
        let item_pack_name = if item_pack_source.is_some() {
            "custom".to_string()
        } else {
            "classic".to_string()
        };
        let item_pack_source = item_pack_source.unwrap_or_else(|| "built-in".to_string());
        let combined_hash = stable_hash([
            "typekart-mod-config",
            word_set.metadata.id.as_str(),
            &word_set_hash.hex(),
            &item_pack_name,
            &item_pack_source,
            &item_registry_hash.hex(),
        ]);

        Self {
            word_set_id: word_set.metadata.id.as_str().to_string(),
            word_set_name: word_set.metadata.name.clone(),
            word_set_source: word_set.metadata.source.label(),
            word_set_hash,
            item_pack_name,
            item_pack_source,
            item_registry_hash,
            combined_hash,
        }
    }

    pub fn log_summary(&self) -> String {
        format!(
            "mods word_set={} source={} word_hash={} item_pack={} item_source={} item_hash={} combined_hash={}",
            self.word_set_id,
            self.word_set_source,
            self.word_set_hash.hex(),
            self.item_pack_name,
            self.item_pack_source,
            self.item_registry_hash.hex(),
            self.combined_hash.hex()
        )
    }
}

fn hash_words(words: &[String]) -> ModHash {
    stable_hash(words.iter().map(String::as_str))
}

fn hash_item_registry(item_registry: &ItemRegistry) -> ModHash {
    let fields = item_registry.items.iter().flat_map(|item| {
        let mut fields = vec![
            item.id.as_str().to_string(),
            item.name.clone(),
            format!("{:?}", item.pickup),
            format!("{:?}", item.activation),
            item.standard_weight.to_string(),
            item.nearby_racer_weight.to_string(),
            item.context_weights.standard.first.to_string(),
            item.context_weights.standard.middle.to_string(),
            item.context_weights.standard.trailing.to_string(),
            item.context_weights.nearby_racer.first.to_string(),
            item.context_weights.nearby_racer.middle.to_string(),
            item.context_weights.nearby_racer.trailing.to_string(),
            item.enabled.to_string(),
        ];
        if let Some(mushroom) = item.effect.mushroom {
            fields.extend([mushroom.boost_words.to_string(), mushroom.wpm.to_string()]);
        }
        if let Some(banana) = item.effect.banana {
            fields.extend([
                banana.range_words.to_string(),
                banana.stun_ms.to_string(),
                banana.impact_blink_ms.to_string(),
                banana.cue_ms.to_string(),
            ]);
        }
        if let Some(shield) = item.effect.shield {
            fields.push(shield.duration_ms.to_string());
        }
        if let Some(focus) = item.effect.focus {
            fields.extend([
                focus.duration_ms.to_string(),
                focus.ai_wpm_boost.to_string(),
            ]);
        }
        if let Some(cyclone) = item.effect.cyclone {
            fields.extend([
                cyclone.affected_words.to_string(),
                cyclone.stun_ms.to_string(),
            ]);
        }
        if let Some(fog) = item.effect.fog {
            fields.extend([
                fog.range_words.to_string(),
                fog.duration_ms.to_string(),
                fog.impact_blink_ms.to_string(),
                fog.cue_ms.to_string(),
                fog.ai_wpm_multiplier_percent.to_string(),
            ]);
        }
        if let Some(banana_display) = &item.display.banana {
            fields.extend([
                banana_display.ascii_ahead.clone(),
                banana_display.ascii_behind.clone(),
                banana_display.ascii_overlap.clone(),
                banana_display.unicode_ahead.clone(),
                banana_display.unicode_behind.clone(),
                banana_display.unicode_overlap.clone(),
            ]);
        }
        fields
    });

    stable_hash(fields)
}

fn stable_hash(parts: impl IntoIterator<Item = impl AsRef<str>>) -> ModHash {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for part in parts {
        for byte in part.as_ref().as_bytes().iter().copied().chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }

    ModHash(hash)
}

#[cfg(test)]
mod tests {
    use super::{ActiveModConfig, ContentId, stable_hash};
    use crate::game::{items::ItemRegistry, words::WordSetDefinition};

    #[test]
    fn content_ids_accept_cli_safe_values() {
        assert_eq!(
            ContentId::new("classic_words").unwrap().as_str(),
            "classic_words"
        );
        assert_eq!(
            ContentId::new("item-pack-2").unwrap().as_str(),
            "item-pack-2"
        );
    }

    #[test]
    fn content_ids_reject_values_that_need_escaping() {
        assert!(ContentId::new("Classic").is_err());
        assert!(ContentId::new("space words").is_err());
        assert!(ContentId::new("").is_err());
    }

    #[test]
    fn stable_hash_is_deterministic() {
        assert_eq!(
            stable_hash(["alpha", "bravo"]),
            stable_hash(["alpha", "bravo"])
        );
        assert_ne!(
            stable_hash(["alpha", "bravo"]),
            stable_hash(["bravo", "alpha"])
        );
    }

    #[test]
    fn active_mod_config_summarizes_builtin_content() {
        let word_set = WordSetDefinition::load_builtin_default().unwrap();
        let item_registry = ItemRegistry::builtin();
        let config = ActiveModConfig::new(&word_set, &item_registry, None);

        assert_eq!(config.word_set_id, "classic");
        assert_eq!(config.item_pack_name, "classic");
        assert!(config.log_summary().contains("combined_hash="));
    }
}
