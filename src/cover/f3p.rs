//! Form 3P cover.

use fec_parser::covers::Form3P;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form3P, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 3P",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
