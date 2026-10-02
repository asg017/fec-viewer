//! Form 4 cover.

use fec_parser::covers::Form4;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form4, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 4",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
