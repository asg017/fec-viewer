//! Form 13 cover.

use fec_parser::covers::Form13;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form13, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 13",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
