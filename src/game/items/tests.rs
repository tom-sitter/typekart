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
    assert_eq!(total_weight, 18);
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
fn first_place_rolls_exclude_cyclone_even_when_weighted() {
    let registry = ItemRegistry::new(vec![
        ItemDefinition::built_in(
            "cyclone",
            "Cyclone",
            ItemPickup::Held(HeldItem::Cyclone),
            ItemActivation::Held,
            100,
            100,
        ),
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
                    has_nearby_racer: true,
                    position: RacePositionBand::First,
                },
            ),
            Some(ItemPickup::Shield)
        );
    }
}

#[test]
fn first_place_roll_returns_none_when_only_cyclone_is_rollable() {
    let registry = ItemRegistry::new(vec![ItemDefinition::built_in(
        "cyclone",
        "Cyclone",
        ItemPickup::Held(HeldItem::Cyclone),
        ItemActivation::Held,
        100,
        100,
    )])
    .unwrap();
    let mut rng = StdRng::seed_from_u64(1);

    assert_eq!(
        registry.roll_pickup(
            &mut rng,
            ItemRollContext {
                has_nearby_racer: true,
                position: RacePositionBand::First,
            },
        ),
        None
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
    let path = std::env::temp_dir().join(format!("typekart-item-pack-{}.json", std::process::id()));
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
                            "duration_ms": 1500,
                            "impact_duration_ms": 900,
                            "cue_duration_ms": 700
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
                            "ai_wpm_boost": 15
                        }
                    },
                    {
                        "id": "cyclone",
                        "effect": {
                            "affected_words": 2,
                            "duration_ms": 3000
                        }
                    },
                    {
                        "id": "fog",
                        "effect": {
                            "range_words": 7,
                            "duration_ms": 2500,
                            "impact_duration_ms": 800,
                            "cue_duration_ms": 600,
                            "ai_wpm_multiplier_percent": 55
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
    assert_eq!(registry.fog_effect().range_words, 7);
    assert_eq!(registry.fog_effect().duration_ms, 2500);
    assert_eq!(registry.fog_effect().impact_blink_ms, 800);
    assert_eq!(registry.fog_effect().cue_ms, 600);
    assert_eq!(registry.fog_effect().ai_wpm_multiplier_percent, 55);

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
    assert_eq!(registry.fog_effect().range_words, 5);
}
