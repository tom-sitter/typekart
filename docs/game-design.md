# TypeKart Game Design

## Summary

TypeKart is a command-line multiplayer racing game that combines the typing challenge of TypeRacer or Monkeytype with item-driven disruption and comeback mechanics inspired by Mario Kart.

Players race across a shared text track made of simple words with no punctuation. Progress is earned by typing the track words in order. Optional bonus words appear above the main track between regular words; typing one detours the player briefly but grants a random item. Items can speed the user up, protect them from mistakes, or interfere with opponents.

The core experience should feel like a typing race first and an item battler second: fast, readable, competitive, and chaotic without making accurate typing irrelevant.

## Goals

- Support real-time command-line multiplayer races.
- Let players host and join race lobbies.
- Represent the race track as a sequence of words.
- Require players to type words in order to advance.
- Use no punctuation in race text, matching Monkeytype-style word streams.
- Add optional bonus words that grant random items.
- Let players hold and activate items during the race.
- Show nearby racers on the track with unique colors.
- Announce placements and winners when racers complete the full track.
- Keep mechanics legible in a terminal UI.

## Non-Goals For The First Version

- Graphical rendering outside the terminal.
- Complex physics, drifting, steering, or lane control.
- Persistent accounts, rankings, or progression.
- Full matchmaking.
- Voice chat or chat moderation.
- Custom word packs beyond a basic local word source.
- Anti-cheat enforcement beyond server-authoritative race state.

## Core Race Model

Each race uses a generated track:

- The main track is an ordered list of words.
- Each word is lowercase alphabetic text.
- Track length is configurable by word count.
- Each player starts before the first word.
- A player advances by correctly typing the current required word.
- A player finishes after completing the final main-track word.

Player progress can be represented as:

- `word_index`: the next main-track word the player must complete.
- `current_input`: the partial text typed for the current word.
- `finished_at`: the server timestamp when the player completed the track.
- `active_effects`: temporary item effects currently modifying typing behavior.
- `held_item`: the item currently available to activate, if any.

## Typing Rules

The default typing rules are intentionally strict:

- The player types the current target word.
- Correct characters advance the partial input.
- The first incorrect character creates a typo state.
- While a typo is present, additional typed characters are accepted into the input buffer but no progress can be made.
- The typo and all following characters are highlighted red so the player knows how far they must backtrack.
- Backspace must be used until the typo is removed before progress can resume.
- Backspace removes the most recent typed character.
- Space submits the current word when it exactly matches the target.
- Pressing Space before the current word is complete counts as a typo and must be corrected with Backspace.
- If the submitted word is correct, the player advances to the next word.
- If the submitted word is incorrect, the player stays on the same word.
- The final word finishes immediately when its last correct character is typed; no trailing Space is required.

The exact input behavior can be adjusted by implementation constraints, but the player should always be able to understand:

- Which word they are typing.
- Which characters are correct.
- Which characters are incorrect.
- What word comes next.

## Bonus Words

Bonus words are optional item boxes represented as words. They appear periodically throughout the race at bonus points, similar to Mario Kart item boxes.

At each bonus point, all racers see the same three available bonus-word choices. A player can choose which one to type. Claiming one choice removes that word from the shared bonus point until its cooldown replaces it with another word.

Bonus words appear offset above the main track between regular words. They should be visible before the racer reaches the bonus point so players can plan whether to go for one.

```text
          turbo
          spark
          drift
the quick brown fox jumps over the lazy driver into bright road
```

In this example, `turbo`, `spark`, and `drift` are the three available choices at the current bonus point.

### Bonus Word Rules

