//! Form 24 (24/48-hour notice of independent expenditures) cover: the
//! committee, the report type and the amendment. There is no paper form and
//! no amounts on the cover; the expenditures are the filing's Schedule E
//! records.

use fec_parser::covers::Form24;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form24, cx: &App) -> Vec<AnyElement> {
    vec![
        banner(
            "Form 24 · Notice of Independent Expenditures",
            f.report_type_label()
                .map(|label| format!("{label} of Independent Expenditures"))
                .unwrap_or_else(|| "24/48-Hour Notice of Independent Expenditures".into()),
            banner_tags(f, cx),
            amendment_text(f.is_amendment(), f.original_amendment_date, "report")
                .or_else(|| threshold_text(f.report_type.as_deref()).map(Into::into)),
            cx,
        ),
        committee(f, cx),
    ]
}

fn banner_tags(f: &Form24, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    match f.report_type.as_deref().map(str::trim) {
        Some("24") => tags.push(tag("24-HOUR", Tone::Danger, cx)),
        Some("48") => tags.push(tag("48-HOUR", Tone::Info, cx)),
        _ => {}
    }
    tags
}

/// When each report is due, per the Schedule E instructions.
fn threshold_text(report_type: Option<&str>) -> Option<&'static str> {
    match report_type?.trim() {
        "24" => Some(
            "Independent expenditures aggregating $1,000 or more made after the 20th day, but \
             more than 24 hours, before an election",
        ),
        "48" => Some(
            "Independent expenditures aggregating $10,000 or more made up to and including the \
             20th day before an election",
        ),
        _ => None,
    }
}

fn committee(f: &Form24, cx: &App) -> AnyElement {
    let id = (!f.filer_committee_id.is_empty()).then_some(f.filer_committee_id.as_str());
    let fields = Fields::new()
        .element("Committee", with_aside(f.committee_name.clone(), id, cx))
        .address("Address", &f.address, false, cx)
        .opt(
            "Report type",
            f.report_type
                .as_deref()
                .map(|code| code_with_label(code, f.report_type_label())),
        )
        .opt(
            "Amends report filed",
            f.is_amendment()
                .then(|| f.original_amendment_date.map(|d| d.to_string()))
                .flatten(),
        )
        .person("Treasurer", &f.treasurer);
    let children = vec![
        fields.render(cx),
        note(
            "The expenditures are itemized under Schedule E. They are reported again on Schedule \
             E of the committee's next regular report, so don't add them to that report's totals.",
            cx,
        ),
    ];
    section("Committee", children, cx)
}
