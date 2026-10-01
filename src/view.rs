use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use gpui_kit::component::{
    ActiveTheme as _, h_flex,
    table::{Column, DataTable, TableDelegate, TableEvent, TableState},
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::index::FilingIndex;

/// Cap on cached parsed rows; plenty for a screenful plus scroll-back.
const ROW_CACHE_CAP: usize = 4_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Cover,
    Group(usize),
}

// ---------------------------------------------------------------------------
// Table delegate: one row type at a time, rows parsed lazily from the mmap.
// ---------------------------------------------------------------------------

pub struct RowsDelegate {
    index: Option<Arc<FilingIndex>>,
    group: usize,
    columns: Vec<String>,
    row_count: usize,
    cache: HashMap<usize, Arc<Vec<String>>>,
}

impl RowsDelegate {
    fn empty() -> Self {
        Self {
            index: None,
            group: 0,
            columns: Vec::new(),
            row_count: 0,
            cache: HashMap::new(),
        }
    }

    fn row(&mut self, row_ix: usize) -> Arc<Vec<String>> {
        if let Some(row) = self.cache.get(&row_ix) {
            return row.clone();
        }
        if self.cache.len() >= ROW_CACHE_CAP {
            self.cache.clear();
        }
        let row = Arc::new(self.read_row(row_ix));
        self.cache.insert(row_ix, row.clone());
        row
    }

    fn read_row(&self, row_ix: usize) -> Vec<String> {
        let Some(index) = &self.index else {
            return Vec::new();
        };
        let offset = {
            let scan = index.scan.lock().expect("scan lock poisoned");
            scan.groups
                .get(self.group)
                .and_then(|g| g.offsets.get(row_ix).copied())
        };
        offset.map(|o| index.read_row(o)).unwrap_or_default()
    }
}

fn is_amount_column(name: &str) -> bool {
    name.contains("amount") || name.contains("aggregate") || name.ends_with("_total")
}

impl TableDelegate for RowsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.row_count
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let name = &self.columns[col_ix];
        let width = (name.len() as f32 * 7.0 + 24.0).clamp(80.0, 260.0);
        let col = Column::new(name.clone(), name.clone()).width(px(width));
        if is_amount_column(name) {
            col.text_right()
        } else {
            col
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = self.row(row_ix);
        let value = row.get(col_ix).cloned().unwrap_or_default();
        div().truncate().child(value)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.read_row(row_ix)
            .get(col_ix)
            .cloned()
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Window root view
// ---------------------------------------------------------------------------

pub struct FilingView {
    index: Option<Arc<FilingIndex>>,
    error: Option<String>,
    selection: Selection,
    selected_row: Option<usize>,
    table: Entity<TableState<RowsDelegate>>,
    _poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl FilingView {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(RowsDelegate::empty(), window, cx)
                .col_movable(false)
                .sortable(false)
        });
        let subscription = cx.subscribe(&table, |this, _, event: &TableEvent, cx| {
            if let TableEvent::SelectRow(ix) = event {
                this.selected_row = Some(*ix);
                cx.notify();
            }
        });
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            gpui_kit::component::Theme::sync_system_appearance(Some(window), cx);
        });
        let mut this = Self {
            index: None,
            error: None,
            selection: Selection::Cover,
            selected_row: None,
            table,
            _poll: None,
            _subscriptions: vec![subscription, appearance],
        };
        if let Some(path) = path {
            this.load(path, window, cx);
        }
        this
    }

    pub fn has_filing(&self) -> bool {
        self.index.is_some()
    }

    pub fn load(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        window.set_window_title(&title);
        window.set_window_edited(false);

        match FilingIndex::open(&path) {
            Ok(index) => {
                self.index = Some(index);
                self.error = None;
                self.selection = Selection::Cover;
                self.selected_row = None;
                self.start_polling(cx);
            }
            Err(e) => {
                self.index = None;
                self.error = Some(format!("{e:#}"));
            }
        }
        cx.notify();
    }

    /// While the background scan runs, refresh counts ~10x/second.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        this.sync_row_count(cx);
                        cx.notify();
                        this.index
                            .as_ref()
                            .is_none_or(|i| i.scan.lock().is_ok_and(|s| s.done))
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }

    fn sync_row_count(&mut self, cx: &mut Context<Self>) {
        let Selection::Group(group) = self.selection else {
            return;
        };
        let Some(index) = &self.index else { return };
        let (count, ncols) = {
            let scan = index.scan.lock().expect("scan lock poisoned");
            scan.groups
                .get(group)
                .map(|g| (g.offsets.len(), g.columns.len()))
                .unwrap_or_default()
        };
        self.table.update(cx, |table, cx| {
            let d = table.delegate_mut();
            let columns_changed = d.columns.len() != ncols;
            if d.row_count != count || columns_changed {
                d.row_count = count;
                if columns_changed {
                    if let Some(g) = index
                        .scan
                        .lock()
                        .expect("scan lock poisoned")
                        .groups
                        .get(group)
                    {
                        d.columns = g.columns.clone();
                    }
                    table.refresh(cx);
                }
                cx.notify();
            }
        });
    }

    fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selection == selection {
            return;
        }
        self.selection = selection;
        self.selected_row = None;
        if let (Selection::Group(group), Some(index)) = (selection, &self.index) {
            let (columns, count) = {
                let scan = index.scan.lock().expect("scan lock poisoned");
                scan.groups
                    .get(group)
                    .map(|g| (g.columns.clone(), g.offsets.len()))
                    .unwrap_or_default()
            };
            let index = index.clone();
            self.table.update(cx, |table, cx| {
                let d = table.delegate_mut();
                d.index = Some(index);
                d.group = group;
                d.columns = columns;
                d.row_count = count;
                d.cache.clear();
                table.clear_selection(cx);
                table.refresh(cx);
                table.scroll_to_row(0, cx);
            });
        }
        cx.notify();
    }

    fn on_drop_paths(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut paths = paths.paths().iter().cloned();
        if let Some(first) = paths.next() {
            self.load(first, window, cx);
        }
        for extra in paths {
            crate::open_filing_window(Some(extra), cx);
        }
    }
}

