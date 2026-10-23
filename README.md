# rsedit

A terminal text editor written in Rust and programmed in its own embedded
Lisp.

rsedit follows the design of Emacs. A core in Rust provides the mechanisms:
buffers, windows, the display, the Lisp interpreter and the scheduling of
background work. Everything above them is written in Lisp modules that ship
with the editor and can be read, changed and reloaded while it runs. The keys
are Emacs's, and an optional module provides Vim-style modal editing.

Status: work in progress. The editor is usable; its interfaces may still
change.

## Features

- Emacs editing: the kill ring, keyboard macros, rectangles, incremental
  search and query-replace, undo with redo.
- Syntax colouring for Rust, C, C++, Python, TOML, Bash and the editor's own
  Lisp, computed in the background as the text changes; indentation rules
  for C/C++, Python, TOML, Bash and Lisp.
- Completion in every prompt (`M-x`, files, buffers) and at point.
- A directory editor, a buffer list, occur and project-wide grep, a
  compilation buffer that jumps to errors, and shell commands with their
  output in a buffer.
- Help that describes the running editor (`C-h f`, `C-h v`, `C-h k`,
  `C-h b`), and manual pages read inside it.
- Auto-save with recovery, reloading of files changed on disk, themes, mouse
  support, line numbers.
- Vim emulation, turned on with `M-x vim-mode`.
- A core library, `rsedit_core`, that knows nothing of terminals: it describes
  the screen and a front end draws it. The one in this repository is a
  terminal front end built on crossterm.

## Getting started

You need a recent stable Rust toolchain: the code is written in the 2024
edition.

```sh
git clone https://github.com/lmascelli/rsedit.git
cd rsedit
cargo run --release -- path/to/file
```

The build copies the Lisp modules, the manual pages and the themes into
`target/<profile>/data`, beside the executable, and the editor reads them from
there. To run it from somewhere else, copy the executable together with its
`data` directory.

On its first start the editor writes a commented `init.lisp` into its
configuration directory. Inside the editor, `C-h m` followed by `guide` opens
the manual at its first page; the `configuration` page says where that
directory is and what to put in the file.

## Documentation

| Document | For | Where |
|---|---|---|
| Manual pages | users: how to use the editor, with examples | `man/`, read in the editor with `C-h m` |
| Built-in help | users and authors of Lisp: every function, variable and key, from the running editor | `C-h f`, `C-h v`, `C-h k`, `C-h b` |
| Developer manual | Rust developers: architecture, threads, locks and design decisions | [`doc/architettura-editor.typ`](doc/architettura-editor.typ) |
| WorkManager design | history: the plan the WorkManager was built by; how background work runs is now in the developer manual | [`doc/workmanager.typ`](doc/workmanager.typ) |
| Documentation rules | contributors: where each kind of information goes, and the plan to get there | [`doc/documentazione.typ`](doc/documentazione.typ) |
| Problem report | maintainers: the known defects, each with an ID | [`doc/rapporto-problemi.typ`](doc/rapporto-problemi.typ) |
| Performance checks | developers: running the performance suite | [`PERFORMANCE.md`](PERFORMANCE.md) |
| The Lisp | the interpreter itself, in its own repository | [lmascelli/risp](https://github.com/lmascelli/risp) |

The documents in `doc/` are in Italian, written in Typst, and each has its PDF
beside it.

## Layout

```text
core/          rsedit_core: the editor as a library, with no terminal code
  src/         the Rust: state, buffers, windows, commands, primitives
  lisp/        the Lisp modules the default init.lisp loads
src/           the terminal front end
man/           the manual pages
themes/        the colour themes
doc/           the documents for developers
```

## Development

```sh
cargo test --workspace
```

Before changing documentation, read `doc/documentazione.typ`: every fact has
one home, and the tests check the copies against it.

## License

MIT; see [LICENSE](LICENSE).
