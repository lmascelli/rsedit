//! The manual pages, checked against the editor they describe: every line of
//! a `KEYS` section resolved as the editor resolves keys, every page a
//! `SEE ALSO` names, the module lists and the SYNOPSIS names. Prose is not
//! checked.
//!
//! Design: `doc/architettura-editor.typ, <test-documentazione>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, SETTINGS, create_global_env};
    use crate::tests::shipped::{
        default_modules, editor_functions, is_private, load_default_modules, man_pages,
        module_variables,
    };
    use risp::{Env, LispExp, Parser, VARIABLE_DOCUMENTATION, eval};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// The editor with every module the default init.lisp loads: everything
    /// a page might mention.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        load_default_modules(&env, &ctx);
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
        page: String,
        mode: Option<String>,
        keys: String,
        command: String,
    }

    /// Returns every `KEYS` and `KEYS IN <mode>` line in every page: indented,
    /// two fields two spaces apart, the second a command name.
    fn claims() -> Vec<Claim> {
        let mut out = Vec::new();
        for (page, body) in man_pages() {
            let mut mode: Option<Option<String>> = None;
            for line in body.lines() {
                if !line.starts_with(' ') && !line.trim().is_empty() {
                    // A heading at column zero ends whatever block we were in.
                    mode = match line.trim() {
                        "KEYS" => Some(None),
                        // `KEYS IN DIRED MODE` names `dired-mode`: a
                        // heading in capitals, as `manpage-mode` colours.
                        heading => heading
                            .strip_prefix("KEYS IN ")
                            .map(|mode| Some(mode.trim().to_lowercase().replace(' ', "-"))),
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
                    page: page.clone(),
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
            // A bare symbol from Rust, a one-element form from `define-key'.
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
        assert!(
            wrong.is_empty(),
            "{} wrong:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn every_page_a_page_points_at_exists() {
        let pages = man_pages();
        let known: BTreeSet<&str> = pages.iter().map(|(name, _)| name.as_str()).collect();
        let mut missing = Vec::new();
        for (page, body) in &pages {
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
        for (page, body) in man_pages() {
            for named in see_also(&body) {
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
        for (page, body) in man_pages() {
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
                // `manpage-mode`'s rule for a heading: a capital, then
                // capitals, digits and spaces.
                let mut characters = line.chars();
                let heading = characters.next().is_some_and(|c| c.is_ascii_uppercase())
                    && characters.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ');
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
        let pages = man_pages();
        let guide = &pages
            .iter()
            .find(|(name, _)| name == "guide")
            .expect("the guide")
            .1;
        // The index entries, not the whole text: the SEE ALSO at the foot is
        // not the reader finding a page.
        let listed: BTreeSet<&str> = guide
            .lines()
            .filter(|line| line.starts_with("        "))
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        let mut unlisted = Vec::new();
        for (page, _) in &pages {
            if page == "guide" {
                continue;
            }
            if !listed.contains(page.as_str()) {
                unlisted.push(page.as_str());
            }
        }
        assert!(
            unlisted.is_empty(),
            "the guide does not mention: {}",
            unlisted.join(", ")
        );
    }

    // ----------------------------------------------------------------
    // The modules, and the names each page is the home of
    // ----------------------------------------------------------------

    /// The body of the section HEADING in TEXT: the lines under it, up to the
    /// next heading.
    fn section(text: &str, heading: &str) -> String {
        let mut inside = false;
        let mut body = String::new();
        for line in text.lines() {
            if !line.starts_with(' ') && !line.trim().is_empty() {
                inside = line.trim() == heading;
                continue;
            }
            if inside {
                body.push_str(line);
                body.push('\n');
            }
        }
        body
    }

    #[test]
    fn the_module_list_is_the_one_init_lisp_loads() {
        // A list line is a module name, two spaces or more, and what it adds;
        // the prose above the list has single spaces between its words.
        let rsedit = man_pages()
            .into_iter()
            .find(|(name, _)| name == "rsedit")
            .expect("the rsedit page")
            .1;
        let listed: Vec<String> = section(&rsedit, "MODULES")
            .lines()
            .filter_map(|line| {
                let (name, what) = line.strip_prefix("    ")?.split_once("  ")?;
                let is_name = !name.is_empty() && !name.contains(' ');
                (is_name && !what.trim().is_empty()).then(|| name.to_string())
            })
            .collect();
        assert_eq!(
            listed,
            default_modules(),
            "rsedit.txt's MODULES must list what the default init.lisp loads, in its order"
        );
    }

    /// Whether NAME is a function, a macro or a variable of the editor.
    fn is_defined(name: &str, env: &Arc<Env<Ctx>>) -> bool {
        env.get_function(name).is_some()
            || env.get_macro(name).is_some()
            || env.get_variable(name).is_some()
            || env.get_property(name, VARIABLE_DOCUMENTATION).is_some()
    }

    /// The names each page's SYNOPSIS lists, as `manpage--synopsis-names`
    /// reads them: the reader `C-h f` uses, so the two cannot disagree.
    fn synopses(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<(String, Vec<String>)> {
        man_pages()
            .into_iter()
            .map(|(page, text)| {
                env.set_variable("guide-tests--page".into(), LispExp::string(text));
                let names = run("(manpage--synopsis-names guide-tests--page)", env, ctx)
                    .iter()
                    .map(|name| match name {
                        LispExp::String(name) => name.to_string(),
                        other => format!("{other:?}"),
                    })
                    .collect();
                (page, names)
            })
            .collect()
    }

    #[test]
    fn every_name_a_synopsis_lists_is_defined() {
        let (ctx, env) = editor();
        let synopses = synopses(&env, &ctx);
        let count: usize = synopses.iter().map(|(_, names)| names.len()).sum();
        assert!(
            count >= 10,
            "the SYNOPSIS sections are not being read: {count} names"
        );
        let undefined: Vec<String> = synopses
            .iter()
            .flat_map(|(page, names)| {
                names
                    .iter()
                    .filter(|name| !is_defined(name, &env))
                    .map(move |name| format!("{page}: {name}"))
            })
            .collect();
        assert!(
            undefined.is_empty(),
            "a SYNOPSIS lists names, and these are not the editor's (keys go under KEYS):\n  {}",
            undefined.join("\n  ")
        );
    }

    #[test]
    fn no_name_has_two_homes() {
        let (ctx, env) = editor();
        let mut homes: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for (page, names) in synopses(&env, &ctx) {
            for name in names {
                homes.entry(name).or_default().push(page.clone());
            }
        }
        let shared: Vec<String> = homes
            .into_iter()
            .filter(|(_, pages)| pages.len() > 1)
            .map(|(name, pages)| format!("{name}: {}", pages.join(", ")))
            .collect();
        assert!(
            shared.is_empty(),
            "a name is listed in one SYNOPSIS, the page that documents it:\n  {}",
            shared.join("\n  ")
        );
    }

    #[test]
    fn every_public_name_has_a_home() {
        let (ctx, env) = editor();
        let homed: BTreeSet<String> = synopses(&env, &ctx)
            .into_iter()
            .flat_map(|(_, names)| names)
            .collect();
        let public: BTreeSet<String> = editor_functions(&env)
            .into_iter()
            .chain(module_variables().into_iter().map(|(_, name)| name))
            .chain(SETTINGS.iter().map(|setting| setting.name.to_string()))
            .filter(|name| !is_private(name))
            .collect();
        assert!(public.len() > 300, "too few public names: {}", public.len());
        let homeless: Vec<&str> = public
            .iter()
            .filter(|name| !homed.contains(*name))
            .map(String::as_str)
            .collect();
        assert!(
            homeless.is_empty(),
            "no page lists these in its SYNOPSIS; list each in the page that documents it:\n{}",
            homeless.join("\n")
        );
    }

    #[test]
    fn the_editing_index_names_each_function_under_its_home_and_misses_none() {
        let (ctx, env) = editor();
        let synopses = synopses(&env, &ctx);
        let homes: std::collections::BTreeMap<&str, &str> = synopses
            .iter()
            .flat_map(|(page, names)| names.iter().map(move |name| (name.as_str(), page.as_str())))
            .collect();
        let index = man_pages()
            .into_iter()
            .find(|(name, _)| name == "editing-index")
            .expect("the editing index")
            .1;
        // An entry is a name eight spaces in; the page it is on is the one
        // the last line ending in "page:" named, within the same section.
        let mut page: Option<String> = None;
        let mut listed = BTreeSet::new();
        let mut wrong = Vec::new();
        for line in index.lines() {
            if !line.starts_with(' ') && !line.trim().is_empty() {
                page = None;
            } else if line.trim_end().ends_with("page:") {
                page = line
                    .split('`')
                    .nth(1)
                    .and_then(|rest| rest.split('\'').next())
                    .map(str::to_string);
            } else if line.starts_with("        ") && !line.starts_with("         ") {
                let name = line.split_whitespace().next().expect("an entry has a name");
                listed.insert(name.to_string());
                if homes.get(name).copied() != page.as_deref() {
                    wrong.push(format!(
                        "{name}: under {page:?}, its home is {:?}",
                        homes.get(name)
                    ));
                }
            }
        }
        assert!(
            listed.len() > 100,
            "the entries are not being found: {}",
            listed.len()
        );
        // Every function these pages are the home of is in the index.
        let missing: Vec<String> = synopses
            .iter()
            .filter(|(page, _)| {
                ["editing", "killing", "rectangles", "indentation"].contains(&page.as_str())
            })
            .flat_map(|(_, names)| names)
            .filter(|name| env.get_function(name).is_some() && !listed.contains(*name))
            .cloned()
            .collect();
        assert!(
            wrong.is_empty(),
            "listed under the wrong page:\n  {}",
            wrong.join("\n  ")
        );
        assert!(
            missing.is_empty(),
            "not in the editing index:\n  {}",
            missing.join("\n  ")
        );
    }
}
