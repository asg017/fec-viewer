//! Form 3X (PAC / party committee report) cover: page 1 identification, the
//! cash-flow headline, the Summary Page (Lines 6–10) and the three sections of
//! the Detailed Summary Page (Lines 11–38), with Column A ("This Period") and
//! Column B ("Calendar Year-to-Date") side by side.

use fec_parser::covers::Form3X;
use gpui_kit::*;

use super::layout::*;

/// Form 3X's column headings.
const COLUMNS: Columns = Columns::Two("This Period", "Year-to-Date");

pub fn render(f: &Form3X, cx: &App) -> Vec<AnyElement> {
    let s = &f.summary;
    vec![
        banner(
            "Form 3X · Report of Receipts and Disbursements",
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
        summary_page(f, cx),
        receipts(f, cx),
        disbursements(f, cx),
        net(f, cx),
    ]
}

fn banner_tags(f: &Form3X, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.form_type.ends_with(['T', 't']) {
        tags.push(tag("TERMINATION", Tone::Danger, cx));
    }
    if f.qualified_committee {
        tags.push(tag("MULTICANDIDATE", Tone::Info, cx));
    }
    tags
}

/// Page 1: committee, address, report type, election, coverage period.
fn identification(f: &Form3X, cx: &App) -> AnyElement {
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
        .text(
            "Multicandidate",
            if f.qualified_committee {
                "Yes — qualified multicandidate committee"
            } else {
                "No"
            },
        )
        .person("Treasurer", &f.treasurer)
        .date("Date signed", f.date_signed);
    section("Committee", vec![fields.render(cx)], cx)
}

/// Page 2, Summary Page (Lines 6–10).
fn summary_page(f: &Form3X, cx: &App) -> AnyElement {
    let s = &f.summary;
    let jan_1 = match s.line6a_year {
        Some(year) => format!("6(a) Cash on hand January 1, {year}"),
        None => "6(a) Cash on hand January 1".to_string(),
    };
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
        .total(
            "8. Cash on hand at close of period",
            &s.line8_cash_on_hand_close_of_period,
        )
        .amount(
            "9. Debts owed TO the committee",
            s.line9_debts_owed_to_committee,
        )
        .amount(
            "10. Debts owed BY the committee",
            s.line10_debts_owed_by_committee,
        );
    section_with("Summary", Some("Lines 6–10"), vec![table.render(cx)], cx)
}

/// Detailed Summary Page, Section I (Lines 11–20).
fn receipts(f: &Form3X, cx: &App) -> AnyElement {
    let r = &f.detailed_summary.receipts;
    let table = MoneyTable::new(COLUMNS)
        .caption("11. Contributions (other than loans) from:")
        .row(
            "  (a)(i) Individuals, itemized",
            &r.line11a_i_individuals_itemized,
        )
        .row(
            "  (a)(ii) Individuals, unitemized",
            &r.line11a_ii_individuals_unitemized,
        )
        .row(
            "  (a)(iii) Individuals, total",
            &r.line11a_iii_individuals_total,
        )
        .row(
            "  (b) Political party committees",
            &r.line11b_political_party_committees,
        )
        .row(
            "  (c) Other political committees",
            &r.line11c_other_political_committees,
        )
        .total("  (d) Total contributions", &r.line11d_total_contributions)
        .row(
            "12. Transfers from affiliated/party committees",
            &r.line12_transfers_from_affiliated,
        )
        .row("13. All loans received", &r.line13_loans_received)
        .row(
            "14. Loan repayments received",
            &r.line14_loan_repayments_received,
        )
        .row(
            "15. Offsets to operating expenditures",
            &r.line15_offsets_to_operating_expenditures,
        )
        .row(
            "16. Refunds of contributions made to federal candidates",
            &r.line16_refunds_of_federal_contributions,
        )
        .row(
            "17. Other federal receipts (dividends, interest, etc.)",
            &r.line17_other_federal_receipts,
        )
        .caption("18. Transfers from nonfederal and Levin funds:")
        .row(
            "  (a) Nonfederal account (Schedule H3)",
            &r.line18a_transfers_from_nonfederal_account,
        )
        .row(
            "  (b) Levin funds (Schedule H5)",
            &r.line18b_transfers_from_levin_funds,
        )
        .row(
            "  (c) Total nonfederal transfers",
            &r.line18c_total_nonfederal_transfers,
        )
        .total("19. Total receipts", &r.line19_total_receipts)
        .total(
            "20. Total federal receipts",
            &r.line20_total_federal_receipts,
        );
    section_with(
        "Receipts",
        Some("Detailed Summary · Lines 11–20"),
        vec![table.render(cx)],
        cx,
    )
}

/// Detailed Summary Page, Section II (Lines 21–32).
fn disbursements(f: &Form3X, cx: &App) -> AnyElement {
    let d = &f.detailed_summary.disbursements;
    let table = MoneyTable::new(COLUMNS)
        .caption("21. Operating expenditures:")
        .row(
            "  (a)(i) Shared federal/nonfederal, federal share (H4)",
            &d.line21a_i_shared_operating_federal_share,
        )
        .row(
            "  (a)(ii) Shared federal/nonfederal, nonfederal share (H4)",
            &d.line21a_ii_shared_operating_nonfederal_share,
        )
        .row(
            "  (b) Other federal operating expenditures",
            &d.line21b_other_federal_operating_expenditures,
        )
        .total(
            "  (c) Total operating expenditures",
            &d.line21c_total_operating_expenditures,
        )
        .row(
            "22. Transfers to affiliated/party committees",
            &d.line22_transfers_to_affiliated,
        )
        .row(
            "23. Contributions to federal candidates/committees",
            &d.line23_contributions_to_federal_candidates,
        )
        .row(
            "24. Independent expenditures",
            &d.line24_independent_expenditures,
        )
        .row(
            "25. Coordinated party expenditures",
            &d.line25_coordinated_party_expenditures,
        )
        .row("26. Loan repayments made", &d.line26_loan_repayments_made)
        .row("27. Loans made", &d.line27_loans_made)
        .caption("28. Refunds of contributions to:")
        .row(
            "  (a) Individuals/persons",
            &d.line28a_refunds_to_individuals,
        )
        .row(
            "  (b) Political party committees",
            &d.line28b_refunds_to_party_committees,
        )
        .row(
            "  (c) Other political committees",
            &d.line28c_refunds_to_other_committees,
        )
        .total(
            "  (d) Total contribution refunds",
            &d.line28d_total_contribution_refunds,
        )
        .row("29. Other disbursements", &d.line29_other_disbursements)
        .caption("30. Federal election activity:")
        .row(
            "  (a)(i) Shared, federal share (H6)",
            &d.line30a_i_fea_federal_share,
        )
        .row(
            "  (a)(ii) Shared, Levin share (H6)",
            &d.line30a_ii_fea_levin_share,
        )
        .row(
            "  (b) Paid entirely with federal funds",
            &d.line30b_fea_all_federal,
        )
        .total(
            "  (c) Total federal election activity",
            &d.line30c_fea_total,
        )
        .total("31. Total disbursements", &d.line31_total_disbursements)
        .total(
            "32. Total federal disbursements",
            &d.line32_total_federal_disbursements,
        );
    section_with(
        "Disbursements",
        Some("Detailed Summary · Lines 21–32"),
        vec![table.render(cx)],
        cx,
    )
}

/// Detailed Summary Page, Section III (Lines 33–38).
fn net(f: &Form3X, cx: &App) -> AnyElement {
    let n = &f.detailed_summary.net;
    let table = MoneyTable::new(COLUMNS)
        .row("33. Total contributions", &n.line33_total_contributions)
        .row(
            "34. Total contribution refunds",
            &n.line34_total_contribution_refunds,
        )
        .total("35. Net contributions", &n.line35_net_contributions)
        .row(
            "36. Total federal operating expenditures",
            &n.line36_total_federal_operating_expenditures,
        )
        .row(
            "37. Offsets to operating expenditures",
            &n.line37_offsets_to_operating_expenditures,
        )
        .total(
            "38. Net operating expenditures",
            &n.line38_net_operating_expenditures,
        );
    section_with(
        "Net Contributions & Operating Expenditures",
        Some("Detailed Summary · Lines 33–38"),
        vec![table.render(cx)],
        cx,
    )
}
