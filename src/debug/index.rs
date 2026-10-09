//! A byte-level index of a filing: every physical line, what it is (header,
//! cover, record, text block…), how it splits into fields, and anything odd
//! about it.
//!
//! `fec_parser` reports where records start, but on 0x1C-delimited files its
//! offsets are those of the `csv` reader (blank lines and the `\n` of a CRLF
//! count toward the next record), and it says nothing about fields or the
//! lines it skips. So this re-reads the bytes itself, following the rules
//! `fec_parser` reads by:
//!
//! - Lines end at `\n`, `\r\n` or a lone `\r`. Every physical line is at most
//!   one record: 0x1C files are read with quoting off, and comma files quote
//!   only within a line.
//! - The delimiter is sniffed once, from the first non-blank line: a `/*`
//!   header block means comma, else 0x1C anywhere in the line means 0x1C.
//! - The header is either an `HDR` record or a `/* Header` … `/*` block, the
//!   cover is the next non-blank line, and `[BEGINTEXT]` … `[ENDTEXT]` lines
//!   are prose, not records.
//! - Column names come from `fec_parser::mappings` for the row type and the
//!   header's version.
//!
//! Indexing is incremental ([`Indexer::step`]) and publishes whole chunks of
//! lines, so a big filing can be browsed while the rest is still indexed.

use std::{collections::HashMap, ops::Range, sync::Arc};

use fec_parser::{
    Delimiter, HeaderStyle,
    mappings::{COLUMN_NAMES, FORM_TYPE_VERSIONS_SET, FORM_TYPES, FORM_TYPES_SET},
};

use super::data::Data;

/// Lines per published chunk. Line `i` lives in chunk `i / CHUNK_LINES`.
pub const CHUNK_LINES: usize = 1 << 16;

const FS: u8 = 0x1c;

// ---------------------------------------------------------------------------
// Lines
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Eol {
    /// The last line, with no line ending.
    None,
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    pub fn len(self) -> u64 {
        match self {
            Eol::None => 0,
            Eol::Lf | Eol::Cr => 1,
            Eol::CrLf => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Eol::None => "none",
            Eol::Lf => "LF",
            Eol::CrLf => "CRLF",
            Eol::Cr => "CR",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Empty or whitespace-only, outside a text block. Skipped by the parser.
    Blank,
    /// A line of a legacy `/* Header` … `/* End Header` block.
    HeaderBlock,
    /// The `HDR` record.
    Header,
    /// The form's cover record (`F3XN`, `F99`, …).
    Cover,
    /// An itemization or other body record (`SA11AI`, `TEXT`, `H4`, …).
    Record,
    TextBegin,
    /// Prose between `[BEGINTEXT]` and `[ENDTEXT]`.
    Text,
    TextEnd,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Blank => "blank",
            Kind::HeaderBlock => "header block",
            Kind::Header => "header",
            Kind::Cover => "cover",
            Kind::Record => "record",
            Kind::TextBegin => "text begin",
            Kind::Text => "text",
            Kind::TextEnd => "text end",
        }
    }

    /// Short tag for the gutter.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Blank => "",
            Kind::HeaderBlock | Kind::Header => "HDR",
            Kind::Cover => "CVR",
            Kind::Record => "",
            Kind::TextBegin | Kind::Text | Kind::TextEnd => "TXT",
        }
    }

    /// Whether the parser reads this line as a record (header, cover or body).
    pub fn is_record(self) -> bool {
        matches!(self, Kind::Header | Kind::Cover | Kind::Record)
    }
}

/// Things worth a second look about a line. Some are problems (`NON_UTF8`),
/// some only notable (`PADDED_TYPE` is routine in paper filings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u16);

impl Flags {
    pub const NON_UTF8: Self = Self(1 << 0);
    pub const CONTROL_BYTES: Self = Self(1 << 1);
    pub const UNMAPPED: Self = Self(1 << 2);
    pub const EXTRA_FIELDS: Self = Self(1 << 3);
    pub const UNTERMINATED_QUOTE: Self = Self(1 << 4);
    pub const STRAY_QUOTE: Self = Self(1 << 5);
    pub const ODD_EOL: Self = Self(1 << 6);
    pub const PADDED_TYPE: Self = Self(1 << 7);
    pub const UNTERMINATED_TEXT: Self = Self(1 << 8);
    pub const BOM: Self = Self(1 << 9);
    pub const TRAILING_EMPTY: Self = Self(1 << 10);

    pub const ALL: [(Self, Severity, &'static str, &'static str); 11] = [
        (
            Self::NON_UTF8,
            Severity::Warning,
            "Not UTF-8",
            "Bytes that aren't valid UTF-8 (often Latin-1); the parser replaces them with U+FFFD.",
        ),
        (
            Self::CONTROL_BYTES,
            Severity::Warning,
            "Control bytes",
            "Control characters other than tab and the delimiter.",
        ),
        (
            Self::UNMAPPED,
            Severity::Warning,
            "No column mapping",
            "fec_parser has no column names for this row type in this version.",
        ),
        (
            Self::EXTRA_FIELDS,
            Severity::Warning,
            "Extra fields",
            "More fields than the mapping has columns, with data in the extras, which have no names.",
        ),
        (
            Self::TRAILING_EMPTY,
            Severity::Notice,
            "Trailing empty fields",
            "Empty fields past the mapping's last column, usually from a delimiter at the end of the line.",
        ),
        (
            Self::UNTERMINATED_QUOTE,
            Severity::Warning,
            "Unterminated quote",
            "A quoted field never closes; it runs to the end of the line.",
        ),
        (
            Self::STRAY_QUOTE,
            Severity::Notice,
            "Stray quote",
            "A quote that doesn't wrap its whole field (`O\"BRIEN`); kept as written.",
        ),
        (
            Self::ODD_EOL,
            Severity::Notice,
            "Mixed line endings",
            "This line ends differently from the first line.",
        ),
        (
            Self::PADDED_TYPE,
            Severity::Notice,
            "Padded row type",
            "Whitespace around the row type, which the parser trims (usual in paper filings).",
        ),
        (
            Self::UNTERMINATED_TEXT,
            Severity::Warning,
            "Unterminated text block",
            "[BEGINTEXT] with no [ENDTEXT]: the parser stops reading here.",
        ),
        (
            Self::BOM,
            Severity::Notice,
            "Byte order mark",
            "The file starts with a UTF-8 BOM, which the parser skips.",
        ),
    ];

