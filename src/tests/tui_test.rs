use crossterm::event::{
    Event, KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers, poll, read,
};
use crossterm::{
    QueueableCommand, cursor, execute,
    style::{
        Attribute, Color as CrossColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal,
};
use rsedit_core::BufferTrait;
use rsedit_core::ELispExp;
use rsedit_core::EditorState;
use rsedit_core::input::{KeyCode, KeyEvent, KeyModifiers};
use rsedit_core::lisp::{Env, LispContext};
use rsedit_core::ui::{Color, Face, FrameSnapshot, Highlight, NAMED_COLORS, Rect, Style, Theme};
use std::{
    io::{Write, stdout},
    sync::Arc,
};
use rsedit_core::buffer::gap_buffer::GapBuffer;
use rsedit_core::create_global_env;
use crate::tests::tui_test::ColorDepth::TrueColor;

use crate::tui::ColorDepth;
use crate::tests::tui_test::ColorDepth::Ansi16;
use crate::tests::tui_test::ColorDepth::Ansi256;
use crate::tui::translate_key;
use crate::tui::resolve;
use crate::tui::render_to;

const COLS: u16 = 80;
const ROWS: u16 = 24;

/// Render one frame into memory and return the bytes.
fn frame(state: &EditorState<GapBuffer>, env: &Arc<Env<EditorState<GapBuffer>>>) -> String {
    let mut out: Vec<u8> = Vec::new();
    let snapshot = state.snapshot(env, COLS as usize, ROWS as usize);
    render_to(&mut out, &snapshot, ColorDepth::TrueColor).expect("rendering must succeed");
    String::from_utf8(out).expect("crossterm emits valid UTF-8")
}

/// Every cursor placement in the stream, as (col, row), in order.
///
/// `MoveTo(x, y)` is written as `ESC [ y+1 ; x+1 H`, so this reads the
/// output the way a terminal would rather than trusting a hard-coded
/// escape string.
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

/// One key as crossterm would report it.
fn terminal_key(code: CrossKeyCode, modifiers: CrossModifiers) -> CrossKeyEvent {
    CrossKeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// The bug this file is being fixed for.
///
/// `minibuffer-mode` has bound Escape to `minibuffer-cancel` since the
/// minibuffer was written, and Escape did nothing: the translation below
/// had no arm for it, so the key was dropped here and never reached a
/// keymap at all.
#[test]
fn escape_reaches_the_editor() {
    assert_eq!(
        translate_key(terminal_key(CrossKeyCode::Esc, CrossModifiers::NONE))
        .expect("Escape must translate")
        .code,
        KeyCode::Esc
    );
}

/// Every key the editor can name has to survive the trip. A missing arm
/// here is invisible from the outside -- the key simply does nothing --
/// which is exactly how the one above went unnoticed.
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

/// A key with no name in the editor is refused, not guessed at. The caller
/// writes it down; what it must never be is silently turned into some
/// other key.
#[test]
fn a_key_the_editor_cannot_name_is_refused() {
    assert!(translate_key(terminal_key(CrossKeyCode::F(5), CrossModifiers::NONE)).is_none());
}

/// And the whole path, joined up: a terminal Escape closes an open prompt.
///
/// The core already had a test that Escape cancels the minibuffer, and it
/// passed throughout -- it called `handle_key_event` directly, entering one
/// step below the step that was broken. This one starts where the terminal
/// does.
#[test]
fn a_terminal_escape_closes_the_minibuffer() {
    let (state, env) = create_global_env::<GapBuffer>().expect("global env");
    env.set_variable("frame-width".into(), ELispExp::number(COLS as f64));
    env.set_variable("frame-height".into(), ELispExp::number(ROWS as f64));
    let ast = rsedit_core::lisp::Parser::new(r#"(minibuffer-read "P:" nil nil nil)"#)
        .next()
        .expect("source must parse");
    rsedit_core::lisp::eval(&ast, env.clone(), &state).expect("prompt must open");
    assert!(
        frame(&state, &env).contains("P:"),
        "the prompt should be on screen to begin with"
    );

    let escape = translate_key(terminal_key(CrossKeyCode::Esc, CrossModifiers::NONE))
        .expect("Escape must translate");
    state.handle_key_event(escape, &env);

    // Asserted on the frame rather than on editor state: what was broken
    // was that the prompt stayed up, and the frame is where "up" is
    // decided.
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
    let (state, env) = create_global_env::<GapBuffer>().expect("global env");
    env.set_variable("frame-width".into(), ELispExp::number(COLS as f64));
    env.set_variable("frame-height".into(), ELispExp::number(ROWS as f64));
    let ast = rsedit_core::lisp::Parser::new("(split-window-right)")
        .next()
        .expect("source must parse");
    rsedit_core::lisp::eval(&ast, env.clone(), &state).expect("split must work");

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
    theme.set(Face::WindowSeparator, Style::fg(Color::BLUE));

    let snapshot = FrameSnapshot {
        views: Vec::new(),
        separators: vec![rsedit_core::ui::Separator {
            rect: Rect {
                x: 4,
                y: 0,
                width: 1,
                height: 2,
            },
            ch: '\u{2502}',
            face: Face::WindowSeparator,
        }],
        echo_message: String::new(),
        pending_input: String::new(),
        theme,
        focused_window_id: 0,
        width: COLS as usize,
        height: ROWS as usize,
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

/// The border of a titled window shows its title, not its buffer name.
///
/// This was dead data before: `minibuffer-read` computed a prompt,
/// `open_floating_window` stored it, and the renderer drew
/// `Some(view.buffer_name)` instead -- so every prompt was labelled
/// `*Minibuffer*` and the question being asked was never shown at all.
#[test]
fn a_titled_window_is_labelled_with_its_title() {
    let (state, env) = create_global_env::<GapBuffer>().expect("global env");
    env.set_variable("frame-width".into(), ELispExp::number(COLS as f64));
    env.set_variable("frame-height".into(), ELispExp::number(ROWS as f64));
    let ast = rsedit_core::lisp::Parser::new(r#"(minibuffer-read "Find file:" nil nil nil)"#)
        .next()
        .expect("source must parse");
    rsedit_core::lisp::eval(&ast, env.clone(), &state).expect("prompt must open");

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

/// The regression this guards.
///
/// The cursor used to be positioned inside the window loop, so the echo
/// area -- drawn afterwards, and like every other row emitting `MoveTo`
/// then `Print` -- left the terminal cursor at the end of its message
/// instead of at point.
#[test]
fn the_cursor_is_placed_after_the_echo_area_not_before_it() {
    let (state, env) = create_global_env::<GapBuffer>().expect("global env");
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
    let (state, env) = create_global_env::<GapBuffer>().expect("global env");
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

/// A one-window frame showing LINES, with HIGHLIGHTS over them.
///
/// Built by hand rather than captured from an editor: what is under test
/// here is the escape sequences the renderer emits for a highlight, and
/// driving that through a buffer, a mark and a snapshot would test the
/// region machinery instead -- which has its own tests, in the core.
fn frame_with(lines: &[&str], highlights: Vec<Highlight>) -> String {
    frame_themed(lines, highlights, Theme::default())
}

/// The same, with a theme of the caller's choosing.
fn frame_themed(lines: &[&str], highlights: Vec<Highlight>, theme: Theme) -> String {
    frame_at_depth(lines, highlights, theme, ColorDepth::TrueColor)
}

/// The same again, pretending the terminal can show only so much. Stated
/// rather than detected, so a test never depends on the environment it
/// happens to run in.
fn frame_at_depth(
    lines: &[&str],
    highlights: Vec<Highlight>,
    theme: Theme,
    depth: ColorDepth,
) -> String {
    let snapshot = FrameSnapshot {
        views: vec![rsedit_core::ui::RenderableWindowView {
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
        }],
        separators: Vec::new(),
        echo_message: String::new(),
        pending_input: String::new(),
        theme,
        focused_window_id: 0,
        width: COLS as usize,
        height: ROWS as usize,
    };
    let mut out: Vec<u8> = Vec::new();
    render_to(&mut out, &snapshot, depth).expect("rendering must succeed");
    String::from_utf8(out).expect("crossterm emits valid UTF-8")
}

/// What lies between START and the next full reset.
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

/// What lies between turning reverse video on and turning it off again.
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
            face: Face::Region,
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
            face: Face::Region,
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
        views: vec![rsedit_core::ui::RenderableWindowView {
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
        }],
        separators: Vec::new(),
        echo_message: String::new(),
        pending_input: String::new(),
        theme: Theme::default(),
        focused_window_id: 0,
        width: COLS as usize,
        height: ROWS as usize,
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

/// An unfocused window's status line is drawn with the other face. There
/// is only ever one tiled window until splits exist (#17), so this is
/// built by hand -- the alternative is a rule nothing checks until the
/// feature that needs it arrives.
#[test]
fn an_unfocused_window_uses_the_inactive_mode_line_face() {
    fn window(name: &str, y: isize, focused: bool) -> rsedit_core::ui::RenderableWindowView {
        rsedit_core::ui::RenderableWindowView {
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
        }
    }
    let snapshot = FrameSnapshot {
        views: vec![window("here", 0, true), window("there", 2, false)],
        separators: Vec::new(),
        echo_message: String::new(),
        pending_input: String::new(),
        theme: Theme::default(),
        focused_window_id: 0,
        width: COLS as usize,
        height: ROWS as usize,
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
        Face::Region,
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
            face: Face::Region,
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
            face: Face::Default,
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

/// The property that replaces "a name is a slot": ask for a conventional
/// colour on a terminal that has to approximate, and you get the palette
/// slot of that name -- which shows whatever the user configured it to.
/// Asking for "blue" still reaches *their* blue, without the editor ever
/// referring to a palette.
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
/// sequences from identical editor state. That is the whole point of the
/// split: the editor said "#3a5fcd" once, and each renderer answered it.
#[test]
fn the_same_face_renders_differently_on_different_terminals() {
    let mut theme = Theme::default();
    theme.set(
        Face::Region,
        Style {
            bg: Some(Color::rgb(0x3a, 0x5f, 0xcd)),
            ..Style::plain()
        },
    );
    let highlight = vec![Highlight {
        row: 0,
        start_col: 0,
        end_col: 5,
        face: Face::Region,
    }];

    let truecolor = frame_at_depth(
        &["alpha beta"],
        highlight.clone(),
        theme,
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
            face: Face::Region,
        }],
    );
    assert!(!rendered.contains("\u{1b}[7m"));
}

/// An empty echo area must not change where the cursor lands.
#[test]
fn the_cursor_lands_at_point_with_no_echo_message() {
    let (state, env) = create_global_env::<GapBuffer>().expect("global env");
    state.set_echo_message("");

    let rendered = frame(&state, &env);
    let last = *placements(&rendered).last().expect("a cursor placement");
    assert_eq!(last, expected_cursor(&state, &env));
}
