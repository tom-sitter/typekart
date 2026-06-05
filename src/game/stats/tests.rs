use std::time::Duration;

use super::{TypingStats, words_per_minute};

#[test]
fn accuracy_is_perfect_before_typing() {
    assert_eq!(TypingStats::default().accuracy(), 100.0);
}

#[test]
fn accuracy_uses_correct_chars_over_typed_chars() {
    let stats = TypingStats {
        typed_chars: 10,
        correct_chars: 8,
        typo_chars: 2,
        backspaces: 1,
        completed_words: 1,
    };

    assert_eq!(stats.accuracy(), 80.0);
}

#[test]
fn wpm_uses_standard_five_character_words() {
    let wpm = words_per_minute(25, Duration::from_secs(30));

    assert_eq!(wpm, 10.0);
}