    pub fn empty() -> Self {
        Self(0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// The set flags, with how serious each is and what it means.
    pub fn iter(self) -> impl Iterator<Item = (Severity, &'static str, &'static str)> {
        Self::ALL
            .into_iter()
            .filter(move |(f, ..)| self.contains(*f))
            .map(|(_, s, label, about)| (s, label, about))
    }

    pub fn severity(self) -> Option<Severity> {
        self.iter().map(|(s, ..)| s).max()
    }
}

impl std::ops::BitOr for Flags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Notice,
    Warning,
}

/// Sentinel for [`Line::row_type`] on lines without one.
pub const NO_TYPE: u16 = u16::MAX;

/// One physical line. 24 bytes, so a few million fit comfortably.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    /// Byte offset of the line's first byte.
    pub start: u64,
    /// Length without the line ending.
    pub len: u32,
    pub eol: Eol,
    pub kind: Kind,
    pub flags: Flags,
    /// Field count, saturating.
    pub fields: u16,
    /// Index into [`IndexState::types`], or [`NO_TYPE`].
    pub row_type: u16,
}

impl Line {
    pub fn content(&self) -> Range<u64> {
        self.start..self.start + self.len as u64
    }

    pub fn end(&self) -> u64 {
        self.start + self.len as u64 + self.eol.len()
    }
}

// ---------------------------------------------------------------------------
// Row types and their mappings
// ---------------------------------------------------------------------------

/// How `fec_parser::mappings` names a row type's columns, and why.
#[derive(Clone, Debug)]
pub struct Mapping {
    pub columns: &'static [String],
    /// The `mappings2.json` form key that matched, e.g. `^sa[^3]`.
    pub form_pattern: &'static str,
    /// Its version key that matched, e.g. `^(?:8\.5|8\.4)`.
    pub version_pattern: String,
}

/// Look up a row type's columns the way `column_names_for_field` does, keeping
/// which patterns matched.
pub fn resolve_mapping(row_type: &str, version: &str) -> Result<Mapping, String> {
    let form_ix = FORM_TYPES_SET
        .matches(row_type)
        .iter()
        .next()
        .ok_or_else(|| format!("No mapping matches row type {row_type:?}"))?;
    let versions = FORM_TYPE_VERSIONS_SET
        .get(form_ix)
        .ok_or_else(|| format!("No version table for {row_type:?}"))?;
    let version_ix = versions.matches(version).iter().next().ok_or_else(|| {
        format!(
            "{} has no layout for version {version:?}",
            FORM_TYPES[form_ix]
        )
    })?;
    let columns = COLUMN_NAMES
        .get(form_ix)
        .and_then(|v| v.get(version_ix))
        .ok_or_else(|| format!("No columns for {row_type:?} in version {version:?}"))?;
    if columns.is_empty() {
        return Err(format!(
            "{} lists no columns for version {version:?}",
            FORM_TYPES[form_ix]
        ));
    }
    Ok(Mapping {
        columns,
        form_pattern: FORM_TYPES[form_ix],
        version_pattern: versions.patterns()[version_ix].clone(),
    })
}

#[derive(Clone, Debug)]
pub struct TypeInfo {
    /// Trimmed, as written (case kept).
    pub name: String,
    pub count: usize,
    pub first_line: usize,
    pub mapping: Result<Mapping, String>,
}

// ---------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quote {
    None,
    /// Quotes wrap the whole field, and are stripped.
    Wrapped,
    /// An opening quote with no close; the field runs to the end of the line.
    Unterminated,
    /// A quote somewhere else, kept as written.
    Stray,
}

/// A field's raw bytes within its line (quotes included).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub span: Range<usize>,
    pub quote: Quote,
}

/// Split a line's content (no line ending) into fields the way `fec_parser`
/// reads a line of this kind.
pub fn split(content: &[u8], kind: Kind, delimiter: Delimiter) -> Vec<Field> {
    let mut out = Vec::new();
    match kind {
        Kind::Blank => {}
        Kind::Text => out.push(Field {
            span: 0..content.len(),
            quote: Quote::None,
        }),
        Kind::HeaderBlock => match content.iter().position(|&b| b == b'=') {
            Some(eq) => {
                out.push(Field {
                    span: 0..eq,
                    quote: Quote::None,
                });
                out.push(Field {
                    span: eq + 1..content.len(),
                    quote: Quote::None,
                });
            }
            None => out.push(Field {
                span: 0..content.len(),
                quote: Quote::None,
            }),
        },
        _ => split_delimited(content, delimiter, |f| out.push(f)),
    }
    out
}

