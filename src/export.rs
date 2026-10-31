//! Collected records as CSV or JSON, for copying or saving.
//!
//! Columns are named for each record's own type, so a schedule whose lines
//! have different layouts (Schedule H) still lines up: CSV gets every type's
//! columns, in first-seen order, and JSON names each record's fields with its
//! own type's columns. Fields past a type's known columns are `extra_N`, as
//! in the table. Values are the file's text, trimmed, so in JSON they're all
//! strings.

use std::collections::HashMap;

use fec_parser::mappings::column_names_for_field;

use crate::filing::{CollectState, Matcher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
}

impl Format {
    pub const ALL: [Format; 2] = [Format::Csv, Format::Json];

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "JSON",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
        }
    }

    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub fn mime(self) -> &'static str {
        match self {
            Format::Csv => "text/csv",
            Format::Json => "application/json",
        }
    }
}

/// What to call an export of `matcher`'s records from filing `filing_id`,
/// e.g. `1944135-SA11AI.csv` or `1944135-schedule-a.json`.
pub fn file_name(filing_id: &str, matcher: &Matcher, format: Format) -> String {
    let what = match matcher {
        Matcher::RowType(t) => t.clone(),
        Matcher::Schedule(s) => s.label().to_lowercase().replace(' ', "-"),
    };
    let name: String = format!("{filing_id}-{what}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "{}.{}",
        name.trim_start_matches(['-', '.']),
        format.extension()
    )
}

/// Every collected row of `state` in `format`.
pub fn render(state: &CollectState, fec_version: &str, format: Format) -> String {
    let mut names = Names::new(fec_version);
    match format {
        Format::Csv => render_csv(state, &mut names),
        Format::Json => render_json(state, &mut names),
    }
}

fn render_csv(state: &CollectState, names: &mut Names) -> String {
    // Every column any row has, in first-seen order. Rows are read twice, but
    // that's cheap next to the scan that collected them.
    let mut header: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for ix in 0..state.rows {
        for name in names.for_row(&state.row(ix)) {
            if !index.contains_key(name) {
                index.insert(name.clone(), header.len());
                header.push(name.clone());
            }
        }
    }

    let mut out = csv::Writer::from_writer(Vec::new());
    // Writing to a Vec can't fail.
    out.write_record(&header).expect("csv write");
    for ix in 0..state.rows {
        let row = state.row(ix);
        let mut record = vec![""; header.len()];
        for (name, value) in names.for_row(&row).iter().zip(&row) {
            record[index[name]] = value;
        }
        out.write_record(&record).expect("csv write");
    }
    String::from_utf8(out.into_inner().expect("csv flush")).expect("csv of strings is UTF-8")
}

/// An array of objects, one per line.
fn render_json(state: &CollectState, names: &mut Names) -> String {
    let mut out = String::from("[");
    for i in 0..state.rows {
        let row = state.row(i);
        out.push_str(if i == 0 { "\n  {" } else { ",\n  {" });
        for (j, (name, value)) in names.for_row(&row).iter().zip(&row).enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            push_json_string(&mut out, name);
            out.push_str(": ");
            push_json_string(&mut out, value);
        }
        out.push('}');
    }
    out.push_str(if out.len() > 1 { "\n]\n" } else { "]\n" });
    out
}

fn push_json_string(out: &mut String, s: &str) {
    use std::fmt::Write as _;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Column names per record type, looked up once each.
struct Names {
    fec_version: String,
    by_type: HashMap<String, Vec<String>>,
}

impl Names {
    fn new(fec_version: &str) -> Self {
        Self {
            fec_version: fec_version.to_owned(),
            by_type: HashMap::new(),
        }
    }

    /// Names for each of `row`'s fields (at least as many as it has).
    fn for_row(&mut self, row: &[String]) -> &[String] {
        let row_type = row.first().map(String::as_str).unwrap_or_default();
        let names = self.by_type.entry(row_type.to_owned()).or_insert_with(|| {
            column_names_for_field(row_type, &self.fec_version)
                .cloned()
                .unwrap_or_default()
        });
        while names.len() < row.len() {
            let n = names.len();
            names.push(format!("extra_{n}"));
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filing::Schedule;

    fn state(rows: &[&[&str]]) -> CollectState {
        CollectState::from_rows(rows)
    }

    #[test]
    fn csv_quotes_and_pads() {
        let s = state(&[&["ZZ", "a,b", "say \"hi\""], &["ZZ", "c"]]);
        assert_eq!(
            render(&s, "8.4", Format::Csv),
            "extra_0,extra_1,extra_2\nZZ,\"a,b\",\"say \"\"hi\"\"\"\nZZ,c,\n"
        );
    }

    #[test]
    fn json_objects_per_row() {
        let s = state(&[&["ZZ", "a\"\\\u{1}"], &["ZZ", "b", "c"]]);
        assert_eq!(
            render(&s, "8.4", Format::Json),
            "[\n  {\"extra_0\": \"ZZ\", \"extra_1\": \"a\\\"\\\\\\u0001\"},\n  \
             {\"extra_0\": \"ZZ\", \"extra_1\": \"b\", \"extra_2\": \"c\"}\n]\n"
        );
        assert_eq!(render(&state(&[]), "8.4", Format::Json), "[]\n");
    }

    #[test]
    fn columns_named_per_record_type() {
        let sa = column_names_for_field("SA11AI", "8.4").unwrap();
        let mut row = vec!["SA11AI".to_owned(), "C00000001".to_owned()];
        row.resize(sa.len() + 1, String::new());
        let refs: Vec<&str> = row.iter().map(String::as_str).collect();
        let s = state(&[&refs, &["ZZ", "x"]]);

        let csv = render(&s, "8.4", Format::Csv);
        let mut lines = csv.lines();
        // The SA columns and SA's overflow field, then ZZ's (unknown) fields.
        assert_eq!(
            lines.next().unwrap(),
            format!("{},extra_{},extra_0,extra_1", sa.join(","), sa.len())
        );
        assert!(lines.next().unwrap().starts_with("SA11AI,C00000001,"));
        let zz = lines.next().unwrap();
        assert_eq!(zz, format!("{}ZZ,x", ",".repeat(sa.len() + 1)));

        let json = render(&s, "8.4", Format::Json);
        assert!(json.contains(&format!(
            "{{\"{}\": \"SA11AI\", \"{}\": \"C00000001\"",
            sa[0], sa[1]
        )));
    }

    #[test]
    fn file_names() {
        let a = Schedule::of("SA11AI").unwrap();
        assert_eq!(
            file_name("1944135", &Matcher::Schedule(a), Format::Csv),
            "1944135-schedule-a.csv"
        );
        assert_eq!(
            file_name("1944135", &Matcher::RowType("SC/9".into()), Format::Json),
            "1944135-SC_9.json"
        );
    }
}