- Bonus words are optional.
- Bonus points appear periodically along the main track.
- Each bonus point presents three shared bonus-word choices.
- All players see the same three choices at that bonus point.
- The three choices should be displayed stacked vertically.
- Choices should be visible ahead of the claim window so players can plan.
- Choices visible ahead of the claim window should render inactive or greyed out for that player until they become claimable.
- Typing one available choice and pressing Space grants a random item.
- Bonus intent is inferred from typed input rather than selected explicitly.
- Bonus choices are only available after the preceding main-track word is completed and before the next main-track word is begun.
- If the player starts typing a bonus word, they may bail out by pressing Backspace until the bonus input is cleared.
- Once a choice is submitted by any player, it is eliminated from that bonus point.
- Eliminated choices are replaced after a cooldown.
- The replacement word appears at the same bonus point and is visible to all players.
- Typing a bonus word costs time because the player is not advancing along the main track while typing it.
- Bonus words should not require punctuation or capitalization.
- Players cannot claim bonus words while a typo is present.
- Players cannot claim bonus words while Shield is active.
- Current local behavior auto-activates items immediately when obtained.
- Later balance may make held items configurable again.
- For players with an active Shield, bonus words should render as disabled or greyed out until Shield expires. The bonus choices themselves remain active for other players.
- If two players complete the same bonus word at nearly the same time, the server awards it to the first valid completion it receives.
- If a player loses that race for the bonus word, they are forced onto the next main-track word and cannot retry a different bonus choice at that bonus point.

### Bonus Placement And Cooldown

Bonus placement and cooldowns should balance item frequency and race readability:

- Bonus points should appear periodically throughout the track.
- Each bonus point should have three choice slots.
- Each choice slot independently enters cooldown when its word is claimed.
- The cooldown should be long enough that players can see that a choice was taken.
- Cooldowns should not be so long that item play disappears.
- Replacement words should not duplicate the current main word or another active choice at the same bonus point.
- Bonus words can be longer, rarer, or more difficult when they grant stronger item chances.

Initial recommendation:

- One bonus point every 8 to 14 main-track words.
- Three shared choices per bonus point.
- A 5 to 10 second cooldown per claimed choice.
- Bonus words between 4 and 8 letters.
- Bonus words drawn from the same lowercase word list as the main track.

## Items

Items are race modifiers gained from bonus words. Current local behavior activates items immediately when they are obtained. A future configuration may restore held items and manual activation.

Items are never dropped as persistent objects on the track. They immediately modify one or more players by changing their track text, changing their current or upcoming words, modifying displayed characters, or temporarily loosening typing rules.

The server should choose items so the race remains competitive:

- Players farther behind should have better odds of strong comeback items.
- Players in first place should mostly receive defensive or minor boost items.
- Highly disruptive items should be rare.

### Attack Warnings

Incoming attacks should provide a short visible warning before they take effect.

- The warning should identify that an attack is incoming.
- The warning should last long enough for an already-active Shield to matter.
- The warning should be short enough that attacks still feel dangerous.
- Blockable attacks should resolve only after the warning window expires.
- If Shield is active when the attack resolves, the attack is blocked.

Initial recommendation:

- A 1 to 2 second warning window for blockable attacks.

### Item Timing

Item durations should vary by effect.

- Boosts can last longer when they are self-only and do not directly disrupt other racers.
- Defensive effects should be short enough that timing matters.
- Disruptive effects should be long enough to matter but short enough that the target keeps racing.

Initial examples:

- Star Power lasts 10 seconds.
- Shield lasts 5 seconds.
- Blue Shell affects the target's next 3 words.
- Mushroom advances 3 words one at a time at a tunable speedboost pace.

### Item Concepts

| Item | Target | Effect |
| --- | --- | --- |
| Star Power | Self | Temporarily lets the player ignore typos without backspacing. |
| Mushroom | Self | Rapidly advances the player by three main-track words, one word at a time. |
| Triple Mushroom | Self | Gives multiple smaller boosts, activated one at a time. |
| Banana | Nearby opponent | Swaps a nearby opponent's current word with a different word while they are typing it. |
| Blue Shell | First place | Randomizes capitalization in the first-place player's next three words. |
| Lightning | All opponents | Briefly slows all other players or increases their typo penalty. |
| Shield | Self | Blocks the next negative item effect. |
| Ink | Nearby opponents | Temporarily obscures upcoming words in the terminal UI. |

## Item Details

### Star Power

Star Power is a self-targeted boost.

While active:

- Incorrect characters do not block progress.
- The player can continue typing without backspacing.
- Word submission may accept the intended target as long as enough characters were typed in sequence.

Initial recommendation:

- Star Power lasts 10 seconds.
- Star Power forgives mistakes for the duration rather than permanently changing raw accuracy stats.

