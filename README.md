# eqterm

A Vim-style terminal music player with AutoEq headphone/IEM presets. It browses folders like a file manager, plays most audio formats via mpv, and applies parametric EQ presets without showing graphs or raw bands.

## Requirements (macOS)
- `mpv` installed (provides `libmpv` for the embedded player)
- Rust toolchain (`cargo`, `rustc`)

Install mpv on macOS:

```
brew install mpv
```

## Run

```
cargo run -- /path/to/music
```

On first run, eqterm downloads AutoEq results and caches the ParametricEQ files under:

```
~/Library/Application Support/eqterm/autoeq/results
```

## Adding your own preset packs

Drop any additional `*ParametricEQ.txt` packs into:

```
~/Library/Application Support/eqterm/autoeq/presets
```

They will be loaded alongside AutoEq defaults. You can also point to extra folders:

```
cargo run -- /path/to/music --presets-dir /path/to/your/packs
```

## Keymap

Files view:
- `j`/`k` or arrows: move
- `h`: go up
- `l` or `Enter`: open folder / play track
- `/`: search/filter
- `e`: open preset picker
- `space`: pause/resume
- `n`/`p`: next/previous track (within current list)
- `q`: quit

Preset picker:
- `j`/`k` or arrows: move
- `Enter`: apply preset
- `/`: search/filter
- `Esc` or `q`: back
