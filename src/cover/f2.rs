//! Form 2 cover.

use fec_parser::covers::Form2;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form2, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 2",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
