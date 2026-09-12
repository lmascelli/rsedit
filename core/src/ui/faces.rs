//! Faces, the styles bound to them, and the theme that holds the bindings.
//!
//! # The indirection, and why it is worth one hop
//!
//! Nothing that decides *what* to highlight also decides *how*. The region
//! code says [`Face::Region`]; a syntax rule says [`Face::Keyword`]; neither
//! knows a colour. A [`Theme`] maps faces to [`Style`]s, and the renderer is
//! the only thing that turns a `Style` into escape sequences.
//!
//! That buys three things at the cost of one lookup per drawn run:
//!
//! - Recolouring is data, not code. `(set-face "region" "#eeeeee" "#3a5fcd")`
//!   from Lisp, no recompile, no renderer change.
//! - A new frontend re-implements one function -- "apply this `Style`" --
//!   rather than a rule per highlighted thing.
//! - A new highlighted feature adds a face and a default, and is drawn
//!   correctly by every frontend that already exists.
//!
//! # What a face asks for, and what a renderer shows
//!
//! A face names a colour; it does not pick one. [`Color`] is a *request* -- a
//! plain colour, in terms every display surface can read -- and the renderer
//! turns the request into whatever it can actually draw. A truecolor terminal
//! draws it as asked; a 256-colour one picks the nearest entry in its cube; a
//! sixteen-colour one picks the nearest palette slot, which is the user's own
//! configured colour. A GUI would do something else again.
//!
//! This is the same split as the interpreter and its host context: the editor
//! knows what it means, and stays ignorant of how the thing in front of the
//! user renders it. An ANSI palette index in this file would break that -- it
//! is not a colour, it is a terminal's encoding of one.
//!
//! # Scaling past this
//!
//! Faces are a closed enum, so a *mode* cannot invent one of its own the way
//! Emacs lets it. That is the deliberate limit of "basic": the set here covers
//! the region and everything syntax highlighting (#22) needs, and adding to it
//! is a variant plus a default. Going further means interning face names as
//! symbols and making [`Theme`] a map -- worth doing when a mode actually needs
//! a face nothing else has, and not before.

/// A thing that can be drawn differently, named rather than coloured.
///
/// # Why this is an interned name and not an enum
///
/// It was an enum, and that was right while the set was closed: the region, the
/// mode line, and the handful of kinds a syntax rule could name. A *grammar*
/// breaks that. Every language wants faces nothing else does -- a macro, an
/// attribute, a doc comment, an operator -- and a closed enum makes each of
/// them a change to this file, a change every frontend recompiles for, and a
/// decision someone has to ask permission for.
///
/// So a face is now a **name**, interned to a small integer. `(set-face
/// 'rust-attribute "yellow" nil)` defines one, and nothing in Rust had to know
/// it was coming.
///
/// # Why an integer rather than the name itself
///
/// A `Face` is copied into every [`Highlight`], of which a coloured screen has
/// hundreds, and looked up in a [`Theme`] for each one. An integer keeps the
/// highlight small and the lookup an array index; the string is paid for once,
/// when the face is first named.
///
/// [`Highlight`]: crate::ui::Highlight
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Face(u16);

