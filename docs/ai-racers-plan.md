# AI Racers Milestone Plan

## Goal

Add local AI racers so TypeKart can exercise multiplayer-like rendering, proximity, item timing, and race pressure before networking exists.

## Scope

- Support 0 to 6 AI racers in local play.
- Let the player choose `easy` or `hard` AI difficulty.
- Render AI racers on the same racer layer as the player.
- Advance AI racers by character-level typing pace derived from WPM.
- Let AI racers claim bonus words and use items.
- Let player-used items affect AI racers when valid targets exist.

## Non-Goals

- No network protocol.
- No human remote clients.
- No sophisticated AI strategy.
- No full item parity. Item effects can be lightweight local approximations if they exercise the same targeting and timing surfaces.

## Initial Difficulty Values

- Easy: random per-racer speed from 28 to 42 WPM.
- Hard: random per-racer speed from 65 to 85 WPM.

These values should be easy to tune after playtesting.

## Local AI Model

Each AI racer owns:

- A `PlayerState`.
- A display name.
- A difficulty.
- A sampled WPM within that difficulty's range.
- A fractional typing budget, so movement stays smooth across terminal ticks.
- A temporary `stunned_until` penalty for local attack effects.

AI typing should use the same `game::typing::apply_key` path as the human player. This keeps word completion, final-word finish, and stats behavior consistent.

## Item Behavior

- Mushroom: same local speed boost idea as the player, advancing three words quickly enough to read.
- Shield: activates immediately when picked up.
- Banana: targets the nearest racer within 10 words, regardless of whether that racer is ahead or behind.
- If Banana hits an AI, stun that AI briefly.
- If Banana hits the player, use the existing attack warning path.

This is intentionally not final multiplayer item behavior. It gives the UI and session model real targets to coordinate against.

## Rendering

AI racers should be rendered through the same track-window column model used by the local racer. That means close races can be visible at character precision, not only word precision.

Each racer should render in its own lane. The local player lane appears immediately below the track text, and AI lanes appear below it. This keeps close positions readable without requiring marker color blending.
