//! Form 9 (24-hour notice of disbursements/obligations for electioneering
//! communications) cover: the communication (Lines 4–6), the two totals
//! (Lines 10–11), the filer (Lines 1–3, 7–8) and the custodian of records
//! (Line 9). Line numbers follow the 01/2018 paper form.

use fec_parser::covers::Form9;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form9, cx: &App) -> Vec<AnyElement> {
    vec![
        banner(
            "Form 9 · 24-Hour Notice of Electioneering Communications",
            f.communication_title
                .as_deref()
                .map(|title| format!("“{title}”"))
                .unwrap_or_else(|| "Electioneering Communication".into()),
            amendment_tag(f.is_amendment(), cx).into_iter().collect(),
            amendment_text(f.is_amendment(), f.original_amendment_date, "notice").or_else(|| {
                f.date_public_distribution
                    .map(|d| format!("First publicly distributed {d}"))
            }),
            cx,
        ),
        stats(
            vec![
                Stat::new("10. Total donations", f.total_donations),
                Stat::new("11. Total disbursements/obligations", f.total_disbursements).emphasis(),
            ],
            cx,
        ),
        communication(f, cx),
        filer(f, cx),
        custodian(f, cx),
    ]
}

/// Lines 4–6 and 8: the communication and how it was paid for.
fn communication(f: &Form9, cx: &App) -> AnyElement {
    let fields = Fields::new()
        .opt("Title", f.communication_title.clone())
        .date("Public distribution", f.date_public_distribution)
        .opt(
            "Covered period",
            period_text(f.coverage_from_date, f.coverage_through_date),
        )
        .date("Amends notice of", f.original_amendment_date)
        .opt(
            "Segregated account",
            f.segregated_bank_account.as_deref().map(|code| {
                match f.used_segregated_bank_account() {
                    Some(true) => {
                        "Yes — paid only from donations to a segregated bank account".into()
                    }
                    Some(false) => "No".into(),
                    None => yes_no_code(code),
                }
            }),
        )
        .person("Completed by", &f.person_completing)
        .date("Date signed", f.date_signed);
    section_with(
        "Communication",
        Some("Lines 4–6, 8"),
        vec![fields.render(cx)],
        cx,
    )
}

/// Lines 1–3 and 7: who made the disbursements.
fn filer(f: &Form9, cx: &App) -> AnyElement {
    let id = (!f.filer_committee_id.trim().is_empty()).then_some(f.filer_committee_id.as_str());
    let filer_is = f.filer_code.as_deref().map(|code| {
        let text = code_with_label(code.trim(), f.filer_code_label());
        match f.filer_code_description.as_deref() {
            Some(desc) => format!("{text}: {desc}"),
            None => text,
        }
    });
    let fields = Fields::new()
        .element("Filer", with_aside(f.filer_name(), id, cx))
        .opt("Filer type", f.entity_type.as_deref().map(entity_label))
        .opt("The filer is", filer_is)
        .address("Address", &f.address, f.change_of_address, cx)
        .opt("Occupation", f.individual_occupation.clone())
        .opt("Employer", f.individual_employer.clone())
        .opt(
            "Qualified nonprofit",
            f.qualified_nonprofit.as_deref().map(yes_no_code),
        );
    section_with("Filer", Some("Lines 1–3, 7"), vec![fields.render(cx)], cx)
}

/// Line 9: who keeps the books behind this notice.
fn custodian(f: &Form9, cx: &App) -> AnyElement {
    let c = &f.custodian;
    let fields = Fields::new()
        .person("Name", &c.name)
        .address("Address", &c.address, false, cx)
        .opt("Employer", c.employer.clone())
        .opt("Occupation", c.occupation.clone());
    let children = if fields.is_empty() {
        vec![note("No custodian of records listed.", cx)]
    } else {
        vec![fields.render(cx)]
    };
    section_with("Custodian of Records", Some("Line 9"), children, cx)
}
