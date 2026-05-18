# Gameplay Guide

TypeKart is a terminal typing racer with kart-style item effects. Players race across a word track, type words in order, collect optional bonus words, and try to finish before the other racers.

## Race Flow

1. The host creates a race.
2. Joiners enter the lobby and press `Enter` to ready up when they want to join the next race.
3. The host presses `Space` to start with the ready racers.
4. A countdown appears beside each player's marker.
5. The race starts when the countdown ends.
6. Results appear after every racer finishes, all active racers disconnect, or the post-first-place timeout expires.

In single player, press `Space` to start the countdown.

## Pre-Race Lobby Controls

Before a race starts, the footer shows the main lifecycle command and `? help`. Press `?` to show or hide a centered command overlay instead of expanding the footer.

```text
Space start | ? help | Esc quit
```

Host command overlay:

```text
Key / Command       Action
Space / start       Start countdown
Up / Down           Select lobby racer
A                   Add AI racer
X                   Remove selected AI or kick human
E / H               Set selected AI to Easy / Hard
?                   Hide this help
Esc / quit          Leave
```

- `A` adds one AI racer using the current AI difficulty.
- `X` removes the selected AI racer. In multiplayer, it kicks the selected human joiner. The host cannot be removed.
- `E` sets the selected AI to Easy. If a human is selected, it changes the default difficulty for newly added AIs.
- `H` sets the selected AI to Hard. If a human is selected, it changes the default difficulty for newly added AIs.
- `Space` starts the countdown once the lobby is ready.
- `Ctrl-R` cancels an active multiplayer countdown or race and returns everyone to the lobby.
- `N` opens rename mode for your lobby player. Duplicate names receive a numbered suffix.

Joiners use:

```text
Enter ready | ? help | Esc quit
```

## Typing Rules

- Type the current word exactly.
- Press `Space` between words.
- The final word finishes immediately when its last character is typed; no trailing `Space` is required.
- Pressing `Space` too early counts as a typo.
- Once a typo exists, progress is blocked until the typo is removed.
- Backspace removes typed characters.
- Red characters show the typo and any extra characters that must be backspaced.
- Bonus words cannot be claimed while a typo is present.

## Track Display

Each racer has a lane under the visible track text.

- Your marker is pinned to your current character.
- Other racer markers show their positions relative to the same visible track window.
- Racers outside the visible window are shown with edge markers.
- Finished racers stay visible at the finish when the finish is visible, or show a finished edge marker when it is offscreen.
- The minimap shows every racer's whole-race position on one compact line.

Words are greyed out before the race starts so it is clear input is locked during lobby and countdown phases.

## Bonus Words

Bonus words appear at periodic points in the track. Each bonus point shows three stacked choices.

- Bonus choices are visible before they are claimable so racers can plan.
- A choice is claimable only after the preceding main word is complete and before the next main word has begun.
- Start typing a bonus word to attempt it.
- Press `Space` after completing the bonus word to claim it.
- Press Backspace until the bonus input is empty to bail out.
- If another racer claims the same bonus first, you are forced back to the main track and cannot retry another choice at that bonus point.
- Claimed choices disappear temporarily and refresh after a cooldown.
- If you already have an active lockout effect such as Shield, bonus words appear unavailable to you until that effect expires.

## Items

Items currently activate automatically when obtained.

| Item | Effect |
| --- | --- |
| Mushroom | Boosts the racer forward by three words one at a time. Input is paused during the boost. |
| Banana | Hits the nearest unfinished racer in range and briefly stuns them. |
| Shield | Protects the racer from the next blockable attack until it expires or is consumed. |
| Star Power | Temporarily discards incorrect keys without adding typo input. Correct keys are still required to advance. |
| Blue Shell | Targets first place and reverses the affected racer's next word. |

## Visual Indicators

Unicode icons are enabled by default. Use `--ascii` if a terminal renders emoji poorly.

| Indicator | Meaning |
| --- | --- |
| `>>🍄` | Mushroom boost is active. |
| Shield icon on marker | Shield is active. |
| Star icon on marker | Star Power is active. |
| `🍌 >>` or `<< 🍌` | Banana fired ahead or behind. |
| Blue shell cue | Blue Shell fired. |
| Yellow blink | Banana impact. |
| Blue blink | Blue Shell impact. |

ASCII fallback keeps plain text markers for terminals without reliable Unicode support.
