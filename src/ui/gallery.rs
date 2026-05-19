//! Interactive renderer previews for development.
//!
//! The gallery builds deterministic race states and renders them through the
//! production local race renderer. It is deliberately not a simulator: each
//! scenario exists to make one cue, effect, or screenshot composition easy to
//! inspect.

use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    game::{
        ai::AiDifficulty,
        bonus::{BonusChoice, BonusPoint, BonusState},
        effects::ActiveEffect,
        player::PlayerState,
        track::Track,
    },
    ui::{
        render::{IconMode, TypingScreen, render},
        session::{
            AiRacer, AttackDirection, EventLog, ImpactCue, ImpactCueKind, ItemCue, ItemCueKind,
            RacePhase, RaceStatus,
        },
    },
};

type GalleryTerminal = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalleryKind {
    Items { scenario: Option<String> },
}

pub fn run_gallery(kind: GalleryKind, icon_mode: IconMode) -> Result<()> {
    match kind {
        GalleryKind::Items { scenario } => run_item_gallery(icon_mode, scenario),
    }
}

fn run_item_gallery(icon_mode: IconMode, scenario: Option<String>) -> Result<()> {
    let scenarios = gallery_scenarios();
    let scenario_index = scenario
        .as_deref()
        .map(|slug| scenario_index_by_slug(&scenarios, slug))
        .transpose()?
        .unwrap_or(0);
    let mut terminal = setup_terminal()?;
    let result = run_item_gallery_loop(&mut terminal, icon_mode, scenarios, scenario_index);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<GalleryTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut GalleryTerminal) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_item_gallery_loop(
    terminal: &mut GalleryTerminal,
    icon_mode: IconMode,
    scenarios: Vec<GalleryScenario>,
    mut scenario_index: usize,
) -> Result<()> {
    let mut icon_mode = icon_mode;
    let mut show_help = false;
    let mut state = scenario_state(scenarios[scenario_index], Instant::now());

    loop {
        let scenario = scenarios[scenario_index];
        terminal.draw(|frame| {
            render(frame, state.screen(icon_mode));
            render_gallery_footer(frame, scenario, scenario_index, scenarios.len(), icon_mode);
            if show_help {
                render_gallery_help(frame, frame.size());
            }
        })?;

        if !event::poll(Duration::from_millis(80))? {
            continue;
        }

        let Event::Key(key_event) = event::read()? else {
            continue;
        };
        if key_event.kind != KeyEventKind::Press {
            continue;
        }

        if key_event.code == KeyCode::Esc
            || (key_event.code == KeyCode::Char('c')
                && key_event.modifiers.contains(KeyModifiers::CONTROL))
        {
            return Ok(());
        }

        match key_event.code {
            KeyCode::Right | KeyCode::Char('l') => {
                scenario_index = (scenario_index + 1) % scenarios.len();
                state = scenario_state(scenarios[scenario_index], Instant::now());
            }
            KeyCode::Left | KeyCode::Char('h') => {
                scenario_index = scenario_index
                    .checked_sub(1)
                    .unwrap_or_else(|| scenarios.len() - 1);
                state = scenario_state(scenarios[scenario_index], Instant::now());
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                icon_mode = match icon_mode {
                    IconMode::Ascii => IconMode::Unicode,
                    IconMode::Unicode => IconMode::Ascii,
                };
            }
            KeyCode::Char('?') => {
                show_help = !show_help;
                terminal.clear()?;
            }
            _ => {}
        }
    }
}

struct GalleryState {
    track: Track,
    player: PlayerState,
    bonuses: BonusState,
    player_impact_cue: Option<ImpactCue>,
    player_item_cue: Option<ItemCue>,
    ai_racers: Vec<AiRacer>,
    events: EventLog,
    rendered_at: Instant,
}