impl Render for FilingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let root = v_flex()
            .id("filing-view")
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_sm()
            .on_drop(cx.listener(Self::on_drop_paths))
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().drop_target));

        match (&self.index, &self.error) {
            (Some(_), _) => root
                .child(self.render_header(cx))
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .child(self.render_sidebar(cx))
                        .child(self.render_main(window, cx)),
                )
                .child(self.render_status_bar(cx)),
            (None, error) => root.child(render_empty(error.clone(), cx)),
        }
    }
}

impl FilingView {
    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let Some(index) = &self.index else {
            return div();
        };
        let s = &index.summary;
        let id = if s.filing_id.bytes().all(|b| b.is_ascii_digit()) {
            format!("FEC-{}", s.filing_id)
        } else {
            s.filing_id.clone()
        };
        let mut meta = vec![id, s.form_type.clone()];
        if let Some(label) = s.report_label {
            meta.push(label.to_string());
        }
        if let (Some(from), Some(through)) = (&s.coverage_from, &s.coverage_through) {
            meta.push(format!("{from} → {through}"));
        }
        meta.push(format!("v{} · {}", s.fec_version, s.software));

        div().child(
            v_flex()
                .px_4()
                .py_3()
                .gap_1()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(s.filer_name.clone()),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(s.filer_id.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(meta.join("  ·  ")),
                ),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let groups: Vec<(String, usize)> = self
            .index
            .as_ref()
            .map(|i| {
                let scan = i.scan.lock().expect("scan lock poisoned");
                scan.groups
                    .iter()
                    .map(|g| (g.row_type.clone(), g.offsets.len()))
                    .collect()
            })
            .unwrap_or_default();

        let item = |id: ElementId, label: String, count: Option<usize>, selected: bool| {
            h_flex()
                .id(id)
                .px_3()
                .py_1()
                .mx_1()
                .rounded_md()
                .justify_between()
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme.sidebar_accent)
                        .text_color(theme.sidebar_accent_foreground)
                })
                .when(!selected, |d| d.hover(|d| d.bg(theme.list_hover)))
                .child(div().font_family("monospace").child(label))
                .children(count.map(|c| {
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format_count(c))
                }))
        };

        let mut list = v_flex()
            .id("sidebar")
            .w(px(220.))
            .h_full()
            .flex_shrink_0()
            .py_2()
            .gap_px()
            .overflow_y_scroll()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                item(
                    "cover".into(),
                    "Cover".into(),
                    None,
                    self.selection == Selection::Cover,
                )
                .on_click(cx.listener(|this, _, _, cx| this.select(Selection::Cover, cx))),
            )
            .child(
                div()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("ROW TYPES"),
            );
        for (ix, (row_type, count)) in groups.into_iter().enumerate() {
            let selected = self.selection == Selection::Group(ix);
            list = list.child(
                item(("group", ix).into(), row_type, Some(count), selected).on_click(
                    cx.listener(move |this, _, _, cx| this.select(Selection::Group(ix), cx)),
                ),
            );
        }
        list
    }

    fn render_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let content = match self.selection {
            Selection::Cover => {
                let kv = self
                    .index
                    .as_ref()
                    .map(|i| i.summary.cover_kv.clone())
                    .unwrap_or_default();
                render_kv("cover-kv", kv, cx).into_any_element()
            }
            Selection::Group(_) => h_flex()
                .size_full()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(DataTable::new(&self.table).stripe(true)),
                )
                .children(self.selected_row.map(|row_ix| {
                    let columns = self.table.read(cx).delegate().columns.clone();
                    let values = self.table.update(cx, |t, _| t.delegate_mut().row(row_ix));
                    let kv = columns
                        .into_iter()
                        .zip(values.iter().cloned())
                        .filter(|(_, v)| !v.is_empty())
                        .collect();
                    div()
                        .w(px(360.))
                        .h_full()
                        .flex_shrink_0()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .child(render_kv(("row-kv", row_ix), kv, cx))
                }))
                .into_any_element(),
        };
        div().flex_1().min_w_0().h_full().child(content)
    }

    fn render_status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let text = self
            .index
            .as_ref()
            .map(|index| {
                let scan = index.scan.lock().expect("scan lock poisoned");
                let size = format_bytes(index.file_len);
                if let Some(err) = &scan.error {
                    format!(
                        "{} rows · stopped early: {err}",
                        format_count(scan.rows_scanned)
                    )
                } else if scan.done {
                    format!(
                        "{} · {size} · indexed in {:.2}s",
                        plural(scan.rows_scanned, "row"),
                        scan.elapsed.as_secs_f64()
                    )
                } else {
                    let pct = scan.bytes_scanned as f64 / index.file_len.max(1) as f64 * 100.0;
                    format!(
                        "Indexing… {pct:.0}% · {} rows · {size}",
                        format_count(scan.rows_scanned)
                    )
                }
            })
            .unwrap_or_default();
        h_flex()
            .px_3()
            .py_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .child(text)
    }
}

fn render_kv(id: impl Into<ElementId>, kv: Vec<(String, String)>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .id(id.into())
        .size_full()
        .overflow_y_scroll()
        .p_3()
        .children(kv.into_iter().map(|(k, v)| {
            h_flex()
                .py_1()
                .gap_3()
                .items_start()
                .border_b_1()
                .border_color(theme.table_row_border)
                .child(
                    div()
                        .w(px(150.))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(k),
                )
                .child(div().flex_1().min_w_0().child(v))
        }))
}

fn render_empty(error: Option<String>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(div().text_lg().child("Drop a .fec file here"))
        .child(
            div()
                .text_color(theme.muted_foreground)
                .child("or choose File → Open… (⌘O / Ctrl+O)"),
        )
        .children(error.map(|e| {
            div()
                .mt_4()
                .max_w(px(560.))
                .text_color(theme.danger)
                .child(e)
        }))
}

fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

fn plural(n: usize, noun: &str) -> String {
    let s = if n == 1 { "" } else { "s" };
    format!("{} {noun}{s}", format_count(n))
}
