use std::time::{Duration, Instant};

use rand::{SeedableRng, rngs::StdRng};

use super::{
    BONUS_CHOICE_COUNT, BONUS_COOLDOWN, BonusChoice, BonusChoiceStatus, BonusPoint, BonusState,
    claim_bonus_choice,
};
use crate::game::{
    items::{ItemRegistry, ItemRollContext, RacePositionBand},
    track::{Track, WordList},
};

fn track(words: &[&str]) -> Track {
    Track::new(words.iter().map(|word| word.to_string()).collect())
}

#[test]
fn bonus_points_are_generated_periodically() {
    let track = track(&[
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ]);
    let word_list = WordList::from_contents("drift\nspark\nturbo\nboost\nracer\n");
    let bonuses = BonusState::generate(&track, &word_list);

    assert_eq!(bonuses.points.len(), 1);
    assert_eq!(bonuses.points[0].after_word_index, 7);
}

#[test]
fn bonus_point_has_three_choices() {
    let track = track(&[
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ]);
    let word_list = WordList::from_contents("drift\nspark\nturbo\nboost\nracer\n");
    let bonuses = BonusState::generate(&track, &word_list);

    assert_eq!(bonuses.points[0].choices.len(), BONUS_CHOICE_COUNT);
}

#[test]
fn claim_bonus_choice_places_choice_on_cooldown_and_rolls_item() {
    let now = Instant::now();
    let mut rng = StdRng::seed_from_u64(1);
    let mut bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice::available("drift"),
                BonusChoice::available("spark"),
                BonusChoice::available("turbo"),
            ],
        )],
        vec!["boost".to_string()],
    );

    let item = claim_bonus_choice(
        &mut bonuses,
        0,
        1,
        now,
        ItemRollContext {
            has_nearby_racer: false,
            position: RacePositionBand::Middle,
        },
        &ItemRegistry::builtin(),
        &mut rng,
    );

    assert!(item.is_some());
    assert!(matches!(
        bonuses.points[0].choices[1].status,
        BonusChoiceStatus::Cooldown { .. }
    ));
    assert_eq!(
        bonuses.points[0].choices[1].status,
        BonusChoiceStatus::Cooldown {
            until: now + BONUS_COOLDOWN
        }
    );
}

#[test]
fn expired_cooldown_replaces_choice() {
    let now = Instant::now();
    let track = track(&["one", "two"]);
    let mut bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice {
                    word: "drift".to_string(),
                    status: BonusChoiceStatus::Cooldown {
                        until: now - Duration::from_secs(1),
                    },
                },
                BonusChoice::available("spark"),
                BonusChoice::available("turbo"),
            ],
        )],
        vec!["boost".to_string(), "racer".to_string()],
    );

    let expired = bonuses.expire_cooldowns(&track, now);

    assert_eq!(expired, 1);
    assert!(matches!(
        bonuses.points[0].choices[0].status,
        BonusChoiceStatus::Available
    ));
}
