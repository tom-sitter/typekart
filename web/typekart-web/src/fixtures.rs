use typekart_protocol::{
    AiDifficultySnapshot, AssignedColor, AttackDirectionSnapshot, BonusChoiceSnapshot,
    BonusChoiceSnapshotStatus, BonusPointSnapshot, ImpactCueSnapshot, ImpactCueSnapshotKind,
    ItemCuePlacementSnapshot, ItemCueSnapshot, ItemCueSnapshotKind, LobbyPlayer, ModConfigSnapshot,
    NetworkRacePhase, PlayerId, PlayerKind, PlayerSnapshot, RaceResultRow, RaceResultStatus,
    RaceSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GalleryScenario {
    pub slug: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub icon_mode_note: &'static str,
    kind: ScenarioKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScenarioKind {
    Lobby,
    Countdown,
    BananaImpact,
    MushroomBoost,
    ShieldFocus,
    CycloneImpact,
    Fog,
    FinishSprint,
    Results,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalleryFrame {
    Lobby(LobbyFrame),
    Race(RaceSnapshot),
    Results(ResultsFrame),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyFrame {
    pub host_id: PlayerId,
    pub players: Vec<LobbyPlayer>,
    pub mod_config: ModConfigSnapshot,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultsFrame {
    pub placements: Vec<PlayerId>,
    pub rows: Vec<RaceResultRow>,
    pub events: Vec<String>,
}

pub const SCENARIOS: &[GalleryScenario] = &[
    scenario(
        "lobby",
        "Lobby",
        "Host, joiner, AI racer, mod metadata, and lobby events.",
        "Before-race state with no track renderer.",
        ScenarioKind::Lobby,
    ),
    scenario(
        "countdown",
        "Countdown",
        "Racers clustered near the start with all bonus choices available.",
        "Track is visible before input starts.",
        ScenarioKind::Countdown,
    ),
    scenario(
        "banana-impact",
        "Banana impact",
        "Banana attack cue and impacted racer marker.",
        "Unicode and ASCII labels should both be legible.",
        ScenarioKind::BananaImpact,
    ),
    scenario(
        "mushroom-boost",
        "Mushroom boost",
        "Boost cue rendered before the local marker.",
        "Unicode and ASCII labels should both be legible.",
        ScenarioKind::MushroomBoost,
    ),
    scenario(
        "shield-focus",
        "Shield and focus",
        "Active defensive and typing-assist effects on racer markers.",
        "Unicode and ASCII labels should both be legible.",
        ScenarioKind::ShieldFocus,
    ),
    scenario(
        "cyclone-impact",
        "Cyclone impact",
        "Cyclone fired at the leader and impact cue on the target.",
        "Unicode and ASCII labels should both be legible.",
        ScenarioKind::CycloneImpact,
    ),
    scenario(
        "fog",
        "Fog",
        "Fog cue, impacted marker, and masked future words for the local affected player.",
        "Unicode and ASCII labels should both be legible.",
        ScenarioKind::Fog,
    ),
    scenario(
        "finish-sprint",
        "Finish sprint",
        "Finished marker, focus state, typo state, and cyclone impact near the finish.",
        "Finished racers should stay visible at the finish edge.",
        ScenarioKind::FinishSprint,
    ),
    scenario(
        "results",
        "Results",
        "Final placement rows for finished, timed-out, and disconnected racers.",
        "Matches the network results concept.",
        ScenarioKind::Results,
    ),
];

const fn scenario(
    slug: &'static str,
    title: &'static str,
    description: &'static str,
    icon_mode_note: &'static str,
    kind: ScenarioKind,
) -> GalleryScenario {
    GalleryScenario {
        slug,
        title,
        description,
        icon_mode_note,
        kind,
    }
}

pub fn scenario_frame(scenario: GalleryScenario) -> GalleryFrame {
    match scenario.kind {
        ScenarioKind::Lobby => GalleryFrame::Lobby(lobby_frame()),
        ScenarioKind::Countdown => GalleryFrame::Race(race_snapshot(
            1,
            NetworkRacePhase::Countdown {
                remaining_seconds: 3,
            },
            opening_players(),
            opening_bonuses(),
            ["Room ready", "Countdown started"],
        )),
        ScenarioKind::BananaImpact => GalleryFrame::Race(race_snapshot(
            4,
            NetworkRacePhase::Racing,
            banana_players(),
            consumed_bonuses(),
            ["you picked up Banana", "alex spun out"],
        )),
        ScenarioKind::MushroomBoost => GalleryFrame::Race(race_snapshot(
            4,
            NetworkRacePhase::Racing,
            mushroom_players(),
            consumed_bonuses(),
            ["you picked up Mushroom", "you boosted ahead"],
        )),
        ScenarioKind::ShieldFocus => GalleryFrame::Race(race_snapshot(
            4,
            NetworkRacePhase::Racing,
            shield_focus_players(),
            consumed_bonuses(),
            ["you shielded", "alex focused"],
        )),
        ScenarioKind::CycloneImpact => GalleryFrame::Race(race_snapshot(
            4,
            NetworkRacePhase::Racing,
            cyclone_players(),
            consumed_bonuses(),
            ["you fired Cyclone", "alex was hit by Cyclone"],
        )),
        ScenarioKind::Fog => GalleryFrame::Race(race_snapshot(
            4,
            NetworkRacePhase::Racing,
            fog_players(),
            consumed_bonuses(),
            ["you fired Fog", "alex was fogged"],
        )),
        ScenarioKind::FinishSprint => GalleryFrame::Race(race_snapshot(
            8,
            NetworkRacePhase::Finished,
            finish_players(),
            Vec::new(),
            ["you finished 1st", "alex was hit by Cyclone"],
        )),
        ScenarioKind::Results => GalleryFrame::Results(results_frame()),
    }
}

pub fn minimap_position(player: &PlayerSnapshot, word_count: usize) -> usize {
    if word_count <= 1 {
        return 0;
    }
    let clamped_word = player.word_index.min(word_count - 1);
    clamped_word * 100 / (word_count - 1)
}

pub fn masked_word(player: &PlayerSnapshot, index: usize, word: &str) -> String {
    if !player.fogged || index <= player.word_index {
        return word.to_string();
    }
    "█".repeat(word.chars().count())
}

pub fn color_class(color: AssignedColor) -> &'static str {
    match color {
        AssignedColor::Cyan => "cyan",
        AssignedColor::Red => "red",
        AssignedColor::Green => "green",
        AssignedColor::Blue => "blue",
        AssignedColor::Yellow => "yellow",
        AssignedColor::Magenta => "magenta",
    }
}

fn lobby_frame() -> LobbyFrame {
    LobbyFrame {
        host_id: PlayerId(1),
        players: vec![
            lobby_player(1, "tom", PlayerKind::Human, AssignedColor::Cyan, true),
            lobby_player(2, "laura", PlayerKind::Human, AssignedColor::Red, true),
            LobbyPlayer {
                ai_difficulty: Some(AiDifficultySnapshot::Easy),
                ai_wpm: Some(48),
                ..lobby_player(3, "ai-1", PlayerKind::Bot, AssignedColor::Green, true)
            },
        ],
        mod_config: mod_config(),
        events: strings(["tom joined", "laura joined", "ai-1 added"]),
    }
}

fn race_snapshot(
    sequence: u64,
    phase: NetworkRacePhase,
    players: Vec<PlayerSnapshot>,
    bonuses: Vec<BonusPointSnapshot>,
    events: impl IntoIterator<Item = &'static str>,
) -> RaceSnapshot {
    RaceSnapshot {
        sequence,
        phase,
        mod_config: mod_config(),
        track_words: track_words(),
        bonuses,
        players,
        events: strings(events),
    }
}

fn results_frame() -> ResultsFrame {
    ResultsFrame {
        placements: vec![PlayerId(1), PlayerId(2), PlayerId(3)],
        rows: vec![
            RaceResultRow {
                placement: 1,
                player_id: PlayerId(1),
                name: "you".to_string(),
                color: AssignedColor::Cyan,
                status: RaceResultStatus::Finished,
                progress_words: 8,
                track_words: 8,
                wpm: 74,
                accuracy_percent: 98,
                typo_chars: 1,
                backspaces: 2,
            },
            RaceResultRow {
                placement: 2,
                player_id: PlayerId(2),
                name: "alex".to_string(),
                color: AssignedColor::Red,
                status: RaceResultStatus::TimedOut,
                progress_words: 7,
                track_words: 8,
                wpm: 62,
                accuracy_percent: 95,
                typo_chars: 3,
                backspaces: 4,
            },
            RaceResultRow {
                placement: 3,
                player_id: PlayerId(3),
                name: "ai-1".to_string(),
                color: AssignedColor::Green,
                status: RaceResultStatus::Disconnected,
                progress_words: 5,
                track_words: 8,
                wpm: 43,
                accuracy_percent: 100,
                typo_chars: 0,
                backspaces: 0,
            },
        ],
        events: strings(["Race complete", "Ready for rematch"]),
    }
}

fn opening_bonuses() -> Vec<BonusPointSnapshot> {
    vec![BonusPointSnapshot {
        after_word_index: 1,
        choices: vec![
            bonus_choice("glide", BonusChoiceSnapshotStatus::Available),
            bonus_choice("lucky", BonusChoiceSnapshotStatus::Available),
            bonus_choice("shield", BonusChoiceSnapshotStatus::Available),
        ],
    }]
}

fn consumed_bonuses() -> Vec<BonusPointSnapshot> {
    vec![BonusPointSnapshot {
        after_word_index: 2,
        choices: vec![
            bonus_choice(
                "boost",
                BonusChoiceSnapshotStatus::Cooldown { remaining_ms: 800 },
            ),
            bonus_choice("orbit", BonusChoiceSnapshotStatus::Available),
            bonus_choice("pearl", BonusChoiceSnapshotStatus::Available),
        ],
    }]
}

fn opening_players() -> Vec<PlayerSnapshot> {
    vec![
        player(1, "you", PlayerKind::Human, AssignedColor::Cyan, 1, "ri"),
        player(2, "alex", PlayerKind::Human, AssignedColor::Red, 1, "r"),
        player(3, "ai-1", PlayerKind::Bot, AssignedColor::Green, 0, "spa"),
    ]
}

fn banana_players() -> Vec<PlayerSnapshot> {
    vec![
        PlayerSnapshot {
            item_cue: Some(ItemCueSnapshot {
                kind: ItemCueSnapshotKind::Banana {
                    direction: AttackDirectionSnapshot::Ahead,
                },
                unicode_label: "🍌 >>".to_string(),
                ascii_label: "))>>".to_string(),
                placement: ItemCuePlacementSnapshot::After,
                remaining_ms: 700,
            }),
            ..player(1, "you", PlayerKind::Human, AssignedColor::Cyan, 3, "cy")
        },
        PlayerSnapshot {
            stunned: true,
            impact_cue: Some(ImpactCueSnapshot {
                kind: ImpactCueSnapshotKind::Banana,
                remaining_ms: 900,
            }),
            ..player(2, "alex", PlayerKind::Human, AssignedColor::Red, 4, "ma")
        },
    ]
}

fn mushroom_players() -> Vec<PlayerSnapshot> {
    vec![
        PlayerSnapshot {
            boosted: true,
            ..player(1, "you", PlayerKind::Human, AssignedColor::Cyan, 4, "ma")
        },
        player(2, "alex", PlayerKind::Human, AssignedColor::Red, 3, "cy"),
    ]
}

fn shield_focus_players() -> Vec<PlayerSnapshot> {
    vec![
        PlayerSnapshot {
            shielded: true,
            ..player(1, "you", PlayerKind::Human, AssignedColor::Cyan, 3, "cy")
        },
        PlayerSnapshot {
            focused: true,
            ..player(2, "alex", PlayerKind::Human, AssignedColor::Red, 3, "cycl")
        },
        player(3, "ai-1", PlayerKind::Bot, AssignedColor::Green, 2, "fo"),
    ]
}

fn cyclone_players() -> Vec<PlayerSnapshot> {
    vec![
        PlayerSnapshot {
            item_cue: Some(ItemCueSnapshot {
                kind: ItemCueSnapshotKind::Cyclone {
                    direction: AttackDirectionSnapshot::Ahead,
                },
                unicode_label: "🌀 >>".to_string(),
                ascii_label: "~~>>".to_string(),
                placement: ItemCuePlacementSnapshot::After,
                remaining_ms: 700,
            }),
            ..player(1, "you", PlayerKind::Human, AssignedColor::Cyan, 4, "ma")
        },
        PlayerSnapshot {
            stunned: true,
            impact_cue: Some(ImpactCueSnapshot {
                kind: ImpactCueSnapshotKind::Cyclone,
                remaining_ms: 900,
            }),
            ..player(2, "alex", PlayerKind::Human, AssignedColor::Red, 5, "rob")
        },
    ]
}

fn fog_players() -> Vec<PlayerSnapshot> {
    vec![
        PlayerSnapshot {
            item_cue: Some(ItemCueSnapshot {
                kind: ItemCueSnapshotKind::Fog,
                unicode_label: "⬛ >>".to_string(),
                ascii_label: "FOG>".to_string(),
                placement: ItemCuePlacementSnapshot::After,
                remaining_ms: 700,
            }),
            ..player(1, "you", PlayerKind::Human, AssignedColor::Cyan, 3, "cy")
        },
        PlayerSnapshot {
            fogged: true,
            impact_cue: Some(ImpactCueSnapshot {
                kind: ImpactCueSnapshotKind::Fog,
                remaining_ms: 900,
            }),
            ..player(2, "alex", PlayerKind::Human, AssignedColor::Red, 3, "cyc")
        },
    ]
}

fn finish_players() -> Vec<PlayerSnapshot> {
    vec![
        PlayerSnapshot {
            focused: true,
            finished: true,
            input: "finish".to_string(),
            ..player(1, "you", PlayerKind::Human, AssignedColor::Cyan, 7, "")
        },
        PlayerSnapshot {
            typo_index: Some(3),
            impact_cue: Some(ImpactCueSnapshot {
                kind: ImpactCueSnapshotKind::Cyclone,
                remaining_ms: 900,
            }),
            ..player(2, "alex", PlayerKind::Human, AssignedColor::Red, 6, "pixl")
        },
    ]
}

fn player(
    id: u64,
    name: &str,
    kind: PlayerKind,
    color: AssignedColor,
    word_index: usize,
    input: &str,
) -> PlayerSnapshot {
    PlayerSnapshot {
        id: PlayerId(id),
        name: name.to_string(),
        kind,
        color,
        word_index,
        input: input.to_string(),
        typo_index: None,
        word_overrides: Vec::new(),
        finished: false,
        connected: true,
        shielded: false,
        focused: false,
        fogged: false,
        boosted: false,
        stunned: false,
        impact_remaining_ms: 0,
        impact_cue: None,
        item_cue: None,
    }
}

fn lobby_player(
    id: u64,
    name: &str,
    kind: PlayerKind,
    color: AssignedColor,
    ready: bool,
) -> LobbyPlayer {
    LobbyPlayer {
        id: PlayerId(id),
        name: name.to_string(),
        kind,
        color,
        ready,
        connected: true,
        ai_difficulty: None,
        ai_wpm: None,
    }
}

fn bonus_choice(word: &str, status: BonusChoiceSnapshotStatus) -> BonusChoiceSnapshot {
    BonusChoiceSnapshot {
        word: word.to_string(),
        status,
    }
}

fn mod_config() -> ModConfigSnapshot {
    ModConfigSnapshot {
        word_set_id: "classic".to_string(),
        word_set_name: "Classic".to_string(),
        word_set_hash: "0000000000000001".to_string(),
        item_pack_name: "classic".to_string(),
        item_registry_hash: "0000000000000002".to_string(),
        combined_hash: "a598dc2b".to_string(),
    }
}

fn track_words() -> Vec<String> {
    strings([
        "spark", "river", "focus", "cyclone", "maple", "harbor", "pixel", "finish",
    ])
}

fn strings(values: impl IntoIterator<Item = &'static str>) -> Vec<String> {
    values.into_iter().map(str::to_string).collect()
}

#[cfg(test)]
fn scenario_slugs() -> Vec<&'static str> {
    SCENARIOS.iter().map(|scenario| scenario.slug).collect()
}

#[cfg(test)]
mod tests;
