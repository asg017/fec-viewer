//! Form 3P (presidential candidate committee report) cover: page 1
//! identification, the cash-flow headline, the Summary Page (Lines 6–15), the
//! two parts of the Detailed Summary Page (Lines 16–30) with Column A ("This
//! Period") and Column B ("Election Cycle-to-Date") side by side, Line 31, and
//! the allocation of primary expenditures by state (pages 5–7).

use fec_parser::covers::{DetailedSummaryRow, Form3P};
use gpui_kit::*;

use super::layout::*;

/// Form 3P's Detailed Summary column headings.
const COLUMNS: Columns = Columns::Two("This Period", "Cycle-to-Date");

pub fn render(f: &Form3P, cx: &App) -> Vec<AnyElement> {
    let s = &f.summary;
    let mut children = vec![
        banner(
            "Form 3P · Presidential Report of Receipts and Disbursements",
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
            s.line6_cash_on_hand_beginning_period,
            s.line7_total_receipts,
            s.line9_total_disbursements,
            s.line10_cash_on_hand_end_period,
            cx,
        ),
        identification(f, cx),
        summary_page(f, cx),
        receipts(f, cx),
        disbursements(f, cx),
    ];
    if !is_zero(f.detailed_summary.line31_items_on_hand_to_be_liquidated) {
        children.push(items_on_hand(f, cx));
    }
    if !f.state_allocations.is_empty() {
        children.push(state_allocations(f, cx));
    }
    children
}

fn is_zero(amount: f64) -> bool {
    amount.abs() < 0.005
}

fn banner_tags(f: &Form3P, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.form_type.ends_with(['T', 't']) || f.report_code.as_deref() == Some("TER") {
        tags.push(tag("TERMINATION", Tone::Danger, cx));
    }
    tags
}

/// The electronic-only "Activity Primary" / "Activity General" boxes.
fn activity_text(f: &Form3P) -> Option<&'static str> {
    match (f.activity_primary, f.activity_general) {
        (true, true) => Some("Primary and general"),
        (true, false) => Some("Primary"),
        (false, true) => Some("General"),
        (false, false) => None,
    }
}

