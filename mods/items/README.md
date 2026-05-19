# Item Pack Templates

`classic.json` mirrors TypeKart's built-in item defaults.

Edit a copy of this file when making a new pack. Item packs can tune names, enabled flags, roll weights, effect durations, and Banana display labels for the built-in item ids:

- `mushroom`
- `banana`
- `shield`
- `focus`
- `cyclone`
- `squid_ink`

Run with:

```sh
typekart play --item-pack-file ./mods/items/classic.json
typekart host --item-pack-file ./mods/items/classic.json
```
