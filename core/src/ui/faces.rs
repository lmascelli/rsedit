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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Default,
    /// The active region, between mark and point.
    Region,
    Keyword,
    Type,
    String,
    Comment,
    Function,
    Builtin,
}

impl Face {
    /// Every face, in theme order. The array and [`Self::index`] have to agree;
    /// `faces_are_indexed_consistently` in the tests is what checks they do.
    pub const ALL: [Face; 8] = [
        Face::Default,
        Face::Region,
        Face::Keyword,
        Face::Type,
        Face::String,
        Face::Comment,
        Face::Function,
        Face::Builtin,
    ];

    /// The name Lisp uses for this face, in `set-face` and `add-syntax-rule`.
    ///
    /// One mapping, used by both, so a face a syntax rule can name is a face a
    /// theme can style. They used to be separate lists, and `region` was in
    /// neither.
    pub const fn name(&self) -> &'static str {
        match self {
            Face::Default => "default",
            Face::Region => "region",
            Face::Keyword => "keyword",
            Face::Type => "type",
            Face::String => "string",
            Face::Comment => "comment",
            Face::Function => "function",
            Face::Builtin => "builtin",
        }
    }

    pub fn from_name(name: &str) -> Option<Face> {
        Face::ALL.into_iter().find(|face| face.name() == name)
    }

    const fn index(&self) -> usize {
        match self {
            Face::Default => 0,
            Face::Region => 1,
            Face::Keyword => 2,
            Face::Type => 3,
            Face::String => 4,
            Face::Comment => 5,
            Face::Function => 6,
            Face::Builtin => 7,
        }
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
/// A fixed array rather than a map: the face set is closed and small, so a
/// lookup is an index, a clone is a memcpy of a couple of hundred bytes, and
/// the whole theme can be copied into a frame snapshot without allocating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    styles: [Style; Face::ALL.len()],
}

impl Theme {
    pub fn style(&self, face: Face) -> Style {
        self.styles[face.index()]
    }

    pub fn set(&mut self, face: Face, style: Style) {
        self.styles[face.index()] = style;
    }

    /// Every face and its style, in theme order -- for `list-faces` and for
    /// anything that wants to show the user what is bound.
    pub fn bindings(&self) -> Vec<(Face, Style)> {
        Face::ALL.into_iter().map(|f| (f, self.style(f))).collect()
    }
}

impl Default for Theme {
    /// Conventional colours, so a renderer that has to approximate lands on
    /// the palette slot of the same name -- and the region uses reverse video,
    /// which needs no colour decision at all and so cannot clash with one.
    fn default() -> Self {
        let mut styles = [Style::plain(); Face::ALL.len()];
        styles[Face::Region.index()] = Style {
            reverse: true,
            ..Style::plain()
        };
        styles[Face::Keyword.index()] = Style::fg(Color::MAGENTA);
        styles[Face::Type.index()] = Style::fg(Color::YELLOW);
        styles[Face::String.index()] = Style::fg(Color::GREEN);
        styles[Face::Comment.index()] = Style {
            italic: true,
            ..Style::fg(Color::BRIGHT_BLACK)
        };
        styles[Face::Function.index()] = Style::fg(Color::CYAN);
        styles[Face::Builtin.index()] = Style::fg(Color::BLUE);
        Self { styles }
    }
}
