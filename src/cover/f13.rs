//! Form 13 (inaugural committee donations report) cover: the committee, the
//! report or supplement and its period, and the cumulative donation totals
//! (Lines 5–7) as headline tiles and a table.

use fec_parser::covers::Form13;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form13, cx: &App) -> Vec<AnyElement> {
    vec![
        banner(
            "Form 13 · Report of Donations Accepted for Inaugural Committee",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label()))
                .unwrap_or_else(|| "Report of Donations Accepted".into()),
            banner_tags(f, cx),
            amendment_text(f.is_amendment(), f.original_amendment_date, "report")
                .or_else(|| period_text(f.coverage_from_date, f.coverage_through_date)),
            cx,
        ),
        headline(f, cx),
        identification(f, cx),
        totals(f, cx),
    ]
}

fn banner_tags(f: &Form13, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.report_code.as_deref() == Some("90S") {
        tags.push(tag("SUPPLEMENT", Tone::Info, cx));
    }
    tags
}

/// Accepted / refunded / net tiles.
fn headline(f: &Form13, cx: &App) -> AnyElement {
    let accepted = f.line5_total_donations_accepted;
    let mut refunded = Stat::new("Donations refunded", f.line6_total_donations_refunded);
    if accepted > 0.0 {
        refunded = refunded.detail(
            format!(
                "{:.1}% of accepted",
                f.line6_total_donations_refunded / accepted * 100.0
            ),
            Tone::Neutral,
        );
    }
    stats(
        vec![
            Stat::new("Donations accepted", accepted).detail("Since inception", Tone::Neutral),
            refunded,
            Stat::new("Net donations", f.line7_net_donations)
                .detail("Since inception", Tone::Neutral)
                .emphasis(),
        ],
        cx,
    )
}

/// Lines 1–4: committee, type of filing, amended filing, period.
fn identification(f: &Form13, cx: &App) -> AnyElement {
    let fields = Fields::new()
        .element(
            "Committee",
            with_aside(f.committee_name.clone(), Some(&f.filer_committee_id), cx),
        )
        .address("Address", &f.address, f.change_of_address, cx)
        .opt(
            "Report",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label())),
        )
        .date("Amends filing of", f.original_amendment_date)
        .opt(
            "Coverage",
            period_text(f.coverage_from_date, f.coverage_through_date),
        )
        .person("Designated officer", &f.designated_officer)
        .date("Date signed", f.date_signed);
    section("Inaugural Committee", vec![fields.render(cx)], cx)
}

/// Lines 5–7, "Cumulative Total (From Committee's Inception)".
fn totals(f: &Form13, cx: &App) -> AnyElement {
    let table = MoneyTable::new(Columns::One)
        .amount(
            "5. Total donations accepted",
            f.line5_total_donations_accepted,
        )
        .amount(
            "6. Total donations refunded",
            f.line6_total_donations_refunded,
        )
        .amount_total("7. Net donations", f.line7_net_donations);
    let mut children = vec![table.render(cx)];
    if f.report_code.as_deref() == Some("90S") {
        children.push(note(
            "Totals are cumulative: they include every Schedule 13-A/13-B filed since the committee's inception, not just this supplement.",
            cx,
        ));
    }
    section_with(
        "Cumulative Totals",
        Some("Lines 5–7 · from committee's inception"),
        children,
        cx,
    )
}
