use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use fec_parser::mappings::column_names_for_field;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex,
    table::{Column, DataTable, TableDelegate, TableEvent, TableState},
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::filing::{CollectState, Collection, Matcher, OpenFiling, Schedule, Source};

/// Cap on cached parsed rows; plenty for a screenful plus scroll-back.
const ROW_CACHE_CAP: usize = 4_000;

#[derive(Clone, PartialEq, Eq)]
enum Selection {
    Cover,
    Records(Matcher),
}

// ---------------------------------------------------------------------------
// Table delegate: rows from the current collection, parsed per row on demand.
// ---------------------------------------------------------------------------

pub struct RowsDelegate {
    state: Option<Arc<Mutex<CollectState>>>,
    columns: Vec<String>,
    row_count: usize,
    cache: HashMap<usize, Arc<Vec<String>>>,
}

impl RowsDelegate {
    fn empty() -> Self {
        Self {
            state: None,
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
        self.state
            .as_ref()
            .map(|s| s.lock().expect("collect lock poisoned").row(row_ix))
            .unwrap_or_default()
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
// Sidebar tree
// ---------------------------------------------------------------------------

struct Tree {
    schedules: BTreeMap<Schedule, Vec<(String, usize)>>,
    other: Vec<(String, usize)>,
}

impl Tree {
    fn new(types: &[(String, usize)]) -> Self {
        let mut schedules: BTreeMap<Schedule, Vec<(String, usize)>> = BTreeMap::new();
        let mut other = Vec::new();
        for (row_type, count) in types {
            match Schedule::of(row_type) {
                Some(s) => schedules
                    .entry(s)
                    .or_default()
                    .push((row_type.clone(), *count)),
                None => other.push((row_type.clone(), *count)),
            }
        }
        for lines in schedules.values_mut() {
            lines.sort_by_key(|(t, _)| natural_key(t));
        }
        Self { schedules, other }
    }
}

/// Sort `SA9` before `SA11AI`: letters, then the first number, then the rest.
fn natural_key(s: &str) -> (String, u64, String) {
    let digits_at = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
    let (head, tail) = s.split_at(digits_at);
    let digits_end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    let n = tail[..digits_end].parse().unwrap_or(0);
    (head.to_owned(), n, tail[digits_end..].to_owned())
}

// ---------------------------------------------------------------------------
// Window root view
// ---------------------------------------------------------------------------

pub struct FilingView {
    filing: Option<Arc<OpenFiling>>,
    error: Option<String>,
    selection: Selection,
    collection: Option<Collection>,
    collapsed: HashSet<Schedule>,
    selected_row: Option<usize>,
    table: Entity<TableState<RowsDelegate>>,
    _poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl FilingView {
    pub fn new(source: Option<Source>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            filing: None,
            error: None,
            selection: Selection::Cover,
            collection: None,
            collapsed: HashSet::new(),
            selected_row: None,
            table,
            _poll: None,
            _subscriptions: vec![subscription, appearance],
        };
        if let Some(source) = source {
            this.load(source, window, cx);
        }
        this
    }

    pub fn has_filing(&self) -> bool {
        self.filing.is_some()
    }

    pub fn load(&mut self, source: Source, window: &mut Window, cx: &mut Context<Self>) {
        window.set_window_title(&source.title());
        window.set_window_edited(false);

        self.selection = Selection::Cover;
        self.collection = None;
        self.collapsed.clear();
        self.selected_row = None;
        match OpenFiling::open(source) {
            Ok(filing) => {
                self.filing = Some(filing);
                self.error = None;
                self.start_polling(cx);
            }
            Err(e) => {
                self.filing = None;
                self.error = Some(format!("{e:#}"));
            }
        }
        cx.notify();
    }

    /// Report a filing that couldn't be loaded. Shown in the empty state, so
    /// an open filing stays put.
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub fn show_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }

    fn counting_done(&self) -> bool {
        self.filing
            .as_ref()
            .is_none_or(|f| f.counts.lock().is_ok_and(|s| s.done))
    }

    fn collecting_done(&self) -> bool {
        self.collection
            .as_ref()
            .is_none_or(|c| c.state.lock().is_ok_and(|s| s.done))
    }

    /// While a background pass runs, refresh ~10x/second.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        this.sync_table(cx);
                        cx.notify();
                        this.counting_done() && this.collecting_done()
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }

    /// Pull newly collected rows (and any extra columns) into the table.
    fn sync_table(&mut self, cx: &mut Context<Self>) {
        let (Some(collection), Some(filing)) = (&self.collection, &self.filing) else {
            return;
        };
        let (rows, max_fields, first_type) = {
            let s = collection.state.lock().expect("collect lock poisoned");
            (s.rows, s.max_fields, s.row_types.first().cloned())
        };
        let version = filing.summary.fec_version.clone();
        self.table.update(cx, |table, cx| {
            let d = table.delegate_mut();
            let mut columns_changed = false;
            if d.columns.is_empty()
                && let Some(t) = &first_type
            {
                d.columns = column_names_for_field(t, &version)
                    .cloned()
                    .unwrap_or_default();
                columns_changed = true;
            }
            while d.columns.len() < max_fields {
                let n = d.columns.len();
                d.columns.push(format!("extra_{n}"));
                columns_changed = true;
            }
            if d.row_count != rows || columns_changed {
                d.row_count = rows;
                if columns_changed {
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
        self.selection = selection.clone();
        self.selected_row = None;
        self.collection = None;
        let Some(filing) = &self.filing else { return };
        if let Selection::Records(matcher) = selection {
            let collection = filing.collect(matcher);
            let state = collection.state.clone();
            self.collection = Some(collection);
            self.table.update(cx, |table, cx| {
                let d = table.delegate_mut();
                d.state = Some(state);
                d.columns.clear();
                d.row_count = 0;
                d.cache.clear();
                table.clear_selection(cx);
                table.refresh(cx);
                table.scroll_to_row(0, cx);
            });
            self.start_polling(cx);
        }
        cx.notify();
    }

    fn toggle_schedule(&mut self, schedule: Schedule, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&schedule) {
            self.collapsed.insert(schedule);
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
            self.load(Source::Path(first), window, cx);
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

        match (&self.filing, &self.error) {
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
        let Some(filing) = &self.filing else {
            return div();
        };
        let s = &filing.summary;
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
        let tree = self
            .filing
            .as_ref()
            .map(|f| Tree::new(&f.counts.lock().expect("count lock poisoned").types))
            .unwrap_or_else(|| Tree::new(&[]));

        let item = |id: ElementId,
                    leading: Option<AnyElement>,
                    label: String,
                    count: Option<usize>,
                    depth: usize,
                    selected: bool| {
            h_flex()
                .id(id)
                .pl(px(8. + depth as f32 * 18.))
                .pr_3()
                .py_1()
                .mx_1()
                .gap_1()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme.sidebar_accent)
                        .text_color(theme.sidebar_accent_foreground)
                })
                .when(!selected, |d| d.hover(|d| d.bg(theme.list_hover)))
                .children(leading)
                .child(div().flex_1().min_w_0().truncate().child(label))
                .children(count.map(|c| {
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format_count(c))
                }))
        };
        let is_selected = |m: &Matcher| matches!(&self.selection, Selection::Records(s) if s == m);

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
                    None,
                    "Cover".into(),
                    None,
                    0,
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
                    .child("RECORDS"),
            );

        for (schedule, lines) in tree.schedules {
            let total = lines.iter().map(|(_, n)| n).sum();
            let collapsed = self.collapsed.contains(&schedule);
            let matcher = Matcher::Schedule(schedule);
            let chevron = div()
                .id(ElementId::Name(
                    format!("toggle-{}", schedule.label()).into(),
                ))
                .flex_shrink_0()
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .xsmall(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_schedule(schedule, cx);
                }));
            list = list.child(
                item(
                    ElementId::Name(format!("schedule-{}", schedule.label()).into()),
                    Some(chevron.into_any_element()),
                    schedule.label().into(),
                    Some(total),
                    0,
                    is_selected(&matcher),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select(Selection::Records(matcher.clone()), cx)
                })),
            );
            if collapsed {
                continue;
            }
            for (row_type, count) in lines {
                let matcher = Matcher::RowType(row_type.clone());
                list = list.child(
                    item(
                        ElementId::Name(format!("line-{row_type}").into()),
                        None,
                        schedule.line_label(&row_type),
                        Some(count),
                        1,
                        is_selected(&matcher),
                    )
                    .font_family("monospace")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(Selection::Records(matcher.clone()), cx)
                    })),
                );
            }
        }
        for (row_type, count) in tree.other {
            let matcher = Matcher::RowType(row_type.clone());
            list = list.child(
                item(
                    ElementId::Name(format!("other-{row_type}").into()),
                    None,
                    row_type,
                    Some(count),
                    0,
                    is_selected(&matcher),
                )
                .font_family("monospace")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select(Selection::Records(matcher.clone()), cx)
                })),
            );
        }
        list
    }

    fn render_main(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.selection {
            Selection::Cover => {
                let kv = self
                    .filing
                    .as_ref()
                    .map(|f| f.summary.cover_kv.clone())
                    .unwrap_or_default();
                render_kv("cover-kv", kv, cx).into_any_element()
            }
            Selection::Records(_) => h_flex()
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
        let mut parts = Vec::new();
        if let Some(filing) = &self.filing {
            let size = format_bytes(filing.file_len);
            let c = filing.counts.lock().expect("count lock poisoned");
            parts.push(if let Some(err) = &c.error {
                format!("{} records · stopped early: {err}", format_count(c.rows))
            } else if c.done {
                format!(
                    "{} · {size} · counted in {:.2}s",
                    plural(c.rows, "record"),
                    c.elapsed.as_secs_f64()
                )
            } else {
                let pct = c.bytes_scanned as f64 / filing.file_len.max(1) as f64 * 100.0;
                format!(
                    "Counting… {pct:.0}% · {} records · {size}",
                    format_count(c.rows)
                )
            });
            if let Some(collection) = &self.collection {
                let s = collection.state.lock().expect("collect lock poisoned");
                parts.push(if let Some(err) = &s.error {
                    format!("load stopped early: {err}")
                } else if s.done {
                    format!(
                        "{} loaded in {:.2}s",
                        plural(s.rows, "row"),
                        s.elapsed.as_secs_f64()
                    )
                } else {
                    let pct = s.bytes_scanned as f64 / filing.file_len.max(1) as f64 * 100.0;
                    format!("Loading… {pct:.0}% · {} rows", format_count(s.rows))
                });
            }
        }
        h_flex()
            .px_3()
            .py_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .child(parts.join("   ·   "))
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
        .child(empty_hint(cx))
        .children(error.map(|e| {
            div()
                .mt_4()
                .max_w(px(560.))
                .text_color(theme.danger)
                .child(e)
        }))
}

#[cfg(not(target_family = "wasm"))]
fn empty_hint(cx: &App) -> impl IntoElement {
    div()
        .text_color(cx.theme().muted_foreground)
        .child("or choose File → Open… (⌘O / Ctrl+O)")
}

/// The browser has no File menu: offer a file picker and the bundled sample.
#[cfg(target_family = "wasm")]
fn empty_hint(_: &App) -> impl IntoElement {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    h_flex()
        .mt_2()
        .gap_2()
        .child(
            Button::new("choose-file")
                .label("Choose a file…")
                .on_click(|_, _, _| crate::web::choose_file()),
        )
        .child(
            Button::new("load-sample")
                .primary()
                .label("Load sample filing")
                .on_click(|_, _, _| crate::web::fetch_file(crate::web::SAMPLE_FILE.into())),
        )
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
