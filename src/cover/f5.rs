//! Form 5 cover.

use fec_parser::covers::Form5;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form5, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 5",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
