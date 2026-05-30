use futures_channel::mpsc::UnboundedSender;
use leptos::prelude::*;
use typekart_protocol::{
    AiDifficultySnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot, ClientMessage,
    ImpactCueSnapshot, ImpactCueSnapshotKind, ItemCuePlacementSnapshot, ItemCueSnapshot,
    LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind, PlayerSnapshot, ProtocolKey,
    RaceResultStatus, RaceSnapshot,
};

use crate::fixtures::{
    self, GalleryFrame, GalleryScenario, LobbyFrame, ResultsFrame, color_class, minimap_position,
    scenario_frame,
};
use crate::session::{
    BrowserCommandSink, BrowserOutboundMessage, ConnectionState, keyboard_event_to_protocol_key,
};

const WEB_TRACK_WORDS_BEHIND: usize = 3;
const WEB_TRACK_VISIBLE_WORDS: usize = 10;

#[component]
pub(crate) fn GalleryFrameView(
    scenario: impl Fn() -> GalleryScenario + Copy + Send + Sync + 'static,
    unicode_icons: ReadSignal<bool>,
) -> impl IntoView {
    move || match scenario_frame(scenario()) {
        GalleryFrame::Lobby(snapshot) => view! {
            <LobbyPanel snapshot=snapshot local_player_id=None />
        }
        .into_any(),
        GalleryFrame::Race(snapshot) => view! {
                <RacePanel
                    snapshot=snapshot
                    local_player_id=None
                    unicode_icons=unicode_icons
                    on_key=|_key| {}
                />
        }
        .into_any(),
        GalleryFrame::Results(snapshot) => view! { <ResultsPanel snapshot=snapshot /> }.into_any(),
    }
}

#[component]
pub(crate) fn LobbyPanel(snapshot: LobbyFrame, local_player_id: Option<PlayerId>) -> impl IntoView {
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
                            <div class="lobby-player" class:self-lane=Some(player.id) == local_player_id>
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
pub(crate) fn BrowserLobbyManagement(
    snapshot: LobbyFrame,
    local_player_id: Option<PlayerId>,
    relay_player_id: Option<PlayerId>,
    outbound: Option<UnboundedSender<BrowserOutboundMessage>>,
    set_connection: WriteSignal<ConnectionState>,
) -> impl IntoView {
    let (rename_value, set_rename_value) = signal(String::new());
    let local_player = local_player_id
        .and_then(|id| snapshot.players.iter().find(|player| player.id == id))
        .cloned();
    let is_host = local_player_id == Some(snapshot.host_id);
    let can_send = relay_player_id.is_some() && outbound.is_some();
    let management_rows = snapshot.players.clone();
    let command_sink = BrowserCommandSink {
        outbound,
        relay_player_id,
        set_connection,
    };

    view! {
        <section class="panel lobby-management" aria-label="Lobby management">
            <div class="phase">"Lobby controls"</div>
            <div class="rename-row">
                <label>
                    <span>"Display name"</span>
                    <input
                        type="text"
                        placeholder={local_player.as_ref().map(|player| player.name.clone()).unwrap_or_else(|| "name".to_string())}
                        prop:value=move || rename_value.get()
                        disabled=move || !can_send
                        on:input=move |event| set_rename_value.set(event_target_value(&event))
                    />
                </label>
                <button
                    type="button"
                    disabled=move || !can_send || rename_value.get().trim().is_empty()
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| {
                        let name = rename_value.get_untracked().trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        command_sink.send(ClientMessage::Rename { name }, "Rename sent");
                        set_rename_value.set(String::new());
                    }
                    }
                >
                    "Rename"
                </button>
            </div>

            <div class="host-controls" hidden=move || !is_host>
                <button
                    type="button"
                    disabled=move || !can_send
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| command_sink.send(ClientMessage::AddAi, "Add AI sent")
                    }
                >
                    "Add AI"
                </button>
                <button
                    type="button"
                    class="secondary"
                    disabled=move || !can_send
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| {
                        command_sink.send(
                            ClientMessage::SetAiDifficulty {
                                player_id: None,
                                difficulty: AiDifficultySnapshot::Easy,
                            },
                            "Set AI difficulty sent",
                        )
                    }
                    }
                >
                    "All AI Easy"
                </button>
                <button
                    type="button"
                    class="secondary"
                    disabled=move || !can_send
                    on:click={
                        let command_sink = command_sink.clone();
                        move |_| {
                        command_sink.send(
                            ClientMessage::SetAiDifficulty {
                                player_id: None,
                                difficulty: AiDifficultySnapshot::Hard,
                            },
                            "Set AI difficulty sent",
                        )
                    }
                    }
                >
                    "All AI Hard"
                </button>
            </div>

            <div class="management-list" hidden=move || !is_host>
                <For
                    each=move || management_rows.clone()
                    key=|player: &LobbyPlayer| player.id
                    children={move |player| {
                        let target_id = player.id;
                        let is_self = Some(target_id) == local_player_id;
                        let is_bot = player.kind == PlayerKind::Bot;
                        let easy_sink = command_sink.clone();
                        let hard_sink = command_sink.clone();
                        let remove_sink = command_sink.clone();
                        view! {
                            <div class="management-player">
                                <span class={format!("marker {}", color_class(player.color))}>{"●"}</span>
                                <span>{player.name.clone()}</span>
                                <span>{kind_label(player.kind)}</span>
                                <span>{ai_detail_label(&player)}</span>
                                <button
                                    type="button"
                                    class="secondary"
                                    hidden=move || !is_bot
                                    disabled=move || !can_send
                                    on:click=move |_| {
                                        easy_sink.send(
                                            ClientMessage::SetAiDifficulty {
                                                player_id: Some(target_id),
                                                difficulty: AiDifficultySnapshot::Easy,
                                            },
                                            "Set AI difficulty sent",
                                        )
                                    }
                                >
                                    "Easy"
                                </button>
                                <button
                                    type="button"
                                    class="secondary"
                                    hidden=move || !is_bot
                                    disabled=move || !can_send
                                    on:click=move |_| {
                                        hard_sink.send(
                                            ClientMessage::SetAiDifficulty {
                                                player_id: Some(target_id),
                                                difficulty: AiDifficultySnapshot::Hard,
                                            },
                                            "Set AI difficulty sent",
                                        )
                                    }
                                >
                                    "Hard"
                                </button>
                                <button
                                    type="button"
                                    class="secondary danger"
                                    hidden=move || is_self
                                    disabled=move || !can_send
                                    on:click=move |_| {
                                        remove_sink.send(
                                            ClientMessage::RemoveLobbyPlayer { player_id: target_id },
                                            "Remove player sent",
                                        )
                                    }
                                >
                                    {if is_bot { "Remove" } else { "Kick" }}
                                </button>
                            </div>
                        }
                    }}
                />
            </div>
        </section>
    }
}

