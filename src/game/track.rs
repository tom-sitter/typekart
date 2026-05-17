//! Word-list loading and race-track generation.
//!
//! `WordList` is the curated source list from disk. `Track` is the concrete
//! sequence of words for one race.

use std::{fs, path::Path};

use anyhow::{Result, bail};
use rand::{seq::SliceRandom, thread_rng};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub words: Vec<String>,
}

impl Track {
    #[cfg(test)]
    pub fn new(words: Vec<String>) -> Self {
        Self { words }
    }

    pub fn generate(word_list: &WordList, word_count: usize) -> Result<Self> {
        if word_count == 0 {
            bail!("track must contain at least one word");
        }

        if word_list.words.is_empty() {
            bail!("word list is empty");
        }

        let mut rng = thread_rng();
        let words = (0..word_count)
            .map(|_| {
                word_list
                    .words
                    .choose(&mut rng)
                    .expect("checked non-empty word list")
                    .clone()
            })
            .collect();

        Ok(Self { words })
    }

    pub fn current_word(&self, word_index: usize) -> Option<&str> {
        self.words.get(word_index).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordList {
    pub words: Vec<String>,
}

impl WordList {
    /// Load the curated word list from disk.
    ///
    /// Runtime code trusts the list as curated data and does not silently filter
    /// words. Tests can validate the file if we want to catch data mistakes.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let contents = fs::read_to_string(path)?;
        Ok(Self::from_contents(&contents))
    }

    pub fn from_static(contents: &'static str) -> Self {
        Self::from_contents(contents)
    }

    pub fn from_contents(contents: &str) -> Self {
        let words = contents
            .lines()
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        Self { words }
    }

    #[cfg(test)]
    pub fn validate_curated_words(&self) -> Result<()> {
        for word in &self.words {
            if !word.chars().all(|ch| ch.is_ascii_lowercase()) {
                bail!("word list contains non-lowercase-ascii word: {word}");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Track, WordList};

    #[test]
    fn word_list_loads_non_empty_lines() {
        let word_list = WordList::from_contents("alpha\n\nbravo\n charlie \n");

        assert_eq!(word_list.words, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn word_list_validation_accepts_lowercase_ascii_words() {
        let word_list = WordList::from_contents("alpha\nbravo\ncharlie\n");

        assert!(word_list.validate_curated_words().is_ok());
    }

    #[test]
    fn word_list_validation_rejects_unplayable_words() {
        let word_list = WordList::from_contents("alpha\nBravo\n");

        assert!(word_list.validate_curated_words().is_err());
    }

    #[test]
    fn track_generation_uses_requested_length() {
        let word_list = WordList::from_contents("alpha\nbravo\ncharlie\n");
        let track = Track::generate(&word_list, 10).unwrap();

        assert_eq!(track.len(), 10);
    }

    #[test]
    fn track_generation_rejects_zero_words() {
        let word_list = WordList::from_contents("alpha\n");

        assert!(Track::generate(&word_list, 0).is_err());
    }
}
