//! The colours a face asks for, as the terminal can draw them: how many it
//! can, and the nearest of those to each request.
//!
//! Design: `doc/architettura-editor.typ, <tui>`.
use crossterm::style::Color as CrossColor;
use rsedit_core::{Color, NAMED_COLORS};

/// How much colour this terminal can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    /// The sixteen palette entries and nothing else.
    Ansi16,
    /// The 256-colour cube.
    Ansi256,
    /// 24-bit colour.
    TrueColor,
}

impl ColorDepth {
    /// Returns the depth the environment claims: 24-bit when `COLORTERM`
    /// says `truecolor` or `24bit`, 256 colours when `TERM` says `256color`,
    /// sixteen otherwise. Guessing low draws an approximate colour, guessing
    /// high nothing useful.
    pub fn detect() -> ColorDepth {
        ColorDepth::from_env(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    /// Returns the depth COLORTERM and TERM claim, as [`Self::detect`] reads
    /// them from the environment.
    pub fn from_env(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
        let colorterm = colorterm.unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return ColorDepth::TrueColor;
        }
        if term.unwrap_or_default().contains("256color") {
            return ColorDepth::Ansi256;
        }
        ColorDepth::Ansi16
    }
}

/// Returns the squared distance between A and B in RGB.
fn distance(a: Color, b: Color) -> u32 {
    let d = |x: u8, y: u8| {
        let diff = x as i32 - y as i32;
        (diff * diff) as u32
    };
    d(a.r, b.r) + d(a.g, b.g) + d(a.b, b.b)
}

/// Returns the slot of the sixteen-colour palette nearest to COLOR, by the
/// core's [`NAMED_COLORS`], which are in the palette's order. The slot shows
/// the colour the terminal's palette gives it.
fn nearest_palette_slot(color: Color) -> u8 {
    NAMED_COLORS
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, entry))| distance(color, *entry))
        .map(|(i, _)| i as u8)
        .expect("the palette is not empty")
}

/// Returns the entry of the 256-colour table nearest to COLOR: the nearer of
/// the cube's, 16 to 231, and the grey ramp's, 232 to 255.
fn nearest_cube_slot(color: Color) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let axis = |v: u8| {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, level)| (v as i32 - **level as i32).abs())
            .map(|(i, _)| i)
            .expect("LEVELS is not empty")
    };
    let (r, g, b) = (axis(color.r), axis(color.g), axis(color.b));
    let cube_index = 16 + 36 * r + 6 * g + b;
    let cube_rgb = Color::rgb(LEVELS[r], LEVELS[g], LEVELS[b]);

    let grey_step = ((color.r as u32 + color.g as u32 + color.b as u32) / 3).clamp(8, 238);
    let grey_slot = ((grey_step - 8) / 10).min(23) as u8;
    let grey_level = (8 + 10 * grey_slot as u32) as u8;
    let grey_rgb = Color::rgb(grey_level, grey_level, grey_level);

    if distance(color, grey_rgb) < distance(color, cube_rgb) {
        232 + grey_slot
    } else {
        cube_index as u8
    }
}

/// Returns the colour a terminal of DEPTH draws for COLOR: COLOR itself in
/// 24 bits, the nearest entry of its table otherwise.
pub fn resolve(color: Color, depth: ColorDepth) -> CrossColor {
    match depth {
        ColorDepth::TrueColor => CrossColor::Rgb {
            r: color.r,
            g: color.g,
            b: color.b,
        },
        ColorDepth::Ansi256 => CrossColor::AnsiValue(nearest_cube_slot(color)),
        ColorDepth::Ansi16 => CrossColor::AnsiValue(nearest_palette_slot(color)),
    }
}
