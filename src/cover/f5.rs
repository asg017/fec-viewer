//! Form 5 (independent expenditures by persons other than political
//! committees) cover: the filer (Lines 1–3), which report this is (Lines 4–5)
//! and the two page-1 totals (Lines 6–7). One record serves both quarterly /
//! year-end reports and 24- / 48-hour reports.

use fec_parser::covers::{Form5, election_code_label};
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form5, cx: &App) -> Vec<AnyElement> {
    vec![
        banner(
            "Form 5 · Independent Expenditures",
            report_text(f).unwrap_or_else(|| "Report of Independent Expenditures".into()),
            banner_tags(f, cx),
            amendment_text(f.is_amendment(), f.original_amendment_date, "report")
                .or_else(|| period_text(f.coverage_from_date, f.coverage_through_date)),
            cx,
        ),
        stats(
            vec![
                Stat::new("6. Total contributions received", f.total_contributions),
                Stat::new(
                    "7. Total independent expenditures",
                    f.total_independent_expenditures,
                )
                .emphasis(),
            ],
            cx,
        ),
        filer(f, cx),
        report(f, cx),
    ]
}

/// Line 4(a): the quarterly report code, or the 24/48-hour code.
fn report_text(f: &Form5) -> Option<String> {
    f.report_code
        .as_deref()
        .map(|code| report_code_text(code, f.report_code_label()))
        .or_else(|| {
            f.report_type
                .as_deref()
                .map(|code| code_with_label(code, f.report_type_label()))
        })
}

fn banner_tags(f: &Form5, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    match f.report_type.as_deref().map(str::trim) {
        Some("24") => tags.push(tag("24-HOUR", Tone::Danger, cx)),
        Some("48") => tags.push(tag("48-HOUR", Tone::Warning, cx)),
        _ => {}
    }
    if f.form_type.ends_with(['T', 't']) {
        tags.push(tag("TERMINATION", Tone::Danger, cx));
    }
    tags
}

/// Lines 1–3: who is filing.
fn filer(f: &Form5, cx: &App) -> AnyElement {
    let id = (!f.filer_committee_id.trim().is_empty()).then_some(f.filer_committee_id.as_str());
    let fields = Fields::new()
        .element("Filer", with_aside(f.filer_name(), id, cx))
        .opt("Filer type", f.entity_type.as_deref().map(entity_label))
        .address("Address", &f.address, f.change_of_address, cx)
        .opt("Occupation", f.individual_occupation.clone())
        .opt("Employer", f.individual_employer.clone())
        .opt(
            "Qualified nonprofit",
            f.qualified_nonprofit.as_deref().map(yes_no_code),
        );
    section_with("Filer", Some("Lines 1–3"), vec![fields.render(cx)], cx)
}

/// Lines 4–5, plus the election on v3/v5.x filings, and who completed it.
fn report(f: &Form5, cx: &App) -> AnyElement {
    let fields = Fields::new()
        .opt("Report", report_text(f))
        .date("Amends report of", f.original_amendment_date)
        .opt(
            "Coverage",
            period_text(f.coverage_from_date, f.coverage_through_date),
        )
        .opt(
            "Election",
            election_text(
                f.election_code.as_deref(),
                f.election_code.as_deref().and_then(election_code_label),
                f.election_date,
                f.election_state.as_deref(),
            ),
        )
        .person("Completed by", &f.person_completing)
        .date("Date signed", f.date_signed);
    section_with("Report", Some("Lines 4–5"), vec![fields.render(cx)], cx)
}
