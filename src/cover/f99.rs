//! Form 99 (Miscellaneous Text) cover: the committee, the kind of document,
//! and the letter itself.

use fec_parser::covers::Form99;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form99, cx: &App) -> Vec<AnyElement> {
    let mut tags: Vec<AnyElement> = amendment_tag(f.is_amendment(), cx).into_iter().collect();
    if f.pdf_attachment {
        tags.push(tag("PDF ATTACHMENT", Tone::Info, cx));
    }
    vec![
        banner(
            "Form 99 · Miscellaneous Text",
            match (f.text_code_label(), f.text_code.as_deref()) {
                (Some(label), _) => label.to_string(),
                (None, Some(code)) => format!("Miscellaneous Submission ({code})"),
                (None, None) => "Miscellaneous Submission".to_string(),
            },
            tags,
            None,
            cx,
        ),
        committee(f, cx),
        message(f, cx),
    ]
}

fn committee(f: &Form99, cx: &App) -> AnyElement {
    let id = (!f.filer_committee_id.is_empty()).then_some(f.filer_committee_id.as_str());
    let fields = Fields::new()
        .element("Committee", with_aside(f.committee_name.clone(), id, cx))
        .address("Address", &f.address, false, cx)
        .opt(
            "Document type",
            f.text_code
                .as_deref()
                .map(|code| code_with_label(code, f.text_code_label())),
        )
        // The meaning of the 8.5 `filing_frequency` codes (M, Q) isn't
        // sourced, so show them as filed.
        .opt("Filing frequency", f.filing_frequency.clone())
        .opt("PDF attachment", f.pdf_attachment.then_some("Yes"))
        .person("Treasurer", &f.treasurer);
    section("Committee", vec![fields.render(cx)], cx)
}

fn message(f: &Form99, cx: &App) -> AnyElement {
    let body = match f.text.as_deref().filter(|t| !t.trim().is_empty()) {
        Some(text) => prose(text.trim_matches('\n'), cx),
        None => note("This submission has no message text.", cx),
    };
    section("Message", vec![body], cx)
}
