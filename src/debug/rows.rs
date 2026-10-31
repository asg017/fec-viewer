//! Turning bytes into display text for the debug view, with the class of
//! every span (separator, quote, invalid byte, …) and the maps between bytes
//! and display positions that clicks, selection and search need.
//!
//! Nothing here touches the theme or gpui, so it's all testable.

use std::ops::Range;

use super::index::{Eol, Field, Kind};

/// Longest stretch of a line drawn in the lines view. A broken file can be
/// one 100 MB "line"; the rest is summarized.
pub const MAX_DRAWN: usize = 16 * 1024;

/// Bytes per hex row.
pub const HEX_WIDTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Text,
    /// Text of every other line in the hex view, so line boundaries show.
    TextAlt,
    RowType,
    /// A `/* Header` block key.
    Key,
    Separator,
    Quote,
    LineEnd,
    /// A control byte (shown as a control picture).
    Control,
    /// Valid UTF-8 beyond ASCII.
    NonAscii,
    /// Not valid UTF-8 (shown as `\xNN`).
    Invalid,
    /// Prose inside a text block.
    Prose,
    /// Blank lines, padding, and the "… more bytes" note.
    Muted,
}

/// A line laid out for display.
#[derive(Debug, Default)]
pub struct DrawnLine {
    pub text: String,
    /// Contiguous spans covering `text`.
    pub runs: Vec<(Range<usize>, Class)>,
    /// Each field's display range (field ranges for clicks and hovers).
    pub fields: Vec<Range<usize>>,
    /// Display offset of each drawn content byte, plus one past the end.
    display_of: Vec<u32>,
}

impl DrawnLine {
    /// Display range covering content bytes `bytes` (clamped to what's drawn).
    pub fn display_range(&self, bytes: Range<usize>) -> Range<usize> {
        let last = self.display_of.len().saturating_sub(1);
        let at = |i: usize| self.display_of.get(i.min(last)).copied().unwrap_or(0) as usize;
        at(bytes.start)..at(bytes.end)
    }

    fn push(&mut self, s: &str, class: Class) {
        let start = self.text.len();
        self.text.push_str(s);
        match self.runs.last_mut() {
            Some((r, c)) if *c == class && r.end == start => r.end = self.text.len(),
            _ => self.runs.push((start..self.text.len(), class)),
        }
    }
}

/// The separator glyph drawn for 0x1C: a broken bar, which monospace fonts
/// carry (control pictures like `␜` fall back to a tiny glyph) and which
/// filings practically never contain.
pub const FS_GLYPH: &str = "¦";

/// Lay out one line: fields in their classes, separators made visible,
/// control bytes and invalid UTF-8 as escapes (`\t`, `\x01`, `\xE9`), and
/// the line ending as `\r\n`.
pub fn draw_line(content: &[u8], eol: Eol, kind: Kind, fields: &[Field]) -> DrawnLine {
    let mut out = DrawnLine::default();
    let drawn = content.len().min(MAX_DRAWN);
    out.display_of = vec![0; drawn + 1];

    let mut i = 0;
    let mut field_ix = 0;
    while i < drawn {
        // Which field `i` is in (fields are ordered; gaps are separators).
        while field_ix < fields.len() && fields[field_ix].span.end < i {
            field_ix += 1;
        }
        let field = fields.get(field_ix).filter(|f| f.span.contains(&i));
        out.display_of[i] = out.text.len() as u32;

        let b = content[i];
        let class = match (field, kind) {
            (None, _) => Class::Separator,
            (Some(_), Kind::Text) => Class::Prose,
            (Some(_), Kind::HeaderBlock) if field_ix == 0 => Class::Key,
            (Some(_), k) if k.is_record() && field_ix == 0 => Class::RowType,
            (Some(_), Kind::TextBegin | Kind::TextEnd) => Class::RowType,
            _ => Class::Text,
        };
        let consumed = if field.is_none() && !matches!(b, 0x1c | b',' | b'=') {
            // Not in a field and not a delimiter: a blank line's whitespace.
            draw_byte(&mut out, &content[i..drawn], Class::Muted)
        } else if field.is_none() {
            out.push(
                if b == 0x1c {
                    FS_GLYPH
                } else {
                    // `,` or a header block's `=`.
                    std::str::from_utf8(&content[i..i + 1]).unwrap_or("?")
                },
                Class::Separator,
            );
            1
        } else {
            draw_byte(&mut out, &content[i..drawn], class)
        };
        for k in 1..consumed {
            out.display_of[i + k] = out.display_of[i];
        }
        i += consumed;
    }
    out.display_of[drawn] = out.text.len() as u32;
    out.fields = fields
        .iter()
        .map(|f| {
            let at = |b: usize| out.display_of[b.min(drawn)] as usize;
            at(f.span.start)..at(f.span.end)
        })
        .collect();

    if drawn < content.len() {
        out.push(
            &format!("  … {} more bytes", content.len() - drawn),
            Class::Muted,
        );
    }
    let ending = match eol {
        Eol::None => "",
        Eol::Lf => "\\n",
        Eol::CrLf => "\\r\\n",
        Eol::Cr => "\\r",
    };
    if !ending.is_empty() {
        out.push(ending, Class::LineEnd);
    }
    out
}

