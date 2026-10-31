//! Form 3 (House / Senate candidate committee report) cover: page 1
//! identification, the cash-flow headline, the Summary Page (Lines 6–10) and
//! the three parts of the Detailed Summary Page (Lines 11–27), with Column A
//! ("This Period") and Column B ("Election Cycle-to-Date") side by side.

use fec_parser::covers::Form3;
use gpui_kit::*;

use super::layout::*;

/// Form 3's column headings. Column B is election cycle-to-date, not
/// calendar year-to-date as on Form 3X.
const COLUMNS: Columns = Columns::Two("This Period", "Cycle-to-Date");

pub fn render(f: &Form3, cx: &App) -> Vec<AnyElement> {
    let c = &f.detailed_summary.cash_summary;
    vec![
        banner(
            "Form 3 · Report of Receipts and Disbursements",
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
            c.line23_cash_on_hand_beginning,
            c.line24_total_receipts,
            c.line26_total_disbursements,
            c.line27_cash_on_hand_close,
            cx,
        ),
        identification(f, cx),
        summary_page(f, cx),
        receipts(f, cx),
        disbursements(f, cx),
        cash_summary(f, cx),
    ]
}

fn banner_tags(f: &Form3, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.form_type.ends_with(['T', 't']) || f.report_code.as_deref() == Some("TER") {
        tags.push(tag("TERMINATION", Tone::Danger, cx));
    }
    tags
}

