//! Faces, the styles bound to them, and the theme that holds the bindings: a
//! face says what is drawn, a style how, and a colour is a request the front
//! end approximates.
//!
//! Design: `doc/architettura-editor.typ, <facce>`.

/// Something drawn in a style of its own, named rather than coloured: a name
/// interned to a small integer, which a [`Highlight`] carries and a [`Theme`]
/// indexes by. A name is defined by being named, from Lisp as from Rust.
///
/// The default is [`Face::DEFAULT`], id zero.
///
/// [`Highlight`]: crate::ui::Highlight
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Face(u32);

/// The names of every face interned so far, indexed by id, and the id of each
/// name: one registry for the process, since a face's name means the same
/// thing everywhere.
///
/// It only grows, by design: a face is kept by its id in themes and in
/// highlights, and a name forgotten could not be told from one never named.
/// What makes it grow is a name nobody has used before, from Lisp while the
/// editor runs -- `set-face`, an overlay's face -- and the id is 32 bits, so
/// that a name new after the first 65 536 is not given an old one's id, as a
/// 16-bit id cut short was.
pub(crate) struct Registry {
    names: Vec<std::sync::Arc<str>>,
    ids: std::collections::HashMap<std::sync::Arc<str>, u32>,
}

impl Registry {
    /// Returns a registry of the built-in faces, at their own ids.
    pub(crate) fn new() -> Self {
        let mut registry = Registry {
            names: Vec::new(),
            ids: std::collections::HashMap::new(),
        };
        for (_, name) in Face::BUILT_IN {
            registry.intern(name);
        }
        registry
    }

    /// Returns the face called NAME here, giving it the next id if nothing
    /// has named it yet; past the last id a 32-bit face can have, which no
    /// session reaches, the default face rather than another face's id.
    pub(crate) fn intern(&mut self, name: &str) -> Face {
        if let Some(id) = self.ids.get(name) {
            return Face(*id);
        }
        let Ok(id) = u32::try_from(self.names.len()) else {
            return Face::DEFAULT;
        };
        let name: std::sync::Arc<str> = name.into();
        self.names.push(name.clone());
        self.ids.insert(name, id);
        Face(id)
    }

    /// Returns the name of FACE here, if it has one.
    pub(crate) fn name(&self, face: Face) -> Option<std::sync::Arc<str>> {
        self.names.get(face.index()).cloned()
    }
}

static FACE_NAMES: std::sync::LazyLock<std::sync::RwLock<Registry>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(Registry::new()));

impl Face {
    /// Text with nothing said about it.
    pub const DEFAULT: Face = Face(0);
    /// The active region, between mark and point.
    pub const REGION: Face = Face(1);
    /// The status line under the focused window.
    pub const MODE_LINE: Face = Face(2);
    /// The status line under any other window.
    pub const MODE_LINE_INACTIVE: Face = Face(3);
    /// The rule between two windows sitting side by side.
    pub const WINDOW_SEPARATOR: Face = Face(4);
    pub const KEYWORD: Face = Face(5);
    pub const TYPE: Face = Face(6);
    pub const STRING: Face = Face(7);
    pub const COMMENT: Face = Face(8);
    pub const FUNCTION: Face = Face(9);
    pub const BUILTIN: Face = Face(10);
    /// A line number in a window's gutter.
    pub const LINE_NUMBER: Face = Face(11);
    /// The number of the line point is on.
    pub const LINE_NUMBER_CURRENT: Face = Face(12);
    /// The match a replace is currently offering.
    pub const REPLACE_MATCH: Face = Face(13);
    /// Every other match of a search or a replace in progress, on screen.
    pub const LAZY_HIGHLIGHT: Face = Face(14);
    /// A delimiter next to point, and the one it pairs with.
    pub const SHOW_PAREN_MATCH: Face = Face(15);
    /// A delimiter next to point that pairs with one of another kind, or with
    /// none.
    pub const SHOW_PAREN_MISMATCH: Face = Face(16);

