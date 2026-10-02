//! Form 3L cover.

use fec_parser::covers::Form3L;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form3L, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 3L",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
