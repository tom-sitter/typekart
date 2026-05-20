mod fixtures;

use fixtures::{
    GalleryFrame, GalleryScenario, LobbyFrame, ResultsFrame, SCENARIOS, color_class,
    minimap_position, scenario_frame,
};
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::{Message, futures::WebSocket};
use leptos::prelude::*;
use typekart_protocol::{
    BonusChoiceSnapshotStatus, ImpactCueSnapshot, ImpactCueSnapshotKind, ItemCuePlacementSnapshot,
    ItemCueSnapshot, LobbyPlayer, NetworkRacePhase, PlayerKind, PlayerSnapshot, RaceDeltaSnapshot,
    RaceResultStatus, RaceSnapshot, RelayClientMessage, RelayServerMessage, RoomCode,
    ServerMessage,
};
use wasm_bindgen_futures::spawn_local;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let (selected, set_selected) = signal(0usize);
    let (unicode_icons, set_unicode_icons) = signal(true);
    let (mode, set_mode) = signal(AppMode::Gallery);
    let current_scenario = move || SCENARIOS[selected.get()];

    view! {
        <main class="shell">
            <header class="app-header">
                <div>
                    <p class="eyebrow">"TypeKart Web"</p>
                    <h1>"Renderer gallery"</h1>
                </div>
                <div class="actions" aria-label="Game setup placeholders">
                    <button
                        type="button"
                        class:selected=move || mode.get() == AppMode::Gallery
                        on:click=move |_| set_mode.set(AppMode::Gallery)
                    >
                        "Gallery"
                    </button>
                    <button
                        type="button"
                        class:selected=move || mode.get() == AppMode::Join
                        on:click=move |_| set_mode.set(AppMode::Join)
                    >
                        "Join room"
                    </button>
                </div>
            </header>

            {move || match mode.get() {
                AppMode::Gallery => view! {
                    <GalleryControls
                        selected=selected
                        set_selected=set_selected
                        current_scenario=current_scenario
                        unicode_icons=unicode_icons
                        set_unicode_icons=set_unicode_icons
                    />
                    <GalleryFrameView
                        scenario=current_scenario
                        unicode_icons=move || unicode_icons.get()
                    />
                }.into_any(),
                AppMode::Join => view! {
                    <JoinRoomPanel unicode_icons=move || unicode_icons.get() />
                }.into_any(),
            }}
        </main>
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Gallery,
    Join,
}

#[component]
fn GalleryControls(
    selected: ReadSignal<usize>,
    set_selected: WriteSignal<usize>,
    current_scenario: impl Fn() -> GalleryScenario + Copy + Send + Sync + 'static,
    unicode_icons: ReadSignal<bool>,
    set_unicode_icons: WriteSignal<bool>,
) -> impl IntoView {
    view! {
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
                        checked=unicode_icons.get_untracked()
                        on:change=move |event| {
                            set_unicode_icons.set(event_target_checked(&event));
                        }
                    />
                    <span>"Unicode icons"</span>
                </label>
                <p class="note">{move || current_scenario().icon_mode_note}</p>
            </div>
        </section>
    }
}

#[component]
fn JoinRoomPanel(unicode_icons: impl Fn() -> bool + Copy + Send + Sync + 'static) -> impl IntoView {
    let (relay_url, set_relay_url) = signal("wss://typekart-relay.fly.dev".to_string());
    let (room_code, set_room_code) = signal(String::new());
    let (name, set_name) = signal("web-player".to_string());
    let (status, set_status) = signal("Not connected".to_string());
    let (live_frame, set_live_frame) = signal(None::<GalleryFrame>);

    let join = move |_| {
        let relay = relay_url.get_untracked();
        let room = room_code.get_untracked();
        let name = name.get_untracked();
        set_status.set("Connecting...".to_string());
        set_live_frame.set(None);

        spawn_local(async move {
            match observe_room(relay, room, name, set_status, set_live_frame).await {
                Ok(()) => set_status.set("Connection closed".to_string()),
                Err(error) => set_status.set(error),
            }
        });
    };

    view! {
        <section class="panel join-panel">
            <div class="join-grid">
                <label>
                    <span>"Relay"</span>
                    <input
                        type="text"
                        prop:value=move || relay_url.get()
                        on:input=move |event| set_relay_url.set(event_target_value(&event))
                    />
                </label>
                <label>
                    <span>"Room"</span>
                    <input
                        type="text"
                        placeholder="rocket-salad-tiger"
                        prop:value=move || room_code.get()
                        on:input=move |event| set_room_code.set(event_target_value(&event))
                    />
                </label>
                <label>
                    <span>"Name"</span>
                    <input
                        type="text"
                        prop:value=move || name.get()
                        on:input=move |event| set_name.set(event_target_value(&event))
                    />
                </label>
                <button type="button" on:click=join>"Observe"</button>
            </div>
            <p class="note">{move || status.get()}</p>
        </section>

        {move || live_frame.get().map(|frame| {
            match frame {
                GalleryFrame::Lobby(snapshot) => view! { <LobbyPanel snapshot=snapshot /> }.into_any(),
                GalleryFrame::Race(snapshot) => {
                    view! { <RacePanel snapshot=snapshot unicode_icons=unicode_icons /> }.into_any()
                }
                GalleryFrame::Results(snapshot) => view! { <ResultsPanel snapshot=snapshot /> }.into_any(),
            }
        })}
    }
}