/// The names of every face interned so far, indexed by id.
///
/// # Why this is global
///
/// A face is a name, and a name means the same thing everywhere -- `keyword` in
/// one buffer is `keyword` in another, and a `Theme` is per-editor precisely so
/// that the *style* can differ while the face does not. Threading a registry
/// through `Highlight`, `SyntaxRule` and `Theme` would put a context on three
/// types that have no other use for one, to describe something that never
/// varies.
///
/// It only ever grows, and only by the names Lisp mentions, so it is bounded by
/// the configuration rather than by anything the editor does at runtime.
static FACE_NAMES: std::sync::LazyLock<std::sync::RwLock<Vec<std::sync::Arc<str>>>> =
    std::sync::LazyLock::new(|| {
        std::sync::RwLock::new(
            Face::BUILT_IN
                .iter()
                .map(|(_, name)| (*name).into())
                .collect(),
        )
    });

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

    /// The faces the editor names itself, with the ids they are interned at.
    ///
    /// These are pre-interned so that Rust can refer to them as constants: the
    /// region code says [`Face::REGION`] without asking a registry, and the ids
    /// are fixed because this array is what fills the registry to begin with.
    /// The order and the constants above have to agree, and
    /// `built_in_faces_are_interned_at_their_own_ids` in the tests is what
    /// checks they do.
    pub const BUILT_IN: [(Face, &'static str); 11] = [
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
    ];

    /// The face called NAME, defining it if nothing has named it yet.
    ///
    /// Defining-on-use is the point: a grammar written in Lisp names the faces
    /// it wants and they exist. A name nobody styles is inert rather than an
    /// error, which is the same outcome a misspelt face had when the set was
    /// closed -- except that the name survives, so `list-faces` shows it and
    /// the mistake is visible.
    pub fn intern(name: &str) -> Face {
        // The write lock is taken even to answer "already known", rather than
        // checking under a read lock first and taking the write lock only on a
        // miss. That would be two checks doing one job -- and the second would
        // still have to be there, since two threads could both miss the first.
        // Interning happens when configuration is read and a grammar is
        // defined, never while drawing, so there is nothing here to optimise.
        let mut names = FACE_NAMES
            .write()
            .expect("Failed to acquire write lock on the face registry");
        if let Some(id) = names.iter().position(|known| &**known == name) {
            return Face(id as u16);
        }
        names.push(name.into());
        Face((names.len() - 1) as u16)
    }

    /// The face called NAME, or `None` if nothing has named it.
    ///
    /// A linear scan, deliberately: naming a face happens when configuration is
    /// read and a grammar is defined, never while drawing, and a few dozen
    /// short strings are quicker to walk than to hash.
    pub fn named(name: &str) -> Option<Face> {
        FACE_NAMES
            .read()
            .expect("Failed to acquire read lock on the face registry")
            .iter()
            .position(|known| &**known == name)
            .map(|id| Face(id as u16))
    }

    /// The name Lisp uses for this face, in `set-face` and `add-syntax-rule`.
    ///
    /// One mapping, used by both, so a face a syntax rule can name is a face a
    /// theme can style. They used to be separate lists, and `region` was in
    /// neither.
    pub fn name(self) -> std::sync::Arc<str> {
        FACE_NAMES
            .read()
            .expect("Failed to acquire read lock on the face registry")
            .get(self.index())
            .cloned()
            // Only reachable for a `Face` built by hand out of a raw id, which
            // nothing outside this module can do.
            .unwrap_or_else(|| "default".into())
    }

    /// Every face named so far, built-ins first and then in the order they were
    /// defined.
    pub fn all() -> Vec<Face> {
        let count = FACE_NAMES
            .read()
            .expect("Failed to acquire read lock on the face registry")
            .len();
        (0..count as u16).map(Face).collect()
    }

    /// Where this face sits in a [`Theme`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A colour a face asks for.
///
/// A plain colour, deliberately: not a palette index, not an enum of "named or
/// exact". A palette index is one terminal's encoding, and a two-case request
/// makes the editor decide something the renderer is better placed to answer.
/// Everything here is a *suggestion*, and every renderer answers it the same
/// way -- by drawing the nearest thing it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
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

    /// Parse a colour as Lisp writes it: `"#rgb"`, `"#rrggbb"`, or one of the
    /// names in [`NAMED_COLORS`].
    ///
    /// A name is shorthand for a specific colour, not a reference to a palette
    /// slot -- there is nothing in this crate that could resolve such a
    /// reference, and pretending otherwise would be a promise the editor
    /// cannot keep. It still tends to reach the user's own palette, because a
    /// terminal that cannot show the exact value falls back to the nearest
    /// slot, which for a conventional colour is the slot of the same name.
    pub fn parse(text: &str) -> Option<Color> {
        if let Some(hex) = text.strip_prefix('#') {
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

    /// How Lisp writes this colour back out, so `face-style` round-trips
    /// through `set-face`.
    pub fn to_text(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// The colour names Lisp understands, and what they conventionally mean.
///
/// Ordered as the sixteen terminal palette slots are, because a renderer that
/// has to approximate uses this same table to find the nearest one -- so the
/// order carries meaning and is not just a list.
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

/// How a face is drawn. `None` means "leave the terminal's own choice alone",
/// which is what lets a face set only a background, or only an attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Swap foreground and background. The one attribute that needs no colour
    /// decision, and therefore the only one that is legible on every terminal
    /// and in every scheme -- including monochrome, and including whichever of
    /// light or dark the user happens to run.
    pub reverse: bool,
}

impl Style {
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

    pub const fn fg(color: Color) -> Self {
        Self {
            fg: Some(color),
            ..Self::plain()
        }
    }

    /// Whether this style would change anything, so a renderer can skip a run
    /// that would be drawn exactly as it already is.
    pub fn is_plain(&self) -> bool {
        *self == Style::plain()
    }

    /// Set or clear one named attribute. Returns false for a name it does not
    /// know, so a caller can report it rather than silently ignoring a typo.
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

    /// The attributes that are on, in the names [`Self::set_attribute`] takes.
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

/// The style bound to each face.
///
/// A `Vec` indexed by face id rather than a map: ids are dense and small, so a
/// lookup stays an array index even though the face set is now open. A face
/// beyond the end is simply unstyled, which is what lets a grammar name a face
/// nobody has themed without every lookup having to check first.
///
/// Cloned into each frame snapshot. That is one allocation per frame where it
/// used to be a memcpy -- worth watching in the perf suite rather than assuming
/// either way, and worth an `Arc` if it ever shows up there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    styles: Vec<Style>,
}

impl Theme {
    pub fn style(&self, face: Face) -> Style {
        self.styles
            .get(face.index())
            .copied()
            .unwrap_or_else(Style::plain)
    }

    pub fn set(&mut self, face: Face, style: Style) {
        if self.styles.len() <= face.index() {
            self.styles.resize(face.index() + 1, Style::plain());
        }
        self.styles[face.index()] = style;
    }

    /// Every face and its style, in id order -- for `list-faces` and for
    /// anything that wants to show the user what is bound.
    pub fn bindings(&self) -> Vec<(Face, Style)> {
        Face::all()
            .into_iter()
            .map(|f| (f, self.style(f)))
            .collect()
    }
}

impl Default for Theme {
    /// Conventional colours, so a renderer that has to approximate lands on
    /// the palette slot of the same name -- and the region uses reverse video,
    /// which needs no colour decision at all and so cannot clash with one.
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
        // Reverse video for the same reason the region uses it: a status bar
        // has to be visible against a background this code cannot know.
        theme.set(
            Face::MODE_LINE,
            Style {
                reverse: true,
                ..Style::plain()
            },
        );
        // Distinguishable without being another colour decision -- an
        // unfocused window's status line is present but not competing.
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
        theme
    }
}
