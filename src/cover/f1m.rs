//! Form 1M cover.

use fec_parser::covers::Form1M;
use gpui_kit::*;

use super::layout::*;

pub fn render(f: &Form1M, cx: &App) -> Vec<AnyElement> {
    vec![banner(
        "Form 1M",
        f.form_type.clone(),
        amendment_tag(f.is_amendment(), cx).into_iter().collect(),
        None,
        cx,
    )]
}
