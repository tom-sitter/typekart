mod fixtures;

use fixtures::{
    GalleryFrame, GalleryScenario, LobbyFrame, ResultsFrame, SCENARIOS, color_class,
    minimap_position, scenario_frame,
};
use leptos::prelude::*;
use typekart_protocol::{
    BonusChoiceSnapshotStatus, ImpactCueSnapshot, ImpactCueSnapshotKind, ItemCuePlacementSnapshot,
    ItemCueSnapshot, LobbyPlayer, NetworkRacePhase, PlayerKind, PlayerSnapshot, RaceResultStatus,
    RaceSnapshot,
};

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let (selected, set_selected) = signal(0usize);
    let (unicode_icons, set_unicode_icons) = signal(true);
    let current_scenario = move || SCENARIOS[selected.get()];

    view! {
        <main class="shell">
            <header class="app-header">
                <div>
                    <p class="eyebrow">"TypeKart Web"</p>
                    <h1>"Renderer gallery"</h1>
                </div>
                <div class="actions" aria-label="Game setup placeholders">
                    <button type="button">"Create room"</button>
                    <button type="button" class="secondary">"Join room"</button>
                </div>
            </header>

            <section class="panel gallery-layout">
                <nav class="scenario-list" aria-label="Gallery scenarios">
                    <For
                        each=|| SCENARIOS.iter().copied().enumerate()
                        key=|(_, scenario)| scenario.slug
                        children={move |(index, scenario)| {
                            view! {
                                <button
                                    type="button"
                                    class:selected=move || selected.get() == index
                                    on:click=move |_| set_selected.set(index)
                                >
                                    {scenario.title}
                                </button>
                            }
                        }}
                    />
                </nav>

                <div class="scenario-copy">
                    <h2>{move || current_scenario().title}</h2>
                    <p>{move || current_scenario().description}</p>
                    <label class="toggle">
                        <input
                            type="checkbox"
                            checked=true
                            on:change=move |event| {
                                set_unicode_icons.set(event_target_checked(&event));
                            }
                        />
                        <span>"Unicode icons"</span>
                    </label>
                    <p class="note">{move || current_scenario().icon_mode_note}</p>
                </div>
            </section>

            <GalleryFrameView
                scenario=current_scenario
                unicode_icons=move || unicode_icons.get()
            />
        </main>
    }
}

