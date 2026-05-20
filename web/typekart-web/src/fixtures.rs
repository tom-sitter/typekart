#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GalleryScenario {
    pub slug: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub icon_mode_note: &'static str,
    pub snapshot: RaceFixture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaceFixture {
    pub phase: RacePhase,
    pub local_player_id: u64,
    pub track_words: &'static [&'static str],
    pub bonuses: &'static [BonusFixture],
    pub players: &'static [PlayerFixture],
    pub events: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RacePhase {
    Countdown(u8),
    Racing,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BonusFixture {
    pub after_word_index: usize,
    pub choices: &'static [BonusChoiceFixture],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BonusChoiceFixture {
    pub word: &'static str,
    pub status: BonusStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BonusStatus {
    Available,
    Cooldown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerFixture {
    pub id: u64,
    pub name: &'static str,
    pub color_class: &'static str,
    pub word_index: usize,
    pub typed: &'static str,
    pub marker: &'static str,
    pub effect: Option<PlayerEffect>,
    pub cue: Option<ItemCueFixture>,
    pub impact: Option<ImpactFixture>,
    pub finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEffect {
    Mushroom,
    Shield,
    Focus,
    Inked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemCueFixture {
    pub unicode_label: &'static str,
    pub ascii_label: &'static str,
    pub placement: CuePlacement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuePlacement {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImpactFixture {
    pub label: &'static str,
    pub class_name: &'static str,
}

const TRACK: &[&str] = &[
    "spark", "river", "focus", "cyclone", "maple", "harbor", "finish",
];

const OPENING_BONUSES: &[BonusFixture] = &[BonusFixture {
    after_word_index: 1,
    choices: &[
        BonusChoiceFixture {
            word: "glide",
            status: BonusStatus::Available,
        },
        BonusChoiceFixture {
            word: "lucky",
            status: BonusStatus::Available,
        },
        BonusChoiceFixture {
            word: "shield",
            status: BonusStatus::Available,
        },
    ],
}];

const CONSUMED_BONUSES: &[BonusFixture] = &[BonusFixture {
    after_word_index: 2,
    choices: &[
        BonusChoiceFixture {
            word: "boost",
            status: BonusStatus::Cooldown,
        },
        BonusChoiceFixture {
            word: "orbit",
            status: BonusStatus::Available,
        },
        BonusChoiceFixture {
            word: "pearl",
            status: BonusStatus::Available,
        },
    ],
}];

const OPENING_PLAYERS: &[PlayerFixture] = &[
    PlayerFixture {
        id: 1,
        name: "you",
        color_class: "cyan",
        word_index: 1,
        typed: "ri",
        marker: "███",
        effect: None,
        cue: None,
        impact: None,
        finished: false,
    },
    PlayerFixture {
        id: 2,
        name: "alex",
        color_class: "red",
        word_index: 1,
        typed: "r",
        marker: "███",
        effect: None,
        cue: None,
        impact: None,
        finished: false,
    },
    PlayerFixture {
        id: 3,
        name: "ai-1",
        color_class: "green",
        word_index: 0,
        typed: "spa",
        marker: "███",
        effect: None,
        cue: None,
        impact: None,
        finished: false,
    },
];

const ITEM_PLAYERS: &[PlayerFixture] = &[
    PlayerFixture {
        id: 1,
        name: "you",
        color_class: "cyan",
        word_index: 3,
        typed: "cy",
        marker: "███",
        effect: Some(PlayerEffect::Mushroom),
        cue: Some(ItemCueFixture {
            unicode_label: ">>🍄",
            ascii_label: ">>>",
            placement: CuePlacement::Before,
        }),
        impact: None,
        finished: false,
    },
    PlayerFixture {
        id: 2,
        name: "alex",
        color_class: "red",
        word_index: 2,
        typed: "fo",
        marker: "███",
        effect: None,
        cue: None,
        impact: Some(ImpactFixture {
            label: "🍌",
            class_name: "banana",
        }),
        finished: false,
    },
    PlayerFixture {
        id: 3,
        name: "mira",
        color_class: "yellow",
        word_index: 4,
        typed: "ma",
        marker: "███",
        effect: Some(PlayerEffect::Shield),
        cue: Some(ItemCueFixture {
            unicode_label: "🌀 >>",
            ascii_label: "~~>>",
            placement: CuePlacement::After,
        }),
        impact: None,
        finished: false,
    },
    PlayerFixture {
        id: 4,
        name: "ai-2",
        color_class: "magenta",
        word_index: 3,
        typed: "cyc",
        marker: "███",
        effect: Some(PlayerEffect::Inked),
        cue: None,
        impact: Some(ImpactFixture {
            label: "ink",
            class_name: "ink",
        }),
        finished: false,
    },
];

const FINISH_PLAYERS: &[PlayerFixture] = &[
    PlayerFixture {
        id: 1,
        name: "you",
        color_class: "cyan",
        word_index: 6,
        typed: "finish",
        marker: "███",
        effect: Some(PlayerEffect::Focus),
        cue: None,
        impact: None,
        finished: true,
    },
    PlayerFixture {
        id: 2,
        name: "alex",
        color_class: "red",
        word_index: 5,
        typed: "har",
        marker: "███",
        effect: None,
        cue: None,
        impact: Some(ImpactFixture {
            label: "🌀",
            class_name: "cyclone",
        }),
        finished: false,
    },
];

pub const SCENARIOS: &[GalleryScenario] = &[
    GalleryScenario {
        slug: "opening-pack",
        title: "Opening pack",
        description: "Racers clustered near the start with all bonus choices available.",
        icon_mode_note: "No active item cues",
        snapshot: RaceFixture {
            phase: RacePhase::Countdown(3),
            local_player_id: 1,
            track_words: TRACK,
            bonuses: OPENING_BONUSES,
            players: OPENING_PLAYERS,
            events: &["Room ready", "Countdown started"],
        },
    },
    GalleryScenario {
        slug: "item-impact",
        title: "Item impact",
        description: "A consumed bonus choice, attack cue, shield, mushroom boost, and impact blink.",
        icon_mode_note: "Unicode and ASCII labels should both be legible",
        snapshot: RaceFixture {
            phase: RacePhase::Racing,
            local_player_id: 1,
            track_words: TRACK,
            bonuses: CONSUMED_BONUSES,
            players: ITEM_PLAYERS,
            events: &[
                "you picked up Mushroom",
                "alex spun out",
                "mira fired Cyclone",
            ],
        },
    },
    GalleryScenario {
        slug: "finish-sprint",
        title: "Finish sprint",
        description: "Finished marker, focus state, and cyclone impact near the finish.",
        icon_mode_note: "Finished racers should stay visible at the finish edge",
        snapshot: RaceFixture {
            phase: RacePhase::Finished,
            local_player_id: 1,
            track_words: TRACK,
            bonuses: &[],
            players: FINISH_PLAYERS,
            events: &["you finished 1st", "alex was hit by Cyclone"],
        },
    },
];

pub fn minimap_position(player: PlayerFixture, word_count: usize) -> usize {
    if word_count <= 1 {
        return 0;
    }
    let clamped_word = player.word_index.min(word_count - 1);
    clamped_word * 100 / (word_count - 1)
}

#[cfg(test)]
mod tests {
    use super::{SCENARIOS, minimap_position};

    #[test]
    fn scenarios_cover_consumed_bonus_and_item_impact() {
        let item_impact = SCENARIOS
            .iter()
            .find(|scenario| scenario.slug == "item-impact")
            .unwrap();

        assert!(
            item_impact
                .snapshot
                .bonuses
                .iter()
                .flat_map(|bonus| bonus.choices)
                .any(|choice| matches!(choice.status, super::BonusStatus::Cooldown))
        );
        assert!(
            item_impact
                .snapshot
                .players
                .iter()
                .any(|player| player.impact.is_some())
        );
    }

    #[test]
    fn minimap_position_pins_finish_to_end() {
        let finished = SCENARIOS[2].snapshot.players[0];

        assert_eq!(
            minimap_position(finished, SCENARIOS[2].snapshot.track_words.len()),
            100
        );
    }
}
