//! Form 1M (Notification of Multicandidate Status) cover: the committee
//! (Lines 1–3), then whichever path to multicandidate status the treasurer
//! certified — Line 4, through affiliation with an existing multicandidate
//! committee, or Line 5, by qualifying itself (five candidates, 51
//! contributors, six months' registration). The Line 5 dates lead as a row of
//! milestone tiles, ending with the date the committee qualified.

use fec_parser::covers::{Form1M, Form1MAffiliation, Form1MCandidate, Form1MQualification};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{prelude::FluentBuilder as _, *};
use jiff::ToSpan as _;

use super::layout::*;

/// Row numbers of the Line 5(a) candidate table.
const ROMAN: [&str; 5] = ["(i)", "(ii)", "(iii)", "(iv)", "(v)"];
/// Widths of the Line 5(a) table's fixed columns.
const ROW_NUMBER_WIDTH: f32 = 40.;
const OFFICE_WIDTH: f32 = 150.;
const DATE_WIDTH: f32 = 110.;
/// Narrowest a milestone tile gets, so all four fit on one row.
const MILESTONE_MIN_WIDTH: f32 = 170.;

pub fn render(f: &Form1M, cx: &App) -> Vec<AnyElement> {
    let mut sections = vec![banner(
        "Form 1M · Notification of Multicandidate Status",
        f.committee_name.clone(),
        banner_tags(f, cx),
        banner_subtitle(f),
        cx,
    )];
    sections.extend(f.qualification.as_ref().map(|q| milestones(q, cx)));
    sections.push(committee(f, cx));
    sections.extend(f.affiliation.as_ref().map(|a| affiliation(a, cx)));
    sections.extend(f.qualification.as_ref().map(|q| qualification(q, cx)));
    if f.affiliation.is_none() && f.qualification.is_none() {
        sections.push(section(
            "Multicandidate Status",
            vec![note("Neither Line 4 nor Line 5 was completed.", cx)],
            cx,
        ));
    }
    sections
}

fn banner_tags(f: &Form1M, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.affiliation.is_some() {
        tags.push(tag("BY AFFILIATION", Tone::Success, cx));
    }
    if f.qualification.is_some() {
        tags.push(tag("BY QUALIFICATION", Tone::Success, cx));
    }
    if is_state_party(f) {
        tags.push(tag("STATE PARTY", Tone::Info, cx));
    }
    tags
}

/// When and how the committee became a multicandidate committee.
fn banner_subtitle(f: &Form1M) -> Option<String> {
    let prefix = if f.is_amendment() {
        "Amended notification · "
    } else {
        ""
    };
    let text = if let Some(date) = f
        .qualification
        .as_ref()
        .and_then(|q| q.requirements_met_date)
    {
        format!("Qualified as a multicandidate committee on {date}")
    } else if let Some(a) = &f.affiliation {
        match &a.committee_name {
            Some(name) => format!("Multicandidate through affiliation with {name}"),
            None => "Multicandidate through affiliation".to_string(),
        }
    } else if f.is_amendment() {
        return Some("Amended notification".to_string());
    } else {
        return None;
    };
    Some(format!("{prefix}{text}"))
}

fn is_state_party(f: &Form1M) -> bool {
    f.committee_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("X"))
}

/// Lines 1–3.
fn committee(f: &Form1M, cx: &App) -> AnyElement {
    let fields = Fields::new()
        .text("Name", f.committee_name.clone())
        .text("FEC ID", f.filer_committee_id.clone())
        .address("Mailing address", &f.address, false, cx)
        .opt(
            "Type",
            f.committee_type
                .as_deref()
                .map(|code| code_with_label(code, f.committee_type_label())),
        )
        .person("Treasurer", &f.treasurer)
        .date("Date signed", f.date_signed);
    let mut children = vec![fields.render(cx)];
    if f.committee_type.is_some() {
        children.push(note(
            "Line 3 is “State Party” or “Other”; national and local party committees, \
             nonconnected committees and separate segregated funds check “Other”.",
            cx,
        ));
    }
    section_with("Committee", Some("Lines 1–3"), children, cx)
}

/// Line 4: status through affiliation with an existing multicandidate
/// committee.
fn affiliation(a: &Form1MAffiliation, cx: &App) -> AnyElement {
    let intro = match a.date_form1_filed {
        Some(date) => format!(
            "The committee submitted its Statement of Organization (Form 1) on {date} and \
             simultaneously qualified as a multicandidate committee through its affiliation with:"
        ),
        None => "The committee qualified as a multicandidate committee through its affiliation \
                 with:"
            .to_string(),
    };
    let mut body = vec![match &a.committee_name {
        Some(name) => strong(name.clone()),
        None => muted("No name given", cx),
    }];
    body.extend(
        a.committee_id
            .as_deref()
            .map(|id| muted(id.to_string(), cx)),
    );
    section_with(
        "Status by Affiliation",
        Some("Line 4"),
        vec![
            div().child(intro).into_any_element(),
            tiles(vec![tile(
                "Affiliated multicandidate committee",
                vec![],
                body,
                true,
                cx,
            )]),
        ],
        cx,
    )
}

