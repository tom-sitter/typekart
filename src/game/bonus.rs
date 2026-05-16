//! Bonus-point generation, cooldowns, and bonus-claim helpers.

use std::time::{Duration, Instant};

use rand::{seq::SliceRandom, thread_rng, Rng};

use super::{
    items::{ItemPickup, ItemRegistry, ItemRollContext},
    track::{Track, WordList},
};

pub const BONUS_INTERVAL_WORDS: usize = 8;
pub const BONUS_CHOICE_COUNT: usize = 3;
pub const BONUS_COOLDOWN: Duration = Duration::from_secs(4);

#[derive(Debug, Clone)]
pub struct BonusState {
    pub points: Vec<BonusPoint>,
    word_pool: Vec<String>,
}

impl BonusState {
    pub fn generate(track: &Track, word_list: &WordList) -> Self {
        let word_pool = bonus_word_pool(word_list);
        let mut rng = thread_rng();
        let mut points = Vec::new();

        if track.len() > BONUS_INTERVAL_WORDS {
            for after_word_index in
                (BONUS_INTERVAL_WORDS - 1..track.len() - 1).step_by(BONUS_INTERVAL_WORDS)
            {
                let avoid_first = track
                    .current_word(after_word_index + 1)
                    .and_then(|word| word.chars().next());
                points.push(BonusPoint::new(
                    after_word_index,
                    build_choices(&word_pool, avoid_first, &mut rng),
                ));
            }
        }

        Self { points, word_pool }
    }

    #[cfg(test)]
    pub fn with_points(points: Vec<BonusPoint>, word_pool: Vec<String>) -> Self {
        Self { points, word_pool }
    }

    pub fn point_for_gap(&self, word_index: usize) -> Option<(usize, &BonusPoint)> {
        let after_word_index = word_index.checked_sub(1)?;
        self.points
            .iter()
            .enumerate()
            .find(|(_, point)| point.after_word_index == after_word_index)
    }

    pub fn expire_cooldowns(&mut self, track: &Track, now: Instant) -> usize {
        let mut expired = 0;
        let mut rng = thread_rng();
        let word_pool = self.word_pool.clone();

        for point in &mut self.points {
            let avoid_first = track
                .current_word(point.after_word_index + 1)
                .and_then(|word| word.chars().next());
            for choice_index in 0..point.choices.len() {
                if point.choices[choice_index].status.is_expired(now) {
                    let used_words = point
                        .choices
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| *index != choice_index)
                        .map(|(_, choice)| choice.word.as_str())
                        .collect::<Vec<_>>();
                    let used_firsts = point
                        .choices
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| *index != choice_index)
                        .filter_map(|(_, choice)| choice.word.chars().next())
                        .collect::<Vec<_>>();
                    let replacement = pick_bonus_word(
                        &word_pool,
                        avoid_first,
                        &used_words,
                        &used_firsts,
                        &mut rng,
                    );
                    point.choices[choice_index] = BonusChoice::available(replacement);
                    expired += 1;
                }
            }
        }

        expired
    }
}

#[derive(Debug, Clone)]
pub struct BonusPoint {
    pub after_word_index: usize,
    pub choices: [BonusChoice; BONUS_CHOICE_COUNT],
}

impl BonusPoint {
    pub fn new(after_word_index: usize, choices: [BonusChoice; BONUS_CHOICE_COUNT]) -> Self {
        Self {
            after_word_index,
            choices,
        }
    }

    pub fn available_choice_starting_with(
        &self,
        ch: char,
        now: Instant,
    ) -> Option<(usize, &BonusChoice)> {
        self.choices
            .iter()
            .enumerate()
            .find(|(_, choice)| choice.is_available(now) && choice.word.starts_with(ch))
    }
}

#[derive(Debug, Clone)]
pub struct BonusChoice {
    pub word: String,
    pub status: BonusChoiceStatus,
}

impl BonusChoice {
    pub fn available(word: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            status: BonusChoiceStatus::Available,
        }
    }

    pub fn is_available(&self, now: Instant) -> bool {
        matches!(self.status, BonusChoiceStatus::Available)
            || matches!(self.status, BonusChoiceStatus::Cooldown { until } if until <= now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BonusChoiceStatus {
    Available,
    Cooldown { until: Instant },
}

impl BonusChoiceStatus {
    fn is_expired(self, now: Instant) -> bool {
        matches!(self, Self::Cooldown { until } if until <= now)
    }
}

pub fn claim_bonus_choice(
    bonus_state: &mut BonusState,
    point_index: usize,
    choice_index: usize,
    now: Instant,
    has_nearby_racer: bool,
    item_registry: &ItemRegistry,
    rng: &mut impl Rng,
) -> Option<ItemPickup> {
    let choice = bonus_state
        .points
        .get_mut(point_index)?
        .choices
        .get_mut(choice_index)?;

    if !choice.is_available(now) {
        return None;
    }

    choice.status = BonusChoiceStatus::Cooldown {
        until: now + BONUS_COOLDOWN,
    };
    item_registry.roll_pickup(rng, ItemRollContext { has_nearby_racer })
}

fn bonus_word_pool(word_list: &WordList) -> Vec<String> {
    word_list
        .words
        .iter()
        .filter(|word| (4..=8).contains(&word.chars().count()))
        .cloned()
        .collect()
}

fn build_choices(
    word_pool: &[String],
    avoid_first: Option<char>,
    rng: &mut impl Rng,
) -> [BonusChoice; BONUS_CHOICE_COUNT] {
    let mut words = Vec::new();
    let mut firsts = Vec::new();

    while words.len() < BONUS_CHOICE_COUNT {
        let word = pick_bonus_word(word_pool, avoid_first, &words, &firsts, rng);
        if let Some(first) = word.chars().next() {
            firsts.push(first);
        }
        words.push(word);
    }

    [
        BonusChoice::available(words[0].clone()),
        BonusChoice::available(words[1].clone()),
        BonusChoice::available(words[2].clone()),
    ]
}

fn pick_bonus_word(
    word_pool: &[String],
    avoid_first: Option<char>,
    used_words: &[impl AsRef<str>],
    used_firsts: &[char],
    rng: &mut impl Rng,
) -> String {
    let mut candidates = word_pool
        .iter()
        .filter(|word| !used_words.iter().any(|used| used.as_ref() == word.as_str()))
        .filter(|word| {
            word.chars()
                .next()
                .is_some_and(|first| Some(first) != avoid_first && !used_firsts.contains(&first))
        })
        .collect::<Vec<_>>();

    candidates.shuffle(rng);
    candidates
        .first()
        .map(|word| (*word).clone())
        .or_else(|| word_pool.choose(rng).cloned())
        .unwrap_or_else(|| "boost".to_string())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use rand::{rngs::StdRng, SeedableRng};

    use super::{
        claim_bonus_choice, BonusChoice, BonusChoiceStatus, BonusPoint, BonusState,
        BONUS_CHOICE_COUNT, BONUS_COOLDOWN,
    };
    use crate::game::{
        items::ItemRegistry,
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
            false,
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
}
