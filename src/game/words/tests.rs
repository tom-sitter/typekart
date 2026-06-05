use super::{
    WordSetCollection, WordSetDefinition, WordSetRegistry, WordSetSelection, validate_word_set,
};
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

#[test]
fn word_set_collection_loads_txt_files_from_directory() {
    let dir = std::env::temp_dir().join(format!("typekart-word-sets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("animals.txt"), "alpha\nbravo\ncharlie\n").unwrap();
    std::fs::write(dir.join("ignored.md"), "delta\necho\nfoxtrot\n").unwrap();

    let collection = WordSetCollection::load_dir(&dir).unwrap();

    assert_eq!(collection.sets.len(), 1);
    assert_eq!(collection.sets[0].metadata.id.as_str(), "animals");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn word_set_collection_rejects_directories_without_txt_files() {
    let dir = std::env::temp_dir().join(format!("typekart-empty-word-sets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), "alpha\nbravo\ncharlie\n").unwrap();

    let result = WordSetCollection::load_dir(&dir);

    assert!(result.is_err());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_can_select_from_word_set_directory() {
    let dir = std::env::temp_dir().join(format!(
        "typekart-registry-word-sets-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("one.txt"), "alpha\nbravo\ncharlie\n").unwrap();
    std::fs::write(dir.join("two.txt"), "delta\necho\nfoxtrot\n").unwrap();

    let word_set = WordSetRegistry::builtin()
        .load(&WordSetSelection::Directory(dir.clone()))
        .unwrap();

    assert!(["one", "two"].contains(&word_set.metadata.id.as_str()));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn shipped_classic_word_template_loads() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("mods")
        .join("words")
        .join("classic.txt");

    let word_set = WordSetDefinition::load_file(path).unwrap();

    assert_eq!(word_set.metadata.id.as_str(), "classic");
    assert!(word_set.words.words.len() > 100);
}
