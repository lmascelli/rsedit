# Classic themes for rsedit

Ten of the colour schemes editors have settled on over the last twenty years,
written in rsedit's own theme format.

| theme | ground | character |
|---|---|---|
| `solarized-dark` | `#002b36` | the designed one: sixteen tones picked in CIELAB |
| `solarized-light` | `#fdf6e3` | the same accents, base ramp reversed |
| `gruvbox-dark` | `#282828` | warm, low contrast, brown-grey rather than neutral |
| `gruvbox-light` | `#fbf1c7` | the faded accent set, because the bright one is unreadable on cream |
| `monokai` | `#272822` | the high-saturation classic; pink keywords, green declarations |
| `zenburn` | `#3f3f3f` | the oldest idea here: nothing on screen should be bright |
| `nord` | `#2e3440` | four groups — polar night, snow storm, frost, aurora |
| `dracula` | `#282a36` | pastel accents on violet-grey; comments are blue, not grey |
| `tomorrow-night` | `#1d1f21` | the scheme Base16 grew out of; the most muted of the three near-blacks |
| `tango-dark` | `#2e3436` | the 2005 GNOME icon palette, which is why your terminal already looks like it |

Each styles all 23 faces the editor and its shipped modules name, so switching
between any two of them leaves nothing behind from the last. The build copies
this directory to `data/themes`, beside the executable, where `C-c t` finds
them.

How a theme is applied, written and installed, the names of the faces, and
what the terminal front end does not draw: the `themes` manual page,
`C-h m themes`.
