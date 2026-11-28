//! The status line: a window's `mode-line-format`, its escapes expanded
//! against the buffer the window shows.
//!
//! Design: `doc/architettura-editor.typ, <riga-di-stato>`.
use crate::ELispExp;
use crate::buffer::{Buffer, BufferTrait};

/// Returns FORMAT with its escapes expanded against BUFFER, shown in a window
/// of HEIGHT rows of text, FINDINGS being what `%!` shows. The escapes are
/// Emacs's, but for `%!`:
///
/// | escape | shows                                        |
/// |--------|----------------------------------------------|
/// | `%b`   | buffer name                                  |
/// | `%i`   | I/O causes and last recovery outcome/time (UTC)     |
/// | `%f`   | file path, or the buffer name when unvisited |
/// | `%d`   | the buffer's directory, as DIRECTORY's style says |
/// | `%s`   | buffer-local mode-line-state string          |
/// | `%m`   | major mode                                   |
/// | `%n`   | the minor modes' lighters, each after a space |
/// | `%l`   | line number, counting from one               |
/// | `%c`   | column, counting from zero as Emacs does     |
/// | `%p`   | all of it, or how far down the buffer point is |
/// | `%!`   | the findings about the file, counted by severity |
/// | `%*`   | `**` when modified, `--` when not, then `(DOS)` or `(Mac)` when the file's lines do not end in LF |
/// | `%%`   | a literal per cent                           |
///
/// An unknown escape is shown as it is written.
pub fn expand_mode_line<B: BufferTrait>(
    format: &str,
    buffer: &Buffer<B>,
    height: usize,
    findings: &str,
    directory: &ModeLineDirectory,
) -> String {
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
            Some('d') => out.push_str(&directory.of(buffer)),
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
            Some('!') => out.push_str(findings),
            Some('*') => {
                out.push_str(if buffer.is_modified { "**" } else { "--" });
                out.push_str(buffer.eol.mnemonic());
            }
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

/// How `%d` writes a buffer's directory: what `mode-line-directory-style`
/// says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryStyle {
    /// The whole path, with the home directory written `~`: the default.
    Abbreviated,
    /// The whole path, as it is.
    Full,
    /// Only its last N directories, after `…/` when some were left out.
    Last(usize),
}

/// What `%d` needs besides the buffer: the directory a buffer with neither
/// a directory of its own nor a file goes by, the home directory, and the
/// style.
#[derive(Clone, Debug)]
pub struct ModeLineDirectory {
    /// The editor's working directory.
    pub fallback: String,
    /// The home directory, written `~` by the abbreviated style.
    pub home: Option<String>,
    pub style: DirectoryStyle,
}

impl ModeLineDirectory {
    /// Returns BUFFER's directory as the style writes it, decided as
    /// `EditorState::directory_of` decides it: its own, else its file's
    /// folder, else the editor's.
    pub fn of<B: BufferTrait>(&self, buffer: &Buffer<B>) -> String {
        let folder = |path: &str| {
            let parent = std::path::Path::new(path).parent()?.display().to_string();
            (!parent.is_empty()).then_some(parent)
        };
        let directory = buffer
            .directory
            .clone()
            .or_else(|| buffer.file_path.as_deref().and_then(folder))
            .unwrap_or_else(|| self.fallback.clone());
        self.written(&directory)
    }

    /// Returns DIRECTORY as the style writes it.
    pub fn written(&self, directory: &str) -> String {
        match self.style {
            DirectoryStyle::Full => directory.to_string(),
            DirectoryStyle::Abbreviated => abbreviate_home(directory, self.home.as_deref()),
            DirectoryStyle::Last(count) => {
                // Counted after home is written ~, which is one directory.
                let separator = std::path::MAIN_SEPARATOR;
                let abbreviated = abbreviate_home(directory, self.home.as_deref());
                let trimmed = abbreviated.trim_end_matches(std::path::is_separator);
                let parts: Vec<&str> = trimmed
                    .split(std::path::is_separator)
                    .filter(|part| !part.is_empty())
                    .collect();
                let count = count.max(1);
                if parts.len() <= count {
                    abbreviated
                } else {
                    format!(
                        "…{separator}{}",
                        parts[parts.len() - count..].join(&separator.to_string())
                    )
                }
            }
        }
    }
}

/// DIRECTORY with HOME at its start written `~`.
fn abbreviate_home(directory: &str, home: Option<&str>) -> String {
    let Some(home) = home.map(|home| home.trim_end_matches(std::path::is_separator)) else {
        return directory.to_string();
    };
    if home.is_empty() {
        return directory.to_string();
    }
    match directory.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with(std::path::is_separator) => format!("~{rest}"),
        _ => directory.to_string(),
    }
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
