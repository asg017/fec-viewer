//! Lazy, offset-based index over a `.fec` file.
//!
//! Opening a filing parses the header + cover synchronously, then a background
//! thread walks the remaining rows with `fec_parser` and records only the byte
//! offset of each row, grouped by row type. Rows are re-parsed on demand from a
//! memory map when the UI needs them, so memory stays at ~8 bytes per row no
//! matter how large the filing is.

use std::{
    collections::HashMap,
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Context as _;
use fec_parser::{Filing, mappings::column_names_for_field, report_code_label};
use memmap2::Mmap;

/// Rows are handed to the shared state in batches to keep lock traffic low.
const FLUSH_EVERY: usize = 25_000;

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

pub struct RowGroup {
    pub row_type: String,
    pub columns: Vec<String>,
    pub offsets: Vec<u64>,
}

#[derive(Default)]
pub struct ScanState {
    pub groups: Vec<RowGroup>,
    pub bytes_scanned: u64,
    pub rows_scanned: usize,
    pub done: bool,
    pub error: Option<String>,
    pub elapsed: Duration,
}

pub struct FilingIndex {
    pub file_len: u64,
    pub summary: FilingSummary,
    pub scan: Arc<Mutex<ScanState>>,
    mmap: Mmap,
}

impl FilingIndex {
    pub fn open(path: &Path) -> anyhow::Result<Arc<Self>> {
        let started = Instant::now();
        let mut filing = Filing::<File>::from_path(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        // SAFETY: the viewer is read-only; if another process truncates the
        // file underneath us we may read garbage, which is acceptable here.
        let mmap = unsafe { Mmap::map(&file)? };

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

        let scan = Arc::new(Mutex::new(ScanState::default()));
        let fec_version = summary.fec_version.clone();
        let thread_scan = scan.clone();
        std::thread::Builder::new()
            .name("fec-scan".into())
            .spawn(move || scan_rows(&mut filing, &fec_version, &thread_scan, started))?;

        Ok(Arc::new(Self {
            file_len,
            summary,
            scan,
            mmap,
        }))
    }

    /// Re-parse the single record starting at `offset`.
    pub fn read_row(&self, offset: u64) -> Vec<String> {
        let Some(slice) = self.mmap.get(offset as usize..) else {
            return Vec::new();
        };
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(0x1c)
            .flexible(true)
            .has_headers(false)
            .from_reader(slice);
        let mut record = csv::ByteRecord::new();
        match rdr.read_byte_record(&mut record) {
            Ok(true) => record
                .iter()
                .map(|f| String::from_utf8_lossy(f).trim().to_owned())
                .collect(),
            _ => Vec::new(),
        }
    }
}

struct PendingGroup {
    max_fields: usize,
    offsets: Vec<u64>,
}

fn scan_rows(
    filing: &mut Filing<File>,
    fec_version: &str,
    scan: &Mutex<ScanState>,
    started: Instant,
) {
    let mut group_ix: HashMap<String, usize> = HashMap::new();
    let mut pending: Vec<PendingGroup> = Vec::new();
    let mut new_groups: Vec<RowGroup> = Vec::new();
    let mut since_flush = 0usize;
    let mut rows = 0usize;
    let mut last_offset;

    let flush = |pending: &mut Vec<PendingGroup>,
                 new_groups: &mut Vec<RowGroup>,
                 rows: usize,
                 last_offset: u64,
                 done: bool,
                 error: Option<String>| {
        let mut state = scan.lock().expect("scan lock poisoned");
        state.groups.append(new_groups);
        for (ix, p) in pending.iter_mut().enumerate() {
            let group = &mut state.groups[ix];
            group.offsets.append(&mut p.offsets);
            while group.columns.len() < p.max_fields {
                let n = group.columns.len();
                group.columns.push(format!("extra_{n}"));
            }
        }
        state.rows_scanned = rows;
        state.bytes_scanned = last_offset;
        state.elapsed = started.elapsed();
        state.done = done;
        state.error = error;
    };

    let mut error = None;
    while let Some(row) = filing.next_row() {
        let row = match row {
            Ok(row) => row,
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        };
        let ix = *group_ix.entry(row.row_type.clone()).or_insert_with(|| {
            let columns = column_names_for_field(&row.row_type, fec_version)
                .cloned()
                .unwrap_or_default();
            new_groups.push(RowGroup {
                row_type: row.row_type.clone(),
                columns,
                offsets: Vec::new(),
            });
            pending.push(PendingGroup {
                max_fields: 0,
                offsets: Vec::new(),
            });
            pending.len() - 1
        });
        let p = &mut pending[ix];
        p.max_fields = p.max_fields.max(row.record.len());
        p.offsets.push(row.byte_offset);
        rows += 1;
        last_offset = row.byte_offset;
        since_flush += 1;
        if since_flush >= FLUSH_EVERY {
            flush(
                &mut pending,
                &mut new_groups,
                rows,
                last_offset,
                false,
                None,
            );
            since_flush = 0;
        }
    }
    let file_len = filing.source_length as u64;
    flush(&mut pending, &mut new_groups, rows, file_len, true, error);
}