impl GalleryState {
    fn screen(&self, icon_mode: IconMode) -> TypingScreen<'_> {
        TypingScreen {
            track: &self.track,
            player: &self.player,
            bonuses: &self.bonuses,
            bonus_attempt: None,
            player_impact_cue: self.player_impact_cue,
            player_item_cue: self.player_item_cue.clone(),
            race_status: RaceStatus::default(),
            race_phase: RacePhase::Racing,
            icon_mode,
            ai_racers: &self.ai_racers,
            selected_ai_index: None,
            show_help: false,
            events: &self.events,
            rendered_at: self.rendered_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GalleryScenario {
    MultiplayerPack,
    MultiplayerOpening,
    BonusScramble,
    ComebackChase,
    BananaHitPack,
    SquidInkPack,
    ItemPileup,
    FinishSprint,
    MushroomBoost,
    ShieldActive,
    FocusActive,
    BananaAhead,
    BananaBehind,
    BananaImpact,
    CycloneAhead,
    CycloneImpact,
    SquidInkCue,
    SquidInkImpact,
    SquidInkMaskedWords,
}

impl GalleryScenario {
    fn slug(self) -> &'static str {
        match self {
            Self::MultiplayerPack => "multiplayer-pack",
            Self::MultiplayerOpening => "multiplayer-opening",
            Self::BonusScramble => "bonus-scramble",
            Self::ComebackChase => "comeback-chase",
            Self::BananaHitPack => "banana-hit-pack",
            Self::SquidInkPack => "squid-ink-pack",
            Self::ItemPileup => "item-pileup",
            Self::FinishSprint => "finish-sprint",
            Self::MushroomBoost => "mushroom-boost",
            Self::ShieldActive => "shield-active",
            Self::FocusActive => "focus-active",
            Self::BananaAhead => "banana-ahead",
            Self::BananaBehind => "banana-behind",
            Self::BananaImpact => "banana-impact",
            Self::CycloneAhead => "cyclone-ahead",
            Self::CycloneImpact => "cyclone-impact",
            Self::SquidInkCue => "squid-ink-cue",
            Self::SquidInkImpact => "squid-ink-impact",
            Self::SquidInkMaskedWords => "squid-ink-masked",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::MultiplayerPack => "Multiplayer pack",
            Self::MultiplayerOpening => "Multiplayer opening",
            Self::BonusScramble => "Bonus scramble",
            Self::ComebackChase => "Comeback chase",
            Self::BananaHitPack => "Banana hit pack",
            Self::SquidInkPack => "Squid Ink pack",
            Self::ItemPileup => "Item pileup",
            Self::FinishSprint => "Finish sprint",
            Self::MushroomBoost => "Mushroom boost",
            Self::ShieldActive => "Shield active",
            Self::FocusActive => "Focus active",
            Self::BananaAhead => "Banana fired ahead",
            Self::BananaBehind => "Banana fired behind",
            Self::BananaImpact => "Banana impact blink",
            Self::CycloneAhead => "Cyclone fired",
            Self::CycloneImpact => "Cyclone impact blink",
            Self::SquidInkCue => "Squid Ink fired",
            Self::SquidInkImpact => "Squid Ink impact blink",
            Self::SquidInkMaskedWords => "Squid Ink masked words",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::MultiplayerPack => "A fuller race scene with six racers, bonus words, and map.",
            Self::MultiplayerOpening => "A tight opening pack just after the race starts.",
            Self::BonusScramble => "Racers converge around a bonus-word pickup point.",
            Self::ComebackChase => "A trailing player lines up a comeback with active effects.",
            Self::BananaHitPack => "Player fires Banana while the target blinks from impact.",
            Self::SquidInkPack => {
                "Squid Ink cue, impacted racers, and masked words beyond the current word."
            }
            Self::ItemPileup => "Multiple simultaneous effects for stress-testing readability.",
            Self::FinishSprint => "Racers near the finish with a finished offscreen marker.",
            Self::MushroomBoost => "Boost prefix on a racer lane.",
            Self::ShieldActive => "Shield icon centered on the kart marker.",
            Self::FocusActive => "Focus icon centered on the kart marker.",
            Self::BananaAhead => "Pickup cue rendered in front of the attacker.",
            Self::BananaBehind => "Pickup cue rendered behind the attacker.",
            Self::BananaImpact => "Yellow blink on the impacted racer.",
            Self::CycloneAhead => "Cyclone cue rendered in front of the attacker.",
            Self::CycloneImpact => "Blue blink on the impacted racer.",
            Self::SquidInkCue => "Squid cue rendered after the attacker.",
            Self::SquidInkImpact => "Black blink on the impacted racer.",
            Self::SquidInkMaskedWords => "Future words hidden until reached or ink expires.",
        }
    }
}

fn gallery_scenarios() -> Vec<GalleryScenario> {
    vec![
        GalleryScenario::MultiplayerPack,
        GalleryScenario::MultiplayerOpening,
        GalleryScenario::BonusScramble,
        GalleryScenario::ComebackChase,
        GalleryScenario::BananaHitPack,
        GalleryScenario::SquidInkPack,
        GalleryScenario::ItemPileup,
        GalleryScenario::FinishSprint,
        GalleryScenario::MushroomBoost,
        GalleryScenario::ShieldActive,
        GalleryScenario::FocusActive,
        GalleryScenario::BananaAhead,
        GalleryScenario::BananaBehind,
        GalleryScenario::BananaImpact,
        GalleryScenario::CycloneAhead,
        GalleryScenario::CycloneImpact,
        GalleryScenario::SquidInkCue,
        GalleryScenario::SquidInkImpact,
        GalleryScenario::SquidInkMaskedWords,
    ]
}

fn scenario_index_by_slug(scenarios: &[GalleryScenario], slug: &str) -> Result<usize> {
    scenarios
        .iter()
        .position(|scenario| scenario.slug() == slug)
        .ok_or_else(|| {
            let valid = scenarios
                .iter()
                .map(|scenario| scenario.slug())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!("unknown gallery scenario '{slug}'. Valid scenarios: {valid}")
        })
}

fn scenario_state(scenario: GalleryScenario, now: Instant) -> GalleryState {
    let track = Track {
        words: [
            "signal", "orbit", "driver", "velvet", "rocket", "planet", "silver", "forest",
            "castle", "finish",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
    };
    let bonuses = gallery_bonuses();
    let mut player = PlayerState::new(now - Duration::from_secs(20));
    player.word_index = 4;
    player.stats.completed_words = 4;

    let mut ai_ahead = ai_racer(1, "ahead", 6, 72.0, now);
    let mut ai_behind = ai_racer(2, "behind", 2, 68.0, now);
    let mut extra_ai = vec![
        ai_racer(3, "rival", 5, 82.0, now),
        ai_racer(4, "boost", 4, 76.0, now),
        ai_racer(5, "shield", 7, 70.0, now),
        ai_racer(6, "sprint", 8, 88.0, now),
    ];

    let mut player_impact_cue = None;
    let mut player_item_cue = None;
    let mut events = EventLog::new(8);
    events.push(scenario.title());
    events.push(scenario.description());

    match scenario {
        GalleryScenario::MultiplayerPack => {
            player.input = "ro".to_string();
            extra_ai[1]
                .player
                .active_effects
                .push(ActiveEffect::Mushroom {
                    remaining_words: 2,
                    next_step_at: now + Duration::from_secs(1),
                    step_interval: Duration::from_millis(350),
                });
            extra_ai[2]
                .player
                .active_effects
                .push(ActiveEffect::Shield {
                    until: now + Duration::from_secs(5),
                });
            events.push("Bonus choices visible ahead");
            events.push("Six racers in the same race window");
        }
        GalleryScenario::MultiplayerOpening => {
            player.word_index = 0;
            player.stats.completed_words = 0;
            player.input = "sig".to_string();
            ai_ahead.player.word_index = 1;
            ai_ahead.player.stats.completed_words = 1;
            ai_behind.player.word_index = 0;
            ai_behind.player.stats.completed_words = 0;
            extra_ai[0].player.word_index = 1;
            extra_ai[0].player.stats.completed_words = 1;
            extra_ai[1].player.word_index = 0;
            extra_ai[1].player.stats.completed_words = 0;
            extra_ai[2].player.word_index = 2;
            extra_ai[2].player.stats.completed_words = 2;
            extra_ai[3].player.word_index = 1;
            extra_ai[3].player.stats.completed_words = 1;
            extra_ai[1].item_cue = Some(item_cue(
                ItemCueKind::Banana {
                    direction: AttackDirection::Ahead,
                },
                " ))>>",
                " 🍌 >>",
                now,
            ));
            extra_ai[2].player.active_effects.push(ActiveEffect::Focus {
                until: now + Duration::from_secs(5),
            });
            events.push("The pack is still bunched together");
            events.push("First item cues are starting to appear");
        }
        GalleryScenario::BonusScramble => {
            player.word_index = 4;
            player.stats.completed_words = 4;
            player.input = String::new();
            ai_ahead.player.word_index = 4;
            ai_ahead.player.stats.completed_words = 4;
            ai_behind.player.word_index = 3;
            ai_behind.player.stats.completed_words = 3;
            extra_ai[0].player.word_index = 4;
            extra_ai[0].player.stats.completed_words = 4;
            extra_ai[1].player.word_index = 3;
            extra_ai[1].player.stats.completed_words = 3;
            extra_ai[2].player.word_index = 5;
            extra_ai[2].player.stats.completed_words = 5;
            extra_ai[3].player.word_index = 2;
            extra_ai[3].player.stats.completed_words = 2;
            ai_ahead.player.active_effects.push(ActiveEffect::Shield {
                until: now + Duration::from_secs(5),
            });
            ai_behind.player.active_effects.push(ActiveEffect::Focus {
                until: now + Duration::from_secs(5),
            });
            extra_ai[0].item_cue = Some(item_cue(ItemCueKind::SquidInk, " ink ", " 🦑 ", now));
            events.push("Bonus words are live in the middle lane");
            events.push("Shield, Focus, and Squid Ink pressure overlap");
        }
        GalleryScenario::ComebackChase => {
            player.word_index = 2;
            player.stats.completed_words = 2;
            player.input = "dr".to_string();
            player.active_effects.push(ActiveEffect::Mushroom {
                remaining_words: 2,
                next_step_at: now + Duration::from_secs(1),
                step_interval: Duration::from_millis(350),
            });
            player.active_effects.push(ActiveEffect::Focus {
                until: now + Duration::from_secs(5),
            });
            ai_ahead.player.word_index = 7;
            ai_ahead.player.stats.completed_words = 7;
            ai_ahead.player.active_effects.push(ActiveEffect::Shield {
                until: now + Duration::from_secs(5),
            });
            ai_behind.player.word_index = 1;
            ai_behind.player.stats.completed_words = 1;
            extra_ai[0].player.word_index = 5;
            extra_ai[0].player.stats.completed_words = 5;
            extra_ai[0].impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::Banana,
                until: now + Duration::from_secs(2),
            });
            extra_ai[1].player.word_index = 4;
            extra_ai[1].player.stats.completed_words = 4;
            extra_ai[1].item_cue = Some(item_cue(
                ItemCueKind::Cyclone {
                    direction: AttackDirection::Ahead,
                },
                " cy>>",
                " 🌀 >>",
                now,
            ));
            extra_ai[2].player.word_index = 6;
            extra_ai[2].player.stats.completed_words = 6;
            extra_ai[3].player.word_index = 3;
            extra_ai[3].player.stats.completed_words = 3;
            events.push("you are boosted and focused");
            events.push("The leader is shielded but still in reach");
        }
        GalleryScenario::BananaHitPack => {
            player_item_cue = Some(item_cue(
                ItemCueKind::Banana {
                    direction: AttackDirection::Ahead,
                },
                " ))>>",
                " 🍌 >>",
                now,
            ));
            ai_ahead.impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::Banana,
                until: now + Duration::from_secs(2),
            });
            extra_ai[0]
                .player
                .active_effects
                .push(ActiveEffect::Shield {
                    until: now + Duration::from_secs(5),
                });
            events.push("you picked up Banana");
            events.push("you hit ahead");
        }
        GalleryScenario::SquidInkPack => {
            player_item_cue = Some(item_cue(ItemCueKind::SquidInk, " ink ", " 🦑 ", now));
            ai_ahead.impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::SquidInk,
                until: now + Duration::from_secs(2),
            });
            ai_behind.impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::SquidInk,
                until: now + Duration::from_secs(2),
            });
            player.inked_word_index = Some(player.word_index);
            player.inked_until = Some(now + Duration::from_secs(5));
            player_impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::SquidInk,
                until: now + Duration::from_secs(2),
            });
            events.push("Squid Ink hit 3 racer(s)");
            events.push("Future words are hidden until reached");
        }
        GalleryScenario::ItemPileup => {
            player.active_effects.push(ActiveEffect::Mushroom {
                remaining_words: 2,
                next_step_at: now + Duration::from_secs(1),
                step_interval: Duration::from_millis(350),
            });
            ai_ahead.impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::Cyclone,
                until: now + Duration::from_secs(2),
            });
            ai_behind.player.active_effects.push(ActiveEffect::Focus {
                until: now + Duration::from_secs(5),
            });
            extra_ai[0]
                .player
                .active_effects
                .push(ActiveEffect::Shield {
                    until: now + Duration::from_secs(5),
                });
            extra_ai[1].item_cue = Some(item_cue(
                ItemCueKind::Banana {
                    direction: AttackDirection::Behind,
                },
                "((<< ",
                "<< 🍌 ",
                now,
            ));
            extra_ai[2].impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::SquidInk,
                until: now + Duration::from_secs(2),
            });
            events.push("Cyclone, Banana, Shield, Focus, and Squid Ink visible");
        }
        GalleryScenario::FinishSprint => {
            player.word_index = 8;
            player.stats.completed_words = 8;
            ai_ahead.player.word_index = track.len();
            ai_ahead.player.stats.completed_words = track.len();
            ai_ahead.player.finished_at = Some(now - Duration::from_secs(1));
            ai_behind.player.word_index = 7;
            ai_behind.player.stats.completed_words = 7;
            extra_ai[0].player.word_index = 9;
            extra_ai[0].player.stats.completed_words = 9;
            extra_ai[0].player.input = "fi".to_string();
            extra_ai.truncate(2);
            events.push("ahead finished");
            events.push("sprint is typing the final word");
        }
        GalleryScenario::MushroomBoost => {
            player.active_effects.push(ActiveEffect::Mushroom {
                remaining_words: 2,
                next_step_at: now + Duration::from_secs(1),
                step_interval: Duration::from_millis(350),
            });
        }
        GalleryScenario::ShieldActive => {
            player.active_effects.push(ActiveEffect::Shield {
                until: now + Duration::from_secs(5),
            });
        }
        GalleryScenario::FocusActive => {
            player.active_effects.push(ActiveEffect::Focus {
                until: now + Duration::from_secs(5),
            });
        }
        GalleryScenario::BananaAhead => {
            player_item_cue = Some(item_cue(
                ItemCueKind::Banana {
                    direction: AttackDirection::Ahead,
                },
                " ))>>",
                " 🍌 >>",
                now,
            ));
        }
        GalleryScenario::BananaBehind => {
            player_item_cue = Some(item_cue(
                ItemCueKind::Banana {
                    direction: AttackDirection::Behind,
                },
                "((<< ",
                "<< 🍌 ",
                now,
            ));
        }
        GalleryScenario::BananaImpact => {
            ai_ahead.impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::Banana,
                until: now + Duration::from_secs(2),
            });
        }
        GalleryScenario::CycloneAhead => {
            player_item_cue = Some(item_cue(
                ItemCueKind::Cyclone {
                    direction: AttackDirection::Ahead,
                },
                " cy>>",
                " 🌀 >>",
                now,
            ));
        }
        GalleryScenario::CycloneImpact => {
            ai_ahead.impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::Cyclone,
                until: now + Duration::from_secs(2),
            });
        }
        GalleryScenario::SquidInkCue => {
            player_item_cue = Some(item_cue(ItemCueKind::SquidInk, " ink ", " 🦑 ", now));
        }
        GalleryScenario::SquidInkImpact => {
            ai_ahead.impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::SquidInk,
                until: now + Duration::from_secs(2),
            });
        }
        GalleryScenario::SquidInkMaskedWords => {
            player.inked_word_index = Some(player.word_index);
            player.inked_until = Some(now + Duration::from_secs(5));
            player_impact_cue = Some(ImpactCue {
                kind: ImpactCueKind::SquidInk,
                until: now + Duration::from_secs(2),
            });
        }
    }

    let mut ai_racers = vec![ai_ahead, ai_behind];
    ai_racers.extend(extra_ai);

    GalleryState {
        track,
        player,
        bonuses,
        player_impact_cue,
        player_item_cue,
        ai_racers,
        events,
        rendered_at: now,
    }
}

