# Character-Level Track Display Refactor

## Goal

Make the race feel closer by tying the local racer marker to the current character instead of the current word.

This refactor keeps gameplay progress word-based for now. `PlayerState.word_index` still owns the current target word, word completion still requires `Space` between non-final words, and bonus windows still depend on word boundaries. The renderer derives a more precise character position from `PlayerState.input`.

## Display Rules

- Correct typed characters render directly on the track in green.
- The next character to type is highlighted on the track.
- If a typo exists, typed characters from the first typo onward render in red.
- Typed overflow after a typo continues across following words and inter-word spaces so the player can see how many characters must be backspaced.
- The racer marker is centered on the next character while input is valid.
- When a typo exists, the racer marker is centered on the first typo character, because that is the player's true blocked race position.

## Current Invariant

The track display may show typed overflow ahead of the real race position, but game progress does not advance while `typo_index` exists.

This distinction matters for multiplayer: visual typo overflow is local feedback, while race placement should use the validated character or word position.

## Future Work

- Promote race position to a server-authoritative character offset for multiplayer.
- Decide how remote typo states should be displayed, if at all.
- Add item effects that mutate character-level display without breaking word-level completion.
