# Manual pages

The user's manual: one plain-text page per topic, read in the editor with
`C-h m` or `M-x manpage`. The build copies every `.txt` file here beside the
executable, into `target/<profile>/data/man`.

The pages are the complete reference for using the editor. A docstring is the
short reference for one name, and `C-h f` points from it to the page that is
the name's home. What goes where across all of the project's documentation is
in `doc/documentazione.typ`.

## A page

    NAME
        dired - a directory in a buffer

    SYNOPSIS
        (dired DIRECTORY)
        (dired-find-recursive)
        dired-sort

    DESCRIPTION
        What it is for, and the idea behind it, in a few paragraphs.

    OPENING A DIRECTORY
        One heading for each thing the reader wants to do.

    EXAMPLES
        A sentence saying what the example is for, then the example,
        complete and ready to paste into init.lisp or M-:.

            (setq dired-sort 'type)        ; directories first

    KEYS
        C-x d                   dired

    KEYS IN DIRED MODE
        <ret>                   dired-find-file

    SEE ALSO
        files, buffers, (list-dir)

Plain text rather than troff or Markdown, because `manpage-mode` gives a page
its faces from its structure: a heading is a line at column 0 made of
capitals, digits and spaces, and the body is indented four spaces. Lines are
at most 78 columns, since a page may be read in a split window.

## The sections

- **NAME**: the page's name and its subject, in one line.
- **SYNOPSIS**: the names this page is the home of, one per line:
  `(name ARGS)` for a function or a command, `name` alone for a variable. A
  name is in the SYNOPSIS of one page only, and `C-h f` and `C-h v` show that
  page as `Manual: page`. Keys do not go here.
- **DESCRIPTION** and the sections after it: what the reader can do and how,
  in the order they would want to do it, limits included. How the code works
  inside belongs in the developer manual.
- **EXAMPLES**: complete examples that can be pasted into `init.lisp` or
  `M-:`, each introduced by a sentence.
- **KEYS** and **KEYS IN NAME MODE**: one binding per line: the keys, two
  spaces or more, and the command.
- **SEE ALSO**: other pages by name, and functions as `(name)`.

Adding a page is adding a file here and a line to the index in `guide.txt`.

## What the tests check

`core/src/tests/guide_tests.rs` and `core/src/tests/doc_faithfulness_tests.rs`
check that:

- every key a KEYS line teaches runs the command it names;
- every page and every function a SEE ALSO names exists;
- every name a SYNOPSIS lists exists, and no name has two homes;
- every public name has a home, or is listed in
  `core/src/tests/doc_exceptions/homeless.txt`, a list that only shrinks;
- every name quoted as `` `name' `` exists, or is listed in one of the
  `quoted_*.txt` files beside it;
- `guide.txt` lists every page, and `rsedit.txt` lists the modules the
  default init.lisp loads, in its order;
- every page is laid out the way `manpage-mode` colours one.