/// Draw the byte (or UTF-8 sequence) at the start of `rest`; returns how
/// many bytes it consumed.
fn draw_byte(out: &mut DrawnLine, rest: &[u8], class: Class) -> usize {
    let b = rest[0];
    match b {
        b'"' => {
            out.push("\"", Class::Quote);
            1
        }
        0x20..=0x7e => {
            out.push(std::str::from_utf8(&rest[..1]).unwrap_or("?"), class);
            1
        }
        0x00..=0x1f | 0x7f => {
            let escaped = match b {
                b'\t' => "\\t".to_string(),
                0 => "\\0".to_string(),
                _ => format!("\\x{b:02X}"),
            };
            out.push(&escaped, Class::Control);
            1
        }
        _ => {
            let width = utf8_width(b);
            match rest.get(..width).map(std::str::from_utf8) {
                Some(Ok(s)) if width > 1 => {
                    out.push(s, Class::NonAscii);
                    width
                }
                _ => {
                    out.push(&format!("\\x{b:02X}"), Class::Invalid);
                    1
                }
            }
        }
    }
}

fn utf8_width(b: u8) -> usize {
    match b {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 1,
    }
}

// ---------------------------------------------------------------------------
// Hex rows
// ---------------------------------------------------------------------------

/// `48 44 52 1C 46 45 43 1C  38 2E 34 1C …`: two hex digits per byte, a space
/// between bytes and an extra one after the eighth.
pub fn hex_text(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(HEX_WIDTH * 3 + 1);
    for i in 0..HEX_WIDTH {
        if i > 0 {
            s.push(' ');
        }
        if i == HEX_WIDTH / 2 {
            s.push(' ');
        }
        match bytes.get(i) {
            Some(b) => {
                s.push(HEX[(b >> 4) as usize] as char);
                s.push(HEX[(b & 0xf) as usize] as char);
            }
            None => s.push_str("  "),
        }
    }
    s
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Display range of byte `i` (of a row) in [`hex_text`].
pub fn hex_range(i: usize) -> Range<usize> {
    let start = i * 3 + (i >= HEX_WIDTH / 2) as usize;
    start..start + 2
}

/// One character per byte: printable ASCII as itself, everything else `.`.
pub fn ascii_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..=0x7e).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

/// What a byte is, for coloring the hex view.
pub fn byte_class(b: u8, in_content: bool, kind: Kind, field: Option<usize>, alt: bool) -> Class {
    if !in_content {
        return Class::LineEnd;
    }
    if field.is_none() && matches!(b, 0x1c | b',' | b'=') {
        return Class::Separator;
    }
    match b {
        b'"' => Class::Quote,
        0x00..=0x1f | 0x7f => Class::Control,
        0x80..=0xff => Class::NonAscii,
        _ if kind == Kind::Blank => Class::Muted,
        _ if kind.is_record() && field == Some(0) => Class::RowType,
        _ if kind == Kind::Text => Class::Prose,
        _ if alt => Class::TextAlt,
        _ => Class::Text,
    }
}