#[component]
pub(crate) fn RacePanel(
    snapshot: RaceSnapshot,
    local_player_id: Option<PlayerId>,
    unicode_icons: ReadSignal<bool>,
    on_key: impl Fn(ProtocolKey) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let players = snapshot.players.clone();
    let local_player = local_player_id
        .and_then(|id| players.iter().find(|player| player.id == id).cloned())
        .or_else(|| players.first().cloned());
    let local_player_id = local_player.as_ref().map(|player| player.id);
    let display_players = ordered_players_for_local_perspective(&players, local_player_id);
    let track_window = build_track_window(&snapshot, local_player.as_ref());
    let track_width_ch = track_window.width_ch;
    let lane_window = track_window.clone();
    let minimap_snapshot = snapshot.clone();
    let events = snapshot.events.clone();

    view! {
        <section
            class="panel track-panel"
            aria-label="Race preview"
            tabindex="0"
            on:keydown=move |event| {
                if let Some(key) = keyboard_event_to_protocol_key(&event) {
                    event.prevent_default();
                    event.stop_propagation();
                    on_key(key);
                }
            }
        >
            <div class="phase">{phase_label(snapshot.phase)} " · seq " {snapshot.sequence}</div>
            <div class="mod-strip">
                <span>{snapshot.mod_config.word_set_name.clone()}</span>
                <span>{snapshot.mod_config.item_pack_name.clone()}</span>
                <span>{snapshot.mod_config.combined_hash.clone()}</span>
            </div>
            <div
                class="race-window"
                style={format!("--track-ch: {};", track_width_ch)}
            >
                <BonusStack bonuses=snapshot.bonuses.clone() window=track_window.clone() local_player=local_player.clone() />
                <TrackWords snapshot=snapshot.clone() window=track_window.clone() local_player=local_player.clone() />
                <For
                    each={move || display_players.clone()}
                    key=|player: &PlayerSnapshot| player.id
                    children={move |player| {
                        view! {
                            <RacerLane
                                player=player.clone()
                                window=lane_window.clone()
                                local=Some(player.id) == local_player_id
                                unicode_icons=unicode_icons
                            />
                        }
                    }}
                />
            </div>
            <Minimap snapshot=minimap_snapshot />
            <EventFeed events=events />
        </section>
    }
}

