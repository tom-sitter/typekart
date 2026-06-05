//! Word-set selection and validation.
//!
//! `track::WordList` remains the simple list of playable words. This module
//! adds the modding-facing wrapper around it: ids, display names, sources, and
//! stricter validation for user-provided files.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use rand::{seq::SliceRandom, thread_rng};

use super::{
    mods::{ContentId, ContentMetadata, ContentSource},
    track::WordList,
};

pub const DEFAULT_WORD_SET_ID: &str = "classic";
const DEFAULT_WORD_SET: &str = include_str!("../../words_alpha.txt");
const MIN_UNIQUE_WORDS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSetDefinition {
    pub metadata: ContentMetadata,
    pub words: WordList,
}

impl WordSetDefinition {
    pub fn load_builtin_default() -> Result<Self> {
        let words = WordList::from_static(DEFAULT_WORD_SET);

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
pub struct WordSetCollection {
    pub name: String,
    pub source_dir: PathBuf,
    pub sets: Vec<WordSetDefinition>,
}

impl WordSetCollection {
    pub fn load_dir(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut entries = fs::read_dir(path)
            .with_context(|| format!("failed to read word set directory {}", path.display()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("failed to read word set directory {}", path.display()))?;
        entries.sort_by_key(|entry| entry.path());

        let word_set_paths = entries
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("txt"))
            .collect::<Vec<_>>();

        if word_set_paths.is_empty() {
            bail!(
                "word set directory {} does not contain any .txt word sets",
                path.display()
            );
        }

        let sets = word_set_paths
            .iter()
            .map(WordSetDefinition::load_file)
            .collect::<Result<Vec<_>>>()?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("word sets")
            .replace(['-', '_'], " ");

        Ok(Self {
            name,
            source_dir: path.to_path_buf(),
            sets,
        })
    }

    pub fn choose_random(&self) -> Result<WordSetDefinition> {
        self.sets
            .choose(&mut thread_rng())
            .cloned()
            .with_context(|| format!("word set collection '{}' is empty", self.name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordSetSelection {
    BuiltIn(String),
    File(PathBuf),
    Directory(PathBuf),
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
            WordSetSelection::Directory(path) => WordSetCollection::load_dir(path)?.choose_random(),
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
mod tests;