/// Split a record line on `delimiter`, reporting each field's span.
///
/// 0x1C: a plain split (the parser reads these with quoting off), with a
/// field quoted end to end counted as [`Quote::Wrapped`].
///
/// Comma: csv-core's rules within one line. A quote opens only at the start
/// of a field, `""` inside is a literal quote, bytes after the closing quote
/// run on to the next comma, and an unclosed quote ends at the end of the
/// line.
pub fn split_delimited(content: &[u8], delimiter: Delimiter, mut emit: impl FnMut(Field)) {
    if content.is_empty() {
        return;
    }
    match delimiter {
        Delimiter::Fs => {
            let mut start = 0;
            for end in memchr::memchr_iter(FS, content).chain([content.len()]) {
                let raw = &content[start..end];
                let quote = if raw.len() >= 2 && raw[0] == b'"' && raw[raw.len() - 1] == b'"' {
                    Quote::Wrapped
                } else if raw.contains(&b'"') {
                    Quote::Stray
                } else {
                    Quote::None
                };
                emit(Field {
                    span: start..end,
                    quote,
                });
                start = end + 1;
            }
        }
        Delimiter::Comma => {
            let n = content.len();
            let mut start = 0;
            loop {
                let mut i = start;
                let mut quote = Quote::None;
                if content.get(i) == Some(&b'"') {
                    i += 1;
                    quote = Quote::Unterminated;
                    while i < n {
                        if content[i] == b'"' {
                            if content.get(i + 1) == Some(&b'"') {
                                i += 2;
                                continue;
                            }
                            i += 1;
                            quote = Quote::Wrapped;
                            break;
                        }
                        i += 1;
                    }
                }
                let rest_start = i;
                while i < n && content[i] != b',' {
                    i += 1;
                }
                // Bytes after a closing quote (`"X" Y`), or a quote inside an
                // unquoted field (`O"BRIEN`).
                if (quote == Quote::Wrapped && i > rest_start)
                    || (quote == Quote::None && content[start..i].contains(&b'"'))
                {
                    quote = Quote::Stray;
                }
                emit(Field {
                    span: start..i,
                    quote,
                });
                if i >= n {
                    break;
                }
                start = i + 1;
                if start == n {
                    // A trailing comma ends with an empty field.
                    emit(Field {
                        span: n..n,
                        quote: Quote::None,
                    });
                    break;
                }
            }
        }
    }
}

/// A field's value as the parser hands it out: quotes stripped, `""` read as
/// `"`, invalid UTF-8 replaced.
pub fn field_value(raw: &[u8], quote: Quote, delimiter: Delimiter) -> String {
    let bytes: Vec<u8> = match (delimiter, quote) {
        (Delimiter::Fs, Quote::Wrapped) => unescape(&raw[1..raw.len() - 1]),
        (Delimiter::Fs, _) => raw.to_vec(),
        (Delimiter::Comma, Quote::None) => raw.to_vec(),
        (Delimiter::Comma, _) if raw.first() == Some(&b'"') => {
            // Quoted part (to the closing quote), then whatever follows as is.
            let mut out = Vec::with_capacity(raw.len());
            let mut i = 1;
            while i < raw.len() {
                if raw[i] == b'"' {
                    if raw.get(i + 1) == Some(&b'"') {
                        out.push(b'"');
                        i += 2;
                        continue;
                    }
                    out.extend_from_slice(&raw[i + 1..]);
                    break;
                }
                out.push(raw[i]);
                i += 1;
            }
            out
        }
        (Delimiter::Comma, _) => raw.to_vec(),
    };
    String::from_utf8_lossy(&bytes).into_owned()
}

fn unescape(inner: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        out.push(inner[i]);
        if inner[i] == b'"' && inner.get(i + 1) == Some(&b'"') {
            i += 1;
        }
        i += 1;
    }
    out
}

/// `[BEGINTEXT]` / `[BEGIN TEXT]`, any case, surrounding whitespace ignored.
pub fn is_begin_text(field: &[u8]) -> bool {
    is_marker(field, b"BEGIN")
}

pub fn is_end_text(field: &[u8]) -> bool {
    is_marker(field, b"END")
}

fn is_marker(field: &[u8], word: &[u8]) -> bool {
    let f = field.trim_ascii();
    let Some(inner) = f.strip_prefix(b"[").and_then(|f| f.strip_suffix(b"]")) else {
        return false;
    };
    if inner.len() < word.len() || !inner[..word.len()].eq_ignore_ascii_case(word) {
        return false;
    }
    let rest = &inner[word.len()..];
    let rest = rest.strip_prefix(b" ").unwrap_or(rest);
    rest.eq_ignore_ascii_case(b"TEXT")
}