// ---------------------------------------------------------------------------
// Jump box
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Jump {
    /// 1-based line number.
    Line(usize),
    Offset(u64),
    /// Record number (0 is the header, 1 the cover).
    Record(u64),
    /// Text to search for.
    Find(String),
    /// A word that's a row type if the filing has one by that name, else text.
    Word(String),
}

/// Read what was typed in the jump box:
///
/// - `120` or `:120`: line 120
/// - `0x1F2A`, `@8000`, `@0x1F2A`: byte offset
/// - `#42`: record 42 (0 is the header, 1 the cover)
/// - `/text` or `"text"`: find text
/// - `SA11AI`: the next row of that type (or else find it as text)
pub fn parse_jump(input: &str) -> Option<Jump> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let hex = |h: &str| u64::from_str_radix(h, 16).ok();
    if let Some(rest) = s.strip_prefix('/') {
        return (!rest.is_empty()).then(|| Jump::Find(rest.to_owned()));
    }
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        return Some(Jump::Find(s[1..s.len() - 1].to_owned()));
    }
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return hex(h).map(Jump::Offset).or(Some(Jump::Find(s.to_owned())));
    }
    if let Some(rest) = s.strip_prefix('@') {
        let rest = rest.trim();
        let n = match rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            Some(h) => hex(h),
            None => rest.replace(',', "").parse().ok(),
        };
        return n.map(Jump::Offset);
    }
    if let Some(rest) = s.strip_prefix('#') {
        return rest.trim().replace(',', "").parse().ok().map(Jump::Record);
    }
    let digits = s.strip_prefix(':').unwrap_or(s).replace(',', "");
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return digits.parse().ok().map(Jump::Line);
    }
    Some(Jump::Word(s.to_owned()))
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

pub fn read_date(v: &str) -> Option<String> {
    if v.len() != 8 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let date = jiff::civil::Date::new(
        v[..4].parse().ok()?,
        v[4..6].parse().ok()?,
        v[6..].parse().ok()?,
    )
    .ok()?;
    Some(date.strftime("%B %-d, %Y").to_string())
}

pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn line_digits(lines: usize) -> usize {
    let commas = lines.max(1).ilog10() as usize / 3;
    (lines.max(1).ilog10() as usize + 1 + commas).max(4)
}

pub fn offset_digits(len: u64) -> usize {
    ((64 - len.max(1).leading_zeros() as usize).div_ceil(4)).max(8)
}

pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::index::split;
    use fec_parser::Delimiter;

    fn draw(content: &[u8], kind: Kind, d: Delimiter, eol: Eol) -> DrawnLine {
        let fields = split(content, kind, d);
        draw_line(content, eol, kind, &fields)
    }

    fn classes(l: &DrawnLine) -> Vec<(&str, Class)> {
        l.runs
            .iter()
            .map(|(r, c)| (&l.text[r.clone()], *c))
            .collect()
    }

    #[test]
    fn draws_an_fs_record() {
        let l = draw(
            b"SA11AI\x1c\"Q\"\x1c\x1cA\xe9\x01",
            Kind::Record,
            Delimiter::Fs,
            Eol::CrLf,
        );
        assert_eq!(l.text, "SA11AI¦\"Q\"¦¦A\\xE9\\x01\\r\\n");
        use Class::*;
        assert_eq!(
            classes(&l),
            [
                ("SA11AI", RowType),
                ("¦", Separator),
                ("\"", Quote),
                ("Q", Text),
                ("\"", Quote),
                ("¦¦", Separator),
                ("A", Text),
                ("\\xE9", Invalid),
                ("\\x01", Control),
                ("\\r\\n", LineEnd),
            ]
        );
        assert_eq!(l.fields.len(), 4);
        assert_eq!(&l.text[l.fields[0].clone()], "SA11AI");
        assert_eq!(&l.text[l.fields[1].clone()], "\"Q\"");
        assert_eq!(l.fields[2].len(), 0);
        assert_eq!(&l.text[l.fields[3].clone()], "A\\xE9\\x01");
        // The separator after SA11AI is byte 6; its glyph is 2 bytes wide.
        assert_eq!(l.display_range(6..7), 6..8);
        let e9 = "SA11AI¦\"Q\"¦¦A".len();
        assert_eq!(l.display_range(13..14), e9..e9 + 4);
    }

    #[test]
    fn draws_utf8_and_comma_quotes() {
        let l = draw(
            "F3PN,\"Smith, Jo\",Peña".as_bytes(),
            Kind::Cover,
            Delimiter::Comma,
            Eol::Lf,
        );
        assert_eq!(l.text, "F3PN,\"Smith, Jo\",Peña\\n");
        assert_eq!(l.fields.len(), 3);
        assert_eq!(&l.text[l.fields[1].clone()], "\"Smith, Jo\"");
        assert!(
            l.runs
                .iter()
                .any(|(r, c)| *c == Class::NonAscii && &l.text[r.clone()] == "ñ")
        );
        // The comma inside quotes is text, not a separator.
        let inner_comma = l.text.find(", Jo").unwrap();
        assert!(
            l.runs
                .iter()
                .any(|(r, c)| r.contains(&inner_comma) && *c == Class::Text)
        );
    }

    #[test]
    fn draws_header_block_lines() {
        let l = draw(
            b"FEC_Ver_# = 2.02",
            Kind::HeaderBlock,
            Delimiter::Comma,
            Eol::Lf,
        );
        use Class::*;
        assert_eq!(
            classes(&l),
            [
                ("FEC_Ver_# ", Key),
                ("=", Separator),
                (" 2.02", Text),
                ("\\n", LineEnd)
            ]
        );
    }

    #[test]
    fn escapes_blank_lines() {
        let l = draw(b" \t\x0c", Kind::Blank, Delimiter::Fs, Eol::Lf);
        assert_eq!(l.text, " \\t\\x0C\\n");
        assert!(l.fields.is_empty());
    }

    #[test]
    fn caps_huge_lines() {
        let big = vec![b'x'; MAX_DRAWN + 10];
        let l = draw(&big, Kind::Text, Delimiter::Fs, Eol::None);
        assert!(l.text.ends_with("… 10 more bytes"));
        assert_eq!(l.display_range(0..big.len()), 0..MAX_DRAWN);
    }

    #[test]
    fn lays_out_hex_rows() {
        let t = hex_text(b"HDR\x1cFEC\x1c8.4\x1cX");
        assert_eq!(t, "48 44 52 1C 46 45 43 1C  38 2E 34 1C 58         ");
        assert_eq!(&t[hex_range(0)], "48");
        assert_eq!(&t[hex_range(8)], "38");
        assert_eq!(&t[hex_range(12)], "58");
        assert_eq!(ascii_text(b"HD\x1c\xe9"), "HD..");
    }

    #[test]
    fn parses_jumps() {
        assert_eq!(parse_jump(" 1,200 "), Some(Jump::Line(1200)));
        assert_eq!(parse_jump(":7"), Some(Jump::Line(7)));
        assert_eq!(parse_jump("0x1f"), Some(Jump::Offset(31)));
        assert_eq!(parse_jump("@0x1F"), Some(Jump::Offset(31)));
        assert_eq!(parse_jump("@1,024"), Some(Jump::Offset(1024)));
        assert_eq!(parse_jump("#3"), Some(Jump::Record(3)));
        assert_eq!(parse_jump("/ACT BLUE"), Some(Jump::Find("ACT BLUE".into())));
        assert_eq!(parse_jump("\"x\""), Some(Jump::Find("x".into())));
        assert_eq!(parse_jump("sa11ai"), Some(Jump::Word("sa11ai".into())));
        assert_eq!(parse_jump("0xZZ"), Some(Jump::Find("0xZZ".into())));
        assert_eq!(parse_jump("  "), None);
        assert_eq!(parse_jump("/"), None);
    }
    #[test]
    fn formats_numbers_dates_and_json() {
        assert_eq!(fmt_count(1234567), "1,234,567");
        assert_eq!(offset_digits(0xff), 8);
        assert_eq!(offset_digits(0x1_0000_0000), 9);
        assert_eq!(line_digits(5), 4);
        assert_eq!(line_digits(1_234_567), 9);
        assert_eq!(read_date("20240315").as_deref(), Some("March 15, 2024"));
        assert_eq!(read_date("2024031"), None);
        assert_eq!(json_string("a\"b\n"), "\"a\\\"b\\n\"");
    }
}