fn gallery_bonuses() -> BonusState {
    BonusState::with_points(
        vec![BonusPoint::new(
            3,
            [
                BonusChoice::available("turbo"),
                BonusChoice::available("shield"),
                BonusChoice::available("squid"),
            ],
        )],
        vec![
            "turbo".to_string(),
            "shield".to_string(),
            "squid".to_string(),
        ],
    )
}

fn ai_racer(id: usize, name: &'static str, word_index: usize, wpm: f64, now: Instant) -> AiRacer {
    let mut ai = AiRacer::new(id, AiDifficulty::Easy, wpm, now);
    ai.name = name.to_string();
    ai.player.word_index = word_index;
    ai.player.stats.completed_words = word_index;
    ai
}

fn item_cue(
    kind: ItemCueKind,
    ascii_label: &'static str,
    unicode_label: &'static str,
    now: Instant,
) -> ItemCue {
    ItemCue {
        kind,
        until: now + Duration::from_secs(2),
        ascii_label: ascii_label.to_string(),
        unicode_label: unicode_label.to_string(),
    }
}

fn render_gallery_footer(
    frame: &mut Frame<'_>,
    scenario: GalleryScenario,
    scenario_index: usize,
    scenario_count: usize,
    icon_mode: IconMode,
) {
    let area = frame.size();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(4)])
        .split(area);
    let mode = match icon_mode {
        IconMode::Ascii => "ASCII",
        IconMode::Unicode => "Unicode",
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(
                    "Gallery {}/{}: {}",
                    scenario_index + 1,
                    scenario_count,
                    scenario.title()
                ),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("    {mode}")),
        ]),
        Line::from(scenario.description()),
        Line::from(format!(
            "Scenario: {} | Left/Right previous/next | A toggle ASCII/Unicode | ? help | Esc quit",
            scenario.slug()
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title("Renderer Gallery")
                .borders(Borders::ALL),
        ),
        rows[1],
    );
}

