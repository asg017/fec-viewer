//! Form 1 cover.

use fec_parser::covers::Form1;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form1, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 1",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