fn ordered_players_for_local_perspective(
    players: &[PlayerSnapshot],
    local_player_id: Option<PlayerId>,
) -> Vec<PlayerSnapshot> {
    let Some(local_player_id) = local_player_id else {
        return players.to_vec();
    };
    let mut ordered = Vec::with_capacity(players.len());
    if let Some(local_player) = players.iter().find(|player| player.id == local_player_id) {
        ordered.push(local_player.clone());
    }
    ordered.extend(
        players
            .iter()
            .filter(|player| player.id != local_player_id)
            .cloned(),
    );
    ordered
}

#[component]
fn BonusStack(
    bonuses: Vec<BonusPointSnapshot>,
    window: TrackWindow,
    local_player: Option<PlayerSnapshot>,
) -> impl IntoView {
    view! {
        <div class="bonus-layer">
            <For
                each={move || visible_bonus_columns(&bonuses, &window)}
                key=|(bonus, _)| bonus.after_word_index
                children={move |(bonus, column)| {
                    let point = bonus.clone();
                    let choices = bonus.choices.clone();
                    let local_player = local_player.clone();
                    view! {
                        <div class="bonus-stack" style={format!("left: {column}ch")}>
                            <For
                                each=move || choices.clone()
                                key=|choice| choice.word.clone()
                                children={move |choice| {
                                    let available_to_local = local_player.as_ref().is_some_and(|player| {
                                        bonus_choice_available_to_player(player, &point, &choice)
                                    });
                                    view! {
                                        <span class:cooldown=move || !available_to_local>
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

fn bonus_choice_available_to_player(
    player: &PlayerSnapshot,
    point: &BonusPointSnapshot,
    choice: &typekart_protocol::BonusChoiceSnapshot,
) -> bool {
    matches!(choice.status, BonusChoiceSnapshotStatus::Available)
        && player.word_index == point.after_word_index.saturating_add(1)
        && !player.finished
        && !player.shielded
        && !player.focused
        && !player.boosted
        && !player.stunned
        && player.typo_index.is_none()
        && (player.input.is_empty() || choice.word.starts_with(&player.input))
}

#[component]
fn TrackWords(
    snapshot: RaceSnapshot,
    window: TrackWindow,
    local_player: Option<PlayerSnapshot>,
) -> impl IntoView {
    view! {
        <div class="track-text">
            <For
                each={move || window.words.clone()}
                key=|word| word.index
                children={move |word| {
                    let rendered = render_word_segments(&snapshot, local_player.as_ref(), &word);
                    view! {
                        <span class="track-word">
                            <For
                                each=move || rendered.clone().into_iter().enumerate()
                                key=|(index, _)| *index
                                children={|(_index, segment)| {
                                    view! {
                                        <span class=segment.class>
                                            {segment.text}
                                        </span>
                                    }
                                }}
                            />
                        </span>
                    }
                }}
            />
        </div>
    }
}

#[component]
fn RacerLane(
    player: PlayerSnapshot,
    window: TrackWindow,
    local: bool,
    unicode_icons: ReadSignal<bool>,
) -> impl IntoView {
    let use_unicode_icons = unicode_icons.get();
    let cue_before = player
        .item_cue
        .as_ref()
        .filter(|cue| cue.placement == ItemCuePlacementSnapshot::Before)
        .map(|cue| cue_label(cue, use_unicode_icons).to_string());
    let cue_after = player
        .item_cue
        .as_ref()
        .filter(|cue| cue.placement == ItemCuePlacementSnapshot::After)
        .map(|cue| cue_label(cue, use_unicode_icons).to_string());
    let marker = marker_position(&player, &window);
    let offscreen = marker.offscreen_class();

    view! {
        <div
            class="lane"
            class:self-lane=local
            class:stunned=player.stunned
            class:disconnected=!player.connected
        >
            <span class="lane-name">{player.name.clone()}</span>
            <span class="lane-kind">{kind_label(player.kind)}</span>
            <span class="lane-track">
                <span
                    class={format!("lane-marker-wrap {offscreen}")}
                    style={format!("left: {}ch", marker.column)}
                >
                    {cue_before}
                    <span class={format!("marker {}", color_class(player.color))}>
                        {effect_prefix(&player, use_unicode_icons)}
                        {marker.glyph}
                        {impact_label(player.impact_cue, use_unicode_icons)}
                    </span>
                    {cue_after}
                </span>
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
pub(crate) fn ResultsPanel(snapshot: ResultsFrame) -> impl IntoView {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackWindow {
    start_word: usize,
    end_word: usize,
    width_ch: usize,
    words: Vec<VisibleWord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleWord {
    index: usize,
    start_ch: usize,
    end_ch: usize,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WordSegment {
    text: String,
    class: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OffscreenSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkerPosition {
    column: usize,
    glyph: &'static str,
    offscreen: Option<OffscreenSide>,
}

impl MarkerPosition {
    fn offscreen_class(self) -> &'static str {
        match self.offscreen {
            Some(OffscreenSide::Left) => "offscreen-left",
            Some(OffscreenSide::Right) => "offscreen-right",
            None => "",
        }
    }
}

fn build_track_window(snapshot: &RaceSnapshot, anchor: Option<&PlayerSnapshot>) -> TrackWindow {
    let word_count = snapshot.track_words.len();
    if word_count == 0 {
        return TrackWindow {
            start_word: 0,
            end_word: 0,
            width_ch: 1,
            words: Vec::new(),
        };
    }

    let anchor_word = anchor
        .map(|player| player.word_index.min(word_count.saturating_sub(1)))
        .unwrap_or(0);
    let mut start_word = anchor_word.saturating_sub(WEB_TRACK_WORDS_BEHIND);
    let mut end_word = (start_word + WEB_TRACK_VISIBLE_WORDS).min(word_count);
    if end_word - start_word < WEB_TRACK_VISIBLE_WORDS {
        start_word = end_word.saturating_sub(WEB_TRACK_VISIBLE_WORDS);
    }
    end_word = end_word.max(start_word + 1).min(word_count);

    let mut cursor = 0usize;
    let mut words = Vec::new();
    for index in start_word..end_word {
        if index > start_word {
            cursor += 1;
        }
        let text = word_for_player(snapshot, anchor, index);
        let start_ch = cursor;
        cursor += text.chars().count();
        words.push(VisibleWord {
            index,
            start_ch,
            end_ch: cursor,
            text,
        });
    }

    TrackWindow {
        start_word,
        end_word,
        width_ch: cursor.max(1),
        words,
    }
}

fn visible_bonus_columns(
    bonuses: &[BonusPointSnapshot],
    window: &TrackWindow,
) -> Vec<(BonusPointSnapshot, usize)> {
    bonuses
        .iter()
        .filter_map(|bonus| {
            let word = window
                .words
                .iter()
                .find(|word| word.index == bonus.after_word_index)?;
            if bonus.after_word_index + 1 >= window.end_word {
                return None;
            }
            Some((bonus.clone(), word.end_ch + 1))
        })
        .collect()
}

fn render_word_segments(
    snapshot: &RaceSnapshot,
    local_player: Option<&PlayerSnapshot>,
    word: &VisibleWord,
) -> Vec<WordSegment> {
    let Some(player) = local_player else {
        return vec![WordSegment {
            text: word.text.clone(),
            class: "",
        }];
    };

    if word.index != player.word_index || player.input.is_empty() {
        return vec![WordSegment {
            text: word.text.clone(),
            class: word_state_class(snapshot.phase),
        }];
    }

    let typo_index = player.typo_index.unwrap_or(usize::MAX);
    let mut segments = Vec::new();
    for (index, ch) in word.text.chars().enumerate() {
        let class = if index >= typo_index {
            "typed typo"
        } else if index < player.input.chars().count() {
            "typed"
        } else if index == player.input.chars().count() {
            "cursor"
        } else {
            word_state_class(snapshot.phase)
        };
        segments.push(WordSegment {
            text: ch.to_string(),
            class,
        });
    }
    segments
}

fn word_state_class(phase: NetworkRacePhase) -> &'static str {
    match phase {
        NetworkRacePhase::Lobby
        | NetworkRacePhase::WaitingForHost
        | NetworkRacePhase::Countdown { .. } => "pending",
        NetworkRacePhase::Racing | NetworkRacePhase::Finished => "",
    }
}

fn marker_position(player: &PlayerSnapshot, window: &TrackWindow) -> MarkerPosition {
    if window.words.is_empty() {
        return MarkerPosition {
            column: 0,
            glyph: "███",
            offscreen: None,
        };
    }

    if player.word_index < window.start_word {
        return MarkerPosition {
            column: 0,
            glyph: "<",
            offscreen: Some(OffscreenSide::Left),
        };
    }

    if player.word_index >= window.end_word {
        return MarkerPosition {
            column: window.width_ch,
            glyph: if player.finished { ">!" } else { ">" },
            offscreen: Some(OffscreenSide::Right),
        };
    }

    let word = window
        .words
        .iter()
        .find(|word| word.index == player.word_index)
        .expect("visible player word must exist in track window");
    let input_chars = player.input.chars().count();
    let typo_index = player.typo_index.unwrap_or(input_chars);
    let progress_chars = input_chars.min(typo_index).min(word.text.chars().count());
    let column = if player.finished {
        word.end_ch
    } else {
        word.start_ch + progress_chars
    };

    MarkerPosition {
        column,
        glyph: "███",
        offscreen: None,
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

fn word_for_player(
    snapshot: &RaceSnapshot,
    player: Option<&PlayerSnapshot>,
    index: usize,
) -> String {
    let Some(base_word) = snapshot.track_words.get(index) else {
        return String::new();
    };
    let Some(player) = player else {
        return base_word.clone();
    };
    if let Some(override_word) = player
        .word_overrides
        .iter()
        .find(|override_word| override_word.word_index == index)
    {
        return override_word.word.clone();
    }
    fixtures::masked_word(player, index, base_word)
}

fn effect_prefix(player: &PlayerSnapshot, unicode_icons: bool) -> &'static str {
    match unicode_icons {
        true if player.boosted => ">>🍄 ",
        false if player.boosted => ">>> ",
        true if player.shielded => "🛡 ",
        false if player.shielded => "[",
        true if player.focused => "⭐ ",
        false if player.focused => "* ",
        true if player.fogged => "🌫 ",
        false if player.fogged => "# ",
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
            (ImpactCueSnapshotKind::Fog, true) => "🌫",
            (ImpactCueSnapshotKind::Fog, false) => "FOG",
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
    } else if player.fogged {
        "fogged"
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

fn ai_detail_label(player: &LobbyPlayer) -> String {
    match (player.ai_difficulty, player.ai_wpm) {
        (Some(difficulty), Some(wpm)) => format!("{difficulty:?} {wpm}wpm"),
        (Some(difficulty), None) => format!("{difficulty:?}"),
        (None, _) => String::new(),
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

#[cfg(test)]
mod tests {
    use super::{build_track_window, marker_position, ordered_players_for_local_perspective};
    use crate::fixtures::{GalleryFrame, SCENARIOS, scenario_frame};
    use typekart_protocol::NetworkRacePhase;

    #[test]
    fn track_window_keeps_anchor_with_context() {
        let GalleryFrame::Race(snapshot) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        let anchor = snapshot.players.first().unwrap();
        let window = build_track_window(&snapshot, Some(anchor));

        assert!(window.start_word <= anchor.word_index);
        assert!(window.end_word > anchor.word_index);
        assert!(window.words.len() <= 10);
    }

    #[test]
    fn marker_position_tracks_current_character() {
        let GalleryFrame::Race(snapshot) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        assert_eq!(snapshot.phase, NetworkRacePhase::Racing);
        let player = snapshot.players.first().unwrap();
        let window = build_track_window(&snapshot, Some(player));
        let word = window
            .words
            .iter()
            .find(|word| word.index == player.word_index)
            .unwrap();

        assert_eq!(
            marker_position(player, &window).column,
            word.start_ch + player.input.chars().count()
        );
    }

    #[test]
    fn local_player_is_rendered_first() {
        let GalleryFrame::Race(snapshot) = scenario_frame(SCENARIOS[2]) else {
            unreachable!();
        };
        let local_player_id = snapshot.players[1].id;

        let ordered =
            ordered_players_for_local_perspective(&snapshot.players, Some(local_player_id));

        assert_eq!(ordered[0].id, local_player_id);
        assert_eq!(ordered.len(), snapshot.players.len());
    }
}
