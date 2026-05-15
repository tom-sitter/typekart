# Minimap Implementation Plan

## Goal

Add a compact one-line minimap inside the Track panel, below all racer lanes, so the player can see every racer's whole-race position even when racers are outside the current visible word window.

## Display

Recommended shape:

```text
Map  |--1------@------2---*----------3-------------|
```

Marker rules:

- `@`: local player.
- `1` through `6`: AI racers by `AiRacer::id`.
- `*`: multiple racers mapped to the same minimap column.
- `|`: start and finish boundaries.
- `-`: empty track space.

Use the same colors as the main racer lanes for non-overlap markers:

- Local player: cyan.
- AI racers: `ai_color(id)`.
- Overlap marker: bold white or another neutral high-contrast style.

## Position Mapping

The minimap should map full race progress to available minimap columns:

```text
progress = completed word index / final word index
column = round(progress * usable_width)
```

Details:

- The minimap uses full track length, not the current `TrackWindow`.
- Active racers use `word_index`.
- Finished racers pin to the finish column.
- Empty or one-word tracks should avoid division by zero by pinning every racer to the finish/start column.
- The local player marker should win ties over AI markers.
- If two or more non-local racers overlap, show `*`.
- If the local player overlaps anyone, show `@` rather than `*` so the player can always find themselves.

## Renderer Integration

Likely code changes:

- Add a minimap line builder in `src/ui/render.rs`.
- Pass the full `Track` into `track_view` or pass `track.len()` where the minimap is built.
- Append the minimap line after `racer_lines`.
- Increase `track_panel_height` by one row so the minimap does not crowd existing racer lanes.
- Add renderer tests for:
  - local player marker appears on the minimap;
  - AI marker appears with the expected label and color;
  - finished racer pins to the finish edge;
  - overlapping racers render according to the tie rules.

## Non-Goals

- No multi-row minimap yet.
- No labels under the minimap yet.
- No item-state encoding on the minimap in the first implementation.
- No networking-specific minimap behavior until remote racers exist.
