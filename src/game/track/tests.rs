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