    /// The faces the editor names itself, with their ids: the first entries
    /// of the registry, so that Rust names them as constants. The order and
    /// the constants above agree, as
    /// `built_in_faces_are_interned_at_their_own_ids` checks.
    pub const BUILT_IN: [(Face, &'static str); 17] = [
        (Face::DEFAULT, "default"),
        (Face::REGION, "region"),
        (Face::MODE_LINE, "mode-line"),
        (Face::MODE_LINE_INACTIVE, "mode-line-inactive"),
        (Face::WINDOW_SEPARATOR, "window-separator"),
        (Face::KEYWORD, "keyword"),
        (Face::TYPE, "type"),
        (Face::STRING, "string"),
        (Face::COMMENT, "comment"),
        (Face::FUNCTION, "function"),
        (Face::BUILTIN, "builtin"),
        (Face::LINE_NUMBER, "line-number"),
        (Face::LINE_NUMBER_CURRENT, "line-number-current"),
        (Face::REPLACE_MATCH, "replace-match"),
        (Face::LAZY_HIGHLIGHT, "lazy-highlight"),
        (Face::SHOW_PAREN_MATCH, "show-paren-match"),
        (Face::SHOW_PAREN_MISMATCH, "show-paren-mismatch"),
    ];

    /// Returns the face called NAME, defining it if nothing has named it yet.
    /// A name nobody styles is drawn plain, and `list-faces` shows it. See
    /// [`Registry::intern`].
    pub fn intern(name: &str) -> Face {
        // The write lock even for a name already known: a check under the
        // read lock first would need a second one under this lock anyway, and
        // interning happens while configuring, never while drawing.
        FACE_NAMES
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .intern(name)
    }

    /// Returns the face called NAME, or `None` if nothing has named it.
    pub fn named(name: &str) -> Option<Face> {
        FACE_NAMES
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ids
            .get(name)
            .map(|id| Face(*id))
    }

    /// Returns the name this face has in Lisp, for `set-face` and
    /// `add-syntax-rule` alike.
    pub fn name(self) -> std::sync::Arc<str> {
        FACE_NAMES
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .name(self)
            // Only for a `Face` built out of a raw id, which only this module
            // can do.
            .unwrap_or_else(|| "default".into())
    }

    /// Returns every face named so far, the built-in ones first and then in
    /// the order they were defined.
    pub fn all() -> Vec<Face> {
        let count = FACE_NAMES
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .names
            .len();
        (0..count as u32).map(Face).collect()
    }

    /// Returns where this face sits in a [`Theme`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A colour a face asks for: a request, which the front end draws as near as
/// it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    /// Returns the colour R, G, B.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const RED: Color = Color::rgb(170, 0, 0);
    pub const GREEN: Color = Color::rgb(0, 170, 0);
    pub const YELLOW: Color = Color::rgb(170, 85, 0);
    pub const BLUE: Color = Color::rgb(0, 0, 170);
    pub const MAGENTA: Color = Color::rgb(170, 0, 170);
    pub const CYAN: Color = Color::rgb(0, 170, 170);
    pub const WHITE: Color = Color::rgb(170, 170, 170);
    pub const BRIGHT_BLACK: Color = Color::rgb(85, 85, 85);
    pub const BRIGHT_RED: Color = Color::rgb(255, 85, 85);
    pub const BRIGHT_GREEN: Color = Color::rgb(85, 255, 85);
    pub const BRIGHT_YELLOW: Color = Color::rgb(255, 255, 85);
    pub const BRIGHT_BLUE: Color = Color::rgb(85, 85, 255);
    pub const BRIGHT_MAGENTA: Color = Color::rgb(255, 85, 255);
    pub const BRIGHT_CYAN: Color = Color::rgb(85, 255, 255);
    pub const BRIGHT_WHITE: Color = Color::rgb(255, 255, 255);

    /// Returns the colour TEXT writes, as Lisp does: `"#rgb"`, `"#rrggbb"`,
    /// or one of the names in [`NAMED_COLORS`]; `None` for anything else.
    pub fn parse(text: &str) -> Option<Color> {
        if let Some(hex) = text.strip_prefix('#') {
            // Hexadecimal digits only, before anything is sliced: a length in
            // bytes cut a character outside ASCII in two, which panicked, and
            // `from_str_radix` takes a sign, which `#+f+f+f` would have had.
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            return match hex.len() {
                // `#rgb` expands each digit, so `#f00` and `#ff0000` agree.
                3 => {
                    let mut digits = hex.chars().map(|c| c.to_digit(16));
                    let mut next = || digits.next().flatten().map(|d| (d * 17) as u8);
                    Some(Color::rgb(next()?, next()?, next()?))
                }
                6 => Some(Color::rgb(
                    u8::from_str_radix(&hex[0..2], 16).ok()?,
                    u8::from_str_radix(&hex[2..4], 16).ok()?,
                    u8::from_str_radix(&hex[4..6], 16).ok()?,
                )),
                _ => None,
            };
        }
        NAMED_COLORS
            .iter()
            .find(|(name, _)| *name == text)
            .map(|(_, color)| *color)
    }

    /// Returns this colour as `#rrggbb`, which [`Self::parse`] reads back: what
    /// `face-style` answers and `set-face` takes.
    pub fn to_text(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// The colour names Lisp understands, and what they conventionally mean, in
/// the order of the sixteen slots of a terminal's palette: a renderer finds
/// the nearest slot by its index here.
pub const NAMED_COLORS: [(&str, Color); 16] = [
    ("black", Color::BLACK),
    ("red", Color::RED),
    ("green", Color::GREEN),
    ("yellow", Color::YELLOW),
    ("blue", Color::BLUE),
    ("magenta", Color::MAGENTA),
    ("cyan", Color::CYAN),
    ("white", Color::WHITE),
    ("bright-black", Color::BRIGHT_BLACK),
    ("bright-red", Color::BRIGHT_RED),
    ("bright-green", Color::BRIGHT_GREEN),
    ("bright-yellow", Color::BRIGHT_YELLOW),
    ("bright-blue", Color::BRIGHT_BLUE),
    ("bright-magenta", Color::BRIGHT_MAGENTA),
    ("bright-cyan", Color::BRIGHT_CYAN),
    ("bright-white", Color::BRIGHT_WHITE),
];

/// How a face is drawn. A colour of `None` leaves the front end's own, so
/// that a face can set only a background, or only an attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Foreground and background swapped: legible whatever the colours are.
    pub reverse: bool,
}

impl Style {
    /// Returns the style that changes nothing.
    pub const fn plain() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            reverse: false,
        }
    }