/// Page 1: committee, address, seat, report type, election, coverage period.
fn identification(f: &Form3, cx: &App) -> AnyElement {
    let fields = Fields::new()
        .element(
            "Committee",
            with_aside(f.committee_name.clone(), Some(&f.filer_committee_id), cx),
        )
        .address("Address", &f.address, f.change_of_address, cx)
        .opt(
            "State / district",
            seat_text(f.election_state.as_deref(), f.election_district.as_deref()),
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
        .opt(
            "Election",
            election_text(
                f.election_code.as_deref(),
                f.election_code_label(),
                f.election_date,
                f.state_of_election.as_deref(),
            ),
        )
        .person("Treasurer", &f.treasurer)
        .date("Date signed", f.date_signed);
    section("Committee", vec![fields.render(cx)], cx)
}

/// Page 2, Summary Page (Lines 6–10).
fn summary_page(f: &Form3, cx: &App) -> AnyElement {
    let s = &f.summary;
    let table = MoneyTable::new(COLUMNS)
        .caption("6. Net contributions (other than loans):")
        .row("  (a) Total contributions", &s.line6a_total_contributions)
        .row(
            "  (b) Total contribution refunds",
            &s.line6b_total_contribution_refunds,
        )
        .total("  (c) Net contributions", &s.line6c_net_contributions)
        .caption("7. Net operating expenditures:")
        .row(
            "  (a) Total operating expenditures",
            &s.line7a_total_operating_expenditures,
        )
        .row(
            "  (b) Total offsets to operating expenditures",
            &s.line7b_total_offsets_to_operating_expenditures,
        )
        .total(
            "  (c) Net operating expenditures",
            &s.line7c_net_operating_expenditures,
        )
        .amount_total(
            "8. Cash on hand at close of period",
            s.line8_cash_on_hand_close_of_period,
        )
        .amount(
            "9. Debts owed TO the committee",
            s.line9_debts_owed_to_committee,
        )
        .amount(
            "10. Debts owed BY the committee",
            s.line10_debts_owed_by_committee,
        );
    let mut children = vec![table.render(cx)];
    // The paper post-general report has a third column the record lacks.
    if f.report_code.as_deref() == Some("30G") {
        children.push(note(
            "Post-general report: the paper form uses a three-column Post-Election \
             Detailed Summary Page; the electronic record carries only two columns, and \
             FEC sources do not say which one Column B holds.",
            cx,
        ));
    }
    section_with("Summary", Some("Lines 6–10"), children, cx)
}

/// Detailed Summary Page, Part I (Lines 11–16).
fn receipts(f: &Form3, cx: &App) -> AnyElement {
    let r = &f.detailed_summary.receipts;
    let table = MoneyTable::new(COLUMNS)
        .caption("11. Contributions (other than loans) from:")
        .row(
            "  (a)(i) Individuals, itemized",
            &r.line11a_i_contributions_from_individuals_itemized,
        )
        .row(
            "  (a)(ii) Individuals, unitemized",
            &r.line11a_ii_contributions_from_individuals_unitemized,
        )
        .row(
            "  (a)(iii) Individuals, total",
            &r.line11a_iii_contributions_from_individuals_total,
        )
        .row(
            "  (b) Political party committees",
            &r.line11b_political_party_committees,
        )
        .row(
            "  (c) Other political committees (PACs)",
            &r.line11c_other_political_committees_pacs,
        )
        .row("  (d) The candidate", &r.line11d_the_candidate)
        .total("  (e) Total contributions", &r.line11e_total_contributions)
        .row(
            "12. Transfers from other authorized committees",
            &r.line12_transfers_from_authorized,
        )
        .caption("13. Loans:")
        .row(
            "  (a) Made or guaranteed by the candidate",
            &r.line13a_loans_from_candidate,
        )
        .row("  (b) All other loans", &r.line13b_other_loans)
        .total("  (c) Total loans", &r.line13c_total_loans)
        .row(
            "14. Offsets to operating expenditures",
            &r.line14_offset_to_operating_expenditures,
        )
        .row(
            "15. Other receipts (dividends, interest, etc.)",
            &r.line15_other_receipts,
        )
        .total("16. Total receipts", &r.line16_total_receipts);
    section_with(
        "Receipts",
        Some("Detailed Summary · Lines 11–16"),
        vec![table.render(cx)],
        cx,
    )
}

/// Detailed Summary Page, Part II (Lines 17–22).
fn disbursements(f: &Form3, cx: &App) -> AnyElement {
    let d = &f.detailed_summary.disbursements;
    let table = MoneyTable::new(COLUMNS)
        .row(
            "17. Operating expenditures",
            &d.line17_operating_expenditures,
        )
        .row(
            "18. Transfers to other authorized committees",
            &d.line18_transfers_to_authorized,
        )
        .caption("19. Loan repayments:")
        .row(
            "  (a) Of loans made or guaranteed by the candidate",
            &d.line19a_candidate_loan_repayments,
        )
        .row("  (b) Of all other loans", &d.line19b_other_loan_repayments)
        .total(
            "  (c) Total loan repayments",
            &d.line19c_total_loan_repayments,
        )
        .caption("20. Refunds of contributions to:")
        .row(
            "  (a) Individuals/persons",
            &d.line20a_refunds_to_individuals,
        )
        .row(
            "  (b) Political party committees",
            &d.line20b_refunds_to_party_committees,
        )
        .row(
            "  (c) Other political committees",
            &d.line20c_refunds_to_other_committees,
        )
        .total("  (d) Total contribution refunds", &d.line20d_total_refunds)
        .row("21. Other disbursements", &d.line21_other_disbursements)
        .total("22. Total disbursements", &d.line22_total_disbursements);
    section_with(
        "Disbursements",
        Some("Detailed Summary · Lines 17–22"),
        vec![table.render(cx)],
        cx,
    )
}

/// Detailed Summary Page, Part III (Lines 23–27): this period only.
fn cash_summary(f: &Form3, cx: &App) -> AnyElement {
    let c = &f.detailed_summary.cash_summary;
    let table = MoneyTable::new(Columns::One)
        .amount(
            "23. Cash on hand at beginning of period",
            c.line23_cash_on_hand_beginning,
        )
        .amount("24. Total receipts this period", c.line24_total_receipts)
        .amount_total("25. Subtotal", c.line25_subtotal)
        .amount(
            "26. Total disbursements this period",
            c.line26_total_disbursements,
        )
        .amount_total(
            "27. Cash on hand at close of period",
            c.line27_cash_on_hand_close,
        );
    section_with(
        "Cash Summary",
        Some("Detailed Summary · Lines 23–27"),
        vec![table.render(cx)],
        cx,
    )
}
