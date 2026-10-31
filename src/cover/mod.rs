//! The cover view: a form-specific layout of a filing's typed cover record
//! ([`fec_parser::covers::Cover`]), one module per form, all built from the
//! shared pieces in [`layout`]. Covers `fec_parser` has no typed struct for
//! fall back to the raw `column → value` list.

mod f1;
mod f13;
mod f1m;
mod f2;
mod f24;
mod f3;
mod f3l;
mod f3p;
mod f3x;
mod f4;
mod f5;
mod f6;
mod f7;
mod f9;
mod f99;
pub mod layout;

use fec_parser::covers::Cover;
use gpui_kit::*;

use crate::filing::FilingSummary;
use layout::{Fields, section};

/// The page children for a typed cover: a banner, then the form's sections.
pub fn render(cover: &Cover, cx: &App) -> Vec<AnyElement> {
    match cover {
        Cover::Form1(f) => f1::render(f, cx),
        Cover::Form1M(f) => f1m::render(f, cx),
        Cover::Form2(f) => f2::render(f, cx),
        Cover::Form3(f) => f3::render(f, cx),
        Cover::Form3L(f) => f3l::render(f, cx),
        Cover::Form3P(f) => f3p::render(f, cx),
        Cover::Form3X(f) => f3x::render(f, cx),
        Cover::Form4(f) => f4::render(f, cx),
        Cover::Form5(f) => f5::render(f, cx),
        Cover::Form6(f) => f6::render(f, cx),
        Cover::Form7(f) => f7::render(f, cx),
        Cover::Form9(f) => f9::render(f, cx),
        Cover::Form13(f) => f13::render(f, cx),
        Cover::Form24(f) => f24::render(f, cx),
        Cover::Form99(f) => f99::render(f, cx),
    }
}

/// The closing "Filing" card every cover ends with: who signed it, the
/// electronic-filing header, and where to find it on fec.gov.
pub fn filing_section(s: &FilingSummary, cx: &App) -> AnyElement {
    let signer = s
        .cover
        .as_ref()
        .and_then(|c| c.signer())
        .map(|p| p.to_string());
    let signed = s.cover.as_ref().and_then(|c| c.date_signed());
    let fields = Fields::new()
        .opt("Signed by", signer)
        .date("Date signed", signed)
        .text("Filing ID", s.filing_id.clone())
        .text("Filer ID", s.filer_id.clone())
        .text("Form type", s.form_type.clone())
        .text("Format version", s.fec_version.clone())
        .text("Software", s.software.clone())
        .opt("Report ID", s.report_id.clone())
        .opt("Report number", s.report_number.clone())
        .opt("Header comment", s.comment.clone())
        .opt("On fec.gov", s.fec_url());
    section("Filing", vec![fields.render(cx)], cx)
}
