//! Form 4 (convention / host committee report) cover: page 1 identification,
//! the cash-flow headline, the Summary Page (Section A, Lines 6–10, and
//! Section B, Lines 11–12(c)) and the Detailed Summary Page (receipts, Lines
//! 13–20; disbursements, Lines 21–25), with Column A ("This Period") and
//! Column B ("Calendar Year-to-Date") side by side.

use fec_parser::covers::{Form4, Form4ItemizedLine, Form4LoanLine};
use gpui_kit::*;

use super::layout::*;

/// Form 4's column headings.
const COLUMNS: Columns = Columns::Two("This Period", "Year-to-Date");

pub fn render(f: &Form4, cx: &App) -> Vec<AnyElement> {
    let s = &f.summary;
    vec![
        banner(
            "Form 4 · Nominating Convention Committee Report",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label()))
                .unwrap_or_else(|| "Report of Receipts and Disbursements".into()),
            banner_tags(f, cx),
            amendment_text(f.is_amendment(), None, "report")
                .or_else(|| period_text(f.coverage_from_date, f.coverage_through_date)),
            cx,
        ),
        cash_flow(
            s.line6b_cash_on_hand_beginning_period,
            s.line6c_total_receipts.column_a,
            s.line7_total_disbursements.column_a,
            s.line8_cash_on_hand_close_of_period.column_a,
            cx,
        ),
        identification(f, cx),
        cash_summary(f, cx),
        limitation(f, cx),
        receipts(f, cx),
        disbursements(f, cx),
    ]
}

fn banner_tags(f: &Form4, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.form_type.ends_with(['T', 't']) || f.report_code.as_deref() == Some("TER") {
        tags.push(tag("TERMINATION", Tone::Danger, cx));
    }
    match f.committee_type.as_deref() {
        Some("A") => tags.push(tag("CONVENTION COMMITTEE", Tone::Info, cx)),
        Some("H") => tags.push(tag("HOST COMMITTEE", Tone::Info, cx)),
        _ => {}
    }
    tags
}

/// Page 1: committee, address, type of committee, report type, period.
fn identification(f: &Form4, cx: &App) -> AnyElement {
    let committee_type = f.committee_type.as_deref().map(|code| {
        let text = code_with_label(code, f.committee_type_label());
        // Line 3's "Other (specify)" text, when given.
        match f.committee_type_description.as_deref() {
            Some(desc) => format!("{text} — {desc}"),
            None => text,
        }
    });
    let fields = Fields::new()
        .element(
            "Committee",
            with_aside(f.committee_name.clone(), Some(&f.filer_committee_id), cx),
        )
        .address("Address", &f.address, false, cx)
        .opt(
            "Type",
            committee_type.or_else(|| f.committee_type_description.clone()),
        )
        .opt(
            "Report",
            f.report_code
                .as_deref()
                .map(|code| report_code_text(code, f.report_code_label())),
        )
        .opt(
            "Coverage",
            period_text(f.coverage_from_date, f.coverage_through_date),
        )
        .person("Treasurer", &f.treasurer)
        .date("Date signed", f.date_signed);
    section("Committee", vec![fields.render(cx)], cx)
}

/// Page 1, Summary Page Section A: cash balance summary (Lines 6–10).
fn cash_summary(f: &Form4, cx: &App) -> AnyElement {
    let s = &f.summary;
    let jan_1 = match s.line6a_year {
        Some(year) => format!("6(a) Cash on hand January 1, {year}"),
        None => "6(a) Cash on hand January 1".to_string(),
    };
    let close = &s.line8_cash_on_hand_close_of_period;
    let table = MoneyTable::new(COLUMNS)
        .row_ab(&jan_1, None, Some(s.line6a_cash_on_hand_jan_1), false)
        .row_ab(
            "6(b) Cash on hand at beginning of period",
            Some(s.line6b_cash_on_hand_beginning_period),
            None,
            false,
        )
        .row("6(c) Total receipts", &s.line6c_total_receipts)
        .row("6(d) Subtotal", &s.line6d_subtotal)
        .row("7. Total disbursements", &s.line7_total_disbursements)
        .total("8. Cash on hand at close of period", close)
        .amount("9. Debts owed TO the committee", s.line9_debts_owed_to_committee)
        .amount("10. Debts owed BY the committee", s.line10_debts_owed_by_committee);
    let mut children = vec![table.render(cx)];
    // The instructions say Line 8 "should be the same for both columns".
    if (close.column_a - close.column_b).abs() >= 0.005 {
        children.push(note(
            "As filed, Line 8 differs between the columns; the instructions say they should match.",
            cx,
        ));
    }
    section_with("Cash Summary", Some("Section A · Lines 6–10"), children, cx)
}

