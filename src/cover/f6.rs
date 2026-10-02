//! Form 6 cover.

use fec_parser::covers::Form6;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form6, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 6",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
