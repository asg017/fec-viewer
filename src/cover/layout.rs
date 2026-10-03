//! Shared building blocks for the per-form cover views.
//!
//! Every form's renderer lays its cover out from these pieces, so the forms
//! read alike:
//!
//! - [`banner`]: the form's name, what it is, and status tags (amendment,
//!   termination, 24-hour, …) at the top.
//! - [`section`]: a titled card holding fields, tables or prose.
//! - [`Fields`]: `label  value` rows with one label width for every form, with
//!   helpers for addresses, people and optional values.
//! - [`cash_flow`]: start / receipts / disbursements / end tiles with the
//!   change over the period.
//! - [`stats`]: a row of headline-number tiles.
//! - [`MoneyTable`]: the summary pages, with one or two right-aligned amount
//!   columns, captions, indented sub-lines and bold totals.
//! - [`note`], [`tag`], [`prose`]: smaller pieces.

use fec_parser::covers::{Address, DetailedSummaryRow, PersonName};
use gpui_kit::component::{ActiveTheme as _, Theme, h_flex, v_flex};
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Width of the label column in [`Fields`].
const LABEL_WIDTH: f32 = 168.;
/// Width of one amount column in a [`MoneyTable`].
const AMOUNT_WIDTH: f32 = 136.;
/// Widest a cover page grows; wider windows center it.
const PAGE_MAX_WIDTH: f32 = 920.;

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// `$1,234.56`, or `-$1,234.56` for negative amounts (including those
/// between -1 and 0, which would otherwise lose their sign).
pub fn format_usd(amount: f64) -> String {
    let rounded = (amount * 100.0).round() as i64;
    let sign = if rounded < 0 { "-" } else { "" };
    let abs = rounded.unsigned_abs();
    format!("{sign}${}.{:02}", group_thousands(abs / 100), abs % 100)
}

fn group_thousands(n: u64) -> String {
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

/// A ZIP code, with ZIP+4 hyphenated: `"940253656"` → `"94025-3656"`.
pub fn format_zip(zip: &str) -> String {
    let zip = zip.trim();
    if zip.len() == 9 && zip.bytes().all(|b| b.is_ascii_digit()) {
        format!("{}-{}", &zip[..5], &zip[5..])
    } else {
        zip.to_owned()
    }
}

/// `"Label (CODE)"`, or just the code when no label is sourced.
pub fn code_with_label(code: &str, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("{label} ({code})"),
        None => code.to_string(),
    }
}

/// A report code as `"October Quarterly (Q3)"`: the form's own label when it
/// has one, else the generic report-code label, else just the code.
pub fn report_code_text(code: &str, label: Option<&str>) -> String {
    let generic = match fec_parser::report_code_label(code) {
        "[Unknown report code]" => None,
        label => Some(label),
    };
    code_with_label(code, label.or(generic))
}

