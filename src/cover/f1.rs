//! Form 1 (Statement of Organization) cover: the committee and its contact
//! details (Lines 1–3), what kind of committee it is (Line 5, with the
//! candidate for authorized committees), its connected organization or
//! affiliate (Line 6), the people responsible for it (Lines 7–8) and its banks
//! (Line 9). People, affiliates and banks are laid out as side-by-side tiles.

use fec_parser::covers::{Form1, Form1Affiliated, Form1Bank, Form1Contact};
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form1, cx: &App) -> Vec<AnyElement> {
    let mut sections = vec![
        banner(
            "Form 1 · Statement of Organization",
            f.committee_name.clone(),
            banner_tags(f, cx),
            Some(banner_subtitle(f)),
            cx,
        ),
        committee(f, cx),
        committee_type(f, cx),
    ];
    sections.extend(candidate(f, cx));
    sections.extend(affiliated(f, cx));
    sections.push(people(f, cx));
    sections.extend(banks(f, cx));
    sections
}

fn banner_tags(f: &Form1, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if let Some(short) = f.committee_type.as_deref().and_then(type_short_name) {
        tags.push(tag(short, Tone::Info, cx));
    }
    if f.pac_flags.leadership_pac {
        tags.push(tag("LEADERSHIP PAC", Tone::Accent, cx));
    }
    if f.pac_flags.is_lobbyist_registrant_pac() {
        tags.push(tag("LOBBYIST/REGISTRANT PAC", Tone::Warning, cx));
    }
    if let Some(party) = &f.party_code {
        tags.push(tag(party.to_uppercase(), Tone::Neutral, cx));
    }
    tags
}

/// `"New statement · effective 2025-07-01"`, or for an amendment which
/// Line 1 items changed and when.
fn banner_subtitle(f: &Form1) -> String {
    let mut parts = vec![];
    if f.is_amendment() {
        parts.push("Amended statement".to_string());
        let changed: Vec<&str> = [
            (f.change_of_committee_name, "name"),
            (f.change_of_address, "address"),
            (f.change_of_committee_email, "e-mail"),
            (f.change_of_committee_url, "website"),
        ]
        .into_iter()
        .filter_map(|(changed, what)| changed.then_some(what))
        .collect();
        if !changed.is_empty() {
            parts.push(format!("{} changed", changed.join(", ")));
        }
    } else {
        parts.push("New statement".to_string());
    }
    if let Some(date) = f.effective_date {
        parts.push(format!("effective {date}"));
    }
    parts.join(" · ")
}

/// Lines 1–3: name, FEC ID, contact details and the Line 2 date, each with
/// its "check if changed" box.
fn committee(f: &Form1, cx: &App) -> AnyElement {
    let name = inline(
        [div().child(f.committee_name.clone()).into_any_element()]
            .into_iter()
            .chain(f.change_of_committee_name.then(|| changed_tag(cx)))
            .collect(),
    );
    let mut fields = Fields::new()
        .element("Name", name)
        .text("FEC ID", f.filer_committee_id.clone())
        .address("Mailing address", &f.address, f.change_of_address, cx);
    if let Some(email) = &f.committee_email {
        // Up to two addresses, separated by a semicolon or a comma.
        let emails = email
            .split([';', ','])
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(|e| div().child(e.to_string()).into_any_element());
        let value = inline(
            emails
                .chain(f.change_of_committee_email.then(|| changed_tag(cx)))
                .collect(),
        );
        fields = fields.element("E-mail", value);
    }
    if let Some(url) = &f.committee_url {
        let value = inline(
            [div().child(url.clone()).into_any_element()]
                .into_iter()
                .chain(f.change_of_committee_url.then(|| changed_tag(cx)))
                .collect(),
        );
        fields = fields.element("Website", value);
    }
    let date_label = if f.is_amendment() {
        "Date of change"
    } else {
        "Date organized"
    };
    let fields = fields.date(date_label, f.effective_date);
    section_with("Committee", Some("Lines 1–3"), vec![fields.render(cx)], cx)
}

