# Renderer Gallery

The renderer gallery is a developer-facing terminal preview for TypeKart race UI states. It is intended to make item pickup cues, active effects, and impact effects easy to inspect without playing repeated races until a specific item appears.

Run it with:

```sh
typekart gallery items
```

In development:

```sh
cargo run -- gallery items
```

Use ASCII fallback rendering:

```sh
typekart gallery items --ascii
```

Jump directly to a screenshot-ready scenario:

```sh
typekart gallery items --scenario multiplayer-pack
typekart gallery items --scenario banana-hit-pack
typekart gallery items --scenario squid-ink-pack
```

## Controls

- Left / Right: previous or next scenario.
- A: toggle ASCII and Unicode icons.
- ?: show or hide help.
- Esc or Ctrl-C: quit.

## Item Scenarios

The item gallery renders preset race-track scenes using the same local race renderer as the game. This keeps the gallery useful as a UI design tool: when an item cue or impact state changes in gameplay, the gallery should show the same rendering.

The gallery includes focused item closeups and fuller static race compositions.

Screenshot-oriented scenarios:

- `multiplayer-pack`: six racers, bonus words, mixed positions, boost and shield markers, and minimap.
- `banana-hit-pack`: the player firing a Banana while another racer shows the impact blink.
- `squid-ink-pack`: Squid Ink fired, multiple racers impacted, and upcoming words hidden.
- `item-pileup`: several simultaneous effects to stress-test readability.
- `finish-sprint`: racers near the finish line with one racer already finished.

Focused item scenarios:

- Mushroom boost marker.
- Shield active marker.
- Focus active marker.
- Banana fired ahead, behind, and impact blink.
- Cyclone fired and impact blink.
- Squid Ink fired and impact blink.
- Squid Ink obscuring words beyond the affected racer's current word.

The gallery is not a full simulator. It uses deterministic race states so visual states can be compared quickly.