/// `"General (G2024) on 2024-11-05 in CA"` for an election report, from
/// whichever parts are present; `None` when none are.
pub fn election_text(
    code: Option<&str>,
    label: Option<&str>,
    date: Option<jiff::civil::Date>,
    state: Option<&str>,
) -> Option<String> {
    let mut parts = vec![];
    if let Some(code) = code {
        parts.push(code_with_label(code, label));
    }
    if let Some(date) = date {
        parts.push(format!("on {date}"));
    }
    if let Some(state) = state {
        parts.push(format!("in {state}"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// `"2025-10-01 – 2025-10-31"` from whichever ends are present.
pub fn period_text(
    from: Option<jiff::civil::Date>,
    through: Option<jiff::civil::Date>,
) -> Option<String> {
    match (from, through) {
        (Some(from), Some(through)) => Some(format!("{from} – {through}")),
        (Some(from), None) => Some(format!("from {from}")),
        (None, Some(through)) => Some(format!("through {through}")),
        (None, None) => None,
    }
}

/// `"CA-32"` for a House seat, `"CA"` for a Senate seat. A blank or all-zero
/// district (`00`, common on Senate and presidential filings) is left off.
pub fn seat_text(state: Option<&str>, district: Option<&str>) -> Option<String> {
    let state = state.map(str::trim).filter(|s| !s.is_empty());
    let district = district
        .map(str::trim)
        .filter(|d| !d.is_empty() && !d.bytes().all(|b| b == b'0'));
    match (state, district) {
        (Some(state), Some(district)) => Some(format!("{state}-{district}")),
        (Some(state), None) => Some(state.to_string()),
        (None, Some(district)) => Some(format!("District {district}")),
        (None, None) => None,
    }
}

/// An electronic-format `entity_type` code as `"Organization (ORG)"`.
pub fn entity_label(code: &str) -> String {
    let label = match code.trim().to_ascii_uppercase().as_str() {
        "IND" => Some("Individual"),
        "ORG" => Some("Organization"),
        "COM" => Some("Committee"),
        "PAC" => Some("PAC"),
        "PTY" => Some("Party organization"),
        _ => None,
    };
    code_with_label(code.trim(), label)
}

/// A `Y` / `N` column as `"Yes"` / `"No"`, or the raw value otherwise.
pub fn yes_no_code(code: &str) -> String {
    match code.trim() {
        "Y" | "y" => "Yes".into(),
        "N" | "n" => "No".into(),
        other => other.into(),
    }
}

// ---------------------------------------------------------------------------
// Page chrome
// ---------------------------------------------------------------------------

/// The scrolling page every cover is laid out in.
pub fn page(id: impl Into<ElementId>, children: Vec<AnyElement>) -> impl IntoElement {
    div().id(id.into()).size_full().overflow_y_scroll().child(
        v_flex()
            .w_full()
            .max_w(px(PAGE_MAX_WIDTH))
            .mx_auto()
            .p_5()
            .gap_4()
            .children(children),
    )
}

/// How a [`tag`] is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
    Accent,
}

impl Tone {
    fn color(self, theme: &Theme) -> Hsla {
        match self {
            Tone::Neutral => theme.muted_foreground,
            Tone::Info => theme.info,
            Tone::Success => theme.success,
            Tone::Warning => theme.warning,
            Tone::Danger => theme.danger,
            Tone::Accent => theme.primary,
        }
    }
}

/// A small rounded label, e.g. `AMENDMENT` or `24-HOUR`.
pub fn tag(text: impl Into<SharedString>, tone: Tone, cx: &App) -> AnyElement {
    let color = tone.color(cx.theme());
    div()
        .flex_shrink_0()
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(color.opacity(0.5))
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
        .into_any_element()
}

/// The top of a cover: the form (`"Form 3X"`), what it is, any status tags,
/// and an optional line under it (e.g. the amendment status).
pub fn banner(
    form: &str,
    title: impl Into<SharedString>,
    tags: Vec<AnyElement>,
    subtitle: Option<String>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child(form.to_uppercase()),
                )
                .children(tags),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .children(subtitle.map(|s| div().text_color(theme.muted_foreground).child(s)))
        .into_any_element()
}

/// The amendment line for a [`banner`]: `"Amends the report filed
/// 2024-01-31"`, or `None` when the cover is not an amendment.
pub fn amendment_text(
    is_amendment: bool,
    original: Option<jiff::civil::Date>,
    noun: &str,
) -> Option<String> {
    match (is_amendment, original) {
        (true, Some(date)) => Some(format!("Amends the {noun} filed {date}")),
        (true, None) => Some(format!("Amends an earlier {noun}")),
        (false, _) => None,
    }
}

/// The `AMENDMENT` tag, when the cover is one.
pub fn amendment_tag(is_amendment: bool, cx: &App) -> Option<AnyElement> {
    is_amendment.then(|| tag("AMENDMENT", Tone::Warning, cx))
}

/// A titled card.
pub fn section(title: impl Into<SharedString>, children: Vec<AnyElement>, cx: &App) -> AnyElement {
    section_with(title, None, children, cx)
}

/// A titled card with a muted aside after the title (e.g. `"Lines 6–10"`).
pub fn section_with(
    title: impl Into<SharedString>,
    aside: Option<&str>,
    children: Vec<AnyElement>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .w_full()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .overflow_hidden()
        .child(
            h_flex()
                .px_4()
                .py_2()
                .gap_2()
                .items_baseline()
                .bg(theme.muted.opacity(0.5))
                .border_b_1()
                .border_color(theme.border)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title.into()))
                .children(aside.map(|a| {
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(a.to_string())
                })),
        )
        .child(v_flex().px_4().py_3().gap_2().children(children))
        .into_any_element()
}

