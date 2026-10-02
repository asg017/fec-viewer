//! Form 6 (48-Hour Notice of Contributions/Loans Received) cover: the
//! committee (Lines 1, 4), the candidate and office sought (Lines 2–3) and
//! the amendment (Line 5). The cover has no total; the contributions are the
//! filing's `F65` records.

use fec_parser::covers::Form6;
use gpui_kit::*;

use super::f2::seat_text;
use super::layout::*;

pub fn render(f: &Form6, cx: &App) -> Vec<AnyElement> {
    vec![
        banner(
            "Form 6 · 48-Hour Notice",
            "48-Hour Notice of Contributions/Loans Received",
            banner_tags(f, cx),
            amendment_text(f.is_amendment(), f.original_amendment_date, "notice").or_else(|| {
                Some(
                    "Contributions and loans of $1,000 or more received within 20 days of an election"
                        .into(),
                )
            }),
            cx,
        ),
        committee(f, cx),
        candidate(f, cx),
    ]
}

fn banner_tags(f: &Form6, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    tags.push(tag("48-HOUR", Tone::Info, cx));
    tags
}

/// Lines 1, 4 and 5.
fn committee(f: &Form6, cx: &App) -> AnyElement {
    let id = (!f.filer_committee_id.is_empty()).then_some(f.filer_committee_id.as_str());
    let fields = Fields::new()
        .element("Committee", with_aside(f.committee_name.clone(), id, cx))
        .address("Address", &f.address, false, cx)
        .opt(
            "Amends notice filed",
            f.is_amendment()
                .then(|| f.original_amendment_date.map(|d| d.to_string()))
                .flatten(),
        );
    let children = vec![
        fields.render(cx),
        note(
            "The contributions are listed in the filing's F65 records. They are itemized again on \
             the committee's first report after the election, so don't add them to that report's \
             receipts.",
            cx,
        ),
    ];
    section_with("Committee", Some("Lines 1, 4–5"), children, cx)
}

/// Lines 2–3.
fn candidate(f: &Form6, cx: &App) -> AnyElement {
    let c = &f.candidate;
    let fields = Fields::new();
    let fields = if c.name.is_empty() {
        fields.opt("Candidate", c.candidate_id.clone())
    } else {
        fields.element(
            "Candidate",
            with_aside(c.name.to_string(), c.candidate_id.as_deref(), cx),
        )
    };
    let fields = fields
        .opt(
            "Office sought",
            c.office
                .as_deref()
                .map(|code| code_with_label(code, c.office_label())),
        )
        .opt(
            "State & district",
            seat_text(c.state.as_deref(), c.district.as_deref()),
        );
    let body = if fields.is_empty() {
        note("Not filled in.", cx)
    } else {
        fields.render(cx)
    };
    section_with("Candidate", Some("Lines 2–3"), vec![body], cx)
}
