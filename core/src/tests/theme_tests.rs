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

    /// `Face::ALL` and `Face::index` have to agree: a face added to the enum
    /// but forgotten in `ALL` would be styleable and never listed, and one
    /// given a duplicate index would silently share another face's style.
    #[test]
    fn faces_are_indexed_consistently() {
        let mut theme = Theme::default();
        for (position, face) in Face::ALL.into_iter().enumerate() {
            // A style no other face has, keyed to this face's position.
            theme.set(face, Style::fg(Color::rgb(position as u8, 0, 0)));
        }
        for (position, face) in Face::ALL.into_iter().enumerate() {
            assert_eq!(
                theme.style(face).fg,
                Some(Color::rgb(position as u8, 0, 0)),
                "{} reads back a style that belongs to another face",
                face.name()
            );
        }
        assert_eq!(
            theme.bindings().len(),
            Face::ALL.len(),
            "every face should be listed exactly once"
        );
    }

    /// One name mapping, shared by `set-face` and `add-syntax-rule`. These
    /// were two lists, and `region` was in neither.
    #[test]
    fn every_face_round_trips_through_its_name() {
        for face in Face::ALL {
            assert_eq!(
                Face::from_name(face.name()),
                Some(face),
                "{} does not parse back from its own name",
                face.name()
            );
        }
        assert_eq!(Face::from_name("no-such-face"), None);
    }

    #[test]
    fn list_faces_reports_the_names_set_face_accepts() {
        let (ctx, env) = editor();
        let listed = strings(&eval_str("(list-faces)", &env, &ctx).expect("list-faces"));

        assert_eq!(listed.len(), Face::ALL.len());
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
        assert!(eval_str("(set-face 'nonesuch \"red\")", &env, &ctx).is_err());
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

        let style = ctx.face_style(Face::Region);
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

        let style = ctx.face_style(Face::Region);
        assert_eq!((style.fg, style.bg), (None, None));
        assert!(style.reverse);
    }

    /// The interactive form prompts for two colours; pressing Enter at one has
    /// to mean "skip", not "fail".
    #[test]
    fn an_empty_colour_string_leaves_that_side_alone() {
        let (ctx, env) = editor();
        eval_str("(set-face 'region \"\" \"blue\")", &env, &ctx).expect("set-face");

        let style = ctx.face_style(Face::Region);
        assert_eq!(style.fg, None, "an empty foreground is not an error");
        assert_eq!(style.bg, Some(Color::BLUE));
    }

    #[test]
    fn faces_are_named_by_string_or_symbol_alike() {
        let (ctx, env) = editor();
        eval_str("(set-face \"comment\" \"green\")", &env, &ctx).expect("by string");
        assert_eq!(ctx.face_style(Face::Comment).fg, Some(Color::GREEN));
        eval_str("(set-face 'comment \"blue\")", &env, &ctx).expect("by symbol");
        assert_eq!(ctx.face_style(Face::Comment).fg, Some(Color::BLUE));
    }

    // ---------------- defaults ----------------

    /// Reverse video needs no colour decision, so it cannot clash with the
    /// user's scheme or vanish on a light background. That is why it is what
    /// the region ships bound to.
    #[test]
    fn the_region_defaults_to_reverse_video_and_no_colour() {
        let (ctx, _env) = editor();
        let style = ctx.face_style(Face::Region);
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
            Face::Keyword,
            Face::Type,
            Face::String,
            Face::Comment,
            Face::Function,
            Face::Builtin,
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
            theme.style(Face::Default).is_plain(),
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
        assert!(before.theme.style(Face::Region).reverse);

        eval_str("(set-face 'region \"red\" nil nil)", &env, &ctx).expect("restyle");
        let after = ctx.snapshot(&env, 80, 24);

        assert_eq!(
            after.theme.style(Face::Region).fg,
            Some(Color::RED),
            "a new frame should see the new theme"
        );
        assert!(
            before.theme.style(Face::Region).reverse,
            "and the old frame should still describe the colours it was \
             composed under"
        );
    }
}
