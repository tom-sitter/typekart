//! Shared helpers for moddable game content.
//!
//! The first moddable surfaces are items and word sets. This module keeps the
//! boring but important rules that both surfaces need: stable ids and small
//! pieces of metadata that can later be sent over the network or shown in a
//! lobby.

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentSource {
    BuiltIn,
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::ContentId;

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
}
