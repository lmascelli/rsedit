//! The terminal front end: keys and the mouse from crossterm, the frame
//! drawn as escape sequences, colours resolved to what the terminal can
//! show, and the clipboard through OSC 52.
//!
//! Design: `doc/architettura-editor.typ, <tui>`.
use crate::tests::tui_tests::ColorDepth::TrueColor;
use crossterm::event::{KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers};
use crossterm::style::Color as CrossColor;
use rsedit_core::ELispExp;
use rsedit_core::EditorState;
use rsedit_core::Env;
use rsedit_core::GapBuffer;
use rsedit_core::KeyCode;
use rsedit_core::{Color, Face, FrameSnapshot, Highlight, NAMED_COLORS, Rect, Style, Theme};
use rsedit_core::{create_global_env, isolate_config_for_tests};
use std::sync::Arc;

use crate::tests::tui_tests::ColorDepth::Ansi16;
use crate::tests::tui_tests::ColorDepth::Ansi256;
use crate::tui::colour::{ColorDepth, resolve};
use crate::tui::input::translate_key;
use crate::tui::render::{base64_encode, osc52_copy, printable, render_to};

const COLS: u16 = 80;
const ROWS: u16 = 24;

/// Returns an editor, with this process's configuration pointed somewhere
/// harmless first: the core's `cfg(test)` does not reach a crate that
/// depends on it, so these tests ask for it themselves.
fn editor() -> (EditorState<GapBuffer>, Arc<Env<EditorState<GapBuffer>>>) {
    isolate_config_for_tests();
    create_global_env::<GapBuffer>().expect("global env")
}

/// Returns one frame, rendered into memory.
fn frame(state: &EditorState<GapBuffer>, env: &Arc<Env<EditorState<GapBuffer>>>) -> String {
    let mut out: Vec<u8> = Vec::new();
    let snapshot = state.snapshot(env, COLS as usize, ROWS as usize);
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("rendering must succeed");
    String::from_utf8(out).expect("crossterm emits valid UTF-8")
}

/// Returns every cursor placement in the stream, as (col, row), in order,
/// read from `ESC [ y+1 ; x+1 H` as a terminal reads it.
fn placements(rendered: &str) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    for chunk in rendered.split('\u{1b}').skip(1) {
        let Some(body) = chunk.strip_prefix('[') else {
            continue;
        };
        let Some(end) = body.find('H') else { continue };
        let coords = &body[..end];
        if let Some((row, col)) = coords.split_once(';') {
            if let (Ok(row), Ok(col)) = (row.parse::<u16>(), col.parse::<u16>()) {
                out.push((col.saturating_sub(1), row.saturating_sub(1)));
            }
        }
    }
    out
}

fn expected_cursor(
    state: &EditorState<GapBuffer>,
    env: &Arc<Env<EditorState<GapBuffer>>>,
) -> (u16, u16) {
    let snapshot = state.snapshot(env, COLS as usize, ROWS as usize);
    let view = snapshot
        .focused_view()
        .expect("a window must have focus")
        .clone();
    let (cx, cy) = view
        .cursor_rel_pos
        .expect("the focused window has a cursor");
    (
        (view.rect.x + cx as isize) as u16,
        (view.rect.y + cy as isize) as u16,
    )
}

// -----------------------------------------------------------------
// Getting a keystroke from the terminal to the editor
// -----------------------------------------------------------------

use crossterm::event::{KeyEvent as CrossKeyEvent, KeyEventKind, KeyEventState};

