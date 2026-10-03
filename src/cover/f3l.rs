//! Form 3L (contributions bundled by lobbyists/registrants) cover: the
//! committee and the candidate's race, the report type and covered period(s),
//! and Line 7's bundled-contribution totals as headline tiles. The bundlers
//! themselves are itemized on `SA3L` rows, not here.

use fec_parser::covers::Form3L;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form3L, cx: &App) -> Vec<AnyElement> {
    vec![
        banner(
            "Form 3L · Report of Contributions Bundled by Lobbyists/Registrants",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label()))
                .unwrap_or_else(|| "Bundled Contributions Report".into()),
            banner_tags(f, cx),
            amendment_text(f.is_amendment(), None, "report")
                .or_else(|| period_text(f.coverage_from_date, f.coverage_through_date)),
            cx,
        ),
        bundled(f, cx),
        identification(f, cx),
    ]
}

fn banner_tags(f: &Form3L, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.covers_semi_annual_period() {
        tags.push(tag("SEMI-ANNUAL", Tone::Info, cx));
    }
    tags
}

/// Line 7: total reportable bundled contributions for each covered period.
fn bundled(f: &Form3L, cx: &App) -> AnyElement {
    let mut a = Stat::new(
        "7(a) Quarterly / monthly / pre / post",
        f.line7a_quarterly_monthly_bundled_contributions,
    )
    .emphasis();
    if let Some(period) = period_text(f.coverage_from_date, f.coverage_through_date) {
        a = a.detail(period, Tone::Neutral);
    }
    let mut tiles = vec![a];
    let semi_annual =
        f.covers_semi_annual_period() || f.line7b_semi_annual_bundled_contributions.is_some();
    if semi_annual {
        let mut b = Stat::new(
            "7(b) Semi-annual period",
            f.line7b_semi_annual_bundled_contributions.unwrap_or(0.0),
        );
        if let Some(period) = semi_annual_text(f) {
            b = b.detail(period, Tone::Neutral);
        }
        tiles.push(b);
    }
    let mut children = vec![stats(tiles, cx)];
    if semi_annual {
        children.push(note(
            "(a) and (b) cover overlapping periods; don't add them together.",
            cx,
        ));
    }
    children.push(note(
        "Sums of the bundled contributions itemized on Schedule A (SA3L); refunds (SB3L) are not subtracted.",
        cx,
    ));
    section_with(
        "Total Reportable Bundled Contributions",
        Some("Line 7"),
        children,
        cx,
    )
}

/// Lines 1–6: committee, candidate's race, report type, covered periods.
fn identification(f: &Form3L, cx: &App) -> AnyElement {
    // Line 4: House and Senate candidates' committees only.
    let race = f
        .election_state
        .as_deref()
        .map(|state| match f.election_district.as_deref() {
            Some(district) => format!("{state}, district {district}"),
            None => state.to_string(),
        });
    let fields = Fields::new()
        .element(
            "Committee",
            with_aside(f.committee_name.clone(), Some(&f.filer_committee_id), cx),
        )
        .address("Address", &f.address, f.change_of_address, cx)
        .opt("Candidate running in", race)
        .opt(
            "Report",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label())),
        )
        // Line 5(c)/(d): the election a pre-/post-election report is for.
        .opt(
            "Election",
            election_text(
                None,
                None,
                f.election_date,
                f.election_held_in_state.as_deref(),
            ),
        )
        .opt(
            "Covered period",
            period_text(f.coverage_from_date, f.coverage_through_date),
        )
        .opt("Semi-annual period", semi_annual_text(f))
        .opt(
            "Also covers semi-annual",
            f.also_covers_semi_annual_period
                .then_some("Yes — Q2 / year-end report waived"),
        )
        .person("Treasurer", &f.treasurer)
        .date("Date signed", f.date_signed);
    section("Committee", vec![fields.render(cx)], cx)
}

/// The semi-annual period the report covers, e.g. `"January 1 – June 30,
/// 2025"`: from the Line 6(b) boxes when checked, else from the report code
/// (filers usually signal it there), else from the covered period's end for
/// a pre-/post-election report with the Line 5 box checked.
fn semi_annual_text(f: &Form3L) -> Option<String> {
    const FIRST: &str = "January 1 – June 30";
    const SECOND: &str = "July 1 – December 31";
    let half = match (f.semi_annual_january_june, f.semi_annual_july_december) {
        (true, true) => return Some(format!("{FIRST} and {SECOND}")),
        (true, false) => FIRST,
        (false, true) => SECOND,
        (false, false) => match f.report_code.as_deref() {
            Some("Q2S" | "QSA" | "QMS" | "M7S" | "MSA") => FIRST,
            Some("QYS" | "QYE" | "MYS" | "MSY") => SECOND,
            _ if f.also_covers_semi_annual_period => match f.coverage_through_date {
                Some(through) if through.month() <= 6 => FIRST,
                Some(_) => SECOND,
                None => return None,
            },
            _ => return None,
        },
    };
    Some(match f.coverage_through_date {
        Some(through) => format!("{half}, {}", through.year()),
        None => half.to_string(),
    })
}