async fn observe_room(
    relay_url: String,
    room_code: String,
    name: String,
    set_status: WriteSignal<String>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    let room = RoomCode::parse(&room_code).map_err(|error| error.to_string())?;
    let websocket_url = relay_join_url(&relay_url, &room);
    let websocket = WebSocket::open(&websocket_url)
        .map_err(|error| format!("failed to open relay websocket: {error:?}"))?;
    let (mut writer, mut reader) = websocket.split();
    let join = RelayClientMessage::JoinRoom {
        room: room.clone(),
        name,
        client_version: APP_VERSION.to_string(),
    };
    let encoded_join = serde_json::to_string(&join)
        .map_err(|error| format!("failed to encode join request: {error}"))?;
    writer
        .send(Message::Text(encoded_join))
        .await
        .map_err(|error| format!("failed to send join request: {error:?}"))?;
    set_status.set(format!("Connected to relay, observing {}", room.display()));

    let mut current_race: Option<RaceSnapshot> = None;
    while let Some(message) = reader.next().await {
        let Message::Text(text) =
            message.map_err(|error| format!("failed to read relay message: {error:?}"))?
        else {
            continue;
        };
        let relay_message = serde_json::from_str::<RelayServerMessage>(&text)
            .map_err(|error| format!("failed to decode relay message: {error}"))?;
        handle_relay_message(relay_message, &mut current_race, set_status, set_live_frame)?;
    }

    Ok(())
}

fn handle_relay_message(
    relay_message: RelayServerMessage,
    current_race: &mut Option<RaceSnapshot>,
    set_status: WriteSignal<String>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    match relay_message {
        RelayServerMessage::HostToClient { message, .. }
        | RelayServerMessage::HostBroadcast { message, .. } => {
            let server_message = serde_json::from_value::<ServerMessage>(message)
                .map_err(|error| format!("failed to decode host message: {error}"))?;
            handle_server_message(server_message, current_race, set_status, set_live_frame);
        }
        RelayServerMessage::Error { message } => {
            set_status.set(format!("Relay error: {message}"));
        }
        RelayServerMessage::RoomClosed { reason } => {
            set_status.set(format!("Room closed: {reason}"));
            set_live_frame.set(None);
        }
        RelayServerMessage::ParticipantDisconnected { player_id, .. } => {
            set_status.set(format!("Participant {} disconnected", player_id.0));
        }
        RelayServerMessage::RoomCreated { .. }
        | RelayServerMessage::JoinForwarded { .. }
        | RelayServerMessage::ClientToHost { .. } => {}
    }
    Ok(())
}

fn handle_server_message(
    message: ServerMessage,
    current_race: &mut Option<RaceSnapshot>,
    set_status: WriteSignal<String>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) {
    match message {
        ServerMessage::Welcome {
            player_id,
            assigned_color,
        } => {
            set_status.set(format!(
                "Joined as player {} ({assigned_color:?})",
                player_id.0
            ));
        }
        ServerMessage::LobbySnapshot {
            players,
            host_id,
            mod_config,
            events,
        } => {
            set_live_frame.set(Some(GalleryFrame::Lobby(LobbyFrame {
                host_id,
                players,
                mod_config,
                events,
            })));
        }
        ServerMessage::RaceSnapshot(snapshot) => {
            *current_race = Some(snapshot.clone());
            set_live_frame.set(Some(GalleryFrame::Race(snapshot)));
        }
        ServerMessage::RaceDelta(delta) => {
            if let Some(snapshot) = apply_delta(current_race.take(), delta) {
                *current_race = Some(snapshot.clone());
                set_live_frame.set(Some(GalleryFrame::Race(snapshot)));
            } else {
                set_status.set("Received race delta before full race snapshot".to_string());
            }
        }
        ServerMessage::RaceEvent { message } => {
            set_status.set(message);
        }
        ServerMessage::RaceResults { placements, rows } => {
            set_live_frame.set(Some(GalleryFrame::Results(ResultsFrame {
                placements,
                rows,
                events: Vec::new(),
            })));
        }
        ServerMessage::Error { message } => {
            set_status.set(format!("Host error: {message}"));
        }
    }
}

fn apply_delta(
    current_race: Option<RaceSnapshot>,
    delta: RaceDeltaSnapshot,
) -> Option<RaceSnapshot> {
    let mut snapshot = current_race?;
    snapshot.sequence = delta.sequence;
    snapshot.phase = delta.phase;
    snapshot.bonuses = delta.bonuses;
    snapshot.players = delta.players;
    snapshot.events = delta.events;
    Some(snapshot)
}

fn relay_join_url(relay: &str, room: &RoomCode) -> String {
    let base = if relay_has_path_or_query(relay) {
        relay.to_string()
    } else {
        format!("{relay}/")
    };
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}typekart_room={}", room.as_str())
}

fn relay_has_path_or_query(relay: &str) -> bool {
    relay.contains('?')
        || relay
            .split_once("://")
            .is_none_or(|(_, rest)| rest.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::relay_join_url;
    use typekart_protocol::RoomCode;

    #[test]
    fn relay_join_url_adds_room_query_to_plain_relay_url() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();

        assert_eq!(
            relay_join_url("ws://127.0.0.1:8080", &room),
            "ws://127.0.0.1:8080/?typekart_room=rocket-salad-tiger"
        );
    }

    #[test]
    fn relay_join_url_preserves_existing_path_and_query() {
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();

        assert_eq!(
            relay_join_url("wss://relay.example/ws?debug=true", &room),
            "wss://relay.example/ws?debug=true&typekart_room=rocket-salad-tiger"
        );
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