/// Returns one key as crossterm reports it.
fn terminal_key(code: CrossKeyCode, modifiers: CrossModifiers) -> CrossKeyEvent {
    CrossKeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// Escape translates, so `minibuffer-cancel`, bound to it by
/// `minibuffer-mode`, can be reached.
#[test]
fn escape_reaches_the_editor() {
    assert_eq!(
        translate_key(terminal_key(CrossKeyCode::Esc, CrossModifiers::NONE))
            .expect("Escape must translate")
            .code,
        KeyCode::Esc
    );
}

/// Every key the editor can name translates: a missing arm is invisible
/// from the outside, as the key simply does nothing.
#[test]
fn every_key_the_editor_can_name_translates() {
    for (from, to) in [
        (CrossKeyCode::Esc, KeyCode::Esc),
        (CrossKeyCode::Enter, KeyCode::Enter),
        (CrossKeyCode::Tab, KeyCode::Tab),
        (CrossKeyCode::Backspace, KeyCode::Backspace),
        (CrossKeyCode::Left, KeyCode::Left),
        (CrossKeyCode::Right, KeyCode::Right),
        (CrossKeyCode::Up, KeyCode::Up),
        (CrossKeyCode::Down, KeyCode::Down),
        (CrossKeyCode::Char('x'), KeyCode::Char('x')),
    ] {
        assert_eq!(
            translate_key(terminal_key(from, CrossModifiers::NONE)).map(|e| e.code),
            Some(to),
            "{from:?} must reach the editor"
        );
    }
}

#[test]
fn modifiers_are_carried_across() {
    let event = translate_key(terminal_key(
        CrossKeyCode::Char('x'),
        CrossModifiers::CONTROL | CrossModifiers::ALT,
    ))
    .expect("a modified key still translates");

    assert!(event.modifiers.ctrl && event.modifiers.alt);
    assert_eq!(event.code, KeyCode::Char('x'));
}

/// The editor spells a shifted letter as the capital itself, so the shift
/// is spent here rather than carried -- otherwise `A` would never match a
/// binding written `A`.
#[test]
fn a_shifted_letter_arrives_as_the_capital() {
    let event = translate_key(terminal_key(CrossKeyCode::Char('a'), CrossModifiers::SHIFT))
        .expect("a shifted letter translates");

    assert_eq!(event.code, KeyCode::Char('A'));
    assert!(!event.modifiers.shift, "the shift was spent on the capital");
}

/// A terminal reports Shift-Tab as a key of its own. The editor has no such
/// key: it is Tab with Shift held, which is what `S-<tab>` binds.
#[test]
fn shift_tab_arrives_as_tab_with_shift_held() {
    for modifiers in [CrossModifiers::NONE, CrossModifiers::SHIFT] {
        let event = translate_key(terminal_key(CrossKeyCode::BackTab, modifiers))
            .expect("Shift-Tab translates");
        assert_eq!(event.code, KeyCode::Tab);
        assert!(
            event.modifiers.shift,
            "whether or not the terminal also said so"
        );
    }
}

/// A key with no name in the editor is refused, not guessed at. The caller
/// writes it down; what it must never be is silently turned into some
/// other key.
#[test]
fn a_key_the_editor_cannot_name_is_refused() {
    assert!(translate_key(terminal_key(CrossKeyCode::F(5), CrossModifiers::NONE)).is_none());
}

/// The whole path: a terminal Escape closes an open prompt. The core's own
/// test enters at `handle_key_event`, one step after the translation.
#[test]
fn a_terminal_escape_closes_the_minibuffer() {
    let (state, env) = editor();
    env.set_variable("frame-width".into(), ELispExp::number(COLS as f64));
    env.set_variable("frame-height".into(), ELispExp::number(ROWS as f64));
    let ast = rsedit_core::Parser::new(r#"(minibuffer-read "P:" nil nil nil)"#)
        .next()
        .expect("source must parse");
    rsedit_core::eval(&ast, env.clone(), &state).expect("prompt must open");
    assert!(
        frame(&state, &env).contains("P:"),
        "the prompt should be on screen to begin with"
    );

    let escape = translate_key(terminal_key(CrossKeyCode::Esc, CrossModifiers::NONE))
        .expect("Escape must translate");
    state.handle_key_event(escape, &env);

    // Asserted on the frame, which is where a prompt is up or not.
    assert!(
        !frame(&state, &env).contains("P:"),
        "Escape should have cancelled the prompt and taken it off the screen"
    );
}

// -----------------------------------------------------------------
// The rule between windows
// -----------------------------------------------------------------

/// The character reaches the screen, in every row of its column.
#[test]
fn the_rule_between_windows_is_drawn_down_its_whole_column() {
    let (state, env) = editor();
    env.set_variable("frame-width".into(), ELispExp::number(COLS as f64));
    env.set_variable("frame-height".into(), ELispExp::number(ROWS as f64));
    let ast = rsedit_core::Parser::new("(split-window-right)")
        .next()
        .expect("source must parse");
    rsedit_core::eval(&ast, env.clone(), &state).expect("split must work");

    let snapshot = state.snapshot(&env, COLS as usize, ROWS as usize);
    let rule = snapshot.separators.first().expect("a rule must be there");
    let (column, height) = (rule.rect.x as u16, rule.rect.height);

    let rendered = frame(&state, &env);
    let drawn = placements(&rendered)
        .into_iter()
        .filter(|(x, _)| *x == column)
        .count();
    assert!(
        drawn >= height,
        "the rule's column should be written to on each of its {height} rows, got {drawn}"
    );
    assert!(
        rendered.contains('\u{2502}'),
        "and the character itself should reach the terminal"
    );
}

/// Drawn with its own face, so `(set-face "window-separator" ...)` shows.
#[test]
fn the_rule_is_drawn_with_the_window_separator_face() {
    let mut theme = Theme::default();
    theme.set(Face::WINDOW_SEPARATOR, Style::fg(Color::BLUE));

    let snapshot = FrameSnapshot {
        separators: vec![rsedit_core::Separator {
            rect: Rect {
                x: 4,
                y: 0,
                width: 1,
                height: 2,
            },
            ch: '\u{2502}',
            face: Face::WINDOW_SEPARATOR,
        }],
        theme: Arc::new(theme),
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("rendering must succeed");
    let rendered = String::from_utf8(out).expect("crossterm emits valid UTF-8");

    let run = styled_run(&rendered, "\u{1b}[38;2;0;0;170m");
    assert!(
        run.contains('\u{2502}'),
        "the rule must be drawn inside its face, got {run:?}"
    );
}

/// The border of a titled window shows its title, not its buffer name: a
/// prompt shows its question, not `*Minibuffer*`.
#[test]
fn a_titled_window_is_labelled_with_its_title() {
    let (state, env) = editor();
    env.set_variable("frame-width".into(), ELispExp::number(COLS as f64));
    env.set_variable("frame-height".into(), ELispExp::number(ROWS as f64));
    let ast = rsedit_core::Parser::new(r#"(minibuffer-read "Find file:" nil nil nil)"#)
        .next()
        .expect("source must parse");
    rsedit_core::eval(&ast, env.clone(), &state).expect("prompt must open");

    let rendered = frame(&state, &env);
    assert!(
        rendered.contains("Find file:"),
        "the prompt should be drawn on the minibuffer's border"
    );
    assert!(
        !rendered.contains("*Minibuffer*"),
        "the buffer name should not be used as the label when a title exists"
    );
}

/// The cursor is placed last: the echo area, drawn after the windows with
/// a `MoveTo` of its own, does not leave it at the end of its message.
#[test]
fn the_cursor_is_placed_after_the_echo_area_not_before_it() {
    let (state, env) = editor();
    state.set_echo_message("a message long enough to move the cursor");

    let rendered = frame(&state, &env);
    let last = *placements(&rendered)
        .last()
        .expect("the frame must position the cursor at least once");

    assert_eq!(
        last,
        expected_cursor(&state, &env),
        "the last cursor placement should be point, not wherever drawing ended"
    );
    assert_ne!(last.1, ROWS - 1, "the cursor was left on the echo-area row");
}

/// The cursor is hidden while the frame is drawn and shown again once it is
/// in the right place, so it is never seen skittering across the screen.
#[test]
fn the_cursor_is_hidden_while_drawing_and_shown_at_the_end() {
    let (state, env) = editor();
    state.set_echo_message("hello");

    let rendered = frame(&state, &env);
    let hide = rendered
        .find("\u{1b}[?25l")
        .expect("the cursor must be hidden first");
    let show = rendered
        .rfind("\u{1b}[?25h")
        .expect("the cursor must be shown again");
    assert!(hide < show, "Hide must come before Show");

    let tail = &rendered[show..];
    assert!(
        !tail.contains("hello"),
        "nothing may be drawn after the cursor is placed and shown"
    );
}

/// Returns a one-window frame showing LINES, with HIGHLIGHTS over them.
/// Built by hand: what is under test is the renderer's escape sequences,
/// and the region machinery has its own tests, in the core.
fn frame_with(lines: &[&str], highlights: Vec<Highlight>) -> String {
    frame_themed(lines, highlights, Theme::default())
}

/// Returns the same, with a theme of the caller's choosing.
fn frame_themed(lines: &[&str], highlights: Vec<Highlight>, theme: Theme) -> String {
    frame_at_depth(lines, highlights, theme, ColorDepth::TrueColor)
}

/// Returns the same again, for a terminal of DEPTH: stated rather than
/// detected, so that no test depends on the terminal it runs in.
fn frame_at_depth(
    lines: &[&str],
    highlights: Vec<Highlight>,
    theme: Theme,
    depth: ColorDepth,
) -> String {
    let snapshot = FrameSnapshot {
        views: vec![rsedit_core::RenderableWindowView {
            rect: Rect {
                x: 0,
                y: 0,
                width: COLS as usize,
                height: lines.len(),
            },
            buffer_name: "*scratch*".into(),
            title: None,
            is_focused: true,
            cursor_rel_pos: Some((0, 0)),
            lines: lines.iter().map(|l| l.to_string()).collect(),
            highlights,
            mode_line: None,
            has_border: false,
            ..Default::default()
        }],
        theme: Arc::new(theme),
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, depth).expect("rendering must succeed");
    String::from_utf8(out).expect("crossterm emits valid UTF-8")
}

/// Returns what lies between START and the next full reset.
fn styled_run(rendered: &str, start_seq: &str) -> String {
    let start = rendered
        .find(start_seq)
        .unwrap_or_else(|| panic!("{start_seq:?} must be emitted"));
    let rest = &rendered[start..];
    let end = rest
        .find("\u{1b}[0m")
        .expect("the style must be turned off again");
    rest[..end].to_string()
}

/// Returns what lies between reverse video turned on and turned off.
fn reversed(rendered: &str) -> String {
    let start = rendered
        .find("\u{1b}[7m")
        .expect("something must be drawn in reverse video");
    let rest = &rendered[start..];
    let end = rest
        .find("\u{1b}[0m")
        .expect("the attribute must be turned off again");
    rest[..end].to_string()
}

#[test]
fn a_highlight_is_drawn_in_reverse_video_over_exactly_its_own_columns() {
    let rendered = frame_with(
        &["alpha beta"],
        vec![Highlight {
            row: 0,
            start_col: 0,
            end_col: 5,
            face: Face::REGION,
        }],
    );

    let run = reversed(&rendered);
    assert!(
        run.contains("alpha"),
        "the highlighted columns should be redrawn reversed: {run:?}"
    );
    assert!(
        !run.contains("beta"),
        "and the rest of the line should not be: {run:?}"
    );
}

/// A selection running past the end of a short line still has to look
/// selected out to where it ends, or a multi-line region appears to have
/// ragged holes in it.
#[test]
fn a_highlight_past_the_end_of_a_line_is_padded_with_spaces() {
    let rendered = frame_with(
        &["ab"],
        vec![Highlight {
            row: 0,
            start_col: 0,
            end_col: 5,
            face: Face::REGION,
        }],
    );

    let run = reversed(&rendered);
    assert!(
        run.contains("ab   "),
        "the highlight should extend past the text: {run:?}"
    );
}

/// The status line is drawn on the row below the window's text, styled and
/// padded across the window's width so it reads as a bar.
#[test]
fn a_mode_line_is_drawn_below_the_text_as_a_bar() {
    let snapshot = FrameSnapshot {
        views: vec![rsedit_core::RenderableWindowView {
            rect: Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 1,
            },
            buffer_name: "*scratch*".into(),
            title: None,
            is_focused: true,
            cursor_rel_pos: Some((0, 0)),
            lines: vec!["alpha".into()],
            highlights: Vec::new(),
            mode_line: Some("status".into()),
            has_border: false,
            ..Default::default()
        }],
        theme: Arc::new(Theme::default()),
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("render");
    let rendered = String::from_utf8(out).expect("valid UTF-8");

    let run = styled_run(&rendered, "\u{1b}[7m");
    assert!(run.contains("status"), "the status text: {run:?}");
    assert!(
        run.contains("status              "),
        "padded across the window so it reads as a bar: {run:?}"
    );
}

/// An unfocused window's status line is drawn with the other face.
#[test]
fn an_unfocused_window_uses_the_inactive_mode_line_face() {
    fn window(name: &str, y: isize, focused: bool) -> rsedit_core::RenderableWindowView {
        rsedit_core::RenderableWindowView {
            rect: Rect {
                x: 0,
                y,
                width: 20,
                height: 1,
            },
            buffer_name: name.into(),
            title: None,
            is_focused: focused,
            cursor_rel_pos: focused.then_some((0, 0)),
            lines: vec!["text".into()],
            highlights: Vec::new(),
            mode_line: Some(name.into()),
            has_border: false,
            ..Default::default()
        }
    }
    let snapshot = FrameSnapshot {
        views: vec![window("here", 0, true), window("there", 2, false)],
        theme: Arc::new(Theme::default()),
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("render");
    let rendered = String::from_utf8(out).expect("valid UTF-8");

    // Each styled run ends at a reset, so the chunk holding a status
    // line's text is the run that drew it.
    let run_with = |needle: &str| {
        rendered
            .split("\u{1b}[0m")
            .find(|chunk| chunk.contains(needle) && chunk.contains("\u{1b}[7m"))
            .unwrap_or_else(|| panic!("{needle} was never drawn styled"))
            .to_string()
    };
    // Any foreground escape, in whichever form this depth resolves to --
    // the point is that one face asks for a colour and the other does not.
    assert!(
        !run_with("here").contains("\u{1b}[38;"),
        "the focused status line asks for no colour: {:?}",
        run_with("here")
    );
    assert!(
        run_with("there").contains("\u{1b}[38;"),
        "the unfocused one is dimmed: {:?}",
        run_with("there")
    );
}

/// Nothing selected, nothing styled -- the ordinary case must not start
/// emitting attribute changes.
#[test]
fn a_frame_with_no_highlights_emits_no_attributes() {
    let rendered = frame_with(&["alpha beta"], Vec::new());
    assert!(
        !rendered.contains("\u{1b}[7m"),
        "an unhighlighted frame should draw exactly as it did before"
    );
}

/// The renderer asks the theme, so restyling a face changes what is drawn
/// without the region code or this function knowing anything about it.
#[test]
fn a_restyled_face_is_drawn_with_its_new_colours() {
    let mut theme = Theme::default();
    theme.set(
        Face::REGION,
        Style {
            fg: Some(Color::rgb(0xf8, 0xf8, 0xf2)),
            bg: Some(Color::BLUE),
            bold: true,
            ..Style::plain()
        },
    );

    let rendered = frame_themed(
        &["alpha beta"],
        vec![Highlight {
            row: 0,
            start_col: 0,
            end_col: 5,
            face: Face::REGION,
        }],
        theme,
    );

    assert!(
        rendered.contains("\u{1b}[38;2;248;248;242m"),
        "a 24-bit foreground should be emitted"
    );
    assert!(
        rendered.contains("\u{1b}[48;2;0;0;170m"),
        "and the background, drawn as asked because this terminal can"
    );
    assert!(rendered.contains("\u{1b}[1m"), "and bold");
    assert!(
        !rendered.contains("\u{1b}[7m"),
        "reverse video was replaced, not added to"
    );

    // The run has to *end*, so nothing drawn afterwards inherits its colours.
    let run = styled_run(&rendered, "\u{1b}[38;2;248;248;242m");
    assert!(
        run.contains("alpha"),
        "the styled run should hold the highlighted text: {run:?}"
    );
    assert!(
        !run.contains("beta"),
        "and must be closed before the rest of the line: {run:?}"
    );
}

/// A face bound to nothing is drawn exactly as it already looks, so the
/// renderer emits nothing at all -- no stray attribute reset either.
#[test]
fn a_face_bound_to_nothing_is_not_redrawn() {
    let rendered = frame_themed(
        &["alpha beta"],
        vec![Highlight {
            row: 0,
            start_col: 0,
            end_col: 5,
            face: Face::DEFAULT,
        }],
        Theme::default(),
    );
    assert!(!rendered.contains("\u{1b}[7m"));
    assert!(
        !rendered.contains("\u{1b}[0m"),
        "nothing was styled, so nothing needs resetting"
    );
}

// ---------------- resolving colour requests ----------------

/// A conventional colour, on a terminal that approximates, is the palette
/// slot of that name: "blue" reaches the person's own blue.
#[test]
fn a_conventional_colour_approximates_to_the_slot_of_its_own_name() {
    for (slot, (name, color)) in NAMED_COLORS.iter().enumerate() {
        assert_eq!(
            resolve(*color, ColorDepth::Ansi16),
            CrossColor::AnsiValue(slot as u8),
            "{name} should land on its own palette slot"
        );
    }
}

/// And they stay distinguishable: two names collapsing to one slot would
/// make them the same colour on any terminal that approximates.
#[test]
fn every_conventional_colour_gets_its_own_slot() {
    let mut slots: Vec<CrossColor> = NAMED_COLORS
        .iter()
        .map(|(_, c)| resolve(*c, ColorDepth::Ansi16))
        .collect();
    slots.dedup();
    assert_eq!(slots.len(), NAMED_COLORS.len());
}

/// An exact colour is shown exactly where that is possible, and
/// approximated where it is not -- and the approximating is the renderer's
/// decision, made here rather than asked of the editor.
#[test]
fn an_exact_colour_is_used_as_is_only_where_it_can_be() {
    let orange = Color::rgb(0xff, 0x88, 0x00);
    assert_eq!(
        resolve(orange, ColorDepth::TrueColor),
        CrossColor::Rgb {
            r: 0xff,
            g: 0x88,
            b: 0x00
        }
    );
    assert!(
        matches!(
            resolve(orange, ColorDepth::Ansi256),
            CrossColor::AnsiValue(n) if (16..=231).contains(&n)
        ),
        "a colour should land in the 256-colour cube, not be sent as 24-bit"
    );
    assert!(
        matches!(resolve(orange, ColorDepth::Ansi16), CrossColor::AnsiValue(n) if n < 16),
        "and on a sixteen-colour terminal, in the palette"
    );
}

#[test]
fn an_approximated_colour_lands_on_something_close() {
    // Pure red is in the palette exactly, so nothing else should win.
    assert_eq!(
        resolve(Color::rgb(0xff, 0x55, 0x55), ColorDepth::Ansi16),
        CrossColor::AnsiValue(9),
        "bright red should resolve to the bright red slot"
    );
    assert_eq!(
        resolve(Color::rgb(0, 0, 0), ColorDepth::Ansi16),
        CrossColor::AnsiValue(0)
    );
    // Mid grey belongs on the 256-colour grey ramp, not in the colour cube
    // -- the cube's nearest step is visibly off, and grey is the case where
    // that shows most.
    assert!(
        matches!(
            resolve(Color::rgb(0x80, 0x80, 0x80), ColorDepth::Ansi256),
            CrossColor::AnsiValue(n) if (232..=255).contains(&n)
        ),
        "a grey should use the grey ramp"
    );
}

/// The same face, drawn on two terminals, produces different escape
/// sequences from the same editor state: the editor says "#3a5fcd", and
/// each renderer answers it.
#[test]
fn the_same_face_renders_differently_on_different_terminals() {
    let mut theme = Theme::default();
    theme.set(
        Face::REGION,
        Style {
            bg: Some(Color::rgb(0x3a, 0x5f, 0xcd)),
            ..Style::plain()
        },
    );
    let highlight = vec![Highlight {
        row: 0,
        start_col: 0,
        end_col: 5,
        face: Face::REGION,
    }];

    let truecolor = frame_at_depth(
        &["alpha beta"],
        highlight.clone(),
        theme.clone(),
        ColorDepth::TrueColor,
    );
    let sixteen = frame_at_depth(&["alpha beta"], highlight, theme, ColorDepth::Ansi16);

    assert!(truecolor.contains("\u{1b}[48;2;58;95;205m"));
    assert!(
        !sixteen.contains("\u{1b}[48;2;"),
        "a sixteen-colour terminal must not be sent a 24-bit colour"
    );
    assert!(
        sixteen.contains("\u{1b}[48;5;"),
        "it should get a palette slot instead"
    );
}

/// Guessing low is the safe direction: a truecolor terminal sent a palette
/// index shows an approximate colour, while a sixteen-colour terminal sent
/// a 24-bit escape shows nothing useful at all.
#[test]
fn colour_depth_is_read_from_the_environment_and_guesses_low() {
    for (colorterm, term, expected) in [
        (Some("truecolor"), Some("xterm-256color"), TrueColor),
        (Some("24bit"), Some("dumb"), TrueColor),
        (None, Some("xterm-256color"), Ansi256),
        (Some(""), Some("screen-256color"), Ansi256),
        (None, Some("xterm"), Ansi16),
        (None, None, Ansi16),
        (Some("nonsense"), Some("vt100"), Ansi16),
    ] {
        assert_eq!(
            ColorDepth::from_env(colorterm, term),
            expected,
            "COLORTERM={colorterm:?} TERM={term:?}"
        );
    }
}

/// A highlight naming a row the view does not have is ignored rather than
/// panicking: rows and spans are computed together, but a renderer that
/// trusts an index it did not check is one resize away from a crash.
#[test]
fn a_highlight_outside_the_drawn_rows_is_ignored() {
    let rendered = frame_with(
        &["only one row"],
        vec![Highlight {
            row: 7,
            start_col: 0,
            end_col: 3,
            face: Face::REGION,
        }],
    );
    assert!(!rendered.contains("\u{1b}[7m"));
}

/// An empty echo area must not change where the cursor lands.
#[test]
fn the_cursor_lands_at_point_with_no_echo_message() {
    let (state, env) = editor();
    state.set_echo_message("");

    let rendered = frame(&state, &env);
    let last = *placements(&rendered).last().expect("a cursor placement");
    assert_eq!(last, expected_cursor(&state, &env));
}

/// Returns every `MoveTo` with the text printed straight after it: what
/// the renderer put where.
fn printed(rendered: &str) -> Vec<(u16, u16, String)> {
    let mut out = Vec::new();
    for chunk in rendered.split('\u{1b}').skip(1) {
        let Some(body) = chunk.strip_prefix('[') else {
            continue;
        };
        let Some(end) = body.find('H') else { continue };
        let coords = &body[..end];
        let Some((row, col)) = coords.split_once(';') else {
            continue;
        };
        let (Ok(row), Ok(col)) = (row.parse::<u16>(), col.parse::<u16>()) else {
            continue;
        };
        out.push((
            col.saturating_sub(1),
            row.saturating_sub(1),
            body[end + 1..].to_string(),
        ));
    }
    out
}

/// Returns a tiled window covering the frame, with a small floating one
/// over it.
fn frame_with_float() -> String {
    let under: Vec<String> = (0..6)
        .map(|n| format!("UNDERNEATH-{n} and more text past the float"))
        .collect();
    let snapshot = FrameSnapshot {
        views: vec![
            rsedit_core::RenderableWindowView {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: COLS as usize,
                    height: under.len(),
                },
                buffer_name: "*under*".into(),
                title: None,
                is_focused: false,
                cursor_rel_pos: None,
                lines: under,
                mode_line: None,
                highlights: vec![],
                has_border: false,
                ..Default::default()
            },
            rsedit_core::RenderableWindowView {
                rect: Rect {
                    x: 10,
                    y: 1,
                    width: 20,
                    height: 3,
                },
                buffer_name: "*float*".into(),
                title: Some("prompt".into()),
                is_focused: true,
                cursor_rel_pos: Some((0, 0)),
                // Two lines for three rows, one of them wider than the window:
                // between them they cover every way a view can fail to own its
                // rectangle -- too few rows, a row too short, and a row too
                // long spilling into the window beside it.
                lines: vec![
                    "hi".into(),
                    "a line much longer than the float is wide".into(),
                ],
                mode_line: None,
                highlights: vec![],
                has_border: true,
                ..Default::default()
            },
        ],
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out = Vec::new();
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("render");
    String::from_utf8(out).expect("utf8")
}

/// A floating window is opaque: each of its rows is drawn across its whole
/// width, however few and short the lines of its buffer.
#[test]
fn a_floating_window_paints_every_cell_of_its_rect() {
    let rendered = frame_with_float();
    let drawn = printed(&rendered);

    for row in 1..4u16 {
        let painted = drawn
            .iter()
            .filter(|(x, y, _)| *x == 10 && *y == row)
            .map(|(_, _, text)| text.chars().count())
            .max();
        assert_eq!(
            painted,
            Some(20),
            "row {row} of the float should be painted across its full width, \
             got {painted:?}"
        );
    }
}

#[test]
fn a_float_does_not_show_the_buffer_underneath_it() {
    let rendered = frame_with_float();
    for (x, y, text) in printed(&rendered) {
        if x == 10 && (1..4).contains(&y) {
            assert!(
                !text.contains("UNDERNEATH"),
                "the float's own rows must not carry the buffer below: {text:?}"
            );
        }
    }
}

/// A line wider than its window belongs to that window, not to its neighbour.
#[test]
fn a_line_is_clipped_to_the_window_it_is_in() {
    let rendered = frame_with_float();
    let widest = printed(&rendered)
        .into_iter()
        .filter(|(x, y, _)| *x == 10 && (1..4).contains(y))
        .map(|(_, _, text)| text.chars().count())
        .max()
        .expect("the float drew something");
    assert_eq!(widest, 20, "never wider than the rect");
}

// ---------------------------------------------------------------------------
// Control characters in the text
// ---------------------------------------------------------------------------

/// A file holding an OSC 52 sequence would otherwise set the clipboard the
/// moment it is shown: the sequence is drawn as pictures, column for column.
#[test]
fn a_control_sequence_in_the_text_is_shown_not_obeyed() {
    let rendered = frame_with(&["a\u{1b}]52;c;aGk=\u{7}b"], vec![]);
    assert!(
        !rendered.contains("\u{1b}]"),
        "no OSC sequence may reach the terminal: {rendered:?}"
    );
    assert!(!rendered.contains('\u{7}'), "nor a BEL");
    assert!(rendered.contains("a\u{241b}]52;c;aGk=\u{2407}b"));
}

/// The same through the editor, where the text and the echo area come from
/// a buffer and a message rather than a frame built by hand.
#[test]
fn control_characters_in_a_buffer_and_a_message_are_shown_not_obeyed() {
    let (state, env) = editor();
    let ast = rsedit_core::Parser::new("(insert \"x\u{1b}]0;title\u{7}y\")")
        .next()
        .expect("source must parse");
    rsedit_core::eval(&ast, env.clone(), &state).expect("insert must work");
    state.set_echo_message("said\u{1b}[6n\r\u{8}");
    let rendered = frame(&state, &env);
    assert!(
        rendered.contains("x\u{241b}]0;title\u{2407}y"),
        "{rendered:?}"
    );
    assert!(!rendered.contains("\u{1b}]"), "{rendered:?}");
    assert!(!rendered.contains("\u{1b}[6n"), "{rendered:?}");
    for c in ['\u{7}', '\r', '\u{8}'] {
        assert!(!rendered.contains(c), "{c:?} reached the terminal");
    }
}

/// Every control character but the Tab becomes something visible, and
/// nothing else changes.
#[test]
fn printable_replaces_exactly_the_control_characters() {
    for code in (0..0x20).chain(0x7f..0xa0) {
        let c = char::from_u32(code).unwrap();
        if c == '\t' {
            assert_eq!(printable(c), '\t');
        } else {
            assert!(!printable(c).is_control(), "{c:?} must be replaced");
        }
    }
    assert_eq!(printable('\u{1b}'), '\u{241b}');
    assert_eq!(printable('\u{7f}'), '\u{2421}');
    for c in ['a', ' ', '\u{a0}', 'è', '─'] {
        assert_eq!(printable(c), c);
    }
}

// ---------------------------------------------------------------------------
// The system clipboard
// ---------------------------------------------------------------------------

/// Base64 against the worked examples in RFC 4648, which exist precisely
/// because the padding cases are where hand-written encoders go wrong.
#[test]
fn base64_matches_the_standard_at_every_padding() {
    // Length 3n: no padding. 3n+1: two `=`. 3n+2: one `=`.
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
}

/// The last two alphabet entries are `+` and `/`, and an encoder that built
/// its table by hand is exactly where those go missing.
#[test]
fn base64_reaches_the_end_of_its_alphabet() {
    assert_eq!(base64_encode(&[0xfb, 0xff, 0xfe]), "+//+");
}

#[test]
fn base64_encodes_bytes_rather_than_characters() {
    // Killed text is arbitrary UTF-8, and a multi-byte character has to
    // survive the round trip as its bytes rather than as a char count.
    assert_eq!(base64_encode("é".as_bytes()), "w6k=");
    assert_eq!(base64_encode("日本".as_bytes()), "5pel5pys");
}

#[test]
fn the_clipboard_escape_is_an_osc_52_write() {
    // `ESC ] 52 ; c ; <payload> ESC \`. The `c` is the clipboard selection,
    // and the terminator is ST rather than BEL because tmux passes ST through
    // without special-casing it.
    assert_eq!(osc52_copy("foo"), "\u{1b}]52;c;Zm9v\u{1b}\\");
}

#[test]
fn a_frame_carrying_clipboard_text_emits_the_escape() {
    let snapshot = FrameSnapshot {
        width: COLS as usize,
        height: ROWS as usize,
        clipboard: Some("hello".to_string()),
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, TrueColor).expect("rendering must succeed");
    let rendered = String::from_utf8(out).expect("crossterm emits valid UTF-8");
    assert!(
        rendered.contains("\u{1b}]52;c;aGVsbG8=\u{1b}\\"),
        "the frame's clipboard payload should reach the terminal, got {rendered:?}"
    );
}

#[test]
fn a_frame_with_no_clipboard_text_emits_no_escape() {
    // The common case by far: this must cost nothing on the frames that carry
    // nothing, and above all must not clear the clipboard by sending an empty
    // payload on every redraw.
    let snapshot = FrameSnapshot {
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, TrueColor).expect("rendering must succeed");
    let rendered = String::from_utf8(out).expect("crossterm emits valid UTF-8");
    assert!(!rendered.contains("]52;c;"));
}

// -----------------------------------------------------------------
// Getting the mouse from the terminal to the editor
// -----------------------------------------------------------------

use crate::tui::input::translate_mouse;
use crossterm::event::{
    MouseButton as CrossMouseButton, MouseEvent as CrossMouseEvent,
    MouseEventKind as CrossMouseKind,
};
use rsedit_core::{MouseButton, MouseKind};

fn terminal_mouse(kind: CrossMouseKind, column: u16, row: u16) -> CrossMouseEvent {
    CrossMouseEvent {
        kind,
        column,
        row,
        modifiers: CrossModifiers::NONE,
    }
}

#[test]
fn a_left_click_arrives_with_its_position() {
    let event = translate_mouse(terminal_mouse(
        CrossMouseKind::Down(CrossMouseButton::Left),
        7,
        3,
    ))
    .expect("a click must translate");
    assert_eq!(event.kind, MouseKind::Down(MouseButton::Left));
    assert_eq!((event.column, event.row), (7, 3));
}

#[test]
fn the_wheel_translates_both_ways() {
    assert_eq!(
        translate_mouse(terminal_mouse(CrossMouseKind::ScrollUp, 0, 0))
            .expect("must translate")
            .kind,
        MouseKind::ScrollUp
    );
    assert_eq!(
        translate_mouse(terminal_mouse(CrossMouseKind::ScrollDown, 0, 0))
            .expect("must translate")
            .kind,
        MouseKind::ScrollDown
    );
}

#[test]
fn a_bare_move_is_refused() {
    // The editor has no word for "the pointer went past". Dropping it here
    // rather than carrying it inwards keeps the vocabulary to things something
    // acts on -- and a terminal reporting every motion would otherwise wake
    // the editor for each one.
    assert!(translate_mouse(terminal_mouse(CrossMouseKind::Moved, 1, 1)).is_none());
}

#[test]
fn the_other_buttons_and_drags_survive_translation() {
    // Nothing is bound to them yet; they translate anyway, as the type is the
    // contract with the front end.
    for (from, to) in [
        (
            CrossMouseKind::Up(CrossMouseButton::Left),
            MouseKind::Up(MouseButton::Left),
        ),
        (
            CrossMouseKind::Drag(CrossMouseButton::Left),
            MouseKind::Drag(MouseButton::Left),
        ),
        (
            CrossMouseKind::Down(CrossMouseButton::Right),
            MouseKind::Down(MouseButton::Right),
        ),
        (
            CrossMouseKind::Down(CrossMouseButton::Middle),
            MouseKind::Down(MouseButton::Middle),
        ),
    ] {
        assert_eq!(
            translate_mouse(terminal_mouse(from, 0, 0))
                .expect("must translate")
                .kind,
            to
        );
    }
}

#[test]
fn modifiers_are_carried_across_a_click() {
    let mut event = terminal_mouse(CrossMouseKind::Down(CrossMouseButton::Left), 0, 0);
    event.modifiers = CrossModifiers::CONTROL | CrossModifiers::SHIFT;
    let translated = translate_mouse(event).expect("must translate");
    assert!(translated.modifiers.ctrl);
    assert!(translated.modifiers.shift);
    assert!(!translated.modifiers.alt);
}

/// A float covers whatever is under it, a rule included. Asserted by order
/// in the byte stream: both writes go to the same cell, and the terminal
/// shows the last.
#[test]
fn a_floating_window_is_drawn_over_the_rule_it_covers() {
    let snapshot = FrameSnapshot {
        separators: vec![rsedit_core::Separator {
            rect: Rect {
                x: 4,
                y: 0,
                width: 1,
                height: 3,
            },
            // Not the usual box character: the float's own border is drawn
            // with those, and the test could not tell them apart.
            ch: '#',
            face: Face::WINDOW_SEPARATOR,
        }],
        views: vec![rsedit_core::RenderableWindowView {
            rect: Rect {
                x: 2,
                y: 0,
                width: 10,
                height: 3,
            },
            buffer_name: "*float*".into(),
            title: Some("PROMPT".into()),
            is_focused: true,
            cursor_rel_pos: None,
            lines: vec!["inside".into()],
            mode_line: None,
            highlights: vec![],
            has_border: true,
            ..Default::default()
        }],
        theme: Arc::new(Theme::default()),
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("render");
    let rendered = String::from_utf8(out).expect("crossterm emits valid UTF-8");

    let rule = rendered.find('#').expect("the rule must be drawn");
    let float = rendered
        .find("inside")
        .expect("the float's text must be drawn");
    assert!(
        rule < float,
        "the rule must be drawn before the float that covers it, so the float \
         wins the cell; rule at {rule}, float at {float}"
    );
}

// ---------------------------------------------------------------------
// The line-number gutter
// ---------------------------------------------------------------------

/// Returns a window with a four-column gutter, whose text therefore starts
/// at column four of the frame. Built by hand: what is under test is the
/// renderer's half, and composition has its own tests.
fn frame_with_gutter(mode_line: Option<&str>) -> String {
    use rsedit_core::{Face, GutterCell};
    let cell = |text: &str, face| GutterCell {
        text: text.into(),
        face,
    };
    let snapshot = FrameSnapshot {
        views: vec![rsedit_core::RenderableWindowView {
            rect: Rect {
                x: 4,
                y: 0,
                width: 16,
                height: 2,
            },
            buffer_name: "*scratch*".into(),
            title: None,
            is_focused: true,
            cursor_rel_pos: Some((0, 0)),
            lines: vec!["alpha".into(), "beta".into()],
            highlights: Vec::new(),
            mode_line: mode_line.map(str::to_string),
            has_border: false,
            gutter: vec![
                cell("  1 ", Face::LINE_NUMBER_CURRENT),
                cell("  2 ", Face::LINE_NUMBER),
            ],
            gutter_width: 4,
        }],
        theme: Arc::new(Theme::default()),
        width: COLS as usize,
        height: ROWS as usize,
        ..Default::default()
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("render");
    String::from_utf8(out).expect("crossterm emits valid UTF-8")
}

#[test]
fn the_gutter_is_drawn_in_the_columns_left_of_the_text() {
    let rendered = frame_with_gutter(None);
    let printed = printed(&rendered);
    assert!(
        printed.contains(&(0, 0, "  1 ".into())),
        "the first number at column zero: {printed:?}"
    );
    assert!(
        printed.contains(&(0, 1, "  2 ".into())),
        "the second on the row below it: {printed:?}"
    );
}

#[test]
fn the_text_is_not_shifted_over_by_its_own_gutter() {
    // The failure this catches is a renderer that draws the rows at the
    // window's left edge and then the numbers on top of them, which loses the
    // first four characters of every line and puts the cursor four columns
    // out.
    let rendered = frame_with_gutter(None);
    let printed = printed(&rendered);
    let row = printed
        .iter()
        .find(|(x, y, text)| *x == 4 && *y == 0 && text.starts_with("alpha"))
        .unwrap_or_else(|| panic!("the text must be drawn at column four: {printed:?}"));
    assert!(
        row.2.chars().count() >= 16,
        "padded across the sixteen columns it was given: {:?}",
        row.2
    );
}

#[test]
fn the_current_line_number_is_drawn_with_its_own_face() {
    // Composition decides which line that is; the renderer's job is only to
    // use the face it was handed, per cell rather than once for the whole
    // gutter. `line-number-current' is bold in the default theme, which is
    // what makes the run findable.
    let rendered = frame_with_gutter(None);
    let bold = styled_run(&rendered, "\u{1b}[1m");
    assert!(
        bold.contains("  1 "),
        "the line point is on is the bold one: {bold:?}"
    );
    assert!(!bold.contains("  2 "), "and the next line is not: {bold:?}");
}

#[test]
fn the_mode_line_spans_the_gutter_as_well_as_the_text() {
    // A bar with a notch cut out of its left end is not a bar.
    let rendered = frame_with_gutter(Some("status"));
    let printed = printed(&rendered);
    let bar = printed
        .iter()
        .find(|(x, y, text)| *x == 0 && *y == 2 && text.starts_with("status"))
        .unwrap_or_else(|| panic!("the status line must start at column zero: {printed:?}"));
    assert_eq!(
        bar.2.chars().count(),
        20,
        "padded across the gutter and the text together: {:?}",
        bar.2
    );
}

#[test]
fn a_window_with_no_gutter_starts_its_text_at_the_left_edge() {
    let rendered = frame_themed(&["alpha"], Vec::new(), Theme::default());
    let printed = printed(&rendered);
    assert!(
        printed
            .iter()
            .any(|(x, y, text)| *x == 0 && *y == 0 && text.starts_with("alpha")),
        "the text still starts at the frame's left edge: {printed:?}"
    );
}
