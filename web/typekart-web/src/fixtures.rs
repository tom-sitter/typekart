#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GalleryScenario {
    pub slug: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub icon_mode_note: &'static str,
    pub frame: GalleryFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalleryFrame {
    Lobby(LobbySnapshotFixture),
    Race(RaceSnapshotFixture),
    Results(ResultsSnapshotFixture),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbySnapshotFixture {
    pub host_id: u64,
    pub players: &'static [LobbyPlayerFixture],
    pub mod_config: ModConfigFixture,
    pub events: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyPlayerFixture {
    pub id: u64,
    pub name: &'static str,
    pub kind: PlayerKindFixture,
    pub color_class: &'static str,
    pub ready: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerKindFixture {
    Human,
    Bot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModConfigFixture {
    pub word_set_name: &'static str,
    pub item_pack_name: &'static str,
    pub combined_hash: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaceSnapshotFixture {
    pub sequence: u64,
    pub phase: RacePhase,
    pub local_player_id: u64,
    pub mod_config: ModConfigFixture,
    pub track_words: &'static [&'static str],
    pub bonuses: &'static [BonusFixture],
    pub players: &'static [PlayerSnapshotFixture],
    pub events: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RacePhase {
    WaitingForHost,
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
    Cooldown { remaining_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerSnapshotFixture {
    pub id: u64,
    pub name: &'static str,
    pub kind: PlayerKindFixture,
    pub color_class: &'static str,
    pub word_index: usize,
    pub input: &'static str,
    pub typo_index: Option<usize>,
    pub finished: bool,
    pub connected: bool,
    pub shielded: bool,
    pub focused: bool,
    pub inked: bool,
    pub boosted: bool,
    pub stunned: bool,
    pub impact_cue: Option<ImpactCueFixture>,
    pub item_cue: Option<ItemCueFixture>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemCueFixture {
    pub kind: ItemCueKindFixture,
    pub unicode_label: &'static str,
    pub ascii_label: &'static str,
    pub placement: CuePlacement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCueKindFixture {
    Banana,
    Mushroom,
    Cyclone,
    SquidInk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuePlacement {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImpactCueFixture {
    pub kind: ImpactCueKindFixture,
    pub unicode_label: &'static str,
    pub ascii_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactCueKindFixture {
    Banana,
    Cyclone,
    SquidInk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultsSnapshotFixture {
    pub rows: &'static [ResultRowFixture],
    pub events: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultRowFixture {
    pub placement: usize,
    pub name: &'static str,
    pub color_class: &'static str,
    pub status: ResultStatusFixture,
    pub progress_words: usize,
    pub track_words: usize,
    pub wpm: u32,
    pub accuracy_percent: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatusFixture {
    Finished,
    TimedOut,
    Disconnected,
}

const MOD_CONFIG: ModConfigFixture = ModConfigFixture {
    word_set_name: "Classic",
    item_pack_name: "classic",
    combined_hash: "a598dc2b",
};

const TRACK: &[&str] = &[
    "spark", "river", "focus", "cyclone", "maple", "harbor", "pixel", "finish",
];

const LOBBY_PLAYERS: &[LobbyPlayerFixture] = &[
    LobbyPlayerFixture {
        id: 1,
        name: "tom",
        kind: PlayerKindFixture::Human,
        color_class: "cyan",
        ready: true,
        connected: true,
    },
    LobbyPlayerFixture {
        id: 2,
        name: "laura",
        kind: PlayerKindFixture::Human,
        color_class: "red",
        ready: true,
        connected: true,
    },
    LobbyPlayerFixture {
        id: 3,
        name: "ai-1",
        kind: PlayerKindFixture::Bot,
        color_class: "green",
        ready: true,
        connected: true,
    },
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
            status: BonusStatus::Cooldown { remaining_ms: 800 },
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

const OPENING_PLAYERS: &[PlayerSnapshotFixture] = &[
    player(1, "you", PlayerKindFixture::Human, "cyan", 1, "ri"),
    player(2, "alex", PlayerKindFixture::Human, "red", 1, "r"),
    player(3, "ai-1", PlayerKindFixture::Bot, "green", 0, "spa"),
];

const BANANA_PLAYERS: &[PlayerSnapshotFixture] = &[
    PlayerSnapshotFixture {
        item_cue: Some(ItemCueFixture {
            kind: ItemCueKindFixture::Banana,
            unicode_label: "🍌 >>",
            ascii_label: "))>>",
            placement: CuePlacement::After,
        }),
        ..player(1, "you", PlayerKindFixture::Human, "cyan", 3, "cy")
    },
    PlayerSnapshotFixture {
        stunned: true,
        impact_cue: Some(ImpactCueFixture {
            kind: ImpactCueKindFixture::Banana,
            unicode_label: "🍌",
            ascii_label: "BAN",
        }),
        ..player(2, "alex", PlayerKindFixture::Human, "red", 4, "ma")
    },
];

const MUSHROOM_PLAYERS: &[PlayerSnapshotFixture] = &[
    PlayerSnapshotFixture {
        boosted: true,
        item_cue: Some(ItemCueFixture {
            kind: ItemCueKindFixture::Mushroom,
            unicode_label: ">>🍄",
            ascii_label: ">>>",
            placement: CuePlacement::Before,
        }),
        ..player(1, "you", PlayerKindFixture::Human, "cyan", 4, "ma")
    },
    player(2, "alex", PlayerKindFixture::Human, "red", 3, "cy"),
];

const SHIELD_FOCUS_PLAYERS: &[PlayerSnapshotFixture] = &[
    PlayerSnapshotFixture {
        shielded: true,
        ..player(1, "you", PlayerKindFixture::Human, "cyan", 3, "cy")
    },
    PlayerSnapshotFixture {
        focused: true,
        ..player(2, "alex", PlayerKindFixture::Human, "red", 3, "cycl")
    },
    player(3, "ai-1", PlayerKindFixture::Bot, "green", 2, "fo"),
];

const CYCLONE_PLAYERS: &[PlayerSnapshotFixture] = &[
    PlayerSnapshotFixture {
        item_cue: Some(ItemCueFixture {
            kind: ItemCueKindFixture::Cyclone,
            unicode_label: "🌀 >>",
            ascii_label: "~~>>",
            placement: CuePlacement::After,
        }),
        ..player(1, "you", PlayerKindFixture::Human, "cyan", 4, "ma")
    },
    PlayerSnapshotFixture {
        stunned: true,
        impact_cue: Some(ImpactCueFixture {
            kind: ImpactCueKindFixture::Cyclone,
            unicode_label: "🌀",
            ascii_label: "CYC",
        }),
        ..player(2, "alex", PlayerKindFixture::Human, "red", 5, "rob")
    },
];

const SQUID_INK_PLAYERS: &[PlayerSnapshotFixture] = &[
    PlayerSnapshotFixture {
        item_cue: Some(ItemCueFixture {
            kind: ItemCueKindFixture::SquidInk,
            unicode_label: "⬛ >>",
            ascii_label: "INK>",
            placement: CuePlacement::After,
        }),
        ..player(1, "you", PlayerKindFixture::Human, "cyan", 3, "cy")
    },
    PlayerSnapshotFixture {
        inked: true,
        impact_cue: Some(ImpactCueFixture {
            kind: ImpactCueKindFixture::SquidInk,
            unicode_label: "⬛",
            ascii_label: "INK",
        }),
        ..player(2, "alex", PlayerKindFixture::Human, "red", 3, "cyc")
    },
];

const FINISH_PLAYERS: &[PlayerSnapshotFixture] = &[
    PlayerSnapshotFixture {
        focused: true,
        finished: true,
        input: "finish",
        ..player(1, "you", PlayerKindFixture::Human, "cyan", 7, "")
    },
    PlayerSnapshotFixture {
        typo_index: Some(3),
        impact_cue: Some(ImpactCueFixture {
            kind: ImpactCueKindFixture::Cyclone,
            unicode_label: "🌀",
            ascii_label: "CYC",
        }),
        ..player(2, "alex", PlayerKindFixture::Human, "red", 6, "pixl")
    },
];

const RESULT_ROWS: &[ResultRowFixture] = &[
    ResultRowFixture {
        placement: 1,
        name: "you",
        color_class: "cyan",
        status: ResultStatusFixture::Finished,
        progress_words: 8,
        track_words: 8,
        wpm: 74,
        accuracy_percent: 98,
    },
    ResultRowFixture {
        placement: 2,
        name: "alex",
        color_class: "red",
        status: ResultStatusFixture::TimedOut,
        progress_words: 7,
        track_words: 8,
        wpm: 62,
        accuracy_percent: 95,
    },
    ResultRowFixture {
        placement: 3,
        name: "ai-1",
        color_class: "green",
        status: ResultStatusFixture::Disconnected,
        progress_words: 5,
        track_words: 8,
        wpm: 43,
        accuracy_percent: 100,
    },
];

pub const SCENARIOS: &[GalleryScenario] = &[
    GalleryScenario {
        slug: "lobby",
        title: "Lobby",
        description: "Host, joiner, AI racer, mod metadata, and lobby events.",
        icon_mode_note: "Before-race state with no track renderer.",
        frame: GalleryFrame::Lobby(LobbySnapshotFixture {
            host_id: 1,
            players: LOBBY_PLAYERS,
            mod_config: MOD_CONFIG,
            events: &["tom joined", "laura joined", "ai-1 added"],
        }),
    },
    GalleryScenario {
        slug: "countdown",
        title: "Countdown",
        description: "Racers clustered near the start with all bonus choices available.",
        icon_mode_note: "Track is visible before input starts.",
        frame: GalleryFrame::Race(RaceSnapshotFixture {
            sequence: 1,
            phase: RacePhase::Countdown(3),
            local_player_id: 1,
            mod_config: MOD_CONFIG,
            track_words: TRACK,
            bonuses: OPENING_BONUSES,
            players: OPENING_PLAYERS,
            events: &["Room ready", "Countdown started"],
        }),
    },
    item_scenario(
        "banana-impact",
        "Banana impact",
        "Banana attack cue and impacted racer marker.",
        BANANA_PLAYERS,
        &["you picked up Banana", "alex spun out"],
    ),
    item_scenario(
        "mushroom-boost",
        "Mushroom boost",
        "Boost cue rendered before the local marker.",
        MUSHROOM_PLAYERS,
        &["you picked up Mushroom", "you boosted ahead"],
    ),
    item_scenario(
        "shield-focus",
        "Shield and focus",
        "Active defensive and typing-assist effects on racer markers.",
        SHIELD_FOCUS_PLAYERS,
        &["you shielded", "alex focused"],
    ),
    item_scenario(
        "cyclone-impact",
        "Cyclone impact",
        "Cyclone fired at the leader and impact cue on the target.",
        CYCLONE_PLAYERS,
        &["you fired Cyclone", "alex was hit by Cyclone"],
    ),
    item_scenario(
        "squid-ink",
        "Squid ink",
        "Squid ink cue, impacted marker, and masked future words for the local affected player.",
        SQUID_INK_PLAYERS,
        &["you fired Squid Ink", "alex was inked"],
    ),
    GalleryScenario {
        slug: "finish-sprint",
        title: "Finish sprint",
        description: "Finished marker, focus state, typo state, and cyclone impact near the finish.",
        icon_mode_note: "Finished racers should stay visible at the finish edge.",
        frame: GalleryFrame::Race(RaceSnapshotFixture {
            sequence: 8,
            phase: RacePhase::Finished,
            local_player_id: 1,
            mod_config: MOD_CONFIG,
            track_words: TRACK,
            bonuses: &[],
            players: FINISH_PLAYERS,
            events: &["you finished 1st", "alex was hit by Cyclone"],
        }),
    },
    GalleryScenario {
        slug: "results",
        title: "Results",
        description: "Final placement rows for finished, timed-out, and disconnected racers.",
        icon_mode_note: "Matches the network results concept.",
        frame: GalleryFrame::Results(ResultsSnapshotFixture {
            rows: RESULT_ROWS,
            events: &["Race complete", "Ready for rematch"],
        }),
    },
];

const fn player(
    id: u64,
    name: &'static str,
    kind: PlayerKindFixture,
    color_class: &'static str,
    word_index: usize,
    input: &'static str,
) -> PlayerSnapshotFixture {
    PlayerSnapshotFixture {
        id,
        name,
        kind,
        color_class,
        word_index,
        input,
        typo_index: None,
        finished: false,
        connected: true,
        shielded: false,
        focused: false,
        inked: false,
        boosted: false,
        stunned: false,
        impact_cue: None,
        item_cue: None,
    }
}

const fn item_scenario(
    slug: &'static str,
    title: &'static str,
    description: &'static str,
    players: &'static [PlayerSnapshotFixture],
    events: &'static [&'static str],
) -> GalleryScenario {
    GalleryScenario {
        slug,
        title,
        description,
        icon_mode_note: "Unicode and ASCII labels should both be legible.",
        frame: GalleryFrame::Race(RaceSnapshotFixture {
            sequence: 4,
            phase: RacePhase::Racing,
            local_player_id: 1,
            mod_config: MOD_CONFIG,
            track_words: TRACK,
            bonuses: CONSUMED_BONUSES,
            players,
            events,
        }),
    }
}

pub fn minimap_position(player: PlayerSnapshotFixture, word_count: usize) -> usize {
    if word_count <= 1 {
        return 0;
    }
    let clamped_word = player.word_index.min(word_count - 1);
    clamped_word * 100 / (word_count - 1)
}

pub fn masked_word(player: PlayerSnapshotFixture, index: usize, word: &str) -> String {
    if !player.inked || index <= player.word_index {
        return word.to_string();
    }
    "█".repeat(word.chars().count())
}

#[cfg(test)]
fn scenario_slugs() -> Vec<&'static str> {
    SCENARIOS.iter().map(|scenario| scenario.slug).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        BonusStatus, GalleryFrame, ImpactCueKindFixture, ItemCueKindFixture, ResultStatusFixture,
        SCENARIOS, masked_word, minimap_position, scenario_slugs,
    };

    #[test]
    fn gallery_covers_all_major_web_states() {
        let slugs = scenario_slugs();

        for expected in [
            "lobby",
            "countdown",
            "banana-impact",
            "mushroom-boost",
            "shield-focus",
            "cyclone-impact",
            "squid-ink",
            "finish-sprint",
            "results",
        ] {
            assert!(slugs.contains(&expected), "missing scenario {expected}");
        }
    }

    #[test]
    fn item_scenarios_cover_current_items_and_impacts() {
        let race_players = SCENARIOS
            .iter()
            .filter_map(|scenario| match scenario.frame {
                GalleryFrame::Race(snapshot) => Some(snapshot.players),
                _ => None,
            });

        let all_players = race_players.flatten().collect::<Vec<_>>();

        assert!(all_players.iter().any(|player| player.shielded));
        assert!(all_players.iter().any(|player| player.focused));
        assert!(all_players.iter().any(|player| player.inked));
        assert!(all_players.iter().any(|player| player.boosted));
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.item_cue)
                .any(|cue| cue.kind == ItemCueKindFixture::Banana)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.item_cue)
                .any(|cue| cue.kind == ItemCueKindFixture::Mushroom)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.item_cue)
                .any(|cue| cue.kind == ItemCueKindFixture::Cyclone)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.item_cue)
                .any(|cue| cue.kind == ItemCueKindFixture::SquidInk)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.impact_cue)
                .any(|impact| impact.kind == ImpactCueKindFixture::Banana)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.impact_cue)
                .any(|impact| impact.kind == ImpactCueKindFixture::Cyclone)
        );
        assert!(
            all_players
                .iter()
                .filter_map(|player| player.impact_cue)
                .any(|impact| impact.kind == ImpactCueKindFixture::SquidInk)
        );
    }

    #[test]
    fn consumed_bonus_and_results_states_are_represented() {
        assert!(SCENARIOS.iter().any(|scenario| {
            match scenario.frame {
                GalleryFrame::Race(snapshot) => snapshot
                    .bonuses
                    .iter()
                    .flat_map(|bonus| bonus.choices)
                    .any(|choice| matches!(choice.status, BonusStatus::Cooldown { .. })),
                _ => false,
            }
        }));
        assert!(SCENARIOS.iter().any(|scenario| {
            match scenario.frame {
                GalleryFrame::Results(results) => results
                    .rows
                    .iter()
                    .any(|row| row.status == ResultStatusFixture::TimedOut),
                _ => false,
            }
        }));
    }

    #[test]
    fn minimap_position_pins_finish_to_end() {
        let finish = SCENARIOS
            .iter()
            .find_map(|scenario| match scenario.frame {
                GalleryFrame::Race(snapshot) if snapshot.phase == super::RacePhase::Finished => {
                    Some(snapshot)
                }
                _ => None,
            })
            .unwrap();
        let finished = finish.players[0];

        assert_eq!(minimap_position(finished, finish.track_words.len()), 100);
    }

    #[test]
    fn squid_ink_masks_only_future_words() {
        let inked = super::SQUID_INK_PLAYERS[1];

        assert_eq!(masked_word(inked, inked.word_index, "cyclone"), "cyclone");
        assert_eq!(masked_word(inked, inked.word_index + 1, "maple"), "█████");
    }
}
