//! Form 99 cover.

use fec_parser::covers::Form99;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form99, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 99",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