/// Line 5: the committee-type box, the connected organization's kind for a
/// separate segregated fund, the party, and the Lobbyist/Registrant PAC and
/// Leadership PAC boxes.
fn committee_type(f: &Form1, cx: &App) -> AnyElement {
    let is_party = type_is(f, "D");
    let mut fields = Fields::new();
    match f.committee_type.as_deref() {
        Some(code) => {
            let value = with_aside(
                code_with_label(code, f.committee_type_label()),
                f.committee_type_line(),
                cx,
            );
            fields = fields.element("Type", strong_element(value));
        }
        None => fields = fields.text("Type", "Not given"),
    }
    fields = fields.opt(
        "Connected organization",
        f.organization_type
            .as_deref()
            .map(|code| code_with_label(code, f.organization_type_label())),
    );
    // A candidate committee's party is the candidate's, shown with them.
    if f.candidate.is_none() || is_party {
        fields = fields.opt(
            "Party",
            f.party_code
                .as_deref()
                .map(|code| code_with_label(code, f.party_code_label())),
        );
    }
    // Some candidate committees fill in a party level too; it only means
    // something for a 5(d) party committee.
    if is_party {
        fields = fields.opt(
            "Party level",
            f.party_type
                .as_deref()
                .map(|code| code_with_label(code, f.party_type_label())),
        );
    }

    let flags = &f.pac_flags;
    let designations: Vec<AnyElement> = [
        (flags.leadership_pac, "Leadership PAC", Tone::Accent),
        (
            flags.lobbyist_registrant_pac_ssf,
            "Lobbyist/Registrant PAC · 5(e)",
            Tone::Warning,
        ),
        (
            flags.lobbyist_registrant_pac_nonconnected,
            "Lobbyist/Registrant PAC · 5(f)",
            Tone::Warning,
        ),
        (
            flags.lobbyist_registrant_pac_super_pac,
            "Lobbyist/Registrant PAC · 5(g)",
            Tone::Warning,
        ),
        (
            flags.lobbyist_registrant_pac_hybrid_pac,
            "Lobbyist/Registrant PAC · 5(h)",
            Tone::Warning,
        ),
    ]
    .into_iter()
    .filter(|(checked, ..)| *checked)
    .map(|(_, label, tone)| tag(label, tone, cx))
    .collect();
    if !designations.is_empty() {
        fields = fields.element("Also", inline(designations));
    }

    let mut children = vec![fields.render(cx)];
    if let Some(about) = f.committee_type.as_deref().and_then(type_description) {
        children.push(note(about, cx));
    }
    if flags.leadership_pac {
        children.push(note(
            "Leadership PAC: established, financed, maintained or controlled by a federal \
             candidate or officeholder, but not an authorized committee or party. Its sponsor \
             is named on Line 6.",
            cx,
        ));
    }
    if flags.is_lobbyist_registrant_pac() {
        children.push(note(
            "Lobbyist/Registrant PAC: established or controlled by a lobbyist or registrant.",
            cx,
        ));
    }
    section_with("Committee Type", Some("Line 5"), children, cx)
}

/// Line 5(a)/(b)/(c): the candidate an authorized or single-candidate
/// committee is tied to.
fn candidate(f: &Form1, cx: &App) -> Option<AnyElement> {
    let c = f.candidate.as_ref()?;
    let office = office_text(
        c.office.as_deref(),
        c.office_label(),
        c.state.as_deref(),
        c.district.as_deref(),
    );
    let name = with_aside(c.full_name(), c.candidate_id.as_deref(), cx);
    let fields = Fields::new()
        .element("Candidate", strong_element(name))
        .opt("Office sought", office)
        .opt(
            "Party",
            f.party_code
                .as_deref()
                .map(|code| code_with_label(code, f.party_code_label())),
        );
    let aside = match f.committee_type_line() {
        Some(line) => format!("Line {line}"),
        None => "Line 5".to_string(),
    };
    Some(section_with(
        "Candidate",
        Some(&aside),
        vec![fields.render(cx)],
        cx,
    ))
}

