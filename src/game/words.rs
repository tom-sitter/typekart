//! Word-set selection and validation.
//!
//! `track::WordList` remains the simple list of playable words. This module
//! adds the modding-facing wrapper around it: ids, display names, sources, and
//! stricter validation for user-provided files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{
    mods::{ContentId, ContentMetadata, ContentSource},
    track::WordList,
};

pub const DEFAULT_WORD_SET_ID: &str = "classic";
const DEFAULT_WORD_SET_PATH: &str = "words_alpha.txt";
const MIN_UNIQUE_WORDS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSetDefinition {
    pub metadata: ContentMetadata,
    pub words: WordList,
}

impl WordSetDefinition {
    pub fn load_builtin_default() -> Result<Self> {
        let words = WordList::load(DEFAULT_WORD_SET_PATH).with_context(|| {
            format!("failed to load built-in word set from {DEFAULT_WORD_SET_PATH}")
        })?;

        Ok(Self {
            metadata: ContentMetadata::built_in(DEFAULT_WORD_SET_ID, "Classic"),
            words,
        })
    }

    pub fn load_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let words = WordList::load(path)
            .with_context(|| format!("failed to load word set file {}", path.display()))?;
        validate_word_set(&words)?;

        let file_stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("custom");
        let id = file_stem
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>();

        Ok(Self {
            metadata: ContentMetadata {
                id: ContentId::new(id).unwrap_or_else(|_| ContentId::builtin("custom")),
                name: file_stem.replace(['-', '_'], " "),
                source: ContentSource::File {
                    path: path.display().to_string(),
                },
            },
            words,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordSetSelection {
    BuiltIn(String),
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct WordSetRegistry {
    builtins: Vec<WordSetMetadata>,
}

impl WordSetRegistry {
    pub fn builtin() -> Self {
        Self {
            builtins: vec![WordSetMetadata {
                id: ContentId::builtin(DEFAULT_WORD_SET_ID),
                name: "Classic".to_string(),
            }],
        }
    }

    pub fn load(&self, selection: &WordSetSelection) -> Result<WordSetDefinition> {
        match selection {
            WordSetSelection::BuiltIn(id) => self.load_builtin(id),
            WordSetSelection::File(path) => WordSetDefinition::load_file(path),
        }
    }

    fn load_builtin(&self, id: &str) -> Result<WordSetDefinition> {
        if self.builtins.iter().any(|set| set.id.as_str() == id) {
            WordSetDefinition::load_builtin_default()
        } else {
            let available = self
                .builtins
                .iter()
                .map(|set| format!("{} ({})", set.id.as_str(), set.name))
                .collect::<Vec<_>>()
                .join(", ");
            bail!("unknown word set '{id}'. Available word sets: {available}");
        }
    }
}

#[derive(Debug, Clone)]
struct WordSetMetadata {
    id: ContentId,
    name: String,
}

fn validate_word_set(word_list: &WordList) -> Result<()> {
    if word_list.words.is_empty() {
        bail!("word set is empty");
    }

    let mut invalid = Vec::new();
    for (index, word) in word_list.words.iter().enumerate() {
        if !is_playable_word(word) {
            invalid.push(format!("line {}: {word}", index + 1));
        }

        if invalid.len() == 5 {
            break;
        }
    }

    if !invalid.is_empty() {
        bail!(
            "word set contains unplayable words; expected lowercase ascii words only: {}",
            invalid.join(", ")
        );
    }

    let mut unique_words = word_list.words.clone();
    unique_words.sort();
    unique_words.dedup();

    if unique_words.len() < MIN_UNIQUE_WORDS {
        bail!("word set must contain at least {MIN_UNIQUE_WORDS} unique words");
    }

    Ok(())
}

fn is_playable_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|ch| ch.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{WordSetDefinition, WordSetRegistry, WordSetSelection, validate_word_set};
    use crate::game::track::WordList;

    #[test]
    fn builtin_registry_loads_classic_word_set() {
        let word_set = WordSetRegistry::builtin()
            .load(&WordSetSelection::BuiltIn("classic".to_string()))
            .unwrap();

        assert_eq!(word_set.metadata.id.as_str(), "classic");
        assert!(!word_set.words.words.is_empty());
    }

    #[test]
    fn builtin_registry_rejects_unknown_word_set() {
        let result = WordSetRegistry::builtin().load(&WordSetSelection::BuiltIn("missing".into()));

        assert!(result.is_err());
    }

    #[test]
    fn validation_rejects_empty_word_sets() {
        let word_list = WordList::from_contents("");

        assert!(validate_word_set(&word_list).is_err());
    }

    #[test]
    fn validation_rejects_unplayable_words() {
        let word_list = WordList::from_contents("alpha\nBravo\nhas-hyphen\n");

        assert!(validate_word_set(&word_list).is_err());
    }

    #[test]
    fn validation_requires_enough_unique_words() {
        let word_list = WordList::from_contents("alpha\nalpha\nbravo\n");

        assert!(validate_word_set(&word_list).is_err());
    }

    #[test]
    fn validation_accepts_lowercase_ascii_words() {
        let word_list = WordList::from_contents("alpha\nbravo\ncharlie\n");

        assert!(validate_word_set(&word_list).is_ok());
    }

    #[test]
    fn file_word_set_uses_file_stem_as_metadata() {
        let path = std::env::temp_dir().join("typekart-test-words.txt");
        std::fs::write(&path, "alpha\nbravo\ncharlie\n").unwrap();

        let word_set = WordSetDefinition::load_file(&path).unwrap();

        assert_eq!(word_set.metadata.id.as_str(), "typekart-test-words");
        assert_eq!(word_set.words.words.len(), 3);

        let _ = std::fs::remove_file(path);
    }
}