### Blue Shell

Blue Shell targets the player currently in first place.

Effect:

- The target's next three words have randomized capitalization.
- The target must type the displayed capitalization correctly during the effect.
- Each affected word should remain recognizable, but the casing pattern should vary enough to force attention.

Balance:

- Should be rare.
- Should usually be unavailable to the player in first place.
- Should have warning feedback before it lands.

### Banana

Banana disrupts a player who is actively typing.

Effect:

- If the target is in the middle of a word, their current target word is swapped with a different word of similar length.
- Existing partial input may become incorrect.
- The target must adapt, backspace, or recover using an active defensive effect.

Targeting:

- Banana targets the nearest racer, whether that racer is ahead, behind, or overlapping the current player.
- The target must be within 10 main-track words.
- If no valid racer is within range, the item misses.
- Banana is consumed whenever it is used, whether or not a valid target exists.

Initial recommendation:

- With automatic item activation, Banana should not require a targeting UI or a forward/backward activation choice.

### Mushroom

Mushroom is a simple fixed boost.

- Rapidly advance three main-track words if the player is not at the finish.
- Advance one word at a time so the player can visually track where they will resume typing.
- Initial speed should be approximately equivalent to 150 WPM.
- The exact speed should be tunable after playtesting.
- If fewer than three words remain, advance to the finish.
- Avoid skipping bonus words unless the implementation explicitly supports it.

### Shield

Shield is defensive.

Effect:

- Blocks the next negative item.
- Activates immediately when picked up from a bonus word.
- Is not held in the item slot.
- Cannot be saved for later.
- Expires if unused.

Initial recommendation:

- 5 second visible duration.
- A successfully blocked attack consumes the active shield.
- Shield pickup probability should increase when other racers are nearby, such as within 5 words ahead or behind.
- Current local tuning uses a 1 in 6 Shield chance normally and a 3 in 10 Shield chance when another racer is nearby.

## Multiplayer Model

The game should use a server-authoritative model:

- One player hosts a game server.
- Other players join using host address and lobby code or port.
- The server owns race state, item rolls, placements, and finish order.
- Clients send input events and receive state updates.
- The server validates progress and broadcasts race snapshots.

This keeps all players synchronized and reduces trivial cheating compared with purely client-side progress.

## Lobby Flow

1. Host starts a lobby.
2. Host selects race settings such as word count and max players.
3. Joining players choose a display name.
4. The server assigns each player a unique color.
5. The lobby shows connected players and readiness.
6. The host starts the race.
7. The host player's Space key confirms the start.
8. A 3 second countdown is shown to all players.
9. The race begins simultaneously.

The host is always also a player. Initial lobbies should support up to 6 players.

## Join Flow

1. Player chooses join.
2. Player enters host address, port, or lobby code.
3. Player enters a display name.
4. Server accepts or rejects the player.
5. Player appears in the lobby list.
6. Player marks ready.

## Player Identity

Each player has:

- A unique connection id.
- A display name.
- A unique terminal color.
- Race progress.
- Held item state.
- Active effect state.
- Finish placement, once completed.

Display names should be unique within a lobby. If a requested name is already taken, the server should reject it or append a suffix.

## Terminal UI

The UI needs to communicate the race without overwhelming the player.

Recommended layout:

```text
TypeKart                         Lap: 1/1

Track:
                         turbo
the quick brown fox jumps over the lazy driver into bright road
          ███       ▓▓▓       ▒▒▒

Players:
1. ana       42/120 words
2. you       39/120 words
3. tom       34/120 words

Events:
ana used Blue Shell
you picked up Banana
```

### Track View

