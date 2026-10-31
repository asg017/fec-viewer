//! Form 2 (Statement of Candidacy) cover: the candidate (Lines 1–6), the
//! principal campaign committee (Line 7) and the first other authorized
//! committee (Line 8). The form carries no amounts, except the legacy
//! declaration of intent to spend personal funds.

use fec_parser::covers::{Form2, Form2Committee};
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form2, cx: &App) -> Vec<AnyElement> {
    let mut children = vec![
        banner(
            "Form 2 · Statement of Candidacy",
            title(f),
            amendment_tag(f.is_amendment(), cx).into_iter().collect(),
            // Form 2 records no original filing date, so the AMENDMENT tag
            // says all an amendment line could; the race says more.
            race_text(f).or_else(|| amendment_text(f.is_amendment(), None, "statement")),
            cx,
        ),
        candidate(f, cx),
        committee(
            "Principal Campaign Committee",
            "Line 7",
            &f.principal_committee,
            cx,
        ),
    ];
    if let Some(c) = &f.authorized_committee {
        children.push(committee("Other Authorized Committee", "Line 8", c, cx));
    }
    if let Some(d) = &f.personal_funds_declaration {
        let fields = Fields::new()
            .amount_opt("Primary", d.primary, cx)
            .amount_opt("General", d.general, cx);
        children.push(section(
            "Declaration of Intent to Spend Personal Funds",
            vec![fields.render(cx)],
            cx,
        ));
    }
    children
}

/// The candidate's name, or their ID when the name is blank.
fn title(f: &Form2) -> String {
    if f.candidate.is_empty() {
        f.candidate_id.clone()
    } else {
        f.candidate.to_string()
    }
}

/// `"Candidate for House, CA-32 · 2028 election"`.
fn race_text(f: &Form2) -> Option<String> {
    let mut parts = vec![];
    if let Some(office) = f.office_label().or(f.office.as_deref()) {
        let mut race = format!("Candidate for {office}");
        if let Some(seat) = seat_text(f.office_state.as_deref(), f.district.as_deref()) {
            race.push_str(&format!(", {seat}"));
        }
        parts.push(race);
    }
    if let Some(year) = f.election_year {
        parts.push(format!("{year} election"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Lines 1–6 plus the running mate.
fn candidate(f: &Form2, cx: &App) -> AnyElement {
    let id = (!f.candidate_id.is_empty()).then_some(f.candidate_id.as_str());
    let fields = Fields::new()
        .element("Candidate", with_aside(title(f), id, cx))
        .address("Address", &f.candidate_address, f.change_of_address, cx)
        .opt(
            "Party",
            f.party_code
                .as_deref()
                .map(|code| code_with_label(code, f.party_code_label())),
        )
        .opt(
            "Office sought",
            f.office
                .as_deref()
                .map(|code| code_with_label(code, f.office_label())),
        )
        .opt(
            "State & district",
            seat_text(f.office_state.as_deref(), f.district.as_deref()),
        )
        .opt("Election year", f.election_year.map(|y| y.to_string()));
    let fields = match &f.vice_president {
        Some(vp) => fields.person("Running mate", vp),
        None => fields,
    };
    section_with("Candidate", Some("Lines 1–6"), vec![fields.render(cx)], cx)
}

/// A designated committee (Line 7 or 8).
fn committee(title: &str, line: &str, c: &Form2Committee, cx: &App) -> AnyElement {
    if c.is_empty() {
        return section_with(title, Some(line), vec![note("Not filled in.", cx)], cx);
    }
    let name = c.name.clone().unwrap_or_else(|| "(no name given)".into());
    let fields = Fields::new()
        .element("Committee", with_aside(name, c.id.as_deref(), cx))
        .address("Address", &c.address, false, cx);
    let mut children = vec![fields.render(cx)];
    if c.id.is_none() {
        children.push(note(
            "No committee ID yet: a new committee is assigned one after it files its Form 1.",
            cx,
        ));
    }
    if line == "Line 8" {
        children.push(note(
            "Any further authorized committees are listed in the filing's F2S records.",
            cx,
        ));
    }
    section_with(title, Some(line), children, cx)
}