    /// Returns the style that sets only the foreground, to COLOR.
    pub const fn fg(color: Color) -> Self {
        Self {
            fg: Some(color),
            ..Self::plain()
        }
    }

    /// Returns whether this style changes nothing, so that a renderer can
    /// skip a run drawn in it.
    pub fn is_plain(&self) -> bool {
        *self == Style::plain()
    }

    /// Sets the attribute NAME, or clears it, and returns true; false for a
    /// name it does not know, for the caller to report.
    pub fn set_attribute(&mut self, name: &str, on: bool) -> bool {
        match name {
            "bold" => self.bold = on,
            "italic" => self.italic = on,
            "underline" => self.underline = on,
            "reverse" => self.reverse = on,
            _ => return false,
        }
        true
    }

    /// Returns the attributes that are on, as [`Self::set_attribute`] names
    /// them.
    pub fn attribute_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        for (on, name) in [
            (self.bold, "bold"),
            (self.italic, "italic"),
            (self.underline, "underline"),
            (self.reverse, "reverse"),
        ] {
            if on {
                names.push(name);
            }
        }
        names
    }
}

/// The style bound to each face, indexed by the face's id. A face past the
/// end is drawn plain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    styles: Vec<Style>,
}

impl Theme {
    /// Returns the style FACE is drawn in.
    pub fn style(&self, face: Face) -> Style {
        self.styles
            .get(face.index())
            .copied()
            .unwrap_or_else(Style::plain)
    }

    /// Binds FACE to STYLE.
    pub fn set(&mut self, face: Face, style: Style) {
        if self.styles.len() <= face.index() {
            self.styles.resize(face.index() + 1, Style::plain());
        }
        self.styles[face.index()] = style;
    }

    /// Returns every face with its style, in the order of the ids: what
    /// `list-faces` shows.
    pub fn bindings(&self) -> Vec<(Face, Style)> {
        Face::all()
            .into_iter()
            .map(|f| (f, self.style(f)))
            .collect()
    }
}

impl Default for Theme {
    /// Returns the theme the editor ships: conventional colours, which an
    /// approximating renderer draws in the palette slot of the same name, and
    /// reverse video where a face must show on any background.
    fn default() -> Self {
        let mut theme = Theme {
            styles: vec![Style::plain(); Face::BUILT_IN.len()],
        };
        theme.set(
            Face::REGION,
            Style {
                reverse: true,
                ..Style::plain()
            },
        );
        theme.set(
            Face::MODE_LINE,
            Style {
                reverse: true,
                ..Style::plain()
            },
        );
        theme.set(
            Face::MODE_LINE_INACTIVE,
            Style {
                reverse: true,
                fg: Some(Color::BRIGHT_BLACK),
                ..Style::plain()
            },
        );
        theme.set(Face::KEYWORD, Style::fg(Color::MAGENTA));
        theme.set(Face::TYPE, Style::fg(Color::YELLOW));
        theme.set(Face::STRING, Style::fg(Color::GREEN));
        theme.set(
            Face::COMMENT,
            Style {
                italic: true,
                ..Style::fg(Color::BRIGHT_BLACK)
            },
        );
        theme.set(Face::FUNCTION, Style::fg(Color::CYAN));
        theme.set(Face::BUILTIN, Style::fg(Color::BLUE));
        theme.set(Face::LINE_NUMBER, Style::fg(Color::BRIGHT_BLACK));
        theme.set(
            Face::REPLACE_MATCH,
            Style {
                reverse: true,
                ..Style::plain()
            },
        );
        theme.set(
            Face::LINE_NUMBER_CURRENT,
            Style {
                bold: true,
                ..Style::plain()
            },
        );
        theme.set(
            Face::LAZY_HIGHLIGHT,
            Style {
                bg: Some(Color::CYAN),
                ..Style::fg(Color::BLACK)
            },
        );
        theme.set(
            Face::SHOW_PAREN_MATCH,
            Style {
                bg: Some(Color::BLUE),
                bold: true,
                ..Style::fg(Color::BRIGHT_WHITE)
            },
        );
        theme.set(
            Face::SHOW_PAREN_MISMATCH,
            Style {
                bg: Some(Color::MAGENTA),
                bold: true,
                ..Style::fg(Color::BRIGHT_WHITE)
            },
        );
        theme
    }
}
