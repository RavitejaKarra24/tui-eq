# eqterm

A **Vim-first** terminal music player with [AutoEq](https://github.com/jaakkopasanen/AutoEq) headphone/IEM presets.

Browse folders like a file manager, play common audio formats through embedded **mpv**, and apply parametric EQ by preset name — no graphs, no band editing.

## Features

- Fast folder browser with live filter search
- Parametric EQ via AutoEq presets (thousands of headphones/IEMs)
- Playback progress, seek, volume, pause, next/prev, auto-advance
- Custom preset packs alongside AutoEq defaults
- Keyboard-first UX (`?` for full help)

## Requirements (macOS)

- `mpv` (provides `libmpv` for the embedded player)
- Rust toolchain (`cargo`, `rustc`)
- Optional but recommended: `git` (faster sparse download of AutoEq)

```bash
brew install mpv
```

## Install & run

```bash
cargo run --release -- /path/to/music
```

Or install a binary:

```bash
cargo install --path .
eqterm ~/Music
```

On first run, eqterm downloads AutoEq results and caches ParametricEQ files under:

```
~/Library/Application Support/eqterm/autoeq/results
```

## Custom preset packs

Drop additional `*ParametricEQ.txt` files into:

```
~/Library/Application Support/eqterm/autoeq/presets
```

Or point at extra folders:

```bash
eqterm /path/to/music --presets-dir /path/to/your/packs
```

## Keymap

Press **`?`** inside the app for the full reference.

### Library

| Key | Action |
|-----|--------|
| `j` / `k` or arrows | Move |
| `h` | Parent folder |
| `l` / `Enter` | Open folder / play track |
| `/` | Search / filter |
| `e` | EQ preset picker |
| `space` | Pause / resume |
| `n` / `p` | Next / previous track |
| `,` / `.` | Seek −5s / +5s |
| `[` / `]` | Seek −30s / +30s |
| `+` / `-` | Volume up / down |
| `s` | Stop |
| `x` | Clear EQ (flat) |
| `r` | Refresh folder |
| `gg` / `G` | Top / bottom |
| `PgUp` / `PgDn` | Page |
| `Ctrl-d` / `Ctrl-u` | Half-page |
| `Ctrl-l` | Clear filter |
| `?` | Help |
| `q` | Quit |

### Preset picker

| Key | Action |
|-----|--------|
| `j` / `k` | Move |
| `Enter` | Apply preset |
| `/` | Search |
| `x` | Clear EQ |
| `Esc` / `q` | Back |

### Search

| Key | Action |
|-----|--------|
| type | Live filter |
| `Enter` | Confirm |
| `Esc` | Cancel |
| `jk` | Cancel (vim-style) |

## Supported formats

mp3, flac, wav, aac, m4a, ogg, opus, alac, aiff, wma, mka (anything mpv can decode)

## License

MIT
