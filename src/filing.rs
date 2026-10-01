//! An open `.fec` file.
//!
//! Opening reads only the header and cover, so a window can show up right
//! away. A background pass then counts records per record type (for the
//! sidebar), and selecting a record type or schedule rescans the file and
//! collects just those records.

use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Context as _;
use fec_parser::{Filing, report_code_label};

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
    pub report_label: Option<&'static str>,
    pub coverage_from: Option<String>,
    pub coverage_through: Option<String>,
    pub fec_version: String,
    pub software: String,
    pub cover_kv: Vec<(String, String)>,
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

pub struct OpenFiling {
    pub path: PathBuf,
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
    pub fn open(path: &Path) -> anyhow::Result<Arc<Self>> {
        let filing = Filing::<File>::from_path(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let cover = &filing.cover;
        let summary = FilingSummary {
            filing_id: filing.filing_id.clone(),
            filer_name: cover.filer_name.clone(),
            filer_id: cover.filer_id.clone(),
            form_type: cover.form_type.clone(),
            report_label: cover.report_code.as_deref().map(report_code_label),
            coverage_from: cover.coverage_from_date.map(|d| d.to_string()),
            coverage_through: cover.coverage_through_date.map(|d| d.to_string()),
            fec_version: filing.header.fec_version.clone(),
            software: format!(
                "{} {}",
                filing.header.software_name, filing.header.software_version
            ),
            cover_kv: cover
                .cover_record_kv
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };

        let counts = Arc::new(Mutex::new(CountState::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let path = path.to_owned();
            let counts = counts.clone();
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("fec-count".into())
                .spawn(move || count_records(&path, &counts, &cancel))?;
        }

        Ok(Arc::new(Self {
            path: path.to_owned(),
            file_len: filing.source_length as u64,
            summary,
            counts,
            cancel,
        }))
    }

    /// Rescan the file on a background thread, collecting records that match.
    pub fn collect(&self, matcher: Matcher) -> Collection {
        let state = Arc::new(Mutex::new(CollectState::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let path = self.path.clone();
        let thread_state = state.clone();
        let thread_cancel = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("fec-collect".into())
            .spawn(move || collect_records(&path, &matcher, &thread_state, &thread_cancel));
        if let Err(e) = spawned {
            let mut s = state.lock().expect("collect lock poisoned");
            s.error = Some(e.to_string());
            s.done = true;
        }
        Collection { state, cancel }
    }
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

/// Call `f(row_type, record, byte_position)` for every record after the
/// header and cover, skipping `[BEGINTEXT]` … `[ENDTEXT]` blocks the way
/// `fec_parser` does. Stops early (returning `Ok`) once `cancel` is set.
fn for_each_record(
    path: &Path,
    cancel: &AtomicBool,
    mut f: impl FnMut(&str, &csv::ByteRecord, u64),
) -> anyhow::Result<()> {
    let file = File::open(path)?;
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(0x1c)
        .flexible(true)
        .has_headers(false)
        .buffer_capacity(READ_BUFFER)
        .from_reader(file);
    let mut record = csv::ByteRecord::new();
    let mut seen = 0usize;
    let mut in_text = false;
    while rdr.read_byte_record(&mut record)? {
        seen += 1;
        if seen <= 2 {
            continue; // HDR + cover
        }
        if seen.is_multiple_of(4096) && cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let first = record.get(0).unwrap_or_default();
        if in_text {
            in_text = first != b"[ENDTEXT]";
            continue;
        }
        if first == b"[BEGINTEXT]" {
            in_text = true;
            continue;
        }
        let row_type = std::str::from_utf8(first).unwrap_or("").trim();
        if row_type.is_empty() {
            continue;
        }
        let pos = record.position().map(|p| p.byte()).unwrap_or(0);
        f(row_type, &record, pos);
    }
    Ok(())
}

fn count_records(path: &Path, state: &Mutex<CountState>, cancel: &AtomicBool) {
    let started = Instant::now();
    let mut types: Vec<(String, usize)> = Vec::new();
    let mut rows = 0usize;
    let mut since_flush = 0usize;
    let publish = |types: &[(String, usize)], rows, pos, done, error: Option<String>| {
        let mut s = state.lock().expect("count lock poisoned");
        s.types.clear();
        s.types.extend_from_slice(types);
        s.rows = rows;
        s.bytes_scanned = pos;
        s.elapsed = started.elapsed();
        s.done = done;
        s.error = error;
    };

    let result = for_each_record(path, cancel, |row_type, _, pos| {
        // A handful of distinct types per filing, so a linear scan beats hashing.
        match types.iter_mut().find(|(t, _)| t == row_type) {
            Some((_, n)) => *n += 1,
            None => types.push((row_type.to_owned(), 1)),
        }
        rows += 1;
        since_flush += 1;
        if since_flush >= COUNT_FLUSH_EVERY {
            since_flush = 0;
            publish(&types, rows, pos, false, None);
        }
    });
    let file_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    publish(
        &types,
        rows,
        file_len,
        true,
        result.err().map(|e| format!("{e:#}")),
    );
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

fn collect_records(
    path: &Path,
    matcher: &Matcher,
    state: &Mutex<CollectState>,
    cancel: &AtomicBool,
) {
    let started = Instant::now();
    let mut chunk = Chunk::default();
    let mut max_fields = 0usize;
    let mut row_types: Vec<String> = Vec::new();
    let mut last_pos = 0u64;
    let publish = |chunk: Option<Chunk>,
                   max_fields: usize,
                   row_types: &[String],
                   pos: u64,
                   done: bool,
                   error: Option<String>| {
        let mut s = state.lock().expect("collect lock poisoned");
        if let Some(chunk) = chunk.filter(|c| c.len() > 0) {
            s.rows += chunk.len();
            s.chunks.push(Arc::new(chunk));
        }
        s.max_fields = max_fields;
        if s.row_types.len() != row_types.len() {
            s.row_types = row_types.to_vec();
        }
        s.bytes_scanned = pos;
        s.elapsed = started.elapsed();
        s.done = done;
        s.error = error;
    };

    let mut seen = 0usize;
    let result = for_each_record(path, cancel, |row_type, record, pos| {
        last_pos = pos;
        seen += 1;
        if seen.is_multiple_of(COUNT_FLUSH_EVERY) {
            state.lock().expect("collect lock poisoned").bytes_scanned = pos;
        }
        if !matcher.matches(row_type) {
            return;
        }
        if !row_types.iter().any(|t| t == row_type) {
            row_types.push(row_type.to_owned());
        }
        max_fields = max_fields.max(record.len());
        chunk.push(record);
        if chunk.len() == CHUNK_ROWS {
            publish(
                Some(std::mem::take(&mut chunk)),
                max_fields,
                &row_types,
                pos,
                false,
                None,
            );
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    let file_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(last_pos);
    publish(
        Some(chunk),
        max_fields,
        &row_types,
        file_len,
        true,
        result.err().map(|e| format!("{e:#}")),
    );
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
        let body: String = rows.map(|r| r.replace('|', &FS.to_string()) + "\n").concat();
        tempfile_path::TempPath::with_contents(&body)
    }

    fn count(path: &Path) -> CountState {
        let state = Mutex::new(CountState::default());
        count_records(path, &state, &AtomicBool::new(false));
        state.into_inner().unwrap()
    }

    fn collect(path: &Path, matcher: Matcher) -> CollectState {
        let state = Mutex::new(CollectState::default());
        collect_records(path, &matcher, &state, &AtomicBool::new(false));
        state.into_inner().unwrap()
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
