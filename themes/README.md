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

Each file styles all 23 faces the editor and its shipped modules name, so
switching between any two of them leaves nothing behind from the last.

## Installing

Themes are data read at runtime from `data/themes`, beside the executable.
Either:

**From the source tree** — drop the files in `themes/` and rebuild.
`core/build.rs` copies them to `target/<profile>/data/themes/`:

```sh
cp *.lisp /path/to/rsedit/themes/
cargo build --release
```

**Into an installed editor** — drop them straight beside the binary, no
rebuild:

```sh
cp *.lisp "$(dirname "$(command -v rsedit)")/data/themes/"
```

Then `C-c t` lists them and Return applies one, or from a config file:

```lisp
(select-theme 'gruvbox-dark)
```

## Two things to know

**Your terminal's background is what you see.** The TUI front end does not
read the `default` face: it prints buffer rows without applying a style and
draws syntax colouring as highlights over them, so a theme's `default` entry —
`"#839496" "#002b36"` for Solarized Dark — currently sets nothing. Everything
else in the theme works; the page itself stays whatever your terminal is set
to. Set your terminal's foreground and background to the values in the
`default` line of whichever theme you pick and the two will agree.

The entry is in every file regardless, because it is the honest statement of
what the scheme's base colours are, and because it starts working the day
`src/tui.rs` applies it before the row loop.

**`results-here` and `results-target` are the current names.** The three
themes that ship with the editor still use `compilation-here` and
`compilation-target`, which nothing has read since the results listings were
generalised out of compile-mode — so the marks `next-error` leaves are
unstyled in those three. Nothing reports it, because `set-face` on an unknown
name interns a new face rather than failing. These ten use the current names.

## Writing your own

Copy any of these and edit the hex values; the format is one entry per face:

```lisp
(put 'my-theme-theme 'faces
     '((keyword "#859900" nil     ("bold"))
       (region  nil       "#073642" nil)))
```

`(FACE FOREGROUND BACKGROUND ATTRIBUTES)` — the same four arguments
`set-face` takes. Colours are `"#rrggbb"`, `"#rgb"`, or one of the sixteen
names (`blue`, `bright-blue`, …); attributes are `"bold"`, `"italic"`,
`"underline"`, `"reverse"`. `nil` anywhere means "say nothing about this".

A file named `my-theme.lisp` must `put` onto `my-theme-theme`: the loader
builds the symbol from the filename.

Be careful with face *names* — a misspelled one is silently a new face that
nothing draws. The names, in full:

```
default  region  mode-line  mode-line-inactive  window-separator
line-number  line-number-current  replace-match
keyword  type  function  builtin  string  comment  doc-comment
attribute  preprocessor  error
results-here  results-target  preview
manpage-bold  manpage-underline
```