- Show a horizontal window around the local player's current position.
- Show completed words, current word, and upcoming words.
- Render the local player's typed progress directly on the track.
- Highlight correctly typed characters in green.
- Highlight the next character to type with a cursor-like style.
- When a typo is present, show typed characters from the first typo onward in red, including overflow across following words and spaces.
- Show racer positions in separate lanes aligned with the same track window.
- Represent each racer with a three-character marker in their unique color.
- The local player lane should appear immediately below the track text.
- Other racer lanes should appear below the local player lane.
- Pin the local racer's marker to the next character while input is valid.
- Pin the local racer's marker to the first typo while a typo is active.
- When Shield is active, encapsulate the racer's marker in brackets to show the protected state, such as `[███]`.
- When Unicode icons are enabled, render the shield icon in the center of the racer marker without a trailing block that can obscure wide emoji glyphs, such as `█🛡`.
- When Mushroom is active, prepend `>>>` to the racer's marker to show the boost.
- When Unicode icons are enabled, use `>>🍄` as the Mushroom boost marker.
- When an item impact lands, briefly blink the impacted racer's lane marker.
- Keep the word layer readable; racer markers should not obscure the text players must type.
- Since each racer has a lane, close positions should be readable without marker color blending.

### Minimap

The track panel should include a single-line minimap below all racer lanes so players can understand the whole-race spread even when other racers are outside the current word window.

Recommended first version:

```text
Map  |--1------@------2---*----------3-------------|
```

Rules:

- Render the minimap inside the Track panel, below the local and other racer lanes.
- Scale each racer's race position across the full track length, not the current visible word window.
- Use `@` for the local player and `1` through `6` for AI racers.
- Use the same racer colors as the main racer lanes.
- Pin finished racers to the finish edge.
- If multiple racers occupy the same minimap column, render `*`.
- If the local player overlaps with other racers, prefer `@` for the minimap marker.
- Keep the minimap to one terminal row for now.
- Do not show item state on the minimap in the first version unless it falls out naturally from the existing marker styling.

### Item Feedback

- Represent active item effects on the racer lanes instead of a separate item panel.
- Briefly show a Banana cue beside the attacker's racer marker when Banana fires.
- In Unicode mode, Banana cues should render as `🍌 >>` for attacks ahead and `<< 🍌` for attacks behind.
- In ASCII mode, Banana cues should render as `))>>` for attacks ahead and `((<<` for attacks behind.
- Grey out bonus words when they are visible but unavailable because the player already has an active lockout effect.

### Icon Mode

The default display should remain ASCII-safe because terminal emoji width and styling support varies. A settings toggle can enable Unicode icons for terminals that render them well.

Current command-line toggle:

```text
--unicode-icons
```

Unicode mode affects:

- Banana attack cues.
- Mushroom boost markers.
- Shield markers.

### Event Feed

The event feed should be short and practical:

- Item pickups.
- Item activations.
- Item hits and blocks.
- Finish placements.
- Player joins or disconnects.

## Controls

Initial control scheme:

| Key | Action |
| --- | --- |
| Letter keys | Type current word or bonus word. |
| Space | Submit current word or completed bonus word. |
| Backspace | Delete last typed character. |
| Enter or configured item key | Reserved for manual item activation if held items are enabled later. |
| Shift+Enter or configured modified item key | Reserved for modified item activation if held items are enabled later. |
| Ctrl+R | Restart with a new track. |
| Esc or Ctrl-C | Leave race or quit. |

### Item Activation Keys

The exact item key should be validated against terminal input library support.

- `Enter` is easy to discover but may conflict with common terminal expectations.
- `Shift+Enter` is a useful modified-use candidate only if the terminal stack can distinguish it reliably.
- Alternatives to evaluate include `Ctrl+J`, `Ctrl+K`, or a single punctuation-free command key outside normal word input.
- The final mapping must support both normal use and modified use for items such as Banana.

Bonus words do not need a selection key. The game infers bonus intent from typed input during the bonus window.

## Race Completion

When a player completes the final word:

- The server records their finish timestamp.
- The player receives a placement.
- The player can continue spectating until the race ends.
- Finished players cannot use new items.
- Active effects from finished players should expire normally or be cleared.
- Players can restart locally with a new track from the results view.

The race ends when:

- All players have finished, or
- A post-first-place timeout expires, or
- All remaining active players disconnect.

After the race:

- Show all racers in one ranked results table.
- Rank finished racers by finish timestamp.
- Rank timed-out racers after finished racers by progress at timeout.
- Show words per minute.
- Show accuracy.
- Show item pickups and item hits.
- Offer rematch from the lobby.

## Scoring And Stats

