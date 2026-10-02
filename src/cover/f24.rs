//! Form 24 cover.

use fec_parser::covers::Form24;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form24, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 24",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