fn render_gallery_help(frame: &mut Frame<'_>, area: Rect) {
    let overlay = centered_rect(area, 68, 12);
    frame.render_widget(Clear, overlay);
    let lines = vec![
        Line::from(vec![
            Span::styled("Key", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("          "),
            Span::styled("Action", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from("Left / Right  Previous or next scenario"),
        Line::from("A             Toggle ASCII and Unicode icons"),
        Line::from("?             Hide this help"),
        Line::from("Esc / Ctrl-C  Quit gallery"),
        Line::from(""),
        Line::from("--scenario banana-hit-pack jumps directly to a screenshot state"),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title("Gallery Help").borders(Borders::ALL)),
        overlay,
    );
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::{GalleryScenario, gallery_scenarios, scenario_index_by_slug};

    #[test]
    fn item_gallery_covers_current_item_effects_and_rollout_states() {
        let scenarios = gallery_scenarios();

        assert!(scenarios.contains(&GalleryScenario::MushroomBoost));
        assert!(scenarios.contains(&GalleryScenario::ShieldActive));
        assert!(scenarios.contains(&GalleryScenario::FocusActive));
        assert!(scenarios.contains(&GalleryScenario::BananaAhead));
        assert!(scenarios.contains(&GalleryScenario::CycloneAhead));
        assert!(scenarios.contains(&GalleryScenario::SquidInkMaskedWords));
        assert!(scenarios.contains(&GalleryScenario::MultiplayerPack));
        assert!(scenarios.contains(&GalleryScenario::MultiplayerOpening));
        assert!(scenarios.contains(&GalleryScenario::BonusScramble));
        assert!(scenarios.contains(&GalleryScenario::ComebackChase));
        assert!(scenarios.contains(&GalleryScenario::BananaHitPack));
        assert!(scenarios.contains(&GalleryScenario::SquidInkPack));
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
        assert_eq!(bonuses.points[0].choices[2].word, "squid");
    }
}