/// Line 6: the first connected organization, affiliated committee, joint
/// fundraising participant or leadership PAC sponsor. Others are on `F1S`
/// records after the cover.
fn affiliated(f: &Form1, cx: &App) -> Option<AnyElement> {
    let a = f.affiliated.as_ref()?;
    let title = match a
        .relationship_code
        .as_deref()
        .map(str::to_ascii_uppercase)
        .as_deref()
    {
        Some("ORG") => "Connected Organization",
        Some("AFF") => "Affiliated Committee",
        Some("JFR") => "Joint Fundraising",
        Some("LPS") => "Leadership PAC Sponsor",
        _ => "Connected Organization / Affiliates",
    };
    let children = if is_placeholder(a) {
        vec![note(
            format!("None — the filer entered “{}”.", a.display_name()),
            cx,
        )]
    } else {
        vec![
            tiles(vec![affiliated_tile(a, cx)]),
            note(
                "Only the first entry is on the cover; any further ones are filed as F1S \
                 supplements.",
                cx,
            ),
        ]
    };
    Some(section_with(title, Some("Line 6"), children, cx))
}

fn affiliated_tile(a: &Form1Affiliated, cx: &App) -> AnyElement {
    let mut body = vec![strong(a.display_name())];
    // A committee name and an individual can both be given (e.g. a sponsor
    // and their committee); show the individual under the committee.
    if a.committee_name.is_some() && !a.name.is_empty() {
        body.push(div().child(a.name.to_string()).into_any_element());
    }
    let ids = [
        a.committee_id
            .as_deref()
            .map(|id| format!("Committee {id}")),
        a.candidate_id
            .as_deref()
            .map(|id| format!("Candidate {id}")),
    ];
    for id in ids.into_iter().flatten() {
        body.push(muted(id, cx));
    }
    body.extend(address_lines(&a.address));
    let tags = a
        .relationship_code
        .as_deref()
        .map(|code| {
            tag(
                code_with_label(code, a.relationship_label()),
                Tone::Info,
                cx,
            )
        })
        .into_iter()
        .collect();
    // A sponsor is an individual; everything else is a committee or an
    // organization.
    let caption = if a.committee_name.is_none() && !a.name.is_empty() {
        "Individual"
    } else if a.committee_id.is_some() {
        "Committee"
    } else {
        "Organization"
    };
    tile(caption, tags, body, false, cx)
}

/// Lines 7–8: treasurer, designated agent and custodian of records.
fn people(f: &Form1, cx: &App) -> AnyElement {
    let mut row = vec![contact_tile(
        "Treasurer · Line 8",
        &f.treasurer,
        vec![],
        true,
        cx,
    )];
    if let Some(agent) = &f.agent {
        row.push(contact_tile(
            "Designated agent · Line 8",
            agent,
            vec![],
            false,
            cx,
        ));
    }
    if let Some(custodian) = &f.custodian {
        let tags = same_person(custodian, &f.treasurer)
            .then(|| tag("SAME AS TREASURER", Tone::Neutral, cx))
            .into_iter()
            .collect();
        row.push(contact_tile(
            "Custodian of records · Line 7",
            custodian,
            tags,
            false,
            cx,
        ));
    }
    let mut children = vec![tiles(row)];
    if f.agent.is_some() {
        children.push(note(
            "Only the first designated agent is on the cover; others are filed as F1S \
             supplements.",
            cx,
        ));
    }
    section_with("Treasurer & Records", Some("Lines 7–8"), children, cx)
}

fn contact_tile(
    caption: &str,
    c: &Form1Contact,
    tags: Vec<AnyElement>,
    emphasis: bool,
    cx: &App,
) -> AnyElement {
    let name = c.name.to_string();
    let mut body = vec![if name.is_empty() {
        muted("No name given", cx)
    } else {
        strong(name)
    }];
    body.extend(c.title.clone().map(|t| muted(t, cx)));
    body.extend(address_lines(&c.address));
    body.extend(
        c.telephone
            .as_deref()
            .map(|p| div().child(format_phone(p)).into_any_element()),
    );
    tile(caption, tags, body, emphasis, cx)
}

