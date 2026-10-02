//! Form 3 cover.

use fec_parser::covers::Form3;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form3, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 3",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
