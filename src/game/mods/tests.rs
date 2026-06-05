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
