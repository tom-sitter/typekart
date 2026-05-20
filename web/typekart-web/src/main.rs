mod fixtures;

use fixtures::{
    BonusStatus, CuePlacement, GalleryScenario, PlayerEffect, RaceFixture, RacePhase, SCENARIOS,
    minimap_position,
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

            <RacePanel
                scenario=current_scenario
                unicode_icons=move || unicode_icons.get()
            />
        </main>
    }
}

#[component]
fn RacePanel(
    scenario: impl Fn() -> GalleryScenario + Copy + Send + Sync + 'static,
    unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let snapshot = move || scenario().snapshot;

    view! {
        <section class="panel track-panel" aria-label="Race preview">
            <div class="phase">{move || phase_label(snapshot().phase)}</div>
            <BonusStack snapshot=snapshot />
            <TrackWords snapshot=snapshot />
            <For
                each={move || snapshot().players.iter().copied().collect::<Vec<_>>()}
                key=|player| player.id
                children={move |player| {
                    view! {
                        <RacerLane
                            player=player
                            local=player.id == snapshot().local_player_id
                            unicode_icons=unicode_icons
                        />
                    }
                }}
            />
            <Minimap snapshot=snapshot />
            <EventFeed snapshot=snapshot />
        </section>
    }
}

#[component]
fn BonusStack(snapshot: impl Fn() -> RaceFixture + Copy + Send + Sync + 'static) -> impl IntoView {
    view! {
        <div class="bonus-row">
            <For
                each={move || snapshot().bonuses.iter().copied().collect::<Vec<_>>()}
                key=|bonus| bonus.after_word_index
                children={|bonus| {
                    view! {
                        <div class="bonus-stack">
                            <For
                                each={move || bonus.choices.iter().copied().collect::<Vec<_>>()}
                                key=|choice| choice.word
                                children={|choice| {
                                    let unavailable = matches!(choice.status, BonusStatus::Cooldown);
                                    view! {
                                        <span class:cooldown=unavailable>{choice.word}</span>
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
fn TrackWords(snapshot: impl Fn() -> RaceFixture + Copy + Send + Sync + 'static) -> impl IntoView {
    view! {
        <div class="track-text">
            <For
                each=move || snapshot().track_words.iter().copied().enumerate()
                key=|(index, _)| *index
                children={move |(index, word)| {
                    view! {
                        <span>{display_word(snapshot(), index, word)}</span>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn RacerLane(
    player: fixtures::PlayerFixture,
    local: bool,
    unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let cue_before = player
        .cue
        .filter(|cue| cue.placement == CuePlacement::Before)
        .map(|cue| cue_label(cue, unicode_icons()));
    let cue_after = player
        .cue
        .filter(|cue| cue.placement == CuePlacement::After)
        .map(|cue| cue_label(cue, unicode_icons()));

    view! {
        <div class="lane" class:self-lane=local>
            <span class="lane-name">{player.name}</span>
            <span class="lane-progress">{player.typed}</span>
            <span class="lane-marker-wrap">
                {cue_before}
                <span class={format!("marker {}", player.color_class)}>
                    {effect_prefix(player.effect, unicode_icons())}
                    {player.marker}
                    {impact_label(player.impact)}
                </span>
                {cue_after}
            </span>
            <span class="lane-status">{status_label(player)}</span>
        </div>
    }
}

#[component]
fn Minimap(snapshot: impl Fn() -> RaceFixture + Copy + Send + Sync + 'static) -> impl IntoView {
    view! {
        <div class="minimap" aria-label="Race minimap">
            <div class="minimap-line"></div>
            <For
                each={move || snapshot().players.iter().copied().collect::<Vec<_>>()}
                key=|player| player.id
                children={move |player| {
                    let left = minimap_position(player, snapshot().track_words.len());
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
fn EventFeed(snapshot: impl Fn() -> RaceFixture + Copy + Send + Sync + 'static) -> impl IntoView {
    view! {
        <div class="events">
            <For
                each={move || snapshot().events.iter().copied().collect::<Vec<_>>()}
                key=|event| *event
                children={|event| view! { <span>{event}</span> }}
            />
        </div>
    }
}

fn phase_label(phase: RacePhase) -> String {
    match phase {
        RacePhase::Countdown(seconds) => format!("Countdown: {seconds}"),
        RacePhase::Racing => "Racing".to_string(),
        RacePhase::Finished => "Finished".to_string(),
    }
}

fn display_word(snapshot: RaceFixture, index: usize, word: &str) -> String {
    let local_player = snapshot
        .players
        .iter()
        .find(|player| player.id == snapshot.local_player_id);
    let Some(local_player) = local_player else {
        return word.to_string();
    };
    if local_player.effect != Some(PlayerEffect::Inked) || index <= local_player.word_index {
        return word.to_string();
    }
    "█".repeat(word.chars().count())
}

fn effect_prefix(effect: Option<PlayerEffect>, unicode_icons: bool) -> &'static str {
    match (effect, unicode_icons) {
        (Some(PlayerEffect::Mushroom), true) => ">>🍄 ",
        (Some(PlayerEffect::Mushroom), false) => ">>> ",
        (Some(PlayerEffect::Shield), true) => "🛡 ",
        (Some(PlayerEffect::Shield), false) => "[",
        (Some(PlayerEffect::Focus), true) => "⭐ ",
        (Some(PlayerEffect::Focus), false) => "* ",
        (Some(PlayerEffect::Inked), true) => "⬛ ",
        (Some(PlayerEffect::Inked), false) => "# ",
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

fn impact_label(impact: Option<fixtures::ImpactFixture>) -> &'static str {
    impact.map(|impact| impact.label).unwrap_or("")
}

fn status_label(player: fixtures::PlayerFixture) -> &'static str {
    if player.finished {
        "finished"
    } else if player.impact.is_some() {
        "hit"
    } else {
        "word"
    }
}
