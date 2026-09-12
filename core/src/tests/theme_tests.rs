//! Faces, the theme that styles them, and the Lisp that binds them.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::ui::{Color, Face, NAMED_COLORS, Style, Theme};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env")
    }

    /// The string items of a returned list. `face-style` puts nil in for a
    /// colour the face leaves alone, and numbers for palette indices, so this
    /// keeps only what is meant to be compared as text.
    fn strings(exp: &LispExp<Ctx>) -> Vec<String> {
        exp.iter()
            .filter_map(|item| match item {
                LispExp::String(text) => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    // ---------------- the face set ----------------

    /// The constants and [`Face::BUILT_IN`] have to agree. A built-in whose
    /// constant did not match its position in that array would silently share
    /// another face's style, and the array is what fills the registry -- so
    /// the two are the same fact written twice.
    #[test]
    fn built_in_faces_are_interned_at_their_own_ids() {
        for (position, (face, name)) in Face::BUILT_IN.into_iter().enumerate() {
            assert_eq!(
                face.index(),
                position,
                "{name} is declared at a different id than it sits at"
            );
            assert_eq!(&*face.name(), name, "{name} does not know its own name");
        }
    }

    /// Distinct ids mean distinct styles -- the property a theme depends on.
    #[test]
    fn faces_do_not_share_a_style() {
        let mut theme = Theme::default();
        for (position, (face, _)) in Face::BUILT_IN.into_iter().enumerate() {
            theme.set(face, Style::fg(Color::rgb(position as u8, 0, 0)));
        }
        for (position, (face, _)) in Face::BUILT_IN.into_iter().enumerate() {
            assert_eq!(
                theme.style(face).fg,
                Some(Color::rgb(position as u8, 0, 0)),
                "{} reads back a style that belongs to another face",
                face.name()
            );
        }
    }

    /// One name mapping, shared by `set-face` and `add-syntax-rule`. These
    /// were two lists, and `region` was in neither.
    #[test]
    fn every_face_round_trips_through_its_name() {
        for (face, _) in Face::BUILT_IN {
            assert_eq!(
                Face::named(&face.name()),
                Some(face),
                "{} does not parse back from its own name",
                face.name()
            );
        }
    }

    // ---------------- the set is open ----------------

    /// The point of the whole thing: a grammar names a face its language needs
    /// and it exists, with no change to any Rust file.
    #[test]
    fn naming_a_new_face_defines_it() {
        let before = Face::named("rust-attribute");
        let face = Face::intern("rust-attribute");

        assert!(
            before.is_none() || before == Some(face),
            "interning twice must give the same face back"
        );
        assert_eq!(&*face.name(), "rust-attribute");
        assert_eq!(Face::named("rust-attribute"), Some(face));
        assert_eq!(
            Face::intern("rust-attribute"),
            face,
            "a name already taken must not define a second face"
        );
    }

    #[test]
    fn a_new_face_can_be_styled_and_listed_from_lisp() {
        let (ctx, env) = editor();
        eval_str(r#"(set-face 'doc-comment "bright-blue" nil)"#, &env, &ctx).expect("set-face");

        let listed = strings(&eval_str("(list-faces)", &env, &ctx).expect("list-faces"));
        assert!(
            listed.iter().any(|name| name == "doc-comment"),
            "a face named from Lisp should be listed, got {listed:?}"
        );
        assert_eq!(
            ctx.face_style(Face::named("doc-comment").expect("just defined"))
                .fg,
            Some(Color::BRIGHT_BLUE)
        );
    }

    /// The headline: a grammar names a face its language needs, and the rule
    /// carries that face rather than falling back to the default.
    #[test]
    fn a_syntax_rule_can_name_a_face_that_did_not_exist() {
        let (ctx, env) = editor();
        eval_str(
            r#"(progn (make-mode 'toy-mode)
                      (add-syntax-rule 'toy-mode "\\bmacro\\b" 'toy-macro))"#,
            &env,
            &ctx,
        )
        .expect("defining the rule");

        let face = Face::named("toy-macro").expect("the rule should have defined the face");
        assert_ne!(face, Face::DEFAULT, "it must not have fallen back");
        assert_eq!(
            ctx.mode_registry
                .read()
                .expect("mode registry")
                .get("toy-mode")
                .expect("toy-mode")
                .grammar
                .rules
                .first()
                .expect("one rule")
                .face,
            face,
            "the rule should carry the face it named"
        );
    }

    /// A face nobody has themed is unstyled rather than out of bounds -- which
    /// is what lets a grammar name faces before any theme mentions them.
    #[test]
    fn an_unstyled_face_reads_back_plain() {
        let theme = Theme::default();
        assert_eq!(theme.style(Face::intern("never-themed")), Style::plain());
    }

    #[test]
    fn list_faces_reports_the_names_set_face_accepts() {
        let (ctx, env) = editor();
        let listed = strings(&eval_str("(list-faces)", &env, &ctx).expect("list-faces"));

        for (_, name) in Face::BUILT_IN {
            assert!(
                listed.iter().any(|listed| listed == name),
                "list-faces should include the built-in {name:?}"
            );
        }
        for name in &listed {
            assert!(
                eval_str(&format!("(set-face \"{name}\" \"red\")"), &env, &ctx).is_ok(),
                "list-faces offered {name:?}, which set-face then rejected"
            );
        }
    }

    // ---------------- colours ----------------

    #[test]
    fn colours_parse_in_every_form_lisp_can_write_them() {
        assert_eq!(Color::parse("#ff8800"), Some(Color::rgb(255, 136, 0)));
        assert_eq!(
            Color::parse("#f80"),
            Some(Color::rgb(255, 136, 0)),
            "the short form should expand to the same colour as the long one"
        );
        assert_eq!(Color::parse("red"), Some(Color::RED));
        assert_eq!(Color::parse("bright-red"), Some(Color::BRIGHT_RED));

        for bad in ["#ff", "#gggggg", "puce", ""] {
            assert_eq!(Color::parse(bad), None, "{bad:?} should not parse");
        }
    }

    /// A palette index is one terminal's encoding of a colour, not a colour.
    /// Letting it into a face would put the renderer's representation in the
    /// editor, which is the thing [`Color`] exists to keep out.
    #[test]
    fn a_palette_index_is_not_a_colour_a_face_can_ask_for() {
        assert_eq!(Color::parse("214"), None);
        let (ctx, env) = editor();
        assert!(eval_str("(set-face 'region \"214\")", &env, &ctx).is_err());
    }

    /// Every colour a face can hold writes back out in a form `set-face` takes,
    /// so a theme can be read out of a running editor and pasted into a config.
    #[test]
    fn every_colour_round_trips_through_its_text() {
        for (name, color) in NAMED_COLORS {
            assert_eq!(
                Color::parse(&color.to_text()),
                Some(color),
                "{name} does not survive being written out and read back"
            );
        }
        let exact = Color::rgb(0x3a, 0x5f, 0xcd);
        assert_eq!(exact.to_text(), "#3a5fcd");
        assert_eq!(Color::parse(&exact.to_text()), Some(exact));
    }

    /// Every name is a distinct colour, so two of them can never be confused
    /// for one another by a renderer picking the nearest match.
    #[test]
    fn the_colour_names_are_all_different_colours() {
        for (i, (name_a, a)) in NAMED_COLORS.iter().enumerate() {
            for (name_b, b) in NAMED_COLORS.iter().skip(i + 1) {
                assert_ne!(a, b, "{name_a} and {name_b} are the same colour");
            }
        }
    }

    #[test]
    fn list_colors_reports_the_names_set_face_accepts() {
        let (ctx, env) = editor();
        let listed = strings(&eval_str("(list-colors)", &env, &ctx).expect("list-colors"));
        assert_eq!(listed.len(), NAMED_COLORS.len());
        for name in &listed {
            assert!(
                Color::parse(name).is_some(),
                "list-colors offered {name:?}, which does not parse"
            );
        }
    }

    #[test]
    fn a_colour_that_does_not_parse_is_reported_rather_than_ignored() {
        let (ctx, env) = editor();
        assert!(
            eval_str("(set-face 'region \"puce\")", &env, &ctx).is_err(),
            "a mistyped colour should say so, not leave the face unstyled"
        );
        assert!(
            eval_str("(set-face 'region nil nil '(\"bolder\"))", &env, &ctx).is_err(),
            "and so should a mistyped attribute"
        );
        // An unknown *face* name is no longer an error: naming one is how a
        // face comes to exist. A colour and an attribute still are, because
        // there is a fixed set of each and nothing a typo could be defining.
        assert!(
            eval_str("(set-face 'nonesuch \"red\")", &env, &ctx).is_ok(),
            "the face set is open -- see `Face`"
        );
    }

    // ---------------- binding faces from Lisp ----------------

    #[test]
    fn set_face_binds_a_style_that_face_style_reads_back() {
        let (ctx, env) = editor();
        eval_str(
            "(set-face 'region \"#f8f8f2\" \"#3a5fcd\" '(\"bold\" \"underline\"))",
            &env,
            &ctx,
        )
        .expect("set-face");

        let style = ctx.face_style(Face::REGION);
        assert_eq!(style.fg, Some(Color::rgb(0xf8, 0xf8, 0xf2)));
        assert_eq!(style.bg, Some(Color::rgb(0x3a, 0x5f, 0xcd)));
        assert!(style.bold && style.underline);
        assert!(
            !style.reverse,
            "set-face replaces the binding rather than adding to it, so the \
             default's reverse must be gone"
        );

        assert_eq!(
            strings(&eval_str("(face-style 'region)", &env, &ctx).expect("face-style")),
            vec!["#f8f8f2", "#3a5fcd", "bold", "underline"],
            "face-style should read back in the form set-face takes"
        );
    }

    #[test]
    fn a_face_can_be_bound_to_attributes_alone() {
        let (ctx, env) = editor();
        eval_str("(set-face 'region nil nil '(\"reverse\"))", &env, &ctx).expect("set-face");

        let style = ctx.face_style(Face::REGION);
        assert_eq!((style.fg, style.bg), (None, None));
        assert!(style.reverse);
    }

    /// The interactive form prompts for two colours; pressing Enter at one has
    /// to mean "skip", not "fail".
    #[test]
    fn an_empty_colour_string_leaves_that_side_alone() {
        let (ctx, env) = editor();
        eval_str("(set-face 'region \"\" \"blue\")", &env, &ctx).expect("set-face");

        let style = ctx.face_style(Face::REGION);
        assert_eq!(style.fg, None, "an empty foreground is not an error");
        assert_eq!(style.bg, Some(Color::BLUE));
    }

    #[test]
    fn faces_are_named_by_string_or_symbol_alike() {
        let (ctx, env) = editor();
        eval_str("(set-face \"comment\" \"green\")", &env, &ctx).expect("by string");
        assert_eq!(ctx.face_style(Face::COMMENT).fg, Some(Color::GREEN));
        eval_str("(set-face 'comment \"blue\")", &env, &ctx).expect("by symbol");
        assert_eq!(ctx.face_style(Face::COMMENT).fg, Some(Color::BLUE));
    }

    // ---------------- defaults ----------------

    /// Reverse video needs no colour decision, so it cannot clash with the
    /// user's scheme or vanish on a light background. That is why it is what
    /// the region ships bound to.
    #[test]
    fn the_region_defaults_to_reverse_video_and_no_colour() {
        let (ctx, _env) = editor();
        let style = ctx.face_style(Face::REGION);
        assert!(style.reverse);
        assert_eq!((style.fg, style.bg), (None, None));
    }

    /// The defaults are conventional colours, so a renderer that has to
    /// approximate lands on the palette slot of the same name -- which is the
    /// user's own configured colour.
    #[test]
    fn the_syntax_defaults_are_conventional_colours() {
        let theme = Theme::default();
        for face in [
            Face::KEYWORD,
            Face::TYPE,
            Face::STRING,
            Face::COMMENT,
            Face::FUNCTION,
            Face::BUILTIN,
        ] {
            assert!(
                NAMED_COLORS
                    .iter()
                    .any(|(_, c)| Some(*c) == theme.style(face).fg),
                "{} should default to one of the conventional colours",
                face.name()
            );
        }
        assert!(
            theme.style(Face::DEFAULT).is_plain(),
            "the default face must ask for nothing, so ordinary text is drawn \
             exactly as it was before any of this existed"
        );
    }

    // ---------------- the frame carries it ----------------

    /// Drawing touches no shared state, so the theme travels in the frame
    /// rather than being looked up by the renderer.
    #[test]
    fn a_snapshot_carries_the_theme_as_it_stood() {
        let (ctx, env) = editor();
        let before = ctx.snapshot(&env, 80, 24);
        assert!(before.theme.style(Face::REGION).reverse);

        eval_str("(set-face 'region \"red\" nil nil)", &env, &ctx).expect("restyle");
        let after = ctx.snapshot(&env, 80, 24);

        assert_eq!(
            after.theme.style(Face::REGION).fg,
            Some(Color::RED),
            "a new frame should see the new theme"
        );
        assert!(
            before.theme.style(Face::REGION).reverse,
            "and the old frame should still describe the colours it was \
             composed under"
        );
    }
}
