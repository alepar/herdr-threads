//! The one terminal-escaping routine every output path shares.
//!
//! Peer-controlled text must never drive a terminal or forge a line of
//! output. [`escape_for_terminal`] makes every C0 control (tab kept in
//! [`Context::MultiLine`], newline kept there too), DEL, C1 control, Unicode
//! line/paragraph separator, bidi control and every Unicode `Cf` (format)
//! character visible as `\u{xxxx}`. [`Context::SingleLine`] also escapes
//! tab, newline and carriage return (as `\t`, `\n`, `\r`) so one field can
//! never span lines.
//!
//! `src/protocol` calls this module too (through [`is_unsafe_char`] and
//! [`push_u4`]), which is an intra-crate module cycle `protocol <-> view`;
//! Rust allows it and the escaper has no dependency back on either side.
//!
//! Column alignment is by display width ([`display_width`], East Asian wide
//! = 2) rather than `char` count.

use std::borrow::Cow;
use unicode_width::UnicodeWidthStr;

/// Whether newlines may pass through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// A multi-line body: newline and tab stay; everything else unsafe is
    /// escaped. Every line is still prefixed by the renderer.
    MultiLine,
    /// A single-line field, argv element or notice: newline, carriage return
    /// and tab are escaped too.
    SingleLine,
}

/// Unicode General_Category=Cf (format) ranges, from UnicodeData.txt of
/// Unicode 16.0.0 (170 code points in 21 ranges). Includes the bidi controls
/// U+061C, U+200E..=U+200F, U+202A..=U+202E and U+2066..=U+2069.
const CF_RANGES: [(u32, u32); 21] = [
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Whether `ch` is a Unicode `Cf` (format) character.
pub fn is_format_char(ch: char) -> bool {
    let cp = ch as u32;
    CF_RANGES
        .binary_search_by(|&(lo, hi)| {
            if cp < lo {
                std::cmp::Ordering::Greater
            } else if cp > hi {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Whether `ch` can drive a terminal or disguise text: any control (C0, DEL,
/// C1), U+2028/U+2029, or a `Cf` format character (which covers every bidi
/// control). Newline and tab count as unsafe here; contexts decide.
pub fn is_unsafe_char(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') || is_format_char(ch)
}

/// `\uXXXX` (lowercase), using a surrogate pair above the BMP so the escape
/// stays valid inside JSON text.
pub fn push_u4(ch: char, out: &mut String) {
    use std::fmt::Write;
    let mut units = [0u16; 2];
    for unit in ch.encode_utf16(&mut units) {
        let _ = write!(out, "\\u{unit:04x}");
    }
}

fn push_visible(ch: char, context: Context, out: &mut String) {
    use std::fmt::Write;
    match ch {
        '\n' if context == Context::SingleLine => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' if context == Context::SingleLine => out.push_str("\\t"),
        ch => {
            let _ = write!(out, "\\u{{{:04x}}}", ch as u32);
        }
    }
}

fn passes(ch: char, context: Context) -> bool {
    match ch {
        '\n' | '\t' if context == Context::MultiLine => true,
        ch => !is_unsafe_char(ch),
    }
}

/// `text` with every unsafe character made visible. Borrowed when nothing
/// needed escaping.
pub fn escape_for_terminal(text: &str, context: Context) -> Cow<'_, str> {
    if text.chars().all(|ch| passes(ch, context)) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        if passes(ch, context) {
            out.push(ch);
        } else {
            push_visible(ch, context, &mut out);
        }
    }
    Cow::Owned(out)
}

/// Terminal columns `text` occupies (East Asian wide characters count 2).
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// `text` followed by spaces up to `width` columns; unchanged when already
/// at least that wide.
pub fn pad_to_width(text: &str, width: usize) -> String {
    let mut out = String::from(text);
    out.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(display_width(text)),
    ));
    out
}
