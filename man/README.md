# Manual pages

Plain text, one file per topic, named `<topic>.txt`. The build copies every
file here into `target/<profile>/data/man`, beside `data/lisp` where the
modules go, and `rsedit-man` reads them from there.

Plain text rather than troff or Markdown because these are read *in the
editor*, by `manpage-mode`, which gives text a face through its mode's syntax
rules. Those rules recognise structure — a heading is a word alone at the left
margin in capitals, an option is an indented dash — so a page written to look
like a manual page is one that gets faced like one. Markup would have to be
stripped before display and would buy nothing.

## Layout

    NAME
        insert - put text in a buffer

    SYNOPSIS
        (insert &rest STRINGS)

    DESCRIPTION
        ...

Headings in capitals at column 0, bodies indented four spaces. Keep lines
under 78 columns: the pages are read in a window that may be split.

## What belongs here

Topics, not functions. Every primitive already carries a doc string, which
`function-doc` and `C-M-i` will show you and which cannot go stale — that is
the reference. These pages are for what a doc string cannot say: how the
pieces fit, why something is the shape it is, and what to reach for.

Adding one is adding a file. Nothing lists them.
