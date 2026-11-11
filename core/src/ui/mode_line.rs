//! The status line: a window's `mode-line-format`, its escapes expanded
//! against the buffer the window shows.
//!
//! Design: `doc/architettura-editor.typ, <riga-di-stato>`.
use crate::ELispExp;
use crate::buffer::{Buffer, BufferTrait};

/// Returns FORMAT with its escapes expanded against BUFFER, shown in a window
/// of HEIGHT rows of text. The escapes are Emacs's:
///
/// | escape | shows                                        |
/// |--------|----------------------------------------------|
/// | `%b`   | buffer name                                  |
/// | `%i`   | file I/O progress, conflicts and recovery errors     |
/// | `%f`   | file path, or the buffer name when unvisited |
/// | `%s`   | buffer-local mode-line-state string          |
/// | `%m`   | major mode                                   |
/// | `%n`   | the minor modes' lighters, each after a space |
/// | `%l`   | line number, counting from one               |
/// | `%c`   | column, counting from zero as Emacs does     |
/// | `%p`   | all of it, or how far down the buffer point is |
/// | `%*`   | `**` when modified, `--` when not            |
/// | `%%`   | a literal per cent                           |
///
/// An unknown escape is shown as it is written.
pub fn expand_mode_line<B: BufferTrait>(format: &str, buffer: &Buffer<B>, height: usize) -> String {
    let (line, column) = buffer.text.cursor_pos();
    let lines = buffer.text.line_count().max(1);
    let mut out = String::with_capacity(format.len() + 16);
    let mut chars = format.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('b') => out.push_str(&buffer.name),
            Some('i') => out.push_str(&buffer.file_io_status()),
            Some('f') => out.push_str(buffer.file_path.as_deref().unwrap_or(&buffer.name)),
            Some('m') => out.push_str(&buffer.current_mode),
            Some('n') => {
                for minor in &buffer.minor_modes {
                    if !minor.lighter.is_empty() {
                        out.push(' ');
                        out.push_str(&minor.lighter);
                    }
                }
            }
            Some('s') => {
                if let Some(ELispExp::String(state)) =
                    buffer.data::<ELispExp<B>>("lisp:mode-line-state")
                {
                    out.push_str(state);
                }
            }
            Some('l') => out.push_str(&(line + 1).to_string()),
            Some('c') => out.push_str(&column.to_string()),
            Some('p') => out.push_str(&position_in_buffer(line, lines, height)),
            Some('*') => out.push_str(if buffer.is_modified { "**" } else { "--" }),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Returns where LINE is among LINES, in a window of HEIGHT rows, as Emacs
/// words it: `All` when the whole buffer fits in the window, then `Top` on
/// the first line, `Bot` on the last, and a percentage in between.
fn position_in_buffer(line: usize, lines: usize, height: usize) -> String {
    if lines <= height.max(1) {
        return "All".to_string();
    }
    if line == 0 {
        return "Top".to_string();
    }
    if line + 1 >= lines {
        return "Bot".to_string();
    }
    format!("{}%", (line * 100) / (lines - 1))
}