Useful race stats:

- Finish placement.
- Elapsed race time.
- Words per minute.
- Accuracy.
- Corrected errors.
- Items collected.
- Items used.
- Item hits.
- Item blocks.
- Bonus words typed.

Accuracy should be defined carefully because item effects can forgive or modify mistakes.

Initial recommendation:

- Track raw typing accuracy separately from item-adjusted progress.
- Raw accuracy counts actual typed characters.
- Item effects may change progress but should not hide raw typing mistakes from stats.

## Networking Requirements

Minimum requirements:

- Host a lobby on a configurable local port.
- Join a lobby by host and port.
- Broadcast lobby state.
- Broadcast race countdown.
- Send client input events to server.
- Broadcast server race snapshots to clients.
- Handle disconnects.
- End the race cleanly.

Nice-to-have later:

- Lobby codes.
- NAT traversal.
- Reconnect support.
- Spectator mode.
- Remote public servers.

### Local Network Vs Internet Play

The first technical decision is whether hosting should target local networks only or internet play.

Local network play is simpler:

- The host can run the server directly.
- Joiners connect to the host's LAN address and port.
- There is no account system, relay server, matchmaking, or NAT traversal requirement.
- This is the best first target for validating gameplay.

Internet play adds significant technical work:

- Most hosts are behind NAT or firewalls, so direct joins may fail without port forwarding.
- A public relay, rendezvous server, or hosted authoritative server may be needed.
- Lobby discovery, connection security, abuse handling, and operational costs become real concerns.
- Latency variation matters more for typing feel and item timing.

Initial recommendation:

- Build local network play first.
- Keep the protocol server-authoritative so internet play can be added later with a relay or hosted server.

## Server State

The server should track:

- Lobby settings.
- Connected players.
- Player readiness.
- Race phase.
- Track words.
- Bonus point positions.
- Active bonus choices and cooldowns.
- Item random seed or item event log.
- Current authoritative player progress.
- Active item effects.
- Finish order.

## Client State

The client should track:

- Local input buffer.
- Last known server snapshot.
- Predicted local typing feedback, if used.
- Terminal rendering state.
- Connection state.

For a first version, the client can avoid prediction and wait for server confirmation. If latency makes the typing feel poor, local prediction can be added later.

## Race Phases

| Phase | Description |
| --- | --- |
| Main Menu | Player chooses host, join, settings, or quit. |
| Lobby | Players connect and ready up. |
| Waiting for Host | Race is visible but locked until the host presses Space. |
| Countdown | Race is locked and starts after a countdown. |
| Racing | Players type, collect items, and finish. |
| Results | Final placements and stats are shown. |
| Rematch | Players return to lobby with the same group. |

## Balancing Principles

- Typing skill should usually decide the winner.
- Items should create tension, recovery opportunities, and memorable reversals.
- Item effects should be readable and short.
- The strongest items should be rare and weighted toward trailing players.
- Effects should avoid forcing players to stop typing for long periods.
- Defensive options should exist so disruption feels interactive.

## Suggested First Playable Version

The first playable version should be intentionally small:

- Host and join local network games.
- Two to six players.
- One generated lowercase word track.
- Basic terminal race UI.
- Unique player colors.
- Main typing progress.
- Periodic bonus points with three shared word choices.
- Inferred bonus-word intent during bonus windows.
- One held item at a time.
- Three items:
  - Mushroom
  - Banana
  - Shield
- Finish order and results screen.

After that works, add:

- Star Power.
- Blue Shell.
- Better item weighting.
- Better stats.
- Improved rendering for dense player clusters.

## Open Questions

- Which terminal key should activate normal item use?
- Which terminal key or key combination should activate modified item use?
- After local network play works, should internet play use direct port forwarding, a relay server, or hosted authoritative servers?

## Glossary

- **Track**: The ordered list of words players race across.
- **Main word**: The required next word in the race.
- **Bonus word**: An optional word that grants an item.
- **Item**: A temporary boost, defensive effect, or attack.
- **Held item**: The item a player currently has available to activate.
- **Active effect**: A temporary modifier currently affecting a player.
- **Race snapshot**: A server broadcast describing current race state.
