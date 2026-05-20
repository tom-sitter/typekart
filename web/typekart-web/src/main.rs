mod fixtures;

use fixtures::{
    BonusStatus, CuePlacement, GalleryFrame, GalleryScenario, LobbySnapshotFixture,
    PlayerKindFixture, PlayerSnapshotFixture, RacePhase, RaceSnapshotFixture,
    ResultsSnapshotFixture, SCENARIOS, minimap_position,
};
use leptos::prelude::*;

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
    move || match scenario().frame {
        GalleryFrame::Lobby(snapshot) => view! { <LobbyPanel snapshot=snapshot /> }.into_any(),
        GalleryFrame::Race(snapshot) => {
            view! { <RacePanel snapshot=snapshot unicode_icons=unicode_icons /> }.into_any()
        }
        GalleryFrame::Results(snapshot) => view! { <ResultsPanel snapshot=snapshot /> }.into_any(),
    }
}

#[component]
fn LobbyPanel(snapshot: LobbySnapshotFixture) -> impl IntoView {
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
                    each={move || snapshot.players.iter().copied().collect::<Vec<_>>()}
                    key=|player| player.id
                    children={move |player| {
                        view! {
                            <div class="lobby-player">
                                <span class={format!("marker {}", player.color_class)}>{"●"}</span>
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
    snapshot: RaceSnapshotFixture,
    unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <section class="panel track-panel" aria-label="Race preview">
            <div class="phase">{phase_label(snapshot.phase)} " · seq " {snapshot.sequence}</div>
            <div class="mod-strip">
                <span>{snapshot.mod_config.word_set_name}</span>
                <span>{snapshot.mod_config.item_pack_name}</span>
                <span>{snapshot.mod_config.combined_hash}</span>
            </div>
            <BonusStack snapshot=snapshot />
            <TrackWords snapshot=snapshot />
            <For
                each={move || snapshot.players.iter().copied().collect::<Vec<_>>()}
                key=|player| player.id
                children={move |player| {
                    view! {
                        <RacerLane
                            player=player
                            local=player.id == snapshot.local_player_id
                            unicode_icons=unicode_icons
                        />
                    }
                }}
            />
            <Minimap snapshot=snapshot />
            <EventFeed events=snapshot.events />
        </section>
    }
}

#[component]
fn BonusStack(snapshot: RaceSnapshotFixture) -> impl IntoView {
    view! {
        <div class="bonus-row">
            <For
                each={move || snapshot.bonuses.iter().copied().collect::<Vec<_>>()}
                key=|bonus| bonus.after_word_index
                children={|bonus| {
                    view! {
                        <div class="bonus-stack">
                            <For
                                each={move || bonus.choices.iter().copied().collect::<Vec<_>>()}
                                key=|choice| choice.word
                                children={|choice| {
                                    let unavailable = matches!(choice.status, BonusStatus::Cooldown { .. });
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
fn TrackWords(snapshot: RaceSnapshotFixture) -> impl IntoView {
    view! {
        <div class="track-text">
            <For
                each={move || snapshot.track_words.iter().copied().enumerate()}
                key=|(index, _)| *index
                children={move |(index, word)| {
                    view! {
                        <span>{display_word(snapshot, index, word)}</span>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn RacerLane(
    player: PlayerSnapshotFixture,
    local: bool,
    unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let cue_before = player
        .item_cue
        .filter(|cue| cue.placement == CuePlacement::Before)
        .map(|cue| cue_label(cue, unicode_icons()));
    let cue_after = player
        .item_cue
        .filter(|cue| cue.placement == CuePlacement::After)
        .map(|cue| cue_label(cue, unicode_icons()));

    view! {
        <div
            class="lane"
            class:self-lane=local
            class:stunned=player.stunned
            class:disconnected=!player.connected
        >
            <span class="lane-name">{player.name}</span>
            <span class:typo=player.typo_index.is_some() class="lane-progress">{player.input}</span>
            <span class="lane-marker-wrap">
                {cue_before}
                <span class={format!("marker {}", player.color_class)}>
                    {effect_prefix(player, unicode_icons())}
                    {"███"}
                    {impact_label(player.impact_cue, unicode_icons())}
                </span>
                {cue_after}
            </span>
            <span class="lane-status">{status_label(player)}</span>
        </div>
    }
}

#[component]
fn Minimap(snapshot: RaceSnapshotFixture) -> impl IntoView {
    view! {
        <div class="minimap" aria-label="Race minimap">
            <div class="minimap-line"></div>
            <For
                each={move || snapshot.players.iter().copied().collect::<Vec<_>>()}
                key=|player| player.id
                children={move |player| {
                    let left = minimap_position(player, snapshot.track_words.len());
                    view! {
                        <span
                            class={format!("minimap-dot {}", player.color_class)}
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
fn ResultsPanel(snapshot: ResultsSnapshotFixture) -> impl IntoView {
    view! {
        <section class="panel results-panel" aria-label="Results preview">
            <div class="phase">"Results"</div>
            <div class="result-grid">
                <For
                    each={move || snapshot.rows.iter().copied().collect::<Vec<_>>()}
                    key=|row| row.placement
                    children={|row| {
                        view! {
                            <div class="result-row">
                                <span>{row.placement}</span>
                                <span class={format!("marker {}", row.color_class)}>{"●"}</span>
                                <span>{row.name}</span>
                                <span>{format!("{:?}", row.status).to_lowercase()}</span>
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
fn EventFeed(events: &'static [&'static str]) -> impl IntoView {
    view! {
        <div class="events">
            <For
                each={move || events.to_vec()}
                key=|event| *event
                children={|event| view! { <span>{event}</span> }}
            />
        </div>
    }
}

fn phase_label(phase: RacePhase) -> String {
    match phase {
        RacePhase::WaitingForHost => "Waiting for host".to_string(),
        RacePhase::Countdown(seconds) => format!("Countdown: {seconds}"),
        RacePhase::Racing => "Racing".to_string(),
        RacePhase::Finished => "Finished".to_string(),
    }
}

fn display_word(snapshot: RaceSnapshotFixture, index: usize, word: &str) -> String {
    let local_player = snapshot
        .players
        .iter()
        .find(|player| player.id == snapshot.local_player_id);
    let Some(local_player) = local_player else {
        return word.to_string();
    };
    fixtures::masked_word(*local_player, index, word)
}

fn effect_prefix(player: PlayerSnapshotFixture, unicode_icons: bool) -> &'static str {
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

fn cue_label(cue: fixtures::ItemCueFixture, unicode_icons: bool) -> &'static str {
    if unicode_icons {
        cue.unicode_label
    } else {
        cue.ascii_label
    }
}

fn impact_label(impact: Option<fixtures::ImpactCueFixture>, unicode_icons: bool) -> &'static str {
    impact
        .map(|impact| {
            if unicode_icons {
                impact.unicode_label
            } else {
                impact.ascii_label
            }
        })
        .unwrap_or("")
}

fn status_label(player: PlayerSnapshotFixture) -> &'static str {
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

fn kind_label(kind: PlayerKindFixture) -> &'static str {
    match kind {
        PlayerKindFixture::Human => "human",
        PlayerKindFixture::Bot => "ai",
    }
}

fn cooldown_label(status: BonusStatus) -> String {
    match status {
        BonusStatus::Available => String::new(),
        BonusStatus::Cooldown { remaining_ms } => format!(" ({remaining_ms}ms)"),
    }
}