/// A muted explanatory line.
pub fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// A block of free text (e.g. a Form 99 letter), wrapped, with its line
/// breaks kept.
pub fn prose(text: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    // No gap between lines, so a letter's own line breaks are spaced the
    // same as a long line's wrapping.
    v_flex()
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .children(text.lines().map(|line| {
            // An empty div collapses; keep blank lines one line tall.
            if line.trim().is_empty() {
                div().child(" ").into_any_element()
            } else {
                div().child(line.to_string()).into_any_element()
            }
        }))
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------

/// `label  value` rows. Build with the methods, then [`Fields::render`].
/// Rows whose value is missing are skipped, so a form can list every field
/// it has and only the filled ones show.
#[derive(Default)]
pub struct Fields {
    rows: Vec<(SharedString, AnyElement)>,
}

impl Fields {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// A text value; blank values are skipped.
    pub fn text(mut self, label: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.trim().is_empty() {
            self.rows.push((
                label.to_string().into(),
                div().child(value).into_any_element(),
            ));
        }
        self
    }

    /// An optional text value.
    pub fn opt(self, label: &str, value: Option<impl Into<String>>) -> Self {
        match value {
            Some(v) => self.text(label, v),
            None => self,
        }
    }

    /// A value built as an element (e.g. a name with tags after it).
    pub fn element(mut self, label: &str, value: impl IntoElement) -> Self {
        self.rows
            .push((label.to_string().into(), value.into_any_element()));
        self
    }

    /// A money amount.
    pub fn amount(self, label: &str, amount: f64, cx: &App) -> Self {
        self.element(label, money(amount, cx))
    }

    /// An optional money amount.
    pub fn amount_opt(self, label: &str, amount: Option<f64>, cx: &App) -> Self {
        match amount {
            Some(a) => self.amount(label, a, cx),
            None => self,
        }
    }

    /// An optional date.
    pub fn date(self, label: &str, date: Option<jiff::civil::Date>) -> Self {
        self.opt(label, date.map(|d| d.to_string()))
    }

    /// A person's name; empty names are skipped.
    pub fn person(self, label: &str, name: &PersonName) -> Self {
        self.text(label, name.to_string())
    }

    /// A person's name with a muted aside after it (e.g. their title).
    pub fn person_with(
        self,
        label: &str,
        name: &PersonName,
        aside: Option<&str>,
        cx: &App,
    ) -> Self {
        if name.is_empty() {
            return self;
        }
        self.element(label, with_aside(name.to_string(), aside, cx))
    }

    /// A mailing address on two lines, with a `CHANGED` tag when the form's
    /// "check if address changed" box is ticked. Empty addresses are skipped.
    pub fn address(self, label: &str, address: &Address, changed: bool, cx: &App) -> Self {
        if address.is_empty() {
            return self;
        }
        let street = [address.street_1.as_deref(), address.street_2.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        let city_state = [address.city.as_deref(), address.state.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        let city_state_zip = match address.zip_code.as_deref().map(format_zip) {
            Some(zip) if !city_state.is_empty() => format!("{city_state} {zip}"),
            Some(zip) => zip,
            None => city_state,
        };
        let value = v_flex()
            .when(!street.is_empty(), |d| d.child(street))
            .child(
                h_flex()
                    .gap_2()
                    .when(!city_state_zip.is_empty(), |d| d.child(city_state_zip))
                    .when(changed, |d| d.child(changed_tag(cx))),
            );
        self.element(label, value)
    }

    pub fn render(self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .gap_1p5()
            .children(self.rows.into_iter().map(|(label, value)| {
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        div()
                            .w(px(LABEL_WIDTH))
                            .flex_shrink_0()
                            .text_color(theme.muted_foreground)
                            .child(label),
                    )
                    .child(div().flex_1().min_w_0().child(value))
            }))
            .into_any_element()
    }
}

/// The `CHANGED` flag for a form's "check if changed" boxes.
pub fn changed_tag(cx: &App) -> AnyElement {
    tag("CHANGED", Tone::Accent, cx)
}

/// Bold text, for names and other values that should stand out.
pub fn strong(text: impl Into<SharedString>) -> AnyElement {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
        .into_any_element()
}

/// `text` followed by a muted aside, e.g. a committee name and its ID.
pub fn with_aside(text: impl Into<SharedString>, aside: Option<&str>, cx: &App) -> AnyElement {
    h_flex()
        .gap_2()
        .flex_wrap()
        .items_baseline()
        .child(div().child(text.into()))
        .children(aside.map(|a| {
            div()
                .text_color(cx.theme().muted_foreground)
                .child(a.to_string())
        }))
        .into_any_element()
}

/// A row of elements (e.g. a name followed by tags).
pub fn inline(children: Vec<AnyElement>) -> AnyElement {
    h_flex()
        .gap_2()
        .flex_wrap()
        .items_center()
        .children(children)
        .into_any_element()
}

/// A money amount in tabular figures; negatives in the danger color.
pub fn money(amount: f64, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .font_family(theme.mono_font_family.clone())
        .when(amount < -0.005, |d| d.text_color(theme.danger))
        .child(format_usd(amount))
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Headline numbers
// ---------------------------------------------------------------------------

/// One headline-number tile.
pub struct Stat {
    pub label: SharedString,
    pub amount: f64,
    /// A line under the amount (e.g. the change over the period).
    pub detail: Option<(String, Tone)>,
    pub emphasis: bool,
}

impl Stat {
    pub fn new(label: impl Into<SharedString>, amount: f64) -> Self {
        Self {
            label: label.into(),
            amount,
            detail: None,
            emphasis: false,
        }
    }

    pub fn detail(mut self, text: impl Into<String>, tone: Tone) -> Self {
        self.detail = Some((text.into(), tone));
        self
    }

    pub fn emphasis(mut self) -> Self {
        self.emphasis = true;
        self
    }
}

/// A row of headline-number tiles that wraps on narrow windows.
pub fn stats(tiles: Vec<Stat>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .w_full()
        .gap_3()
        .flex_wrap()
        .items_stretch()
        .children(tiles.into_iter().map(|t| {
            v_flex()
                .flex_1()
                .min_w(px(160.))
                .px_4()
                .py_3()
                .gap_0p5()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(if t.emphasis {
                    theme.primary.opacity(0.6)
                } else {
                    theme.border
                })
                .when(t.emphasis, |d| d.bg(theme.primary.opacity(0.06)))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t.label),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_family(theme.mono_font_family.clone())
                        .when(t.amount < -0.005, |d| d.text_color(theme.danger))
                        .child(format_usd(t.amount)),
                )
                .children(
                    t.detail.map(|(text, tone)| {
                        div().text_xs().text_color(tone.color(theme)).child(text)
                    }),
                )
        }))
        .into_any_element()
}

/// Start / receipts / disbursements / end tiles, with the change in cash
/// over the period under the ending balance.
pub fn cash_flow(begin: f64, receipts: f64, disbursements: f64, end: f64, cx: &App) -> AnyElement {
    let change_cents = ((end - begin) * 100.0).round();
    let (change, tone) = if change_cents == 0.0 {
        ("No change".to_string(), Tone::Neutral)
    } else {
        let sign = if change_cents > 0.0 { "+" } else { "" };
        let mut text = format!("{sign}{}", format_usd(end - begin));
        if begin > 0.0 {
            let pct = (end - begin) / begin * 100.0;
            // A small change rounds to "0%"; say "<1%" rather than "-0%".
            if pct.abs() < 0.5 {
                text.push_str(" (<1%)");
            } else {
                text.push_str(&format!(" ({sign}{pct:.0}%)"));
            }
        }
        let tone = if change_cents > 0.0 {
            Tone::Success
        } else {
            Tone::Danger
        };
        (text, tone)
    };
    stats(
        vec![
            Stat::new("Cash on hand – start", begin),
            Stat::new("Receipts", receipts),
            Stat::new("Disbursements", disbursements),
            Stat::new("Cash on hand – end", end)
                .detail(change, tone)
                .emphasis(),
        ],
        cx,
    )
}

// ---------------------------------------------------------------------------
// Money tables
// ---------------------------------------------------------------------------

/// Column headings of a [`MoneyTable`].
#[derive(Debug, Clone, Copy)]
pub enum Columns {
    /// One unlabeled amount column.
    One,
    /// Column A and Column B, with their headings.
    Two(&'static str, &'static str),
}

enum Row {
    Caption(String),
    Amounts {
        label: String,
        a: Option<f64>,
        b: Option<f64>,
        total: bool,
    },
}

/// A summary-page table: labels on the left, one or two right-aligned amount
/// columns. Labels may start with spaces to indent sub-lines (`"  (a) …"`).
pub struct MoneyTable {
    columns: Columns,
    rows: Vec<Row>,
}

impl MoneyTable {
    pub fn new(columns: Columns) -> Self {
        Self {
            columns,
            rows: Vec::new(),
        }
    }

    /// A label-only line introducing the sub-lines under it.
    pub fn caption(mut self, label: &str) -> Self {
        self.rows.push(Row::Caption(label.to_string()));
        self
    }

    /// A two-column line.
    pub fn row(self, label: &str, row: &DetailedSummaryRow) -> Self {
        self.row_ab(label, Some(row.column_a), Some(row.column_b), false)
    }

    /// A two-column total line, in bold above a rule.
    pub fn total(self, label: &str, row: &DetailedSummaryRow) -> Self {
        self.row_ab(label, Some(row.column_a), Some(row.column_b), true)
    }

    /// A one-column line (Column A only on a two-column table).
    pub fn amount(self, label: &str, amount: f64) -> Self {
        self.row_ab(label, Some(amount), None, false)
    }

    /// A one-column total line.
    pub fn amount_total(self, label: &str, amount: f64) -> Self {
        self.row_ab(label, Some(amount), None, true)
    }

    /// A line with either column possibly blank (e.g. Form 3X line 6(a) has
    /// only a Column B amount).
    pub fn row_ab(mut self, label: &str, a: Option<f64>, b: Option<f64>, total: bool) -> Self {
        self.rows.push(Row::Amounts {
            label: label.to_string(),
            a,
            b,
            total,
        });
        self
    }

    pub fn render(self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let two = matches!(self.columns, Columns::Two(..));
        let amount_cell = |amount: Option<f64>, total: bool| {
            div()
                .w(px(AMOUNT_WIDTH))
                .flex_shrink_0()
                .text_right()
                .font_family(theme.mono_font_family.clone())
                .when(total, |d| d.font_weight(FontWeight::SEMIBOLD))
                .when(amount.is_some_and(|a| a < -0.005), |d| {
                    d.text_color(theme.danger)
                })
                // Zero amounts recede, so the lines with money stand out.
                .when(amount.is_some_and(|a| a.abs() < 0.005) && !total, |d| {
                    d.text_color(theme.muted_foreground)
                })
                .children(amount.map(format_usd))
        };
        let label_cell = |label: &str, total: bool| {
            let indent = label.len() - label.trim_start().len();
            div()
                .flex_1()
                .min_w_0()
                .pl(px(indent as f32 * 8.))
                .when(total, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(label.trim_start().to_string())
        };

        let header = match self.columns {
            Columns::One => None,
            Columns::Two(a, b) => Some(
                h_flex()
                    .pb_1()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1())
                    .child(div().w(px(AMOUNT_WIDTH)).text_right().child(a))
                    .child(div().w(px(AMOUNT_WIDTH)).text_right().child(b)),
            ),
        };

        let rows = self.rows.into_iter().map(|row| match row {
            Row::Caption(label) => h_flex()
                .pt_1()
                .child(label_cell(&label, false).text_color(theme.muted_foreground))
                .into_any_element(),
            Row::Amounts { label, a, b, total } => h_flex()
                .gap_2()
                .py_0p5()
                .items_start()
                .when(total, |d| d.border_t_1().border_color(theme.border).pt_1())
                .child(label_cell(&label, total))
                .child(amount_cell(a, total))
                .when(two, |d| d.child(amount_cell(b, total)))
                .into_any_element(),
        });

        v_flex()
            .w_full()
            .children(header)
            .children(rows)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Tiles: small cards inside a section
// ---------------------------------------------------------------------------

/// Default narrowest width of a [`tile`] before tiles wrap.
const TILE_MIN_WIDTH: f32 = 240.;

/// `"House · VA-06"`, `"Senate · OK"`, `"President"`. Only House races have
/// districts; Senate and presidential filings often carry `00`.
pub fn office_text(
    office: Option<&str>,
    label: Option<&str>,
    state: Option<&str>,
    district: Option<&str>,
) -> Option<String> {
    let house = office.is_some_and(|o| o.eq_ignore_ascii_case("H"));
    let mut parts = vec![];
    if let Some(office) = office {
        parts.push(label.unwrap_or(office).to_string());
    }
    match (state, district) {
        (Some(state), Some(district)) if house => parts.push(format!("{state}-{district}")),
        (Some(state), _) => parts.push(state.to_string()),
        _ => {}
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `(405) 826-6448` for ten-digit numbers, otherwise as filed.
pub fn format_phone(phone: &str) -> String {
    let phone = phone.trim();
    if phone.len() == 10 && phone.bytes().all(|b| b.is_ascii_digit()) {
        format!("({}) {}-{}", &phone[0..3], &phone[3..6], &phone[6..])
    } else {
        phone.to_string()
    }
}

/// An address as street and city/state/ZIP lines, for a [`tile`].
pub fn address_lines(address: &Address) -> Vec<AnyElement> {
    let street = [address.street_1.as_deref(), address.street_2.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    let city_state = [address.city.as_deref(), address.state.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    let last = match address.zip_code.as_deref().map(format_zip) {
        Some(zip) if !city_state.is_empty() => format!("{city_state} {zip}"),
        Some(zip) => zip,
        None => city_state,
    };
    [street, last]
        .into_iter()
        .filter(|line| !line.is_empty())
        .map(|line| div().child(line).into_any_element())
        .collect()
}

/// Muted text.
pub fn muted(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// An element in bold.
pub fn strong_element(child: AnyElement) -> AnyElement {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .child(child)
        .into_any_element()
}

/// A small bordered card inside a section: a muted caption with optional tags,
/// then its lines. `emphasis` gives it the accent border (e.g. the treasurer).
pub fn tile(
    caption: &str,
    tags: Vec<AnyElement>,
    body: Vec<AnyElement>,
    emphasis: bool,
    cx: &App,
) -> AnyElement {
    tile_sized(TILE_MIN_WIDTH, caption, tags, body, emphasis, cx)
}

/// A [`tile`] that wraps below `min_width` instead of the default.
pub fn tile_sized(
    min_width: f32,
    caption: &str,
    tags: Vec<AnyElement>,
    body: Vec<AnyElement>,
    emphasis: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .flex_1()
        .min_w(px(min_width))
        .px_3()
        .py_2()
        .gap_0p5()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(if emphasis {
            theme.primary.opacity(0.6)
        } else {
            theme.border
        })
        .when(emphasis, |d| d.bg(theme.primary.opacity(0.04)))
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .items_center()
                .pb_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child(caption.to_uppercase()),
                )
                .children(tags),
        )
        .children(body)
        .into_any_element()
}

/// A row of [`tile`]s that wraps on narrow windows.
pub fn tiles(children: Vec<AnyElement>) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .flex_wrap()
        .items_stretch()
        .children(children)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` attribute.
    use super::{election_text, format_phone, format_usd, format_zip, office_text, seat_text};

    #[test]
    fn formats_seats_and_offices() {
        assert_eq!(seat_text(Some("CA"), Some("32")).as_deref(), Some("CA-32"));
        assert_eq!(seat_text(Some("TX"), Some("00")).as_deref(), Some("TX"));
        assert_eq!(seat_text(Some("TX"), None).as_deref(), Some("TX"));
        assert_eq!(seat_text(None, None), None);
        assert_eq!(
            office_text(Some("H"), Some("House"), Some("VA"), Some("06")).as_deref(),
            Some("House · VA-06")
        );
        assert_eq!(
            office_text(Some("S"), Some("Senate"), Some("OK"), Some("00")).as_deref(),
            Some("Senate · OK")
        );
        assert_eq!(office_text(None, None, None, None), None);
    }

    #[test]
    fn formats_phones() {
        assert_eq!(format_phone("4058266448"), "(405) 826-6448");
        assert_eq!(format_phone("x123"), "x123");
    }

    #[test]
    fn formats_zips() {
        assert_eq!(format_zip("940253656"), "94025-3656");
        assert_eq!(format_zip("94025"), "94025");
        assert_eq!(format_zip("K1A 0B1"), "K1A 0B1");
    }

    #[test]
    fn formats_money() {
        assert_eq!(format_usd(0.0), "$0.00");
        assert_eq!(format_usd(1234567.891), "$1,234,567.89");
        assert_eq!(format_usd(-0.5), "-$0.50");
        assert_eq!(format_usd(999.995), "$1,000.00");
    }

    #[test]
    fn formats_elections() {
        let date = jiff::civil::date(2024, 11, 5);
        assert_eq!(
            election_text(Some("G2024"), Some("General"), Some(date), Some("CA")).as_deref(),
            Some("General (G2024) on 2024-11-05 in CA")
        );
        assert_eq!(election_text(None, None, None, None), None);
    }
}
