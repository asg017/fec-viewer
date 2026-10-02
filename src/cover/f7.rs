//! Form 7 cover.

use fec_parser::covers::Form7;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form7, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 7",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
