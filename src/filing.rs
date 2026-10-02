//! An open `.fec` file.
//!
//! Opening reads only the header and cover, so a window can show up right
//! away. A background pass then counts records per record type (for the
//! sidebar), and selecting a record type or schedule rescans the file and
//! collects just those records.
//!
//! A filing is read from a [`Source`]: a path on disk (reopened for each
//! pass) or bytes already in memory (the browser demo, which has no
//! filesystem). Natively each pass runs on its own thread. In the browser,
//! which has no threads, passes run on the page's event loop in short time
//! slices so the UI keeps painting.

use std::{
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
#[cfg(target_family = "wasm")]
use web_time::Instant;

use anyhow::Context as _;
use fec_parser::{Filing, covers::Cover, report_code_label};

/// Rows per collected chunk. Chunks are published whole, so row `i` always
/// lives in chunk `i / CHUNK_ROWS`.
pub const CHUNK_ROWS: usize = 8_192;

/// How often the count pass publishes progress.
const COUNT_FLUSH_EVERY: usize = 50_000;

const READ_BUFFER: usize = 1 << 20;

pub struct FilingSummary {
    pub filing_id: String,
    pub filer_name: String,
    pub filer_id: String,
    pub form_type: String,
    /// The form-specific report type for 24/48-hour notices (F24, F5), else
    /// the report code's label.
    pub report_label: Option<&'static str>,
    pub coverage_from: Option<String>,
    pub coverage_through: Option<String>,
    pub fec_version: String,
    pub software: String,
    pub report_id: Option<String>,
    pub report_number: Option<String>,
    pub comment: Option<String>,
    pub cover_kv: Vec<(String, String)>,
    /// The typed cover, when `fec_parser` understands the form.
    pub cover: Option<Cover>,
}

impl FilingSummary {
    /// The filing's page on docquery.fec.gov, for filings with a numeric ID.
    pub fn fec_url(&self) -> Option<String> {
        let numeric =
            !self.filing_id.is_empty() && self.filing_id.bytes().all(|b| b.is_ascii_digit());
        (numeric && !self.filer_id.is_empty()).then(|| {
            format!(
                "https://docquery.fec.gov/cgi-bin/forms/{}/{}",
                self.filer_id, self.filing_id
            )
        })
    }
}

fn report_label(cover: Option<&Cover>, report_code: Option<&str>) -> Option<&'static str> {
    let form_specific = match cover {
        Some(Cover::Form24(f)) => f.report_type_label(),
        Some(Cover::Form5(f)) => f.report_type_label(),
        _ => None,
    };
    form_specific.or_else(|| match report_code_label(report_code?) {
        "[Unknown report code]" => None,
        label => Some(label),
    })
}

#[derive(Default)]
pub struct CountState {
    /// Record type → number of records, in first-seen order.
    pub types: Vec<(String, usize)>,
    pub rows: usize,
    pub bytes_scanned: u64,
    pub done: bool,
    pub error: Option<String>,
    pub elapsed: Duration,
}

/// Where a filing's bytes come from.
#[derive(Clone)]
pub enum Source {
    /// A file on disk, reopened for every pass.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    Path(PathBuf),
    /// A whole file already in memory, shared by every pass.
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    Bytes { name: String, bytes: Arc<[u8]> },
}