/// The first field's raw bytes, without splitting the whole line.
fn first_field(content: &[u8], delimiter: Delimiter) -> &[u8] {
    let mut first = 0..0;
    let mut seen = false;
    // Splitting stops early only for 0x1C; comma lines are short enough.
    match delimiter {
        Delimiter::Fs => {
            let end = memchr::memchr(FS, content).unwrap_or(content.len());
            return &content[..end];
        }
        Delimiter::Comma => split_delimited(content, delimiter, |f| {
            if !seen {
                first = f.span;
                seen = true;
            }
        }),
    }
    &content[first]
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// What the first lines say about the whole file.
#[derive(Clone, Debug)]
pub struct FileFacts {
    pub delimiter: Delimiter,
    pub style: HeaderStyle,
    /// The header's version, trimmed (`8.4`, `P3.4`, `2.02`); empty if none.
    pub version: String,
    pub bom: bool,
    /// The first line's ending, which [`Flags::ODD_EOL`] compares against.
    pub eol: Eol,
}

#[derive(Default)]
pub struct IndexState {
    pub facts: Option<FileFacts>,
    pub chunks: Vec<Arc<[Line]>>,
    /// Lines in `chunks`.
    pub lines: usize,
    /// Records (header, cover and body) before each chunk, for [`Self::record_number`].
    pub records_before: Vec<u64>,
    pub types: Vec<TypeInfo>,
    /// Lines with a warning, in order.
    pub warnings: Vec<usize>,
    /// Lines carrying each of [`Flags::ALL`], in that order.
    pub flag_counts: [usize; Flags::ALL.len()],
    /// Line counts per kind and per line ending.
    pub kinds: HashMap<Kind, usize>,
    pub eols: HashMap<Eol, usize>,
    pub bytes_scanned: u64,
    pub done: bool,
    /// Flags found after a line was published (an unterminated text block).
    pub late_flags: HashMap<usize, Flags>,
}

impl IndexState {
    pub fn line(&self, ix: usize) -> Option<Line> {
        let mut line = *self.chunks.get(ix / CHUNK_LINES)?.get(ix % CHUNK_LINES)?;
        if let Some(extra) = self.late_flags.get(&ix) {
            line.flags.insert(*extra);
        }
        Some(line)
    }

    /// The line holding byte `offset` (a line ending belongs to its line).
    pub fn line_at(&self, offset: u64) -> Option<usize> {
        if self.lines == 0 {
            return None;
        }
        let chunk = self
            .chunks
            .partition_point(|c| c.first().is_some_and(|l| l.start <= offset))
            .checked_sub(1)?;
        let lines = &self.chunks[chunk];
        let i = lines
            .partition_point(|l| l.start <= offset)
            .checked_sub(1)?;
        let ix = chunk * CHUNK_LINES + i;
        (offset < lines[i].end()).then_some(ix)
    }

    /// 0-based position of a record line among the records the parser reads
    /// (0 is the header, 1 the cover), or `None` for other lines.
    pub fn record_number(&self, ix: usize) -> Option<u64> {
        let line = self.line(ix)?;
        if line.kind == Kind::HeaderBlock {
            return Some(0);
        }
        if !line.kind.is_record() {
            return None;
        }
        let chunk = ix / CHUNK_LINES;
        let before = self.chunks[chunk][..ix % CHUNK_LINES]
            .iter()
            .filter(|l| l.kind.is_record())
            .count() as u64;
        let header_block = self.kinds.get(&Kind::HeaderBlock).is_some_and(|&n| n > 0);
        Some(self.records_before[chunk] + before + header_block as u64)
    }

    /// The line of record number `n` (see [`Self::record_number`]).
    pub fn record_line(&self, n: u64) -> Option<usize> {
        let header_block = self.kinds.get(&Kind::HeaderBlock).is_some_and(|&n| n > 0);
        if header_block && n == 0 {
            return self.find_line(0, |l| l.kind == Kind::HeaderBlock);
        }
        let n = n - header_block as u64;
        let chunk = self
            .records_before
            .partition_point(|&b| b <= n)
            .checked_sub(1)?;
        let mut seen = self.records_before[chunk];
        for (i, l) in self.chunks[chunk].iter().enumerate() {
            if l.kind.is_record() {
                if seen == n {
                    return Some(chunk * CHUNK_LINES + i);
                }
                seen += 1;
            }
        }
        None
    }

    /// The first line at or after `from` that matches.
    pub fn find_line(&self, from: usize, f: impl Fn(&Line) -> bool) -> Option<usize> {
        (from..self.lines).find(|&i| self.line(i).is_some_and(|l| f(&l)))
    }

    /// The last line before `before` that matches.
    pub fn rfind_line(&self, before: usize, f: impl Fn(&Line) -> bool) -> Option<usize> {
        (0..before.min(self.lines))
            .rev()
            .find(|&i| self.line(i).is_some_and(|l| f(&l)))
    }

    pub fn type_info(&self, line: &Line) -> Option<&TypeInfo> {
        self.types.get(line.row_type as usize)
    }
}

/// Builds an [`IndexState`] a slice at a time.
pub struct Indexer {
    data: Data,
    state: Arc<std::sync::Mutex<IndexState>>,
    pos: u64,
    facts: Option<FileFacts>,
    phase: Phase,
    in_text: Option<usize>,
    chunk: Vec<Line>,
    lines: usize,
    records: u64,
    types: Vec<TypeInfo>,
    type_ids: HashMap<Vec<u8>, u16>,
    /// Warning lines not yet published.
    warnings: Vec<usize>,
    flag_counts: [usize; Flags::ALL.len()],
    kinds: HashMap<Kind, usize>,
    eols: HashMap<Eol, usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    HeaderBlock,
    Cover,
    Body,
}

impl Indexer {
    pub fn new(data: Data, state: Arc<std::sync::Mutex<IndexState>>) -> Self {
        Self {
            data,
            state,
            pos: 0,
            facts: None,
            phase: Phase::Start,
            in_text: None,
            chunk: Vec::with_capacity(CHUNK_LINES),
            lines: 0,
            records: 0,
            types: Vec::new(),
            type_ids: HashMap::new(),
            warnings: Vec::new(),
            flag_counts: [0; Flags::ALL.len()],
            kinds: HashMap::new(),
            eols: HashMap::new(),
        }
    }

    /// Index up to `max_lines` more lines. Returns `false` once the whole
    /// file is indexed (and published).
    pub fn step(&mut self, max_lines: usize) -> bool {
        let data = self.data.clone();
        let bytes = data.as_slice();
        if self.facts.is_none() {
            self.facts = Some(sniff(bytes));
            if self.facts.as_ref().is_some_and(|f| f.bom) {
                self.pos = 3;
            }
        }
        for _ in 0..max_lines {
            if self.pos >= bytes.len() as u64 {
                self.finish();
                return false;
            }
            let start = self.pos as usize;
            let (len, eol) = line_extent(&bytes[start..]);
            self.pos += (len as u64) + eol.len();
            self.push_line(start as u64, &bytes[start..start + len], eol);
            if self.chunk.len() == CHUNK_LINES {
                self.publish(false);
            }
        }
        // Keep progress moving for the status bar between chunks.
        if let Ok(mut s) = self.state.lock() {
            s.bytes_scanned = self.pos;
        }
        true
    }

    fn push_line(&mut self, start: u64, content: &[u8], eol: Eol) {
        let facts = self.facts.clone().expect("sniffed before the first line");
        let delimiter = facts.delimiter;
        let ix = self.lines;
        let blank = content.trim_ascii().is_empty();

        let kind = if let Some(_begin) = self.in_text {
            if !blank && is_end_text(first_field(content, delimiter)) {
                self.in_text = None;
                Kind::TextEnd
            } else {
                Kind::Text
            }
        } else {
            match self.phase {
                Phase::HeaderBlock => {
                    if content.trim_ascii_start().starts_with(b"/*") {
                        self.phase = Phase::Cover;
                    }
                    Kind::HeaderBlock
                }
                _ if blank => Kind::Blank,
                Phase::Start => {
                    if facts.style == HeaderStyle::LegacyBlock {
                        self.phase = Phase::HeaderBlock;
                        Kind::HeaderBlock
                    } else {
                        self.phase = Phase::Cover;
                        Kind::Header
                    }
                }
                Phase::Cover => {
                    self.phase = Phase::Body;
                    Kind::Cover
                }
                Phase::Body => {
                    if is_begin_text(first_field(content, delimiter)) {
                        self.in_text = Some(ix);
                        Kind::TextBegin
                    } else {
                        Kind::Record
                    }
                }
            }
        };

        let mut flags = Flags::empty();
        if ix == 0 && facts.bom {
            flags.insert(Flags::BOM);
        }
        if !content.is_ascii() && std::str::from_utf8(content).is_err() {
            flags.insert(Flags::NON_UTF8);
        }
        if content.iter().any(|&b| {
            (b < 0x20 && b != b'\t' && !(b == FS && delimiter == Delimiter::Fs)) || b == 0x7f
        }) {
            flags.insert(Flags::CONTROL_BYTES);
        }
        if eol != Eol::None && eol != facts.eol && facts.eol != Eol::None {
            flags.insert(Flags::ODD_EOL);
        }

        let mut fields = 0usize;
        let mut row_type = NO_TYPE;
        if kind.is_record() {
            let mut first: Option<Range<usize>> = None;
            let mut spans = Vec::new();
            split_delimited(content, delimiter, |f| {
                fields += 1;
                spans.push(f.span.clone());
                match f.quote {
                    Quote::Unterminated => flags.insert(Flags::UNTERMINATED_QUOTE),
                    Quote::Stray => flags.insert(Flags::STRAY_QUOTE),
                    _ => {}
                }
                if first.is_none() {
                    first = Some(f.span);
                }
            });
            if let Some(span) = first {
                let raw = &content[span];
                // Row types are rarely quoted, but HDR sometimes is.
                let unquoted = if raw.len() >= 2 && raw[0] == b'"' && raw[raw.len() - 1] == b'"' {
                    &raw[1..raw.len() - 1]
                } else {
                    raw
                };
                let trimmed = unquoted.trim_ascii();
                if trimmed.len() != unquoted.len() {
                    flags.insert(Flags::PADDED_TYPE);
                }
                row_type = self.intern(trimmed, kind, ix, &facts.version);
                let info = &self.types[row_type as usize];
                match &info.mapping {
                    Ok(m) if fields > m.columns.len() => {
                        let data = spans[m.columns.len()..].iter().any(|s| {
                            let raw = content[s.clone()].trim_ascii();
                            !raw.is_empty() && raw != b"\"\""
                        });
                        flags.insert(if data {
                            Flags::EXTRA_FIELDS
                        } else {
                            Flags::TRAILING_EMPTY
                        });
                    }
                    Ok(_) => {}
                    Err(_) => flags.insert(Flags::UNMAPPED),
                }
            }
            self.records += 1;
        } else if !matches!(kind, Kind::Blank) {
            fields = split(content, kind, delimiter).len();
        }

        let line = Line {
            start,
            len: content.len().min(u32::MAX as usize) as u32,
            eol,
            kind,
            flags,
            fields: fields.min(u16::MAX as usize) as u16,
            row_type,
        };
        *self.kinds.entry(kind).or_default() += 1;
        *self.eols.entry(eol).or_default() += 1;
        if !flags.is_empty() {
            self.count_flags(flags);
        }
        if flags.severity() == Some(Severity::Warning) {
            self.warnings.push(ix);
        }
        self.chunk.push(line);
        self.lines += 1;
    }

    fn intern(&mut self, name: &[u8], kind: Kind, line: usize, version: &str) -> u16 {
        if let Some(&id) = self.type_ids.get(name) {
            self.types[id as usize].count += 1;
            return id;
        }
        if self.types.len() >= NO_TYPE as usize {
            return NO_TYPE;
        }
        let name_str = String::from_utf8_lossy(name).into_owned();
        let lookup = if kind == Kind::Header {
            "hdr"
        } else {
            &name_str
        };
        let id = self.types.len() as u16;
        self.types.push(TypeInfo {
            mapping: resolve_mapping(lookup, version),
            name: name_str,
            count: 1,
            first_line: line,
        });
        self.type_ids.insert(name.to_vec(), id);
        id
    }

    fn count_flags(&mut self, flags: Flags) {
        for (i, (f, ..)) in Flags::ALL.iter().enumerate() {
            if flags.contains(*f) {
                self.flag_counts[i] += 1;
            }
        }
    }

    fn finish(&mut self) {
        let unterminated = self.in_text.take();
        if unterminated.is_some() {
            self.count_flags(Flags::UNTERMINATED_TEXT);
        }
        self.publish(true);
        if let Some(begin) = unterminated {
            let mut s = self.state.lock().expect("index lock poisoned");
            s.late_flags.insert(begin, Flags::UNTERMINATED_TEXT);
            let at = s.warnings.partition_point(|&w| w < begin);
            if s.warnings.get(at) != Some(&begin) {
                s.warnings.insert(at, begin);
            }
        }
    }

    fn publish(&mut self, done: bool) {
        let chunk = std::mem::replace(&mut self.chunk, Vec::with_capacity(CHUNK_LINES));
        let mut s = self.state.lock().expect("index lock poisoned");
        if !chunk.is_empty() {
            let records_in_chunk = chunk.iter().filter(|l| l.kind.is_record()).count() as u64;
            let before = self.records - records_in_chunk;
            s.records_before.push(before);
            s.lines += chunk.len();
            s.chunks.push(chunk.into());
        }
        s.facts = self.facts.clone();
        s.types = self.types.clone();
        s.warnings.append(&mut self.warnings);
        s.flag_counts = self.flag_counts;
        s.kinds = self.kinds.clone();
        s.eols = self.eols.clone();
        s.bytes_scanned = self.pos;
        s.done = done;
    }
}

/// Length of the first line of `bytes` and how it ends.
fn line_extent(bytes: &[u8]) -> (usize, Eol) {
    match memchr::memchr2(b'\n', b'\r', bytes) {
        None => (bytes.len(), Eol::None),
        Some(i) if bytes[i] == b'\n' => (i, Eol::Lf),
        Some(i) if bytes.get(i + 1) == Some(&b'\n') => (i, Eol::CrLf),
        Some(i) => (i, Eol::Cr),
    }
}

/// Read the delimiter, header style and version from the first lines.
pub fn sniff(bytes: &[u8]) -> FileFacts {
    let bom = bytes.starts_with(b"\xEF\xBB\xBF");
    let body = if bom { &bytes[3..] } else { bytes };
    let (first_len, first_eol) = line_extent(body);

    // The first non-blank line decides the format.
    let mut rest = body;
    let mut first = &body[..0];
    while !rest.is_empty() {
        let (len, eol) = line_extent(rest);
        let line = &rest[..len];
        rest = &rest[len + eol.len() as usize..];
        if !line.trim_ascii().is_empty() {
            first = line;
            break;
        }
    }

    let eol = if first_len == body.len() {
        Eol::None
    } else {
        first_eol
    };

    if first.trim_ascii_start().starts_with(b"/*") {
        return FileFacts {
            delimiter: Delimiter::Comma,
            style: HeaderStyle::LegacyBlock,
            version: legacy_block_version(rest).unwrap_or_default(),
            bom,
            eol,
        };
    }
    let delimiter = if first.contains(&FS) {
        Delimiter::Fs
    } else {
        Delimiter::Comma
    };
    let mut values = Vec::new();
    split_delimited(first, delimiter, |f| {
        values.push(field_value(&first[f.span.clone()], f.quote, delimiter))
    });
    let at = |i: usize| {
        values
            .get(i)
            .map(|v| v.trim().to_owned())
            .unwrap_or_default()
    };
    let field1 = at(1);
    let paper = field1.len() >= 2
        && field1.as_bytes()[0].eq_ignore_ascii_case(&b'P')
        && field1.as_bytes()[1].is_ascii_digit();
    FileFacts {
        delimiter,
        style: if paper {
            HeaderStyle::Paper
        } else {
            HeaderStyle::Hdr
        },
        version: if paper { field1 } else { at(2) },
        bom,
        eol,
    }
}

/// `FEC_Ver_# = 2.02` from the lines of a `/* Header` block.
fn legacy_block_version(mut rest: &[u8]) -> Option<String> {
    while !rest.is_empty() {
        let (len, eol) = line_extent(rest);
        let line = &rest[..len];
        rest = &rest[len + eol.len() as usize..];
        if line.trim_ascii_start().starts_with(b"/*") {
            return None;
        }
        let Some(eq) = line.iter().position(|&b| b == b'=') else {
            continue;
        };
        if line[..eq].trim_ascii().eq_ignore_ascii_case(b"FEC_Ver_#") {
            return Some(String::from_utf8_lossy(line[eq + 1..].trim_ascii()).into_owned());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// The next occurrence of `needle` at or after `from`, wrapping to the start.
/// ASCII case is ignored when `needle` has no uppercase letters.
pub fn find(bytes: &[u8], needle: &[u8], from: u64) -> Option<u64> {
    if needle.is_empty() || needle.len() > bytes.len() {
        return None;
    }
    let from = (from as usize).min(bytes.len());
    let smart_case = !needle.iter().any(u8::is_ascii_uppercase);
    let search = |hay: &[u8]| -> Option<usize> {
        if !smart_case || !needle.iter().any(u8::is_ascii_alphabetic) {
            return memchr::memmem::find(hay, needle);
        }
        let (lo, up) = (
            needle[0].to_ascii_lowercase(),
            needle[0].to_ascii_uppercase(),
        );
        memchr::memchr2_iter(lo, up, hay).find(|&i| {
            hay.get(i..i + needle.len())
                .is_some_and(|w| w.eq_ignore_ascii_case(needle))
        })
    };
    search(&bytes[from..])
        .map(|i| (from + i) as u64)
        .or_else(|| {
            let end = (from + needle.len() - 1).min(bytes.len());
            search(&bytes[..end]).map(|i| i as u64)
        })
}

/// Every occurrence of `needle` in `hay`, with [`find`]'s case rule.
pub fn find_all(hay: &[u8], needle: &[u8]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let smart_case = !needle.iter().any(u8::is_ascii_uppercase);
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let w = &hay[i..i + needle.len()];
        if (smart_case && w.eq_ignore_ascii_case(needle)) || w == needle {
            out.push(i..i + needle.len());
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(text: &[u8]) -> IndexState {
        let state = Arc::new(std::sync::Mutex::new(IndexState::default()));
        let mut ix = Indexer::new(Data::from(text.to_vec()), state.clone());
        while ix.step(3) {}
        drop(ix);
        Arc::into_inner(state).unwrap().into_inner().unwrap()
    }

    fn fs(s: &str) -> Vec<u8> {
        s.replace('|', "\x1c").into_bytes()
    }

    fn spans(content: &str, d: Delimiter) -> Vec<(String, Quote)> {
        let mut out = Vec::new();
        split_delimited(content.as_bytes(), d, |f| {
            out.push((
                field_value(&content.as_bytes()[f.span.clone()], f.quote, d),
                f.quote,
            ))
        });
        out
    }

    #[test]
    fn indexes_an_fs_filing() {
        let s = index(&fs(
            "HDR|FEC|8.4|Soft|1.0\r\nF3XN|C001|Cmte\r\n\r\nSA11AI|C001|a\r\n[BEGINTEXT]\r\nprose|not a record\r\n[ENDTEXT]\r\nZZZ|C001\r\nSA11AI|C001|b",
        ));
        let facts = s.facts.clone().unwrap();
        assert_eq!(facts.delimiter, Delimiter::Fs);
        assert_eq!(facts.style, HeaderStyle::Hdr);
        assert_eq!(facts.version, "8.4");
        assert_eq!(facts.eol, Eol::CrLf);
        let kinds: Vec<Kind> = (0..s.lines).map(|i| s.line(i).unwrap().kind).collect();
        use Kind::*;
        assert_eq!(
            kinds,
            [
                Header, Cover, Blank, Record, TextBegin, Text, TextEnd, Record, Record
            ]
        );
        // Offsets are exact, past blank lines and CRLFs alike.
        let l3 = s.line(3).unwrap();
        assert_eq!(l3.start, 40);
        assert_eq!(l3.fields, 3);
        assert_eq!(s.line(8).unwrap().eol, Eol::None);
        assert_eq!(s.line_at(40), Some(3));
        assert_eq!(s.line_at(39), Some(2));
        assert_eq!(s.line_at(10_000), None);

        assert_eq!(s.record_number(3), Some(2));
        assert_eq!(s.record_number(5), None);
        assert_eq!(s.record_number(8), Some(4));
        assert_eq!(s.record_line(4), Some(8));
        assert_eq!(s.record_line(0), Some(0));

        let zzz = s.line(7).unwrap();
        assert!(zzz.flags.contains(Flags::UNMAPPED));
        assert_eq!(s.warnings, [7]);
        let sa = s.type_info(&l3).unwrap();
        assert_eq!(sa.count, 2);
        let m = sa.mapping.as_ref().unwrap();
        assert_eq!(m.columns[0], "form_type");
        assert_eq!(m.form_pattern, "^sa[^3]");
        let hdr = s.type_info(&s.line(0).unwrap()).unwrap();
        assert!(hdr.mapping.is_ok());
    }

    #[test]
    fn indexes_a_legacy_block_filing() {
        let s = index(
            b"/* Header\nFEC_Ver_# = 2.02\nSoft_Name = X\n/* End Header\nF3PN,C001,\"Smith, Jo\"\nSA17A,C001,\"O\"\"BRIEN\"\n[BEGIN TEXT]\nhello, \"world\nEND\n",
        );
        let facts = s.facts.clone().unwrap();
        assert_eq!(facts.style, HeaderStyle::LegacyBlock);
        assert_eq!(facts.delimiter, Delimiter::Comma);
        assert_eq!(facts.version, "2.02");
        use Kind::*;
        let kinds: Vec<Kind> = (0..s.lines).map(|i| s.line(i).unwrap().kind).collect();
        assert_eq!(
            kinds,
            [
                HeaderBlock,
                HeaderBlock,
                HeaderBlock,
                HeaderBlock,
                Cover,
                Record,
                TextBegin,
                Text,
                Text
            ]
        );
        assert_eq!(s.record_number(4), Some(1));
        assert_eq!(s.record_number(5), Some(2));
        assert_eq!(s.record_line(1), Some(4));
        assert!(s.line(6).unwrap().flags.contains(Flags::UNTERMINATED_TEXT));
        assert!(s.line(5).unwrap().flags.is_empty());
    }

    #[test]
    fn sniffs_paper_and_comma_headers() {
        let f = sniff(&fs("HDR|P3.4|FEC|\nF3XN|C1\n"));
        assert_eq!((f.style, f.version.as_str()), (HeaderStyle::Paper, "P3.4"));
        let f = sniff(b"\n\"HDR\",\"FEC\",\"5.00\",\"Soft\"\n");
        assert_eq!(
            (f.delimiter, f.style, f.version.as_str()),
            (Delimiter::Comma, HeaderStyle::Hdr, "5.00")
        );
        let f = sniff(b"\xEF\xBB\xBFHDR,FEC,3.00\n");
        assert!(f.bom);
        assert_eq!(f.version, "3.00");
    }

    #[test]
    fn splits_comma_like_csv_core() {
        use Quote::*;
        let d = Delimiter::Comma;
        assert_eq!(
            spans(r#"a,"b,c",,"d""e""#, d),
            [
                ("a".into(), None),
                ("b,c".into(), Wrapped),
                ("".into(), None),
                ("d\"e".into(), Wrapped)
            ]
        );
        assert_eq!(
            spans(r#""BUD" SMITH,O"BRIEN,"open"#, d),
            [
                ("BUD SMITH".into(), Stray),
                ("O\"BRIEN".into(), Stray),
                ("open".into(), Unterminated)
            ]
        );
        assert_eq!(spans("a,", d), [("a".into(), None), ("".into(), None)]);
        assert_eq!(spans("", d), []);
    }

    #[test]
    fn splits_fs_without_quoting() {
        use Quote::*;
        let d = Delimiter::Fs;
        let line = String::from_utf8(fs(r#""HDR"|a,b|"x""y"|O"B||"#)).unwrap();
        assert_eq!(
            spans(&line, d),
            [
                ("HDR".into(), Wrapped),
                ("a,b".into(), None),
                ("x\"y".into(), Wrapped),
                ("O\"B".into(), Stray),
                ("".into(), None),
                ("".into(), None),
            ]
        );
    }

    /// Field values agree with `fec_parser::format::split_line`.
    #[test]
    fn values_match_fec_parser() {
        let lines: [(&[u8], Delimiter); 4] = [
            (br#"a,"b,c",,"d""e",x"y,"q" r"#, Delimiter::Comma),
            (b"\"open,ended", Delimiter::Comma),
            (b"\"HDR\"\x1ca,b\x1c\"x\"\"y\"\x1cO\"B\x1c", Delimiter::Fs),
            (b"SA11AI\x1c\xe9t\xe9", Delimiter::Fs),
        ];
        for (line, d) in lines {
            let ours: Vec<String> = split(line, Kind::Record, d)
                .into_iter()
                .map(|f| field_value(&line[f.span], f.quote, d))
                .collect();
            let theirs: Vec<String> = fec_parser::format::split_line(line, d)
                .unwrap()
                .unwrap()
                .iter()
                .map(str::to_owned)
                .collect();
            assert_eq!(ours, theirs, "{}", String::from_utf8_lossy(line));
        }
    }

    #[test]
    fn flags_odd_bytes() {
        let mut text = fs("HDR|FEC|8.4\nF3XN|C1\nSB23 |C1|");
        text.extend(b"\xe9\x01\r\n");
        text.extend(fs("SA11AI|C1\n"));
        let s = index(&text);
        let l = s.line(2).unwrap();
        assert!(l.flags.contains(Flags::NON_UTF8));
        assert!(l.flags.contains(Flags::CONTROL_BYTES));
        assert!(l.flags.contains(Flags::PADDED_TYPE));
        assert!(l.flags.contains(Flags::ODD_EOL));
        assert_eq!(s.type_info(&l).unwrap().name, "SB23");
        assert!(s.line(3).unwrap().flags.is_empty());
    }

    #[test]
    fn flags_extra_fields() {
        let mut row = String::from("SA11AI");
        for _ in 0..60 {
            row.push('|');
        }
        let s = index(&fs(&format!("HDR|FEC|8.4\nF3XN|C1\n{row}\n{row}x\n")));
        assert_eq!(s.line(2).unwrap().flags, Flags::TRAILING_EMPTY);
        assert_eq!(s.line(3).unwrap().flags, Flags::EXTRA_FIELDS);
        assert_eq!(s.warnings, [3]);
    }

    #[test]
    fn spans_chunks() {
        let mut text = fs("HDR|FEC|8.4\nF3XN|C1\n");
        for i in 0..CHUNK_LINES + 10 {
            text.extend(fs(&format!("SA11AI|{i}\n")));
        }
        let s = index(&text);
        assert_eq!(s.chunks.len(), 2);
        assert_eq!(s.lines, CHUNK_LINES + 12);
        let last = s.lines - 1;
        assert_eq!(s.record_number(last), Some(last as u64));
        assert_eq!(s.record_line(last as u64), Some(last));
        let l = s.line(CHUNK_LINES + 1).unwrap();
        assert_eq!(s.line_at(l.start + 2), Some(CHUNK_LINES + 1));
    }

    #[test]
    fn finds_text() {
        let hay = b"abc SA11AI def sa11ai";
        assert_eq!(find(hay, b"sa11ai", 0), Some(4));
        assert_eq!(find(hay, b"sa11ai", 5), Some(15));
        assert_eq!(find(hay, b"SA11AI", 5), Some(4)); // case-sensitive, wraps
        assert_eq!(find(hay, b"zzz", 0), None);
        assert_eq!(find_all(hay, b"sa11ai"), [4..10, 15..21]);
        let starts: Vec<usize> = find_all(hay, b"SA11AI").iter().map(|r| r.start).collect();
        assert_eq!(starts, [4]);
    }

    /// Every fixture in a directory indexes, and record positions agree with
    /// `fec_parser`'s on files where its offsets are exact (comma and LF).
    /// Point `FEC_TEST_FILES` at a directory of `.fec` files to run it.
    #[test]
    fn agrees_with_fec_parser_on_real_filings() {
        let Ok(dir) = std::env::var("FEC_TEST_FILES") else {
            return;
        };
        let mut compared = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "fec") {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let s = index(&bytes);
            let Ok(mut filing) = fec_parser::Filing::<std::fs::File>::from_path(&path) else {
                continue;
            };
            let delimiter = s.facts.as_ref().unwrap().delimiter;
            let ours: Vec<(String, Vec<String>)> = (0..s.lines)
                .filter_map(|i| s.line(i).filter(|l| l.kind == Kind::Record))
                .map(|l| {
                    let t = s.type_info(&l).map(|t| t.name.clone()).unwrap_or_default();
                    let content = &bytes[l.content().start as usize..l.content().end as usize];
                    let values = split(content, l.kind, delimiter)
                        .into_iter()
                        .map(|f| field_value(&content[f.span], f.quote, delimiter))
                        .collect();
                    (t, values)
                })
                .collect();
            let mut theirs = Vec::new();
            while let Some(Ok(row)) = filing.next_row() {
                let values = row.record.iter().map(str::to_owned).collect::<Vec<_>>();
                theirs.push((row.row_type, values));
            }
            // fec_parser stops at an unterminated text block; we keep going.
            let n = theirs.len().min(ours.len());
            assert_eq!(
                theirs.len(),
                n,
                "{}: fec_parser read more rows",
                path.display()
            );
            for (a, b) in ours[..n].iter().zip(&theirs) {
                assert_eq!(a, b, "{}", path.display());
            }
            compared += 1;
        }
        eprintln!("compared {compared} filings in {dir}");
    }
}
