//! Form 7 (communication costs by corporations and membership organizations)
//! cover: the organization and its type, the report type, election and
//! period, and the total communication costs. Each communication is an `F76`
//! row, not shown here.

use fec_parser::covers::Form7;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form7, cx: &App) -> Vec<AnyElement> {
    vec![
        banner(
            "Form 7 · Report of Communication Costs",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label()))
                .unwrap_or_else(|| "Report of Communication Costs".into()),
            banner_tags(f, cx),
            amendment_text(f.is_amendment(), None, "report")
                .or_else(|| period_text(f.coverage_from_date, f.coverage_through_date)),
            cx,
        ),
        costs(f, cx),
        identification(f, cx),
    ]
}

fn banner_tags(f: &Form7, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if let Some(label) = f.organization_type_label() {
        tags.push(tag(label.to_uppercase(), Tone::Info, cx));
    }
    tags
}

/// "Total Communication Costs for This Period".
fn costs(f: &Form7, cx: &App) -> AnyElement {
    let mut total = Stat::new(
        "Total communication costs this period",
        f.total_communication_costs,
    )
    .emphasis();
    if let Some(period) = period_text(f.coverage_from_date, f.coverage_through_date) {
        total = total.detail(period, Tone::Neutral);
    }
    section(
        "Communication Costs",
        vec![
            stats(vec![total], cx),
            note(
                "The sum of the itemized communications (F76) in this report.",
                cx,
            ),
        ],
        cx,
    )
}

/// Lines 1–5: organization, type, report, election, period, signer.
fn identification(f: &Form7, cx: &App) -> AnyElement {
    let fields = Fields::new()
        .element(
            "Organization",
            with_aside(f.organization_name.clone(), Some(&f.filer_committee_id), cx),
        )
        .address("Address", &f.address, false, cx)
        .opt(
            "Type",
            f.organization_type
                .as_deref()
                .map(|code| code_with_label(code, f.organization_type_label())),
        )
        .opt(
            "Report",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label())),
        )
        // Line 4(a): the general election a 12-day pre-general report is for.
        .opt(
            "Election",
            election_text(None, None, f.election_date, f.election_state.as_deref()),
        )
        .opt(
            "Coverage",
            period_text(f.coverage_from_date, f.coverage_through_date),
        )
        .person_with(
            "Designated signer",
            &f.person_designated,
            f.person_designated_title.as_deref(),
            cx,
        )
        .date("Date signed", f.date_signed);
    section("Organization", vec![fields.render(cx)], cx)
}