#[component]
fn GalleryFrameView(
    scenario: impl Fn() -> GalleryScenario + Copy + Send + Sync + 'static,
    unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> impl IntoView {
    move || match scenario_frame(scenario()) {
        GalleryFrame::Lobby(snapshot) => view! { <LobbyPanel snapshot=snapshot /> }.into_any(),
        GalleryFrame::Race(snapshot) => {
            view! { <RacePanel snapshot=snapshot unicode_icons=unicode_icons /> }.into_any()
        }
        GalleryFrame::Results(snapshot) => view! { <ResultsPanel snapshot=snapshot /> }.into_any(),
    }
}

#[component]
fn LobbyPanel(snapshot: LobbyFrame) -> impl IntoView {
    view! {
        <section class="panel lobby-panel" aria-label="Lobby preview">
            <div class="phase">"Lobby"</div>
            <div class="mod-strip">
                <span>"Words " {snapshot.mod_config.word_set_name}</span>
                <span>"Items " {snapshot.mod_config.item_pack_name}</span>
                <span>"Mod " {snapshot.mod_config.combined_hash}</span>
            </div>
            <div class="lobby-list">
                <For
                    each={move || snapshot.players.clone()}
                    key=|player: &LobbyPlayer| player.id
                    children={move |player| {
                        view! {
                            <div class="lobby-player">
                                <span class={format!("marker {}", color_class(player.color))}>{"●"}</span>
                                <span>{player.name}</span>
                                <span>{kind_label(player.kind)}</span>
                                <span>{if player.id == snapshot.host_id { "host" } else { "" }}</span>
                                <span>{if player.ready { "ready" } else { "not ready" }}</span>
                                <span>{if player.connected { "" } else { "offline" }}</span>
                            </div>
                        }
                    }}
                />
            </div>
            <EventFeed events=snapshot.events />
        </section>
    }
}

#[component]
fn RacePanel(
    snapshot: RaceSnapshot,
    unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let players = snapshot.players.clone();
    let local_player_id = players.first().map(|player| player.id);
    let minimap_snapshot = snapshot.clone();
    let events = snapshot.events.clone();

    view! {
        <section class="panel track-panel" aria-label="Race preview">
            <div class="phase">{phase_label(snapshot.phase)} " · seq " {snapshot.sequence}</div>
            <div class="mod-strip">
                <span>{snapshot.mod_config.word_set_name.clone()}</span>
                <span>{snapshot.mod_config.item_pack_name.clone()}</span>
                <span>{snapshot.mod_config.combined_hash.clone()}</span>
            </div>
            <BonusStack snapshot=snapshot.clone() />
            <TrackWords snapshot=snapshot.clone() />
            <For
                each={move || players.clone()}
                key=|player: &PlayerSnapshot| player.id
                children={move |player| {
                    view! {
                        <RacerLane
                            player=player.clone()
                            local=Some(player.id) == local_player_id
                            unicode_icons=unicode_icons
                        />
                    }
                }}
            />
            <Minimap snapshot=minimap_snapshot />
            <EventFeed events=events />
        </section>
    }
}

#[component]
fn BonusStack(snapshot: RaceSnapshot) -> impl IntoView {
    view! {
        <div class="bonus-row">
            <For
                each={move || snapshot.bonuses.clone()}
                key=|bonus| bonus.after_word_index
                children={|bonus| {
                    view! {
                        <div class="bonus-stack">
                            <For
                                each=move || bonus.choices.clone()
                                key=|choice| choice.word.clone()
                                children={|choice| {
                                    let unavailable = matches!(choice.status, BonusChoiceSnapshotStatus::Cooldown { .. });
                                    view! {
                                        <span class:cooldown=unavailable>
                                            {choice.word}
                                            {cooldown_label(choice.status)}
                                        </span>
                                    }
                                }}
                            />
                        </div>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn TrackWords(snapshot: RaceSnapshot) -> impl IntoView {
    let words = snapshot.track_words.clone();
    let word_snapshot = snapshot.clone();

    view! {
        <div class="track-text">
            <For
                each={move || words.clone().into_iter().enumerate()}
                key=|(index, _)| *index
                children={move |(index, word)| {
                    view! {
                        <span>{display_word(&word_snapshot, index, &word)}</span>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn RacerLane(
    player: PlayerSnapshot,
    local: bool,
    unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let cue_before = player
        .item_cue
        .as_ref()
        .filter(|cue| cue.placement == ItemCuePlacementSnapshot::Before)
        .map(|cue| cue_label(cue, unicode_icons()).to_string());
    let cue_after = player
        .item_cue
        .as_ref()
        .filter(|cue| cue.placement == ItemCuePlacementSnapshot::After)
        .map(|cue| cue_label(cue, unicode_icons()).to_string());

    view! {
        <div
            class="lane"
            class:self-lane=local
            class:stunned=player.stunned
            class:disconnected=!player.connected
        >
            <span class="lane-name">{player.name.clone()}</span>
            <span class:typo=player.typo_index.is_some() class="lane-progress">{player.input.clone()}</span>
            <span class="lane-marker-wrap">
                {cue_before}
                <span class={format!("marker {}", color_class(player.color))}>
                    {effect_prefix(&player, unicode_icons())}
                    {"███"}
                    {impact_label(player.impact_cue, unicode_icons())}
                </span>
                {cue_after}
            </span>
            <span class="lane-status">{status_label(&player)}</span>
        </div>
    }
}

#[component]
fn Minimap(snapshot: RaceSnapshot) -> impl IntoView {
    let players = snapshot.players.clone();
    let word_count = snapshot.track_words.len();

    view! {
        <div class="minimap" aria-label="Race minimap">
            <div class="minimap-line"></div>
            <For
                each={move || players.clone()}
                key=|player: &PlayerSnapshot| player.id
                children={move |player| {
                    let left = minimap_position(&player, word_count);
                    view! {
                        <span
                            class={format!("minimap-dot {}", color_class(player.color))}
                            style={format!("left: {left}%")}
                            title={player.name}
                        ></span>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn ResultsPanel(snapshot: ResultsFrame) -> impl IntoView {
    view! {
        <section class="panel results-panel" aria-label="Results preview">
            <div class="phase">"Results"</div>
            <div class="result-grid">
                <For
                    each={move || snapshot.rows.clone()}
                    key=|row| row.placement
                    children={|row| {
                        view! {
                            <div class="result-row">
                                <span>{row.placement}</span>
                                <span class={format!("marker {}", color_class(row.color))}>{"●"}</span>
                                <span>{row.name}</span>
                                <span>{result_status_label(row.status)}</span>
                                <span>{row.progress_words} "/" {row.track_words}</span>
                                <span>{row.wpm} " wpm"</span>
                                <span>{row.accuracy_percent} "%"</span>
                            </div>
                        }
                    }}
                />
            </div>
            <EventFeed events=snapshot.events />
        </section>
    }
}

#[component]
fn EventFeed(events: Vec<String>) -> impl IntoView {
    view! {
        <div class="events">
            <For
                each={move || events.clone()}
                key=|event| event.clone()
                children={|event| view! { <span>{event}</span> }}
            />
        </div>
    }
}

fn phase_label(phase: NetworkRacePhase) -> String {
    match phase {
        NetworkRacePhase::Lobby => "Lobby".to_string(),
        NetworkRacePhase::WaitingForHost => "Waiting for host".to_string(),
        NetworkRacePhase::Countdown { remaining_seconds } => {
            format!("Countdown: {remaining_seconds}")
        }
        NetworkRacePhase::Racing => "Racing".to_string(),
        NetworkRacePhase::Finished => "Finished".to_string(),
    }
}

fn display_word(snapshot: &RaceSnapshot, index: usize, word: &str) -> String {
    let Some(local_player) = snapshot.players.first() else {
        return word.to_string();
    };
    fixtures::masked_word(local_player, index, word)
}

fn effect_prefix(player: &PlayerSnapshot, unicode_icons: bool) -> &'static str {
    match unicode_icons {
        true if player.boosted => ">>🍄 ",
        false if player.boosted => ">>> ",
        true if player.shielded => "🛡 ",
        false if player.shielded => "[",
        true if player.focused => "⭐ ",
        false if player.focused => "* ",
        true if player.inked => "⬛ ",
        false if player.inked => "# ",
        _ => "",
    }
}

fn cue_label(cue: &ItemCueSnapshot, unicode_icons: bool) -> &str {
    if unicode_icons {
        &cue.unicode_label
    } else {
        &cue.ascii_label
    }
}

fn impact_label(impact: Option<ImpactCueSnapshot>, unicode_icons: bool) -> &'static str {
    impact
        .map(|impact| match (impact.kind, unicode_icons) {
            (ImpactCueSnapshotKind::Banana, true) => "🍌",
            (ImpactCueSnapshotKind::Banana, false) => "BAN",
            (ImpactCueSnapshotKind::Cyclone, true) => "🌀",
            (ImpactCueSnapshotKind::Cyclone, false) => "CYC",
            (ImpactCueSnapshotKind::SquidInk, true) => "⬛",
            (ImpactCueSnapshotKind::SquidInk, false) => "INK",
            (ImpactCueSnapshotKind::ShieldBlock, true) => "🛡",
            (ImpactCueSnapshotKind::ShieldBlock, false) => "BLK",
        })
        .unwrap_or("")
}

fn status_label(player: &PlayerSnapshot) -> &'static str {
    if player.finished {
        "finished"
    } else if player.stunned {
        "stunned"
    } else if player.inked {
        "inked"
    } else if !player.connected {
        "offline"
    } else {
        "word"
    }
}

fn kind_label(kind: PlayerKind) -> &'static str {
    match kind {
        PlayerKind::Human => "human",
        PlayerKind::Bot => "ai",
    }
}

fn result_status_label(status: RaceResultStatus) -> &'static str {
    match status {
        RaceResultStatus::Finished => "finished",
        RaceResultStatus::TimedOut => "timed out",
        RaceResultStatus::Disconnected => "disconnected",
    }
}

fn cooldown_label(status: BonusChoiceSnapshotStatus) -> String {
    match status {
        BonusChoiceSnapshotStatus::Available => String::new(),
        BonusChoiceSnapshotStatus::Cooldown { remaining_ms } => format!(" ({remaining_ms}ms)"),
    }
}