impl Source {
    /// What to call the filing in a window title.
    pub fn title(&self) -> String {
        match self {
            Source::Path(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            Source::Bytes { name, .. } => name.clone(),
        }
    }

    fn display(&self) -> String {
        match self {
            Source::Path(path) => path.display().to_string(),
            Source::Bytes { name, .. } => name.clone(),
        }
    }

    fn reader(&self) -> std::io::Result<Box<dyn Read + Send>> {
        Ok(match self {
            Source::Path(path) => Box::new(File::open(path)?),
            Source::Bytes { bytes, .. } => Box::new(Cursor::new(bytes.clone())),
        })
    }

    fn len(&self) -> Option<u64> {
        match self {
            Source::Path(path) => std::fs::metadata(path).map(|m| m.len()).ok(),
            Source::Bytes { bytes, .. } => Some(bytes.len() as u64),
        }
    }
}

pub struct OpenFiling {
    pub source: Source,
    pub file_len: u64,
    pub summary: FilingSummary,
    pub counts: Arc<Mutex<CountState>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for OpenFiling {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl OpenFiling {
    pub fn open(source: Source) -> anyhow::Result<Arc<Self>> {
        let (summary, file_len) = match &source {
            Source::Path(path) => Filing::<File>::from_path(path).map(summarize),
            Source::Bytes { name, bytes } => {
                let id = Path::new(name)
                    .file_stem()
                    .map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
                Filing::from_reader(Cursor::new(bytes.clone()), id, bytes.len()).map(summarize)
            }
        }
        .with_context(|| format!("Failed to read {}", source.display()))?;

        let counts = Arc::new(Mutex::new(CountState::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        spawn_pass(
            "fec-count",
            source.clone(),
            cancel.clone(),
            Counter::new(counts.clone()),
        )?;

        Ok(Arc::new(Self {
            source,
            file_len,
            summary,
            counts,
            cancel,
        }))
    }

    /// Rescan the file in the background, collecting records that match.
    pub fn collect(&self, matcher: Matcher) -> Collection {
        let state = Arc::new(Mutex::new(CollectState::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let collector = Collector::new(state.clone(), matcher);
        let spawned = spawn_pass(
            "fec-collect",
            self.source.clone(),
            cancel.clone(),
            collector,
        );
        if let Err(e) = spawned {
            let mut s = state.lock().expect("collect lock poisoned");
            s.error = Some(e.to_string());
            s.done = true;
        }
        Collection { state, cancel }
    }
}

/// The header and cover, plus the file's length.
fn summarize<R: Read>(filing: Filing<R>) -> (FilingSummary, u64) {
    let cover = &filing.cover;
    let summary = FilingSummary {
        filing_id: filing.filing_id.clone(),
        filer_name: cover.filer_name.clone(),
        filer_id: cover.filer_id.clone(),
        form_type: cover.form_type.clone(),
        report_label: report_label(cover.cover_data.as_ref(), cover.report_code.as_deref()),
        coverage_from: cover.coverage_from_date.map(|d| d.to_string()),
        coverage_through: cover.coverage_through_date.map(|d| d.to_string()),
        fec_version: filing.header.fec_version.clone(),
        software: format!(
            "{} {}",
            filing.header.software_name, filing.header.software_version
        ),
        report_id: filing.header.report_id.clone(),
        report_number: filing.header.report_number.clone(),
        comment: filing.header.comment.clone(),
        cover_kv: cover
            .cover_record_kv
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        cover: cover.cover_data.clone(),
    };
    (summary, filing.source_length as u64)
}

// ---------------------------------------------------------------------------
// Record types and schedules
// ---------------------------------------------------------------------------

/// Schedules that group several record types (usually one per line number),
/// longest prefix first.
const SCHEDULES: &[(&str, &str)] = &[
    ("SC1", "Schedule C1"),
    ("SC2", "Schedule C2"),
    ("SA", "Schedule A"),
    ("SB", "Schedule B"),
    ("SC", "Schedule C"),
    ("SD", "Schedule D"),
    ("SE", "Schedule E"),
    ("SF", "Schedule F"),
    ("SI", "Schedule I"),
    ("SL", "Schedule L"),
    ("H", "Schedule H"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Schedule(usize);

impl Schedule {
    pub fn of(row_type: &str) -> Option<Self> {
        SCHEDULES
            .iter()
            .position(|(prefix, _)| {
                row_type
                    .get(..prefix.len())
                    .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
            })
            .map(Self)
    }

    pub fn label(self) -> &'static str {
        SCHEDULES[self.0].1
    }

    /// What to call `row_type` under this schedule: the line number for
    /// `SA11AI` → `11AI`, but the whole type for `H4`.
    pub fn line_label(self, row_type: &str) -> String {
        let prefix = SCHEDULES[self.0].0;
        if prefix == "H" {
            return row_type.to_owned();
        }
        let rest = row_type[prefix.len()..].trim_start_matches('/');
        if rest.is_empty() {
            row_type.to_owned()
        } else {
            rest.to_owned()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Matcher {
    RowType(String),
    Schedule(Schedule),
}

impl Matcher {
    fn matches(&self, row_type: &str) -> bool {
        match self {
            Matcher::RowType(t) => t.eq_ignore_ascii_case(row_type),
            Matcher::Schedule(s) => Schedule::of(row_type) == Some(*s),
        }
    }
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Lines read between checks for cancellation.
const SCAN_STEP: usize = 4096;

/// Reads the records after the header and cover, skipping `[BEGINTEXT]` …
/// `[ENDTEXT]` blocks the way `fec_parser` does.
struct Scanner {
    rdr: csv::Reader<Box<dyn Read + Send>>,
    record: csv::ByteRecord,
    seen: usize,
    in_text: bool,
}

impl Scanner {
    fn new(source: &Source) -> anyhow::Result<Self> {
        let rdr = csv::ReaderBuilder::new()
            .delimiter(0x1c)
            .flexible(true)
            .has_headers(false)
            .buffer_capacity(READ_BUFFER)
            .from_reader(source.reader()?);
        Ok(Self {
            rdr,
            record: csv::ByteRecord::new(),
            seen: 0,
            in_text: false,
        })
    }

    /// Read up to `lines` more lines, handing each record to `pass` along
    /// with its byte position. Returns `false` at the end of the file.
    fn step(&mut self, lines: usize, pass: &mut impl Pass) -> anyhow::Result<bool> {
        for _ in 0..lines {
            if !self.rdr.read_byte_record(&mut self.record)? {
                return Ok(false);
            }
            self.seen += 1;
            if self.seen <= 2 {
                continue; // HDR + cover
            }
            let first = self.record.get(0).unwrap_or_default();
            if self.in_text {
                self.in_text = first != b"[ENDTEXT]";
                continue;
            }
            if first == b"[BEGINTEXT]" {
                self.in_text = true;
                continue;
            }
            let row_type = std::str::from_utf8(first).unwrap_or("").trim();
            if row_type.is_empty() {
                continue;
            }
            let pos = self.record.position().map(|p| p.byte()).unwrap_or(0);
            pass.record(row_type, &self.record, pos);
        }
        Ok(true)
    }
}

/// One scan over a filing's records, publishing into shared state.
trait Pass: Send + 'static {
    fn record(&mut self, row_type: &str, record: &csv::ByteRecord, pos: u64);
    /// Called when the scan ends, unless it was cancelled.
    fn finish(self, file_len: Option<u64>, result: anyhow::Result<()>);
}

/// Run `pass` to completion (or cancellation) on this thread.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
fn run_pass(source: &Source, cancel: &AtomicBool, mut pass: impl Pass) {
    let result = (|| {
        let mut scanner = Scanner::new(source)?;
        while !cancel.load(Ordering::Relaxed) && scanner.step(SCAN_STEP, &mut pass)? {}
        anyhow::Ok(())
    })();
    if !cancel.load(Ordering::Relaxed) {
        pass.finish(source.len(), result);
    }
}

#[cfg(not(target_family = "wasm"))]
fn spawn_pass(
    name: &str,
    source: Source,
    cancel: Arc<AtomicBool>,
    pass: impl Pass,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || run_pass(&source, &cancel, pass))?;
    Ok(())
}

/// The browser has one thread, shared with rendering and input. Scan in
/// slices of a few milliseconds and hand the thread back between them, so a
/// big filing fills in progressively instead of freezing the page.
#[cfg(target_family = "wasm")]
fn spawn_pass(
    _name: &str,
    source: Source,
    cancel: Arc<AtomicBool>,
    mut pass: impl Pass,
) -> std::io::Result<()> {
    const SLICE: Duration = Duration::from_millis(12);

    async fn scan(
        source: &Source,
        cancel: &AtomicBool,
        pass: &mut impl Pass,
    ) -> anyhow::Result<()> {
        let mut scanner = Scanner::new(source)?;
        loop {
            let slice = Instant::now();
            while slice.elapsed() < SLICE {
                if !scanner.step(256, pass)? {
                    return Ok(());
                }
            }
            yield_to_browser().await;
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
        }
    }

    wasm_bindgen_futures::spawn_local(async move {
        let result = scan(&source, &cancel, &mut pass).await;
        if !cancel.load(Ordering::Relaxed) {
            pass.finish(source.len(), result);
        }
    });
    Ok(())
}

/// Resolve on a fresh macrotask, so the browser can paint and handle input.
#[cfg(target_family = "wasm")]
async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

struct Counter {
    state: Arc<Mutex<CountState>>,
    started: Instant,
    types: Vec<(String, usize)>,
    rows: usize,
    since_flush: usize,
}

impl Counter {
    fn new(state: Arc<Mutex<CountState>>) -> Self {
        Self {
            state,
            started: Instant::now(),
            types: Vec::new(),
            rows: 0,
            since_flush: 0,
        }
    }

    fn publish(&self, pos: u64, done: bool, error: Option<String>) {
        let mut s = self.state.lock().expect("count lock poisoned");
        s.types.clear();
        s.types.extend_from_slice(&self.types);
        s.rows = self.rows;
        s.bytes_scanned = pos;
        s.elapsed = self.started.elapsed();
        s.done = done;
        s.error = error;
    }
}

impl Pass for Counter {
    fn record(&mut self, row_type: &str, _: &csv::ByteRecord, pos: u64) {
        // A handful of distinct types per filing, so a linear scan beats hashing.
        match self.types.iter_mut().find(|(t, _)| t == row_type) {
            Some((_, n)) => *n += 1,
            None => self.types.push((row_type.to_owned(), 1)),
        }
        self.rows += 1;
        self.since_flush += 1;
        if self.since_flush >= COUNT_FLUSH_EVERY {
            self.since_flush = 0;
            self.publish(pos, false, None);
        }
    }

    fn finish(self, file_len: Option<u64>, result: anyhow::Result<()>) {
        let error = result.err().map(|e| format!("{e:#}"));
        self.publish(file_len.unwrap_or(0), true, error);
    }
}

// ---------------------------------------------------------------------------
// Collected records
// ---------------------------------------------------------------------------

/// Up to [`CHUNK_ROWS`] records, each stored as its fields joined by 0x1c.
#[derive(Default)]
pub struct Chunk {
    data: Vec<u8>,
    /// Start of each row in `data`, plus one trailing end offset.
    bounds: Vec<u32>,
}

impl Chunk {
    fn push(&mut self, record: &csv::ByteRecord) {
        if self.bounds.is_empty() {
            self.bounds.push(0);
        }
        for (i, field) in record.iter().enumerate() {
            if i > 0 {
                self.data.push(0x1c);
            }
            self.data.extend_from_slice(field);
        }
        self.bounds.push(self.data.len() as u32);
    }

    fn len(&self) -> usize {
        self.bounds.len().saturating_sub(1)
    }

    pub fn row(&self, i: usize) -> Vec<String> {
        let (Some(&start), Some(&end)) = (self.bounds.get(i), self.bounds.get(i + 1)) else {
            return Vec::new();
        };
        self.data[start as usize..end as usize]
            .split(|&b| b == 0x1c)
            .map(|f| String::from_utf8_lossy(f).trim().to_owned())
            .collect()
    }
}

#[derive(Default)]
pub struct CollectState {
    pub chunks: Vec<Arc<Chunk>>,
    pub rows: usize,
    pub max_fields: usize,
    /// Record types seen among the collected rows, in first-seen order.
    pub row_types: Vec<String>,
    pub bytes_scanned: u64,
    pub done: bool,
    pub error: Option<String>,
    pub elapsed: Duration,
}

impl CollectState {
    pub fn row(&self, ix: usize) -> Vec<String> {
        self.chunks
            .get(ix / CHUNK_ROWS)
            .map(|c| c.row(ix % CHUNK_ROWS))
            .unwrap_or_default()
    }
}

/// Records being (or done being) collected. Dropping it stops the scan.
pub struct Collection {
    pub state: Arc<Mutex<CollectState>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Collection {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Collector {
    state: Arc<Mutex<CollectState>>,
    matcher: Matcher,
    started: Instant,
    chunk: Chunk,
    max_fields: usize,
    row_types: Vec<String>,
    last_pos: u64,
    seen: usize,
}

impl Collector {
    fn new(state: Arc<Mutex<CollectState>>, matcher: Matcher) -> Self {
        Self {
            state,
            matcher,
            started: Instant::now(),
            chunk: Chunk::default(),
            max_fields: 0,
            row_types: Vec::new(),
            last_pos: 0,
            seen: 0,
        }
    }

    fn publish(&mut self, pos: u64, done: bool, error: Option<String>) {
        let chunk = std::mem::take(&mut self.chunk);
        let mut s = self.state.lock().expect("collect lock poisoned");
        if chunk.len() > 0 {
            s.rows += chunk.len();
            s.chunks.push(Arc::new(chunk));
        }
        s.max_fields = self.max_fields;
        if s.row_types.len() != self.row_types.len() {
            s.row_types = self.row_types.clone();
        }
        s.bytes_scanned = pos;
        s.elapsed = self.started.elapsed();
        s.done = done;
        s.error = error;
    }
}

impl Pass for Collector {
    fn record(&mut self, row_type: &str, record: &csv::ByteRecord, pos: u64) {
        self.last_pos = pos;
        self.seen += 1;
        if self.seen.is_multiple_of(COUNT_FLUSH_EVERY) {
            self.state
                .lock()
                .expect("collect lock poisoned")
                .bytes_scanned = pos;
        }
        if !self.matcher.matches(row_type) {
            return;
        }
        if !self.row_types.iter().any(|t| t == row_type) {
            self.row_types.push(row_type.to_owned());
        }
        self.max_fields = self.max_fields.max(record.len());
        self.chunk.push(record);
        if self.chunk.len() == CHUNK_ROWS {
            self.publish(pos, false, None);
        }
    }

    fn finish(mut self, file_len: Option<u64>, result: anyhow::Result<()>) {
        let error = result.err().map(|e| format!("{e:#}"));
        self.publish(file_len.unwrap_or(self.last_pos), true, error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: char = '\x1c';

    /// A small filing with the awkward bits: a quoted field holding a newline,
    /// a `[BEGINTEXT]` block whose lines look like records, and mixed case.
    fn sample() -> tempfile_path::TempPath {
        let rows = [
            "HDR|FEC|8.4|Test|1.0",
            "F3XN|C00000001|Test Committee",
            "SA11AI|C00000001|a",
            "SA11AI|C00000001|b",
            "sa17|C00000001|c",
            "SB23|C00000001|\"two\nlines\"",
            "[BEGINTEXT]",
            "SA11AI|this is prose, not a record",
            "[ENDTEXT]",
            "SA11AI|C00000001|d|extra",
            "SC/9|C00000001|loan",
            "SC1/9|C00000001|loan terms",
            "H4|C00000001|alloc",
            "TEXT|C00000001|memo",
        ];
        let body: String = rows
            .map(|r| r.replace('|', &FS.to_string()) + "\n")
            .concat();
        tempfile_path::TempPath::with_contents(&body)
    }

    fn count_source(source: &Source) -> CountState {
        let state = Arc::new(Mutex::new(CountState::default()));
        run_pass(source, &AtomicBool::new(false), Counter::new(state.clone()));
        Arc::into_inner(state).unwrap().into_inner().unwrap()
    }

    fn collect_source(source: &Source, matcher: Matcher) -> CollectState {
        let state = Arc::new(Mutex::new(CollectState::default()));
        run_pass(
            source,
            &AtomicBool::new(false),
            Collector::new(state.clone(), matcher),
        );
        Arc::into_inner(state).unwrap().into_inner().unwrap()
    }

    fn count(path: &Path) -> CountState {
        count_source(&Source::Path(path.to_owned()))
    }

    fn collect(path: &Path, matcher: Matcher) -> CollectState {
        collect_source(&Source::Path(path.to_owned()), matcher)
    }

    /// The in-memory source the browser demo uses reads the same as a file.
    #[test]
    fn bytes_source_matches_path_source() {
        let file = sample();
        let bytes = Source::Bytes {
            name: "1234.fec".into(),
            bytes: std::fs::read(file.path()).unwrap().into(),
        };
        let (a, b) = (count(file.path()), count_source(&bytes));
        assert_eq!(a.types, b.types);
        assert_eq!((a.rows, a.bytes_scanned), (b.rows, b.bytes_scanned));

        let matcher = Matcher::RowType("SA11AI".into());
        let (a, b) = (
            collect(file.path(), matcher.clone()),
            collect_source(&bytes, matcher),
        );
        assert_eq!(a.rows, b.rows);
        assert_eq!(
            (0..a.rows).map(|i| a.row(i)).collect::<Vec<_>>(),
            (0..b.rows).map(|i| b.row(i)).collect::<Vec<_>>()
        );

        let filing = OpenFiling::open(bytes).unwrap();
        assert_eq!(filing.summary.filing_id, "1234");
        assert_eq!(filing.summary.form_type, "F3XN");
        assert_eq!(
            filing.file_len,
            std::fs::metadata(file.path()).unwrap().len()
        );
    }

    #[test]
    fn counts_record_types() {
        let file = sample();
        let c = count(file.path());
        assert!(c.done && c.error.is_none());
        assert_eq!(
            c.types,
            [
                ("SA11AI", 3),
                ("sa17", 1),
                ("SB23", 1),
                ("SC/9", 1),
                ("SC1/9", 1),
                ("H4", 1),
                ("TEXT", 1),
            ]
            .map(|(t, n)| (t.to_owned(), n))
        );
        assert_eq!(c.rows, 9);
    }

    #[test]
    fn collects_a_record_type() {
        let file = sample();
        let s = collect(file.path(), Matcher::RowType("SA11AI".into()));
        assert!(s.done && s.error.is_none());
        assert_eq!(s.rows, 3);
        assert_eq!(s.max_fields, 4);
        assert_eq!(s.row(0), ["SA11AI", "C00000001", "a"]);
        assert_eq!(s.row(2), ["SA11AI", "C00000001", "d", "extra"]);
        assert!(s.row(3).is_empty());
    }

    #[test]
    fn collects_a_schedule() {
        let file = sample();
        let schedule = Schedule::of("SA11AI").unwrap();
        let s = collect(file.path(), Matcher::Schedule(schedule));
        assert_eq!(s.rows, 4);
        assert_eq!(s.row_types, ["SA11AI", "sa17"]);

        let b = collect(file.path(), Matcher::RowType("SB23".into()));
        assert_eq!(b.row(0), ["SB23", "C00000001", "two\nlines"]);
    }

    #[test]
    fn rows_span_chunks() {
        let mut body = String::from("HDR\x1cFEC\x1c8.4\nF3XN\x1cC1\n");
        for i in 0..CHUNK_ROWS * 2 + 5 {
            body += &format!("SA11AI\x1c{i}\n");
        }
        let file = tempfile_path::TempPath::with_contents(&body);
        let s = collect(file.path(), Matcher::RowType("SA11AI".into()));
        assert_eq!(s.rows, CHUNK_ROWS * 2 + 5);
        assert_eq!(s.chunks.len(), 3);
        for ix in [0, CHUNK_ROWS - 1, CHUNK_ROWS, CHUNK_ROWS * 2 + 4] {
            assert_eq!(s.row(ix)[1], ix.to_string());
        }
    }

    #[test]
    fn groups_record_types_into_schedules() {
        let label = |t: &str| Schedule::of(t).map(|s| (s.label(), s.line_label(t)));
        assert_eq!(label("SA11AI"), Some(("Schedule A", "11AI".into())));
        assert_eq!(label("sa17"), Some(("Schedule A", "17".into())));
        assert_eq!(label("SB28A"), Some(("Schedule B", "28A".into())));
        assert_eq!(label("SC/9"), Some(("Schedule C", "9".into())));
        assert_eq!(label("SC1/9"), Some(("Schedule C1", "9".into())));
        assert_eq!(label("SC2/10"), Some(("Schedule C2", "10".into())));
        assert_eq!(label("SE"), Some(("Schedule E", "SE".into())));
        assert_eq!(label("H4"), Some(("Schedule H", "H4".into())));
        assert_eq!(label("TEXT"), None);
        assert_eq!(label("F3L"), None);
    }

    /// Counts must agree with `fec_parser` on real filings. Point
    /// `FEC_TEST_FILES` at a directory of `.fec` files to run it.
    #[test]
    fn counts_match_fec_parser() {
        let Ok(dir) = std::env::var("FEC_TEST_FILES") else {
            return;
        };
        let mut compared = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "fec") {
                continue;
            }
            let Ok(mut filing) = Filing::<File>::from_path(&path) else {
                continue;
            };
            let mut expected: Vec<(String, usize)> = Vec::new();
            while let Some(Ok(row)) = filing.next_row() {
                match expected.iter_mut().find(|(t, _)| *t == row.row_type) {
                    Some((_, n)) => *n += 1,
                    None => expected.push((row.row_type, 1)),
                }
            }
            let got = count(&path);
            assert_eq!(got.types, expected, "{}", path.display());
            compared += 1;
        }
        eprintln!("compared {compared} filings in {dir}");
    }

    /// Minimal self-deleting temp file, to avoid a dev-dependency.
    mod tempfile_path {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};

        pub struct TempPath(PathBuf);

        impl TempPath {
            pub fn with_contents(contents: &str) -> Self {
                static N: AtomicUsize = AtomicUsize::new(0);
                let path = std::env::temp_dir().join(format!(
                    "fec-viewer-test-{}-{}.fec",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::write(&path, contents).unwrap();
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempPath {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }
}
