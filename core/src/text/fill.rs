//! Filling: the words of some lines laid out again in lines no wider than a
//! column, each behind a prefix, and where a line being typed is broken. It
//! knows prefixes and columns, and nothing of buffers or modes.
//!
//! Design: `doc/architettura-editor.typ, <riempimento>`.

/// How a fill measures and lays out: the column no line goes past, how many
/// columns a Tab reaches to, and how many cells a character takes.
pub struct Measure<F: Fn(char) -> usize> {
    pub width: usize,
    pub tab: usize,
    pub cells: F,
}

impl<F: Fn(char) -> usize> Measure<F> {
    /// Returns the column TEXT ends at when it starts at column FROM: each
    /// character its cells, a Tab to the next multiple of the Tab's width.
    pub fn column_after(&self, text: &str, from: usize) -> usize {
        text.chars().fold(from, |column, c| {
            if c == '\t' {
                let tab = self.tab.max(1);
                (column / tab + 1) * tab
            } else {
                column + (self.cells)(c)
            }
        })
    }
}

/// Returns WORDS laid out in lines that end at or before the measure's
/// width, the first behind FIRST and the others behind REST, joined by
/// newlines, with none at the end. A word too wide for any line goes on a
/// line of its own.
pub fn lay_out<F: Fn(char) -> usize>(
    words: &[&str],
    first: &str,
    rest: &str,
    measure: &Measure<F>,
) -> String {
    let mut text = first.to_string();
    let mut column = measure.column_after(first, 0);
    let mut line_empty = true;
    for word in words {
        let width = measure.column_after(word, 0);
        if line_empty {
            text.push_str(word);
            column += width;
            line_empty = false;
        } else if column + 1 + width <= measure.width {
            text.push(' ');
            text.push_str(word);
            column += 1 + width;
        } else {
            text.push('\n');
            text.push_str(rest);
            text.push_str(word);
            column = measure.column_after(rest, 0) + width;
        }
    }
    text
}

/// Returns the indentation LINE begins with.
pub fn indentation(line: &str) -> &str {
    let end = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..end]
}

/// Returns a bullet or numbered item's marker, including its following
/// horizontal whitespace. TEXT starts after indentation or a comment opener.
pub fn list_marker(text: &str) -> Option<&str> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    let end = if matches!(first, '-' | '+' | '*' | '•' | '‣' | '◦') {
        first.len_utf8()
    } else {
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || !matches!(text.as_bytes().get(digits), Some(b'.' | b')')) {
            return None;
        }
        digits + 1
    };
    let blanks = indentation(&text[end..]);
    (!blanks.is_empty()).then(|| &text[..end + blanks.len()])
}

/// Returns the comment prefix LINE begins with: its indentation, one of
/// OPENERS, any more of the opener's last character or a `!` (`///`, `//!`,
/// `;;;`), and the blanks after; `None` when LINE does not begin with a
/// comment. The longest opener wins.
pub fn comment_prefix<'a>(line: &'a str, openers: &[String]) -> Option<&'a str> {
    let indent = indentation(line).len();
    let after = &line[indent..];
    let opener = openers
        .iter()
        .filter(|opener| !opener.is_empty() && after.starts_with(opener.as_str()))
        .max_by_key(|opener| opener.len())?;
    let last = opener.chars().last()?;
    let mut end = indent + opener.len();
    for c in line[end..].chars() {
        if c == last || c == '!' {
            end += c.len_utf8();
        } else {
            break;
        }
    }
    let blanks = line[end..].len() - line[end..].trim_start_matches([' ', '\t']).len();
    Some(&line[..end + blanks])
}

/// Returns where LINE, being typed, is broken: the run of blanks to replace
/// with a line break, as byte offsets, or `None` when it fits.
///
/// LINE is what comes before point. It fits when its text, without the blanks
/// it ends with, ends at or before the width: breaking is for a word that
/// went past it. The break is the last run of blanks whose word before it
/// ends at or before the width or, when the first word is too wide itself,
/// the first run after it. Nothing within PREFIX is ever broken.
pub fn break_at<F: Fn(char) -> usize>(
    line: &str,
    prefix: &str,
    measure: &Measure<F>,
) -> Option<(usize, usize)> {
    let text = line.trim_end_matches([' ', '\t']);
    if text.len() <= prefix.len() || measure.column_after(text, 0) <= measure.width {
        return None;
    }
    let mut runs = Vec::new();
    let mut at = prefix.len();
    let bytes = line.as_bytes();
    while at < line.len() {
        if bytes[at] == b' ' || bytes[at] == b'\t' {
            let start = at;
            while at < line.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
                at += 1;
            }
            // Blanks right after the prefix have no word before them.
            if start > prefix.len() {
                runs.push((start, at));
            }
        } else {
            at += line[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    runs.iter()
        .rev()
        .find(|(start, _)| measure.column_after(&line[..*start], 0) <= measure.width)
        .or_else(|| runs.first())
        .copied()
}