/// The Line 5 dates as tiles, in the order the requirements are usually met,
/// ending with the qualification date.
fn milestones(q: &Form1MQualification, cx: &App) -> AnyElement {
    let six_months = q
        .original_registration_date
        .and_then(|d| d.checked_add(6.months()).ok());
    let latest_candidate = q
        .candidates
        .iter()
        .filter_map(|c| c.contribution_date)
        .max();
    let line = |l: &str, extra: Option<String>| match extra {
        Some(extra) => format!("Line {l} · {extra}"),
        None => format!("Line {l}"),
    };
    tiles(vec![
        milestone(
            "Registered",
            q.original_registration_date,
            line("5(c)", six_months.map(|d| format!("six months on {d}"))),
            false,
            cx,
        ),
        milestone(
            "51st contributor",
            q.fifty_first_contributor_date,
            line("5(b)", None),
            false,
            cx,
        ),
        milestone(
            "Five candidates",
            latest_candidate,
            line(
                "5(a)",
                Some(format!("latest of {} listed", q.candidates.len())),
            ),
            false,
            cx,
        ),
        milestone(
            "Qualified",
            q.requirements_met_date,
            line("5(d)", Some("final requirement met".to_string())),
            true,
            cx,
        ),
    ])
}

fn milestone(
    caption: &str,
    date: Option<jiff::civil::Date>,
    detail: String,
    emphasis: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let value = match date {
        Some(date) => div()
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .font_family(theme.mono_font_family.clone())
            .child(date.to_string())
            .into_any_element(),
        None => div()
            .text_lg()
            .text_color(theme.muted_foreground)
            .child("—")
            .into_any_element(),
    };
    let detail = div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(detail)
        .into_any_element();
    tile_sized(
        MILESTONE_MIN_WIDTH,
        caption,
        vec![],
        vec![value, detail],
        emphasis,
        cx,
    )
}

/// Line 5: the five candidates, then the 51st-contributor, registration and
/// qualification dates.
fn qualification(q: &Form1MQualification, cx: &App) -> AnyElement {
    let candidates = if q.candidates.is_empty() {
        note(
            "No candidates listed — only State party committees may leave Line 5(a) blank.",
            cx,
        )
    } else {
        candidate_table(&q.candidates, cx)
    };
    let fields = Fields::new()
        .date("51st contributor", q.fifty_first_contributor_date)
        .date("Form 1 submitted", q.original_registration_date)
        .element(
            "Requirements met",
            match q.requirements_met_date {
                Some(date) => strong(date.to_string()),
                None => muted("—", cx),
            },
        );
    section_with(
        "Status by Qualification",
        Some("Line 5"),
        vec![
            caption("(a) Contributions to five federal candidates", cx),
            candidates,
            caption("(b)–(d) Contributors, registration and qualification", cx),
            fields.render(cx),
        ],
        cx,
    )
}

/// A muted sub-heading inside a section.
fn caption(text: &str, cx: &App) -> AnyElement {
    div()
        .pt_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text.to_string())
        .into_any_element()
}

/// Line 5(a) rows (i)–(v): name and ID, office, and the contribution date.
fn candidate_table(candidates: &[Form1MCandidate], cx: &App) -> AnyElement {
    let theme = cx.theme();
    let header = h_flex()
        .gap_3()
        .pb_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .border_b_1()
        .border_color(theme.border)
        .child(div().w(px(ROW_NUMBER_WIDTH)).flex_shrink_0())
        .child(div().flex_1().min_w_0().child("Candidate"))
        .child(
            div()
                .w(px(OFFICE_WIDTH))
                .flex_shrink_0()
                .child("Office sought"),
        )
        .child(
            div()
                .w(px(DATE_WIDTH))
                .flex_shrink_0()
                .text_right()
                .child("Contributed"),
        );
    let rows = candidates.iter().enumerate().map(|(i, c)| {
        let name = c.name.to_string();
        let name = if name.is_empty() {
            muted("No name given", cx)
        } else {
            strong_element(with_aside(name, c.candidate_id.as_deref(), cx))
        };
        let office = office_text(
            c.office.as_deref(),
            c.office_label(),
            c.state.as_deref(),
            c.district.as_deref(),
        );
        h_flex()
            .gap_3()
            .py_1()
            .items_start()
            .when(i > 0, |d| {
                d.border_t_1().border_color(theme.border.opacity(0.5))
            })
            .child(
                div()
                    .w(px(ROW_NUMBER_WIDTH))
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground)
                    .child(ROMAN.get(i).copied().unwrap_or("")),
            )
            .child(div().flex_1().min_w_0().child(name))
            .child(
                div()
                    .w(px(OFFICE_WIDTH))
                    .flex_shrink_0()
                    .child(office.unwrap_or_default()),
            )
            .child(
                div()
                    .w(px(DATE_WIDTH))
                    .flex_shrink_0()
                    .text_right()
                    .font_family(theme.mono_font_family.clone())
                    .child(
                        c.contribution_date
                            .map(|d| d.to_string())
                            .unwrap_or_default(),
                    ),
            )
    });
    v_flex()
        .w_full()
        .child(header)
        .children(rows)
        .into_any_element()
}