/// Page 1: committee, address, report type, election, coverage period.
fn identification(f: &Form3P, cx: &App) -> AnyElement {
    let fields = Fields::new()
        .element(
            "Committee",
            with_aside(f.committee_name.clone(), Some(&f.filer_committee_id), cx),
        )
        .address("Address", &f.address, f.change_of_address, cx)
        .opt("Activity", activity_text(f))
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

/// Page 2, Summary Page (Lines 6–15): one amount per line.
fn summary_page(f: &Form3P, cx: &App) -> AnyElement {
    let s = &f.summary;
    let table = MoneyTable::new(Columns::One)
        .amount(
            "6. Cash on hand at beginning of period",
            s.line6_cash_on_hand_beginning_period,
        )
        .amount("7. Total receipts", s.line7_total_receipts)
        .amount("8. Subtotal", s.line8_subtotal)
        .amount("9. Total disbursements", s.line9_total_disbursements)
        .amount_total(
            "10. Cash on hand at close of period",
            s.line10_cash_on_hand_end_period,
        )
        .amount(
            "11. Debts owed TO the committee",
            s.line11_debts_owed_to_committee,
        )
        .amount(
            "12. Debts owed BY the committee",
            s.line12_debts_owed_by_committee,
        )
        .amount(
            "13. Expenditures subject to limitation",
            s.line13_expenditures_subject_to_limits,
        )
        .caption("Net election cycle-to-date contributions and expenditures:")
        .amount(
            "  14. Net contributions (other than loans)",
            s.line14_net_contributions_other_than_loans,
        )
        .amount(
            "  15. Net operating expenditures",
            s.line15_net_operating_expenditures,
        );
    section_with("Summary", Some("Lines 6–15"), vec![table.render(cx)], cx)
}

/// Detailed Summary Page, Part I (Lines 16–22).
fn receipts(f: &Form3P, cx: &App) -> AnyElement {
    let r = &f.detailed_summary.receipts;
    let table = MoneyTable::new(COLUMNS)
        .row("16. Federal funds", &r.line16_federal_funds)
        .caption("17. Contributions (other than loans) from:")
        .row(
            "  (a)(i) Individuals, itemized",
            &r.line17a_i_contributions_from_individuals_itemized,
        )
        .row(
            "  (a)(ii) Individuals, unitemized",
            &r.line17a_ii_contributions_from_individuals_unitemized,
        )
        .row(
            "  (a)(iii) Individuals, total",
            &r.line17a_iii_contributions_from_individuals_total,
        )
        .row(
            "  (b) Political party committees",
            &r.line17b_political_party_committees,
        )
        .row(
            "  (c) Other political committees",
            &r.line17c_other_political_committees,
        )
        .row("  (d) The candidate", &r.line17d_the_candidate)
        .total("  (e) Total contributions", &r.line17e_total_contributions)
        .row(
            "18. Transfers from other authorized committees",
            &r.line18_transfers_from_other_authorized_committee,
        )
        .caption("19. Loans:")
        .row(
            "  (a) Received from or guaranteed by the candidate",
            &r.line19a_loans_received_from_or_guaranteed_by_candidate,
        )
        .row("  (b) Other loans", &r.line19b_other_loans)
        .total("  (c) Total loans", &r.line19c_total_loans)
        .caption("20. Offsets to expenditures:")
        .row(
            "  (a) Operating",
            &r.line20a_offsets_to_expenditures_operating,
        )
        .row(
            "  (b) Fundraising",
            &r.line20b_offsets_to_expenditures_fundraising,
        )
        .row(
            "  (c) Legal and accounting",
            &r.line20c_offsets_to_expenditures_legal_and_accounting,
        )
        .total(
            "  (d) Total offsets",
            &r.line20d_offsets_to_expenditures_total,
        )
        .row("21. Other receipts", &r.line21_other_receipts)
        .total("22. Total receipts", &r.line22_total_receipts);
    section_with(
        "Receipts",
        Some("Detailed Summary · Lines 16–22"),
        vec![table.render(cx)],
        cx,
    )
}

/// Detailed Summary Page, Part II (Lines 23–30).
fn disbursements(f: &Form3P, cx: &App) -> AnyElement {
    let d = &f.detailed_summary.disbursements;
    let table = MoneyTable::new(COLUMNS)
        .row(
            "23. Operating expenditures",
            &d.line23_operating_expenditures,
        )
        .row(
            "24. Transfers to other authorized committees",
            &d.line24_transfers_to_other_authorized_committees,
        )
        .row(
            "25. Fundraising disbursements",
            &d.line25_fundraising_disbursements,
        )
        .row(
            "26. Exempt legal and accounting disbursements",
            &d.line26_exempt_legal_and_accounting_disbursements,
        )
        .caption("27. Loan repayments:")
        .row(
            "  (a) Of loans made or guaranteed by the candidate",
            &d.line27a_loan_repayments_candidate,
        )
        .row("  (b) Of all other loans", &d.line27b_loan_repayments_other)
        .total(
            "  (c) Total loan repayments",
            &d.line27c_loan_repayments_total,
        )
        .caption("28. Refunds of contributions to:")
        .row(
            "  (a) Individuals/persons",
            &d.line28a_refunds_to_individuals,
        )
        .row(
            "  (b) Political party committees",
            &d.line28b_refunds_to_political_party_committees,
        )
        .row(
            "  (c) Other political committees",
            &d.line28c_refunds_to_other_political_committees,
        )
        .total("  (d) Total contribution refunds", &d.line28d_refunds_total)
        .row("29. Other disbursements", &d.line29_other_disbursements)
        .total("30. Total disbursements", &d.line30_total_disbursements);
    section_with(
        "Disbursements",
        Some("Detailed Summary · Lines 23–30"),
        vec![table.render(cx)],
        cx,
    )
}

/// Detailed Summary Page, Part III (Line 31), shown only when non-zero.
fn items_on_hand(f: &Form3P, cx: &App) -> AnyElement {
    let table = MoneyTable::new(Columns::One).amount(
        "31. Items on hand to be liquidated",
        f.detailed_summary.line31_items_on_hand_to_be_liquidated,
    );
    section_with(
        "Items on Hand",
        Some("Detailed Summary · Line 31"),
        vec![
            table.render(cx),
            note(
                "Contributions received as stocks, bonds, art and similar items, valued \
                 at the close of the period.",
                cx,
            ),
        ],
        cx,
    )
}

/// Pages 5–7, allocation of primary expenditures by state. Only states with
/// an allocation are listed; the form prints all 54.
fn state_allocations(f: &Form3P, cx: &App) -> AnyElement {
    let a = &f.state_allocations;
    let nonzero = |r: &DetailedSummaryRow| !is_zero(r.column_a) || !is_zero(r.column_b);
    let listed: Vec<_> = a.states.iter().filter(|s| nonzero(&s.allocation)).collect();
    let omitted = a.states.len() - listed.len();
    let table = listed
        .iter()
        .fold(
            MoneyTable::new(Columns::Two("This Period", "To Date")),
            |t, s| t.row(s.state, &s.allocation),
        )
        .total("Totals", &a.totals);
    let mut text = String::from(
        "Each expenditure is allocated to the state it was intended to influence, not \
         necessarily where it was incurred or paid.",
    );
    if omitted > 0 {
        text.push_str(&format!(
            " {omitted} of {} jurisdictions with no allocation are not shown.",
            a.states.len()
        ));
    }
    section_with(
        "Primary Expenditures by State",
        Some("Pages 5–7"),
        vec![table.render(cx), note(text, cx)],
        cx,
    )
}