/// Line 9: banks and other depositories (at most two on the cover).
fn banks(f: &Form1, cx: &App) -> Option<AnyElement> {
    if f.banks.is_empty() {
        return None;
    }
    let row = f
        .banks
        .iter()
        .enumerate()
        .map(|(i, bank)| {
            let tags = (i > 0 && *bank == f.banks[0])
                .then(|| tag("SAME AS FIRST", Tone::Neutral, cx))
                .into_iter()
                .collect();
            bank_tile(i, bank, tags, cx)
        })
        .collect();
    Some(section_with(
        "Banks & Depositories",
        Some("Line 9"),
        vec![
            tiles(row),
            note(
                "At most two depositories fit on the cover; others are filed as F1S \
                 supplements.",
                cx,
            ),
        ],
        cx,
    ))
}

fn bank_tile(i: usize, bank: &Form1Bank, tags: Vec<AnyElement>, cx: &App) -> AnyElement {
    let mut body = vec![match &bank.name {
        Some(name) => strong(name.clone()),
        None => muted("No name given", cx),
    }];
    body.extend(address_lines(&bank.address));
    tile(&format!("Depository {}", i + 1), tags, body, false, cx)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn type_is(f: &Form1, code: &str) -> bool {
    f.committee_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case(code))
}

/// A short banner tag for each Line 5 box.
fn type_short_name(code: &str) -> Option<&'static str> {
    Some(match code.trim().to_ascii_uppercase().as_str() {
        "A" => "PRINCIPAL CAMPAIGN COMMITTEE",
        "B" => "AUTHORIZED COMMITTEE",
        "C" => "SINGLE-CANDIDATE COMMITTEE",
        "D" => "PARTY COMMITTEE",
        "E" => "SEPARATE SEGREGATED FUND",
        "F" => "NONCONNECTED PAC",
        "G" => "SUPER PAC",
        "H" => "HYBRID PAC",
        "I" | "J" => "JOINT FUNDRAISING",
        _ => return None,
    })
}

/// A plain-language line about each Line 5 box, after the Form 1
/// instructions.
fn type_description(code: &str) -> Option<&'static str> {
    Some(match code.trim().to_ascii_uppercase().as_str() {
        "A" => "The candidate's principal campaign committee, designated on their Form 2.",
        "B" => {
            "An authorized committee of the candidate other than the principal campaign committee."
        }
        "C" => "Supports or opposes a single candidate without being authorized by them.",
        "D" => "A national, state or subordinate committee of a political party.",
        "E" => {
            "A PAC established by a corporation, labor organization, membership organization, \
             trade association or cooperative — its connected organization."
        }
        "F" => {
            "A nonconnected committee: supports or opposes more than one federal candidate and \
             is neither a separate segregated fund nor a party committee."
        }
        "G" => "Makes only independent expenditures, so it may accept unlimited contributions.",
        "H" => {
            "Keeps a contribution account plus a separate non-contribution account for \
             independent expenditures."
        }
        "I" => {
            "Joint fundraising representative; at least one participant is an authorized committee."
        }
        "J" => "Joint fundraising representative; no participant is an authorized committee.",
        _ => return None,
    })
}

/// True for a Line 6 entry that only says there is none (`NONE`, `N/A`, …).
fn is_placeholder(a: &Form1Affiliated) -> bool {
    let name = a.display_name();
    let name = name.trim().trim_end_matches('.').to_ascii_uppercase();
    matches!(name.as_str(), "" | "NONE" | "N/A" | "NA" | "NOT APPLICABLE")
        && a.committee_id.is_none()
        && a.candidate_id.is_none()
        && a.address.is_empty()
        && a.relationship_code.is_none()
}

/// Same name and address (titles differ: "Treasurer" vs "Custodian").
fn same_person(a: &Form1Contact, b: &Form1Contact) -> bool {
    a.name == b.name && a.address == b.address
}
