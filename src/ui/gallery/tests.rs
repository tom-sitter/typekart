use std::time::Instant;

use super::{GalleryScenario, gallery_scenarios, scenario_index_by_slug};
use crate::game::bonus::BonusChoiceStatus;

#[test]
fn item_gallery_covers_current_item_effects_and_rollout_states() {
    let scenarios = gallery_scenarios();

    assert!(scenarios.contains(&GalleryScenario::MushroomBoost));
    assert!(scenarios.contains(&GalleryScenario::ShieldActive));
    assert!(scenarios.contains(&GalleryScenario::FocusActive));
    assert!(scenarios.contains(&GalleryScenario::BananaAhead));
    assert!(scenarios.contains(&GalleryScenario::CycloneAhead));
    assert!(scenarios.contains(&GalleryScenario::FogMaskedWords));
    assert!(scenarios.contains(&GalleryScenario::MultiplayerPack));
    assert!(scenarios.contains(&GalleryScenario::MultiplayerOpening));
    assert!(scenarios.contains(&GalleryScenario::BonusScramble));
    assert!(scenarios.contains(&GalleryScenario::ComebackChase));
    assert!(scenarios.contains(&GalleryScenario::BananaHitPack));
    assert!(scenarios.contains(&GalleryScenario::FogPack));
    assert!(scenarios.contains(&GalleryScenario::ItemPileup));
    assert!(scenarios.contains(&GalleryScenario::FinishSprint));
}

#[test]
fn gallery_can_jump_to_named_scenario() {
    let scenarios = gallery_scenarios();

    let index = scenario_index_by_slug(&scenarios, "banana-hit-pack").unwrap();

    assert_eq!(scenarios[index], GalleryScenario::BananaHitPack);
    assert!(scenario_index_by_slug(&scenarios, "missing").is_err());
}

#[test]
fn gallery_bonus_words_are_deterministic() {
    let bonuses = super::gallery_bonuses();

    assert_eq!(bonuses.points.len(), 1);
    assert_eq!(bonuses.points[0].after_word_index, 3);
    assert_eq!(bonuses.points[0].choices[0].word, "turbo");
    assert_eq!(bonuses.points[0].choices[1].word, "shield");
    assert_eq!(bonuses.points[0].choices[2].word, "fog");
}

#[test]
fn item_pickup_gallery_scenarios_show_consumed_bonus_choice() {
    let state = super::scenario_state(GalleryScenario::BananaAhead, Instant::now());

    assert_eq!(state.player.word_index, 4);
    assert!(matches!(
        state.bonuses.points[0].choices[0].status,
        BonusChoiceStatus::Cooldown { .. }
    ));
}

#[test]
fn non_pickup_gallery_scenarios_leave_local_player_off_bonus_gap() {
    let state = super::scenario_state(GalleryScenario::BananaImpact, Instant::now());

    assert_ne!(state.player.word_index, 4);
    assert!(
        state.bonuses.points[0]
            .choices
            .iter()
            .all(|choice| matches!(choice.status, BonusChoiceStatus::Available))
    );
}
