//! The manual pages, checked against the editor they describe.
//!
//! # Why a guide needs a test at all
//!
//! Because a page that lists a key which does nothing is worse than no page.
//! A reader who presses it and gets "undefined" has learnt that the manual
//! lies, and from then on will not trust the parts that are true.
//!
//! Nothing stops that happening by itself. A key moved in
//! `common-keymaps.lisp`, a command renamed, a module whose binding went --
//! each is an ordinary change with no reason to think about prose in a
//! directory somewhere else.
//!
//! # What is checked, and what is not
//!
//! Each page may carry a `KEYS` section listing key sequences and the
//! commands they run, and `KEYS IN <mode>` sections for a mode's own. Every
//! line of those is resolved through the same walk the editor itself does, so
//! a page and the editor cannot disagree about a key without this failing.
//!
//! Every page named in a `SEE ALSO` must exist, and every `(name)` mentioned
//! there must be a function the editor has.
//!
//! Prose is *not* checked. A key mentioned in a sentence is not verified, and
//! the `KEYS` block at the foot of a page is there so that the keys it teaches
//! are somewhere a test can read them. That is a real limit and not a claim
//! that the pages cannot drift; it is the part of the drift that can be
//! caught cheaply.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// Every page, as (name, contents).
    ///
    /// Included at compile time rather than read from `man/`, so the test
    /// checks the pages that will be installed rather than whatever happens to
    /// be on the disk of the machine running it.
    const PAGES: &[(&str, &str)] = &[
        ("guide", include_str!("../../../man/guide.txt")),
        ("first-steps", include_str!("../../../man/first-steps.txt")),
        ("editing", include_str!("../../../man/editing.txt")),
        ("files", include_str!("../../../man/files.txt")),
        ("searching", include_str!("../../../man/searching.txt")),
        ("getting-help", include_str!("../../../man/getting-help.txt")),
        ("completion", include_str!("../../../man/completion.txt")),
        ("keymaps", include_str!("../../../man/keymaps.txt")),
        ("modes", include_str!("../../../man/modes.txt")),
        ("configuration", include_str!("../../../man/configuration.txt")),
        (
            "writing-commands",
            include_str!("../../../man/writing-commands.txt"),
        ),
        ("rsedit", include_str!("../../../man/rsedit.txt")),
        ("buffers", include_str!("../../../man/buffers.txt")),
        ("windows", include_str!("../../../man/windows.txt")),
        ("overlays", include_str!("../../../man/overlays.txt")),
        ("compile", include_str!("../../../man/compile.txt")),
        ("themes", include_str!("../../../man/themes.txt")),
        ("background", include_str!("../../../man/background.txt")),
        ("rectangles", include_str!("../../../man/rectangles.txt")),
        ("virtual-text", include_str!("../../../man/virtual-text.txt")),
        ("macros", include_str!("../../../man/macros.txt")),
    ];

    /// The editor with everything a page might mention loaded.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/common-keymaps.lisp"),
            include_str!("../../lisp/indent.lisp"),
            include_str!("../../lisp/minibuffer.lisp"),
            include_str!("../../lisp/completion.lisp"),
            include_str!("../../lisp/completion-at-point.lisp"),
            include_str!("../../lisp/clipboard.lisp"),
            include_str!("../../lisp/electric-pair.lisp"),
            include_str!("../../lisp/buffer-list.lisp"),
            include_str!("../../lisp/shell.lisp"),
            include_str!("../../lisp/compile.lisp"),
            include_str!("../../lisp/dired.lisp"),
            include_str!("../../lisp/occur.lisp"),
            include_str!("../../lisp/manpage.lisp"),
            include_str!("../../lisp/help.lisp"),
            include_str!("../../lisp/preview.lisp"),
            include_str!("../../lisp/theme.lisp"),
            include_str!("../../lisp/rust-mode.lisp"),
            include_str!("../../lisp/risp-mode.lisp"),
            include_str!("../../lisp/cc-mode.lisp"),
        ] {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .expect("the shipped lisp must parse");
            eval(&ast, env.clone(), &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .unwrap_or_else(|why| panic!("{src} must parse: {why:?}"));
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// One claim a page makes about a key.
    struct Claim {
        page: &'static str,
        mode: Option<String>,
        keys: String,
        command: String,
    }

    /// Every `KEYS` and `KEYS IN <mode>` line in every page.
    ///
    /// A line counts when it is indented, has two or more spaces between two
    /// fields, and the second field looks like a command name. That shape is
    /// what the blocks are written in, and anything else in a page -- prose,
    /// a code example, a blank line -- fails it and is skipped.
    fn claims() -> Vec<Claim> {
        let mut out = Vec::new();
        for (page, body) in PAGES {
            let mut mode: Option<Option<String>> = None;
            for line in body.lines() {
                if !line.starts_with(' ') && !line.trim().is_empty() {
                    // A heading at column zero ends whatever block we were in.
                    mode = match line.trim() {
                        "KEYS" => Some(None),
                        // `KEYS IN DIRED MODE` names `dired-mode`. Spelt in
                        // capitals because `manpage-mode` colours a heading
                        // only when it is one -- its rule is
                        // `^[A-Z][A-Z0-9 ]*$`, which a hyphen fails -- so a
                        // heading written the obvious way would be read as
                        // body text by the very mode these pages are for.
                        heading => heading.strip_prefix("KEYS IN ").map(|mode| {
                            Some(mode.trim().to_lowercase().replace(' ', "-"))
                        }),
                    };
                    continue;
                }
                let Some(in_block) = mode.as_ref() else {
                    continue;
                };
                let Some((keys, command)) = split_claim(line) else {
                    continue;
                };
                out.push(Claim {
                    page,
                    mode: in_block.clone(),
                    keys,
                    command,
                });
            }
        }
        out
    }

    /// A `KEYS` line split into what is pressed and what it runs.
    fn split_claim(line: &str) -> Option<(String, String)> {
        let trimmed = line.trim_end();
        let (keys, command) = trimmed.rsplit_once("  ")?;
        let keys = keys.trim();
        let command = command.trim();
        if keys.is_empty() || command.is_empty() {
            return None;
        }
        // A command name, and nothing else: lower case, letters, digits and
        // dashes. That rules out prose, which has capitals and punctuation.
        let is_name = command
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !is_name {
            return None;
        }
        Some((keys.to_string(), command.to_string()))
    }

    #[test]
    fn every_key_a_page_teaches_runs_what_it_says() {
        let (ctx, env) = editor();
        let claims = claims();
        // Both guards against the parser silently matching nothing, which
        // would make this test pass by checking an empty list.
        assert!(
            claims.len() > 50,
            "the KEYS blocks are not being found at all: {} claims",
            claims.len()
        );
        assert!(
            claims.iter().filter(|claim| claim.mode.is_some()).count() >= 10,
            "the `KEYS IN <MODE>' blocks are not being found"
        );
        let mut wrong = Vec::new();
        for claim in &claims {
            // The mode of the buffer the keys would be typed into is what
            // decides the walk, so a claim about a mode is checked in one.
            let mode = claim.mode.as_deref().unwrap_or("fundamental-mode");
            ctx.with_current_buffer_mut(|buf| buf.current_mode = mode.to_string());
            // The target is a bare symbol when the binding was built in Rust
            // and a one-element form when `define-key' made it, and both mean
            // "run this command". Asking for the car of a symbol is an error,
            // so which it is has to be decided before taking it.
            let found = run(
                &format!(
                    r#"(let ((target (nth 1 (key-binding "{}"))))
                         (if (listp target) (car target) target))"#,
                    // `M-\` is a real binding, and a backslash in a Lisp
                    // string literal is an escape -- so the key has to be
                    // written the way the reader of the string will take it.
                    claim.keys.replace('\\', "\\\\")
                ),
                &env,
                &ctx,
            );
            let found = match found {
                LispExp::Symbol(name) => name.to_string(),
                other => format!("{other:?}"),
            };
            if found != claim.command {
                wrong.push(format!(
                    "{}: {} is {found}, not {}",
                    claim.page, claim.keys, claim.command
                ));
            }
        }
        assert!(wrong.is_empty(), "{} wrong:\n{}", wrong.len(), wrong.join("\n"));
    }

    #[test]
    fn every_page_a_page_points_at_exists() {
        // The bug this was written for: `rsedit.txt` pointed at `completion',
        // `modes' and `keymaps', none of which had been written. A reader
        // following a cross-reference was told the page did not exist, by the
        // manual, about itself.
        let known: BTreeSet<&str> = PAGES.iter().map(|(name, _)| *name).collect();
        let mut missing = Vec::new();
        for (page, body) in PAGES {
            for named in see_also(body) {
                // A `(name)` in a SEE ALSO is a function, not a page.
                if named.starts_with('(') {
                    continue;
                }
                if !known.contains(named.as_str()) {
                    missing.push(format!("{page} points at {named}"));
                }
            }
        }
        assert!(missing.is_empty(), "{}", missing.join("\n"));
    }

    #[test]
    fn every_function_a_page_points_at_exists() {
        let (ctx, env) = editor();
        let mut missing = Vec::new();
        for (page, body) in PAGES {
            for named in see_also(body) {
                let Some(name) = named.strip_prefix('(').and_then(|n| n.strip_suffix(')')) else {
                    continue;
                };
                if run(&format!("(functionp '{name})"), &env, &ctx).is_nil() {
                    missing.push(format!("{page} points at ({name})"));
                }
            }
        }
        assert!(missing.is_empty(), "{}", missing.join("\n"));
    }

    /// The names in a page's SEE ALSO, pages and `(functions)` alike.
    fn see_also(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut inside = false;
        for line in body.lines() {
            if !line.starts_with(' ') && !line.trim().is_empty() {
                inside = line.trim() == "SEE ALSO";
                continue;
            }
            if !inside {
                continue;
            }
            for part in line.split(',') {
                let part = part.trim();
                if !part.is_empty() {
                    out.push(part.to_string());
                }
            }
        }
        out
    }

    #[test]
    fn every_page_is_laid_out_the_way_the_mode_colours_one() {
        // `manpage-mode` colours a heading because it is a word alone at the
        // left margin in capitals, and nothing else. A page that put its
        // headings anywhere else would be read as unformatted text.
        for (page, body) in PAGES {
            assert!(
                body.starts_with("NAME\n"),
                "{page} should begin with a NAME heading"
            );
            for (number, line) in body.lines().enumerate() {
                assert!(
                    line.chars().count() <= 78,
                    "{page}:{} is {} columns; pages are read in a window that \
                     may be split",
                    number + 1,
                    line.chars().count()
                );
                if line.starts_with(' ') || line.trim().is_empty() {
                    continue;
                }
                // `manpage-mode`'s own rule, which is what decides whether
                // this is drawn as a heading: a capital, then capitals,
                // digits and spaces. A comma or a hyphen in one leaves it
                // looking like body text that happens to shout.
                let mut characters = line.chars();
                let heading = characters.next().is_some_and(|c| c.is_ascii_uppercase())
                    && characters
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ');
                assert!(
                    heading,
                    "{page}:{} is at the left margin and `manpage-mode' would \
                     not colour it as a heading: {line:?}",
                    number + 1
                );
            }
        }
    }

    #[test]
    fn the_guide_lists_every_page_there_is() {
        // So that a page written and not listed is a test failure rather than
        // a page nobody finds.
        let guide = PAGES
            .iter()
            .find(|(name, _)| *name == "guide")
            .expect("the guide")
            .1;
        // The *index* entries, not the whole text. Looking anywhere in the
        // page was the first version and it passed an ablation that deleted
        // an index line, because the name was still in the SEE ALSO at the
        // foot -- which is not the reader finding it.
        let listed: BTreeSet<&str> = guide
            .lines()
            .filter(|line| line.starts_with("        "))
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        let mut unlisted = Vec::new();
        for (page, _) in PAGES {
            if *page == "guide" {
                continue;
            }
            if !listed.contains(page) {
                unlisted.push(*page);
            }
        }
        assert!(
            unlisted.is_empty(),
            "the guide does not mention: {}",
            unlisted.join(", ")
        );
    }
}