/// Page 1, Summary Page Section B: expenditures subject to limitation
/// (Lines 11–12(c)).
fn limitation(f: &Form4, cx: &App) -> AnyElement {
    let s = &f.summary;
    let table = MoneyTable::new(COLUMNS)
        .row("11. Convention expenditures", &s.line11_convention_expenditures)
        .row(
            "12. Refunds, rebates and returns of deposits",
            &s.line12_convention_refunds,
        )
        .total(
            "  (a) Expenditures subject to limitation",
            &s.line12a_expenditures_subject_to_limitation,
        )
        .row(
            "  (b) Prior years' expenditures subject to limitation",
            &s.line12b_prior_years_expenditures_subject_to_limitation,
        )
        .row_ab(
            "  (c) Total expenditures subject to limitation",
            None,
            Some(s.line12c_total_expenditures_subject_to_limitation),
            true,
        );
    section_with(
        "Expenditures Subject to Limitation",
        Some("Section B · Lines 11–12(c)"),
        vec![table.render(cx)],
        cx,
    )
}

/// Detailed Summary Page, receipts (Lines 13–20).
fn receipts(f: &Form4, cx: &App) -> AnyElement {
    let r = &f.detailed_summary.receipts;
    let table = MoneyTable::new(COLUMNS)
        .row("13. Federal funds (Presidential Election Campaign Fund)", &r.line13_federal_funds);
    let table = itemized(
        table,
        "14. Contributions to defray convention expenses:",
        &r.line14_contributions,
    )
    .row(
        "15. Transfers from affiliated committees",
        &r.line15_transfers_from_affiliated_committees,
    );
    let table = loans(
        table,
        "16. Loans and loan repayments received:",
        ("  (a) Loans received", "  (b) Loan repayments received"),
        &r.line16_loans_received,
    );
    let table = itemized(
        table,
        "17. Refunds, rebates and returns of deposits (convention expenditures):",
        &r.line17_convention_refunds,
    );
    let table = itemized(
        table,
        "18. Other refunds, rebates and returns of deposits:",
        &r.line18_other_refunds,
    );
    let table = itemized(table, "19. Other income:", &r.line19_other_income)
        .total("20. Total receipts", &r.line20_total_receipts);
    section_with("Receipts", Some("Detailed Summary · Lines 13–20"), vec![table.render(cx)], cx)
}

/// Detailed Summary Page, disbursements (Lines 21–25).
fn disbursements(f: &Form4, cx: &App) -> AnyElement {
    let d = &f.detailed_summary.disbursements;
    let table = itemized(
        MoneyTable::new(COLUMNS),
        "21. Convention expenditures:",
        &d.line21_convention_expenditures,
    )
    .row(
        "22. Transfers to affiliated committees",
        &d.line22_transfers_to_affiliated_committees,
    );
    let table = loans(
        table,
        "23. Loans and loan repayments made:",
        ("  (a) Loans made", "  (b) Loan repayments made"),
        &d.line23_loans_made,
    );
    let table = itemized(table, "24. Other disbursements:", &d.line24_other_disbursements)
        .total("25. Total disbursements", &d.line25_total_disbursements);
    section_with(
        "Disbursements",
        Some("Detailed Summary · Lines 21–25"),
        vec![table.render(cx)],
        cx,
    )
}

/// An (a) itemized / (b) unitemized / (c) subtotal line; (a) and (b) are
/// This Period only.
fn itemized(table: MoneyTable, caption: &str, line: &Form4ItemizedLine) -> MoneyTable {
    table
        .caption(caption)
        .amount("  (a) Itemized", line.itemized)
        .amount("  (b) Unitemized", line.unitemized)
        .total("  (c) Subtotal", &line.subtotal)
}

/// An (a) loans / (b) loan repayments / (c) subtotal line; (a) and (b) are
/// This Period only.
fn loans(
    table: MoneyTable,
    caption: &str,
    (loans, repayments): (&str, &str),
    line: &Form4LoanLine,
) -> MoneyTable {
    table
        .caption(caption)
        .amount(loans, line.loans)
        .amount(repayments, line.loan_repayments)
        .total("  (c) Subtotal", &line.subtotal)
}
