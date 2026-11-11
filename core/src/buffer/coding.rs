//! The codings a file is read and written in: UTF-8, UTF-8 behind its
//! signature, and Windows-1252, which every file that is not UTF-8 is read in.
//! Windows-1252 gives every byte a character of its own, so no file fails to
//! read in it, and one read and written back unchanged comes back byte for
//! byte, a binary file included.
//!
//! Design: `doc/architettura-editor.typ, <codifiche>`.
use std::path::Path;

/// How a file's bytes are its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Coding {
    /// UTF-8, as the editor holds text: what every new file is written in.
    #[default]
    Utf8,
    /// UTF-8 with the signature, the three bytes some Windows programs put
    /// first. Taken off when read, so the buffer does not begin with an
    /// invisible character, and put back when written.
    Utf8WithSignature,
    /// The one-byte encoding of Windows for western languages: Latin-1, with
    /// typographic characters in place of most of the C1 controls.
    Windows1252,
}

/// The bytes UTF-8 may begin with to say that it is UTF-8.
const SIGNATURE: &[u8] = b"\xEF\xBB\xBF";

/// What Windows-1252 has at 0x80 to 0x9F, where it parts from Latin-1. The
/// five bytes the encoding leaves undefined keep the C1 control they are in
/// Latin-1, as the WHATWG table has them: that is what lets every byte read.
const HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

impl Coding {
    /// Every coding, in the order they are offered.
    pub const ALL: [Coding; 3] = [Coding::Utf8, Coding::Utf8WithSignature, Coding::Windows1252];

    /// Returns the name Lisp and the messages use.
    pub fn name(self) -> &'static str {
        match self {
            Coding::Utf8 => "utf-8",
            Coding::Utf8WithSignature => "utf-8-with-signature",
            Coding::Windows1252 => "windows-1252",
        }
    }

    /// Returns the coding called NAME, ignoring case, if there is one.
    pub fn named(name: &str) -> Option<Coding> {
        Coding::ALL
            .into_iter()
            .find(|coding| coding.name().eq_ignore_ascii_case(name))
    }
}

/// Returns BYTES as text, and the coding they were read in: UTF-8 when they
/// are UTF-8, without its signature when they begin with one, and
/// Windows-1252 otherwise, which never fails.
pub fn decode(bytes: &[u8]) -> (String, Coding) {
    if let Some(rest) = bytes.strip_prefix(SIGNATURE)
        && let Ok(text) = std::str::from_utf8(rest)
    {
        return (text.to_string(), Coding::Utf8WithSignature);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), Coding::Utf8),
        Err(_) => (
            bytes.iter().map(|&byte| from_windows_1252(byte)).collect(),
            Coding::Windows1252,
        ),
    }
}

/// Returns TEXT as the bytes CODING gives it, or the first character CODING
/// has no byte for: written as something else, it would not come back.
pub fn encode(text: &str, coding: Coding) -> Result<Vec<u8>, char> {
    match coding {
        Coding::Utf8 => Ok(text.as_bytes().to_vec()),
        Coding::Utf8WithSignature => {
            let mut bytes = SIGNATURE.to_vec();
            bytes.extend_from_slice(text.as_bytes());
            Ok(bytes)
        }
        Coding::Windows1252 => text.chars().map(|c| to_windows_1252(c).ok_or(c)).collect(),
    }
}

/// How far into a file [`looks_binary`] looks, as Git does.
const SNIFFED: usize = 8000;

/// Answers whether BYTES look like a binary file rather than text: a NUL
/// among the first few thousand, which no text in these codings has, and
/// which Git takes as the sign of a binary file too. Every byte decodes in
/// Windows-1252, so decoding is no test of being text.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(SNIFFED).any(|&byte| byte == 0)
}

/// Reads the file at PATH as text, and says which coding it was in. Fails
/// only when the file cannot be read: what is in it always decodes.
pub fn read_text(path: impl AsRef<Path>) -> std::io::Result<(String, Coding)> {
    Ok(decode(&std::fs::read(path)?))
}

/// A snapshot is adopted only when its stamp stayed stable across the read.
pub(crate) fn read_stable(path: &str) -> std::io::Result<(String, Coding, super::FileStamp)> {
    let before = super::FileStamp::try_of(path)?;
    let (text, coding) = read_text(path)?;
    let after = super::FileStamp::try_of(path)?;
    match (before, after) {
        (Some(before), Some(after)) if before == after => Ok((text, coding, after)),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "file changed while being read; try again",
        )),
    }
}

/// Returns the character Windows-1252 gives BYTE.
fn from_windows_1252(byte: u8) -> char {
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// Returns the byte Windows-1252 gives C, if it has one.
fn to_windows_1252(c: char) -> Option<u8> {
    match u32::from(c) {
        // The same as Latin-1, and as Unicode's first 256 code points.
        code @ (0x00..=0x7F | 0xA0..=0xFF) => u8::try_from(code).ok(),
        _ => HIGH
            .iter()
            .position(|&high| high == c)
            .and_then(|index| u8::try_from(0x80 + index).ok()),
    }
}
