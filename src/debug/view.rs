//! The debug view: a filing's bytes, line by line or as a hex dump, with
//! each line's place in the filing (header, cover, record, text block), its
//! fields named from the mapping, and anything odd about it.
//!
//! Scrolling is by row index, not pixels: a 10 GB filing has hundreds of
//! millions of hex rows, more than an `f32` pixel offset can tell apart.

use std::{
    cell::Cell,
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use fec_parser::{
    Delimiter, HeaderStyle,
    mappings::{DATE_COLUMNS, FLOAT_COLUMNS},
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Theme,
    button::{Button, ButtonGroup, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use super::{
    data::Data,
    index::{
        Eol, Field, Flags, IndexState, Indexer, Kind, Line, Quote, Severity, field_value, find,
        find_all, split,
    },
    rows::{
        Class, HEX_WIDTH, Jump, ascii_text, byte_class, draw_line, fmt_count, hex_range, hex_text,
        json_string, line_digits, offset_digits, parse_jump, read_date,
    },
};
use crate::filing::{Schedule, Source, spawn_steps};

const ROW_H: f32 = 20.;
const INSPECTOR_W: f32 = 400.;
const SCROLLBAR_W: f32 = 12.;
/// Lines indexed per background step.
const STEP_LINES: usize = 4096;
/// Most scrollbar ticks drawn for warnings.
const MAX_TICKS: usize = 400;
/// Field splits kept for drawing; a screenful is ~60 lines.
const SPLIT_CACHE_CAP: usize = 1024;

actions!(
    debug_view,
    [
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        PageUp,
        PageDown,
        MoveToStart,
        MoveToEnd,
        ToggleMode,
        FocusJump,
        FindText,
        NextWarning,
        PreviousWarning,
        NextSameType,
        PreviousSameType,
        CopySelection,
        ClearField,
    ]
);

const CONTEXT: &str = "DebugView";

pub fn init(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, c),
        KeyBinding::new("down", MoveDown, c),
        KeyBinding::new("k", MoveUp, c),
        KeyBinding::new("j", MoveDown, c),
        KeyBinding::new("left", MoveLeft, c),
        KeyBinding::new("right", MoveRight, c),
        KeyBinding::new("h", MoveLeft, c),
        KeyBinding::new("l", MoveRight, c),
        KeyBinding::new("pageup", PageUp, c),
        KeyBinding::new("pagedown", PageDown, c),
        KeyBinding::new("space", PageDown, c),
        KeyBinding::new("home", MoveToStart, c),
        KeyBinding::new("end", MoveToEnd, c),
        KeyBinding::new("secondary-up", MoveToStart, c),
        KeyBinding::new("secondary-down", MoveToEnd, c),
        KeyBinding::new("x", ToggleMode, c),
        KeyBinding::new("secondary-g", FocusJump, c),
        KeyBinding::new("secondary-l", FocusJump, c),
        KeyBinding::new(":", FocusJump, c),
        KeyBinding::new("secondary-f", FindText, c),
        KeyBinding::new("/", FindText, c),
        KeyBinding::new("f8", NextWarning, c),
        KeyBinding::new("shift-f8", PreviousWarning, c),
        KeyBinding::new("]", NextSameType, c),
        KeyBinding::new("[", PreviousSameType, c),
        KeyBinding::new("secondary-c", CopySelection, c),
        KeyBinding::new("escape", ClearField, c),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Lines,
    Hex,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Line,
    File,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// Leave the scroll position alone.
    None,
    /// Scroll just enough to show the row.
    Nearest,
    /// Put the row mid-screen.
    Center,
}

/// Somewhere to go once indexing gets that far.
#[derive(Clone, Copy, Debug)]
pub enum Target {
    /// The first record starting at or after this byte (positions from the
    /// records table, which may point at the blank line or `\n` before it).
    RecordAt(u64),
    Cover,
}

pub struct DebugView {
    data: Data,
    index: Arc<Mutex<IndexState>>,
    cancel: Arc<AtomicBool>,
    error: Option<String>,
    mode: Mode,
    tab: Tab,
    /// First visible row (a line, or a hex row of [`HEX_WIDTH`] bytes).
    top: usize,
    /// Wheel movement not yet worth a whole row.
    scroll_rem: f32,
    scroll_x: f32,
    max_scroll_x: f32,
    body: Rc<Cell<Bounds<Pixels>>>,
    dragging_scrollbar: bool,
    line: usize,
    field: Option<usize>,
    byte: u64,
    find: Option<Vec<u8>>,
    pending: Option<Target>,
    notice: Option<(String, bool)>,
    jump: Entity<InputState>,
    focus: FocusHandle,
    splits: HashMap<usize, Arc<Vec<Field>>>,
    _poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Drop for DebugView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Focusable for DebugView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DebugView {
    pub fn new(source: &Source, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let index = Arc::new(Mutex::new(IndexState::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (data, mut error) = match Data::open(source) {
            Ok(data) => (data, None),
            Err(e) => (
                Data::from(Vec::new()),
                Some(format!("Couldn't read the file: {e}")),
            ),
        };
        if error.is_none() {
            let mut indexer = Indexer::new(data.clone(), index.clone());
            if let Err(e) = spawn_steps("fec-index", cancel.clone(), move || {
                indexer.step(STEP_LINES)
            }) {
                error = Some(format!("Couldn't start indexing: {e}"));
            }
        }

        let jump = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Line, 0xOffset, #record, SA11AI or /text")
                .clean_on_escape()
        });
        let subscription =
            cx.subscribe_in(&jump, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift, .. } = event {
                    this.run_jump(*shift, window, cx);
                }
            });

        let mut this = Self {
            data,
            index,
            cancel,
            error,
            mode: Mode::Lines,
            tab: Tab::Line,
            top: 0,
            scroll_rem: 0.,
            scroll_x: 0.,
            max_scroll_x: 0.,
            body: Rc::new(Cell::new(Bounds::default())),
            dragging_scrollbar: false,
            line: 0,
            field: None,
            byte: 0,
            find: None,
            pending: None,
            notice: None,
            jump,
            focus: cx.focus_handle(),
            splits: HashMap::new(),
            _poll: None,
            _subscriptions: vec![subscription],
        };
        this.start_polling(cx);
        this
    }

    /// Go to `target`, now or once indexing reaches it.
    pub fn reveal(&mut self, target: Target, mode: Option<Mode>, cx: &mut Context<Self>) {
        if let Some(mode) = mode {
            self.mode = mode;
        }
        self.pending = Some(target);
        self.apply_pending(cx);
        cx.notify();
    }

    fn apply_pending(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.pending else { return };
        let found = {
            let s = self.index.lock().expect("index lock poisoned");
            match target {
                Target::Cover => s.find_line(0, |l| l.kind == Kind::Cover),
                Target::RecordAt(pos) => s
                    .line_at(pos)
                    .and_then(|ix| s.find_line(ix, |l| l.kind.is_record() && l.start >= pos)),
            }
            .map(Some)
            .or_else(|| s.done.then_some(None))
        };
        if let Some(found) = found {
            self.pending = None;
            if let Some(ix) = found {
                self.select_line(ix, None, Reveal::Center, cx);
            }
        }
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        this.apply_pending(cx);
                        cx.notify();
                        this.index.lock().is_ok_and(|s| s.done)
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }

    fn bytes(&self) -> &[u8] {
        self.data.as_slice()
    }

    fn content<'a>(&'a self, line: &Line) -> &'a [u8] {
        let r = line.content();
        &self.bytes()[r.start as usize..r.end as usize]
    }

    fn delimiter(state: &IndexState) -> Delimiter {
        state.facts.as_ref().map_or(Delimiter::Fs, |f| f.delimiter)
    }

    fn fields(&mut self, ix: usize, line: &Line, delimiter: Delimiter) -> Arc<Vec<Field>> {
        if let Some(f) = self.splits.get(&ix) {
            return f.clone();
        }
        if self.splits.len() >= SPLIT_CACHE_CAP {
            self.splits.clear();
        }
        let fields = Arc::new(split(self.content(line), line.kind, delimiter));
        self.splits.insert(ix, fields.clone());
        fields
    }

    // -----------------------------------------------------------------------
    // Cursor
    // -----------------------------------------------------------------------

    fn rows(&self, state: &IndexState) -> usize {
        match self.mode {
            Mode::Lines => state.lines,
            Mode::Hex => (self.data.len() as usize).div_ceil(HEX_WIDTH),
        }
    }

    fn visible_rows(&self) -> usize {
        ((self.body.get().size.height.as_f32() / ROW_H).floor() as usize).max(1)
    }

    fn cursor_row(&self) -> usize {
        match self.mode {
            Mode::Lines => self.line,
            Mode::Hex => self.byte as usize / HEX_WIDTH,
        }
    }

    fn reveal_row(&mut self, row: usize, how: Reveal) {
        let visible = self.visible_rows();
        match how {
            Reveal::None => {}
            Reveal::Nearest => {
                if row < self.top {
                    self.top = row;
                } else if row >= self.top + visible {
                    self.top = row + 1 - visible;
                }
            }
            Reveal::Center => {
                if row < self.top || row >= self.top + visible {
                    self.top = row.saturating_sub(visible / 2);
                }
            }
        }
    }

    fn select_line(
        &mut self,
        ix: usize,
        field: Option<usize>,
        how: Reveal,
        cx: &mut Context<Self>,
    ) {
        let line = {
            let s = self.index.lock().expect("index lock poisoned");
            s.line(ix).map(|l| (l, Self::delimiter(&s)))
        };
        let Some((line, delimiter)) = line else {
            return;
        };
        let fields = self.fields(ix, &line, delimiter);
        let field = field.filter(|&f| f < fields.len());
        self.line = ix;
        self.field = field;
        self.byte = line.start + field.map_or(0, |f| fields[f].span.start as u64);
        if how != Reveal::None {
            self.scroll_x = 0.;
        }
        self.reveal_row(self.cursor_row(), how);
        cx.notify();
    }

    fn select_byte(&mut self, offset: u64, how: Reveal, cx: &mut Context<Self>) {
        let offset = offset.min(self.data.len().saturating_sub(1));
        let found = {
            let s = self.index.lock().expect("index lock poisoned");
            s.line_at(offset)
                .and_then(|ix| Some((ix, s.line(ix)?, Self::delimiter(&s))))
        };
        self.byte = offset;
        self.field = None;
        if let Some((ix, line, delimiter)) = found {
            self.line = ix;
            let rel = (offset - line.start) as usize;
            self.field = self
                .fields(ix, &line, delimiter)
                .iter()
                .position(|f| f.span.contains(&rel));
        }
        self.reveal_row(self.cursor_row(), how);
        cx.notify();
    }

    fn move_rows(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Lines => {
                let lines = self.index.lock().map_or(0, |s| s.lines);
                let ix = self
                    .line
                    .saturating_add_signed(delta)
                    .min(lines.saturating_sub(1));
                // Keep the same column when moving between lines of a type.
                self.select_line(ix, self.field, Reveal::Nearest, cx);
            }
            Mode::Hex => {
                let b = self.byte as i64 + delta as i64 * HEX_WIDTH as i64;
                if b >= 0 && (b as u64) < self.data.len() {
                    self.select_byte(b as u64, Reveal::Nearest, cx);
                }
            }
        }
    }

    fn move_across(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Lines => {
                let count = self
                    .index
                    .lock()
                    .ok()
                    .and_then(|s| s.line(self.line))
                    .map_or(0, |l| l.fields as usize);
                if count == 0 {
                    return;
                }
                let next = match self.field {
                    None if delta > 0 => 0,
                    None => count - 1,
                    Some(f) => f.saturating_add_signed(delta).min(count - 1),
                };
                self.select_line(self.line, Some(next), Reveal::None, cx);
                self.reveal_field_x();
            }
            Mode::Hex => {
                let b = self.byte.saturating_add_signed(delta as i64);
                self.select_byte(b, Reveal::Nearest, cx);
            }
        }
    }

    /// Scroll sideways so the selected field is in view.
    fn reveal_field_x(&mut self) {
        let Some(f) = self.field else { return };
        let Some(fields) = self.splits.get(&self.line).cloned() else {
            return;
        };
        let Some(field) = fields.get(f) else { return };
        // Close enough: one column per byte, plus the gutter.
        let cw = 7.8;
        let start = field.span.start as f32 * cw;
        let end = field.span.end as f32 * cw;
        let width = self.body.get().size.width.as_f32() - self.gutter_width() - 40.;
        if start < self.scroll_x {
            self.scroll_x = (start - 40.).max(0.);
        } else if end > self.scroll_x + width {
            self.scroll_x = end - width;
        }
    }

    fn gutter_width(&self) -> f32 {
        // Line number, offset and kind tag columns, roughly.
        200.
    }

    fn next_matching(
        &mut self,
        forward: bool,
        what: &str,
        f: impl Fn(&Line) -> bool,
        cx: &mut Context<Self>,
    ) {
        let found = {
            let s = self.index.lock().expect("index lock poisoned");
            if forward {
                s.find_line(self.line + 1, &f)
                    .map(|i| (i, false))
                    .or_else(|| s.find_line(0, &f).map(|i| (i, true)))
            } else {
                s.rfind_line(self.line, &f)
                    .map(|i| (i, false))
                    .or_else(|| s.rfind_line(s.lines, &f).map(|i| (i, true)))
            }
        };
        match found {
            Some((ix, wrapped)) => {
                self.notice = wrapped.then(|| {
                    (
                        format!(
                            "Wrapped around to the {}",
                            if forward { "start" } else { "end" }
                        ),
                        false,
                    )
                });
                self.select_line(ix, None, Reveal::Center, cx);
            }
            None => {
                self.notice = Some((format!("No {what}"), true));
                cx.notify();
            }
        }
    }

    fn step_warning(&mut self, forward: bool, cx: &mut Context<Self>) {
        let found = {
            let s = self.index.lock().expect("index lock poisoned");
            let w = &s.warnings;
            if forward {
                let at = w.partition_point(|&l| l <= self.line);
                w.get(at).or(w.first()).copied()
            } else {
                let at = w.partition_point(|&l| l < self.line);
                at.checked_sub(1)
                    .and_then(|i| w.get(i))
                    .or(w.last())
                    .copied()
            }
        };
        match found {
            Some(ix) => {
                self.notice = None;
                self.select_line(ix, None, Reveal::Center, cx);
            }
            None => {
                self.notice = Some(("No warnings".into(), false));
                cx.notify();
            }
        }
    }

    fn next_flag(&mut self, flag: Flags, cx: &mut Context<Self>) {
        // Late flags aren't in the chunks, so go through `line()`.
        self.next_matching(true, "such lines", move |l| l.flags.contains(flag), cx);
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let (text, what) = {
            let s = self.index.lock().expect("index lock poisoned");
            let Some(line) = s.line(self.line) else {
                return;
            };
            let delimiter = Self::delimiter(&s);
            let content = self.content(&line);
            match self.field {
                Some(f) => {
                    let fields = split(content, line.kind, delimiter);
                    let Some(field) = fields.get(f) else { return };
                    (
                        field_value(&content[field.span.clone()], field.quote, delimiter),
                        "field",
                    )
                }
                None => (String::from_utf8_lossy(content).into_owned(), "line"),
            }
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.notice = Some((format!("Copied the {what}"), false));
        cx.notify();
    }

    /// The selected line as a JSON object of column → value.
    fn line_json(&self, s: &IndexState) -> Option<String> {
        let line = s.line(self.line)?;
        let delimiter = Self::delimiter(s);
        let content = self.content(&line);
        let columns = s
            .type_info(&line)
            .and_then(|t| t.mapping.as_ref().ok())
            .map(|m| m.columns)
            .unwrap_or_default();
        let mut out = String::from("{");
        for (i, f) in split(content, line.kind, delimiter).iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let name = columns
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("extra_{i}"));
            let value = field_value(&content[f.span.clone()], f.quote, delimiter);
            out.push_str(&format!("{}: {}", json_string(&name), json_string(&value)));
        }
        out.push('}');
        Some(out)
    }

    fn run_jump(&mut self, backward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.jump.read(cx).value().to_string();
        let Some(jump) = parse_jump(&text) else {
            return;
        };
        self.notice = None;
        let (lines, done, word_type) = {
            let s = self.index.lock().expect("index lock poisoned");
            let word_type = match &jump {
                Jump::Word(w) => s
                    .types
                    .iter()
                    .position(|t| t.name.eq_ignore_ascii_case(w))
                    .map(|t| t as u16),
                _ => None,
            };
            (s.lines, s.done, word_type)
        };
        let mut keep_focus = false;
        match jump {
            Jump::Line(n) => {
                if n >= 1 && n <= lines {
                    self.select_line(n - 1, None, Reveal::Center, cx);
                } else {
                    let more = if done { "" } else { " so far" };
                    self.notice = Some((
                        format!(
                            "Line {n} is past the end ({} lines{more})",
                            fmt_count(lines as u64)
                        ),
                        true,
                    ));
                }
            }
            Jump::Offset(o) => {
                if o < self.data.len() {
                    self.select_byte(o, Reveal::Center, cx);
                } else {
                    self.notice = Some((
                        format!(
                            "Byte {o} is past the end ({} bytes)",
                            fmt_count(self.data.len())
                        ),
                        true,
                    ));
                }
            }
            Jump::Record(r) => {
                let found = self.index.lock().ok().and_then(|s| s.record_line(r));
                match found {
                    Some(ix) => self.select_line(ix, None, Reveal::Center, cx),
                    None => self.notice = Some((format!("No record #{r}"), true)),
                }
            }
            Jump::Word(w) if word_type.is_some() => {
                let t = word_type.unwrap_or_default();
                self.next_matching(
                    !backward,
                    &format!("{w} rows"),
                    move |l| l.row_type == t && l.kind.is_record(),
                    cx,
                );
                keep_focus = true;
            }
            Jump::Word(w) | Jump::Find(w) => {
                self.find_text(w.into_bytes(), backward, cx);
                keep_focus = true;
            }
        }
        if !keep_focus {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn find_text(&mut self, needle: Vec<u8>, backward: bool, cx: &mut Context<Self>) {
        let again = self.find.as_deref() == Some(needle.as_slice());
        let from = if again { self.byte + 1 } else { self.byte };
        let hit = if backward {
            // Search backward by searching the bytes before the cursor.
            let before = &self.bytes()[..self.byte as usize];
            let all = find_all(before, &needle);
            all.last()
                .map(|r| r.start as u64)
                .or_else(|| find(self.bytes(), &needle, self.byte))
        } else {
            find(self.bytes(), &needle, from)
        };
        match hit {
            Some(at) => {
                if !backward && at < from {
                    self.notice = Some(("Wrapped around to the start".into(), false));
                }
                self.find = Some(needle);
                self.select_byte(at, Reveal::Center, cx);
                self.reveal_field_x();
            }
            None => {
                self.notice = Some((
                    format!("Not found: {}", String::from_utf8_lossy(&needle)),
                    true,
                ));
                self.find = None;
            }
        }
    }

    fn focus_jump(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.jump.update(cx, |input, cx| {
            if !prefix.is_empty() && !input.value().starts_with(prefix) {
                input.set_value(prefix.to_string(), window, cx);
            }
            input.focus(window, cx);
        });
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            self.scroll_x = 0.;
            self.top = 0;
            self.reveal_row(self.cursor_row(), Reveal::Center);
            cx.notify();
        }
    }

    // -----------------------------------------------------------------------
    // Scrolling
    // -----------------------------------------------------------------------

    fn on_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(ROW_H));
        let (mut dx, dy) = (delta.x.as_f32(), delta.y.as_f32());
        let mut dy = dy;
        if event.modifiers.shift && dx == 0. {
            dx = dy;
            dy = 0.;
        }
        let total = self.index.lock().map_or(0, |s| self.rows(&s));
        self.scroll_rem -= dy;
        let rows = (self.scroll_rem / ROW_H).trunc();
        self.scroll_rem -= rows * ROW_H;
        let max_top = total.saturating_sub(self.visible_rows());
        self.top = (self.top as i64 + rows as i64).clamp(0, max_top as i64) as usize;
        if self.top == 0 || self.top == max_top {
            self.scroll_rem = 0.;
        }
        self.scroll_x = (self.scroll_x - dx).clamp(0., self.max_scroll_x.max(0.));
        cx.stop_propagation();
        cx.notify();
    }

    fn scroll_to_fraction(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let b = self.body.get();
        let frac = ((y - b.origin.y).as_f32() / b.size.height.as_f32().max(1.)).clamp(0., 1.);
        let total = self.index.lock().map_or(0, |s| self.rows(&s));
        let max_top = total.saturating_sub(self.visible_rows());
        self.top = (frac as f64 * max_top as f64).round() as usize;
        self.scroll_rem = 0.;
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

struct Palette {
    theme: Theme,
}

impl Palette {
    fn class(&self, class: Class) -> HighlightStyle {
        let t = &self.theme;
        let color = match class {
            Class::Text => t.foreground,
            Class::TextAlt => t.foreground.opacity(0.7),
            Class::RowType => t.blue,
            Class::Key => t.cyan,
            Class::Separator => t.magenta,
            Class::Quote => t.yellow,
            Class::LineEnd => t.muted_foreground.opacity(0.6),
            Class::Control => t.warning,
            Class::NonAscii => t.green,
            Class::Invalid => t.danger,
            Class::Prose => t.foreground.opacity(0.85),
            Class::Muted => t.muted_foreground,
        };
        HighlightStyle {
            color: Some(color),
            background_color: (class == Class::Invalid).then(|| t.danger.opacity(0.15)),
            ..Default::default()
        }
    }

    fn bg(color: Hsla) -> HighlightStyle {
        HighlightStyle {
            background_color: Some(color),
            ..Default::default()
        }
    }

    fn kind(&self, line: &Line, row_type: Option<&str>) -> Hsla {
        let t = &self.theme;
        match line.kind {
            Kind::Blank => t.transparent(),
            Kind::HeaderBlock | Kind::Header => t.magenta,
            Kind::Cover => t.green,
            Kind::TextBegin | Kind::Text | Kind::TextEnd => t.yellow,
            Kind::Record => {
                let charts = [
                    t.chart_1, t.chart_2, t.chart_3, t.chart_4, t.chart_5, t.cyan,
                ];
                match row_type.and_then(Schedule::of) {
                    Some(s) => charts[s.index() % charts.len()],
                    None => t.muted_foreground,
                }
            }
        }
    }
}

trait Transparent {
    fn transparent(&self) -> Hsla;
}

impl Transparent for Theme {
    fn transparent(&self) -> Hsla {
        self.background.opacity(0.)
    }
}

impl Render for DebugView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let root = v_flex()
            .id("debug-view")
            .size_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_rows(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_rows(1, cx)))
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.move_across(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.move_across(1, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| {
                let n = this.visible_rows().saturating_sub(1).max(1) as isize;
                this.top = this.top.saturating_sub(n as usize);
                this.move_rows(-n, cx)
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| {
                let n = this.visible_rows().saturating_sub(1).max(1) as isize;
                this.top += n as usize;
                this.move_rows(n, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveToStart, _, cx| match this.mode {
                Mode::Lines => this.select_line(0, None, Reveal::Center, cx),
                Mode::Hex => this.select_byte(0, Reveal::Center, cx),
            }))
            .on_action(cx.listener(|this, _: &MoveToEnd, _, cx| match this.mode {
                Mode::Lines => {
                    let n = this.index.lock().map_or(0, |s| s.lines);
                    this.select_line(n.saturating_sub(1), None, Reveal::Center, cx)
                }
                Mode::Hex => this.select_byte(u64::MAX, Reveal::Center, cx),
            }))
            .on_action(cx.listener(|this, _: &ToggleMode, _, cx| {
                let next = match this.mode {
                    Mode::Lines => Mode::Hex,
                    Mode::Hex => Mode::Lines,
                };
                this.set_mode(next, cx)
            }))
            .on_action(
                cx.listener(|this, _: &FocusJump, window, cx| this.focus_jump("", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FindText, window, cx| this.focus_jump("/", window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextWarning, _, cx| this.step_warning(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousWarning, _, cx| this.step_warning(false, cx)))
            .on_action(cx.listener(|this, _: &NextSameType, _, cx| this.step_same_type(true, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousSameType, _, cx| this.step_same_type(false, cx)),
            )
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| this.copy_selection(cx)))
            .on_action(cx.listener(|this, _: &ClearField, _, cx| {
                this.field = None;
                this.find = None;
                this.notice = None;
                cx.notify();
            }))
            .bg(theme.background)
            .text_color(theme.foreground);

        if let Some(error) = &self.error {
            return root.child(div().p_4().text_color(theme.danger).child(error.clone()));
        }

        let state = self.index.clone();
        let state = state.lock().expect("index lock poisoned");
        let palette = Palette { theme };
        root.child(self.render_toolbar(&state, &palette, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_body(&state, &palette, window, cx))
                    .child(self.render_inspector(&state, &palette, cx)),
            )
            .child(self.render_footer(&state, &palette))
    }
}

impl DebugView {
    fn step_same_type(&mut self, forward: bool, cx: &mut Context<Self>) {
        let t = {
            let s = self.index.lock().expect("index lock poisoned");
            s.line(self.line).map(|l| (l.row_type, l.kind))
        };
        let Some((t, kind)) = t else { return };
        if t == super::index::NO_TYPE {
            // Text and blank lines: hop between lines of the same kind.
            self.next_matching(forward, "more such lines", move |l| l.kind == kind, cx);
        } else {
            self.next_matching(
                forward,
                "more rows of this type",
                move |l| l.row_type == t,
                cx,
            );
        }
    }

    fn render_toolbar(
        &self,
        state: &IndexState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = &p.theme;
        let facts = state.facts.as_ref().map(|f| {
            let style = match f.style {
                HeaderStyle::Hdr => "HDR",
                HeaderStyle::LegacyBlock => "/* Header",
                HeaderStyle::Paper => "paper",
            };
            let delim = match f.delimiter {
                Delimiter::Fs => "0x1C ¦",
                Delimiter::Comma => "comma",
            };
            let version = if f.version.is_empty() {
                "?"
            } else {
                &f.version
            };
            format!("v{version} · {style} · {delim} · {}", f.eol.label())
        });
        let progress = (!state.done).then(|| {
            let pct = state.bytes_scanned as f64 / self.data.len().max(1) as f64 * 100.;
            format!(
                "Indexing… {pct:.0}% · {} lines",
                fmt_count(state.lines as u64)
            )
        });
        let warnings = state.warnings.len();

        h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                ButtonGroup::new("debug-mode")
                    .small()
                    .outline()
                    .child(
                        Button::new("mode-lines")
                            .label("Lines")
                            .selected(self.mode == Mode::Lines),
                    )
                    .child(
                        Button::new("mode-hex")
                            .label("Hex")
                            .selected(self.mode == Mode::Hex),
                    )
                    .on_click(cx.listener(|this, selected: &Vec<usize>, _, cx| {
                        let mode = if selected.first() == Some(&1) {
                            Mode::Hex
                        } else {
                            Mode::Lines
                        };
                        this.set_mode(mode, cx);
                    })),
            )
            .child(Input::new(&self.jump).small().w(px(300.)))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("prev-warning")
                            .small()
                            .ghost()
                            .label("◀")
                            .tooltip("Previous warning (⇧F8)")
                            .disabled(warnings == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.step_warning(false, cx))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if warnings > 0 {
                                theme.warning
                            } else {
                                theme.muted_foreground
                            })
                            .child(match warnings {
                                0 => "No warnings".to_string(),
                                1 => "1 warning".to_string(),
                                n => format!("{} warnings", fmt_count(n as u64)),
                            }),
                    )
                    .child(
                        Button::new("next-warning")
                            .small()
                            .ghost()
                            .label("▶")
                            .tooltip("Next warning (F8)")
                            .disabled(warnings == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.step_warning(true, cx))),
                    ),
            )
            .child(div().flex_1())
            .children(self.notice.as_ref().map(|(text, bad)| {
                div()
                    .text_xs()
                    .text_color(if *bad {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(text.clone())
            }))
            .children(progress.map(|p| div().text_xs().text_color(theme.muted_foreground).child(p)))
            .children(facts.map(|f| {
                div()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(f)
            }))
    }

    fn render_body(
        &mut self,
        state: &IndexState,
        p: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = &p.theme;
        let total = self.rows(state);
        let visible = self.visible_rows();
        let max_top = total.saturating_sub(visible);
        self.top = self.top.min(max_top);
        let end = (self.top + visible + 1).min(total);

        let font_size = theme.mono_font_size;
        let font_id = window
            .text_system()
            .resolve_font(&font(theme.mono_font_family.clone()));
        let cw = window
            .text_system()
            .ch_advance(font_id, font_size)
            .map_or(7.8, |w| w.as_f32());

        let mut max_chars = 0usize;
        let rows: Vec<AnyElement> = (self.top..end)
            .map(|row| match self.mode {
                Mode::Lines => {
                    let (el, chars) = self.render_line_row(row, state, p, cw, cx);
                    max_chars = max_chars.max(chars);
                    el
                }
                Mode::Hex => self.render_hex_row(row, state, p, cx),
            })
            .collect();
        let text_width =
            self.body.get().size.width.as_f32() - self.gutter_px(state, cw) - SCROLLBAR_W - 16.;
        self.max_scroll_x = match self.mode {
            Mode::Lines => (max_chars as f32 * cw - text_width + cw * 4.).max(0.),
            Mode::Hex => 0.,
        };
        self.scroll_x = self.scroll_x.min(self.max_scroll_x);

        let body = self.body.clone();
        let measure = canvas(
            move |bounds, window, _| {
                if body.get() != bounds {
                    body.set(bounds);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .id("debug-body")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .font_family(theme.mono_font_family.clone())
            .text_size(font_size)
            .line_height(px(ROW_H))
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus.focus(window, cx)),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if this.dragging_scrollbar {
                    if e.pressed_button == Some(MouseButton::Left) {
                        this.scroll_to_fraction(e.position.y, cx);
                    } else {
                        this.dragging_scrollbar = false;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging_scrollbar = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging_scrollbar = false),
            )
            .child(measure)
            .child(v_flex().w_full().pr(px(SCROLLBAR_W)).children(rows))
            .when(total == 0, |d| {
                d.child(
                    div()
                        .p_4()
                        .text_color(theme.muted_foreground)
                        .child(if state.done {
                            "Empty file"
                        } else {
                            "Indexing…"
                        }),
                )
            })
            .child(self.render_scrollbar(state, total, p, cx))
    }

    fn gutter_px(&self, state: &IndexState, cw: f32) -> f32 {
        let chars = match self.mode {
            Mode::Lines => line_digits(state.lines) + 2 + offset_digits(self.data.len()) + 2 + 4,
            Mode::Hex => offset_digits(self.data.len()) + 2,
        };
        chars as f32 * cw + 12.
    }

    /// One line in the lines view, and how many characters it draws.
    fn render_line_row(
        &mut self,
        ix: usize,
        state: &IndexState,
        p: &Palette,
        cw: f32,
        cx: &mut Context<Self>,
    ) -> (AnyElement, usize) {
        let theme = &p.theme;
        let Some(line) = state.line(ix) else {
            return (div().h(px(ROW_H)).into_any_element(), 0);
        };
        let delimiter = Self::delimiter(state);
        let fields = self.fields(ix, &line, delimiter);
        let content = self.content(&line);
        let drawn = draw_line(content, line.eol, line.kind, &fields);
        let selected = ix == self.line;

        let base = drawn.runs.iter().map(|(r, c)| (r.clone(), p.class(*c)));
        let mut overlays: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        if let Some(needle) = &self.find {
            for hit in find_all(content, needle) {
                overlays.push((
                    drawn.display_range(hit),
                    Palette::bg(theme.yellow.opacity(0.35)),
                ));
            }
        }
        if selected
            && let Some(f) = self.field
            && let Some(r) = drawn.fields.get(f)
        {
            let r = if r.is_empty() {
                // Show an empty field as its following separator, if any.
                r.start
                    ..(r.start
                        + drawn.text[r.start..]
                            .chars()
                            .next()
                            .map_or(0, char::len_utf8))
            } else {
                r.clone()
            };
            overlays.push((r, Palette::bg(theme.selection)));
        }
        let highlights: Vec<_> = combine_highlights(base, overlays).collect();
        let chars = drawn.text.chars().count();

        let type_info = state.type_info(&line);
        let row_type = type_info.map(|t| t.name.as_str());
        let columns: &'static [String] = type_info
            .and_then(|t| t.mapping.as_ref().ok())
            .map(|m| m.columns)
            .unwrap_or_default();
        let field_ranges = Arc::new(drawn.fields.clone());
        let tooltip_fields = fields.clone();
        let tooltip_ranges = field_ranges.clone();
        let start = line.start;
        let kind = line.kind;
        let entity = cx.weak_entity();
        let text = InteractiveText::new(
            ElementId::NamedInteger("line-text".into(), ix as u64),
            StyledText::new(drawn.text).with_highlights(highlights),
        )
        .on_click((*field_ranges).clone(), move |field, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.select_line(ix, Some(field), Reveal::None, cx)
            });
        })
        .tooltip(move |char_ix, window, cx| {
            let f = tooltip_ranges.iter().position(|r| r.contains(&char_ix))?;
            let span = &tooltip_fields.get(f)?.span;
            let name = match kind {
                Kind::HeaderBlock => if f == 0 { "key" } else { "value" }.to_string(),
                Kind::Text => "text".to_string(),
                _ => columns
                    .get(f)
                    .cloned()
                    .unwrap_or_else(|| format!("(no column {f})")),
            };
            let text = format!(
                "[{f}] {name}  ·  bytes {:#x}–{:#x} ({} B)",
                start + span.start as u64,
                start + span.end as u64,
                span.len()
            );
            Some(Tooltip::new(text).build(window, cx))
        });

        let warn = line.flags.severity();
        let issue_color = match warn {
            Some(Severity::Warning) => theme.warning,
            Some(Severity::Notice) => theme.muted_foreground.opacity(0.5),
            None => theme.transparent(),
        };
        let gutter_color = theme.muted_foreground;
        let entity = cx.weak_entity();
        let row = h_flex()
            .id(ElementId::NamedInteger("line".into(), ix as u64))
            .h(px(ROW_H))
            .w_full()
            .flex_shrink_0()
            .whitespace_nowrap()
            .when(selected, |d| d.bg(theme.list_active))
            .when(!selected, |d| d.hover(|d| d.bg(theme.list_hover)))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let _ = entity.update(cx, |this, cx| {
                    if this.line != ix {
                        this.select_line(ix, None, Reveal::None, cx)
                    }
                });
            })
            // Kind bar, colored per kind (and per schedule for records).
            .child(
                div()
                    .w(px(3.))
                    .h_full()
                    .flex_shrink_0()
                    .bg(p.kind(&line, row_type)),
            )
            .child(
                div()
                    .w(px(6.))
                    .h(px(6.))
                    .mx(px(3.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(issue_color),
            )
            .child(
                div()
                    .w(px(line_digits(state.lines) as f32 * cw))
                    .flex_shrink_0()
                    .text_right()
                    .text_color(gutter_color)
                    .child(fmt_count(ix as u64 + 1)),
            )
            .child(
                div()
                    .w(px((offset_digits(self.data.len()) + 2) as f32 * cw))
                    .ml(px(cw * 2.))
                    .flex_shrink_0()
                    .text_color(gutter_color.opacity(0.7))
                    .child(format!(
                        "{:0w$X}",
                        line.start,
                        w = offset_digits(self.data.len())
                    )),
            )
            .child(
                div()
                    .w(px(cw * 4.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(p.kind(&line, row_type))
                    .child(line.kind.tag()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(div().ml(px(-self.scroll_x)).child(text)),
            );
        (row.into_any_element(), chars)
    }

    fn render_hex_row(
        &mut self,
        row: usize,
        state: &IndexState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &p.theme;
        let start = (row * HEX_WIDTH) as u64;
        let end = (start + HEX_WIDTH as u64).min(self.data.len());
        let bytes: Vec<u8> = self.bytes()[start as usize..end as usize].to_vec();
        let delimiter = Self::delimiter(state);

        // Classify each byte by the line and field it falls in.
        let mut classes = Vec::with_capacity(bytes.len());
        let mut in_line = Vec::with_capacity(bytes.len());
        let mut in_field = Vec::with_capacity(bytes.len());
        // Lines starting in this row, for the offset's color and tooltip.
        let mut starts: Vec<(String, Hsla)> = Vec::new();
        let mut ix = state.line_at(start);
        let mut current: Option<(usize, Line, Arc<Vec<Field>>)> = None;
        for (i, &b) in bytes.iter().enumerate() {
            let off = start + i as u64;
            while let Some(lix) = ix {
                let need = current.as_ref().is_none_or(|(c, ..)| *c != lix);
                if need {
                    match state.line(lix) {
                        Some(l) => {
                            let fields = self.fields(lix, &l, delimiter);
                            current = Some((lix, l, fields));
                        }
                        None => {
                            current = None;
                            ix = None;
                            break;
                        }
                    }
                }
                let (_, l, _) = current.as_ref().expect("set above");
                if off >= l.end() && lix + 1 < state.lines {
                    ix = Some(lix + 1);
                    continue;
                }
                break;
            }
            match &current {
                Some((lix, l, fields)) => {
                    if off == l.start {
                        let what = state
                            .type_info(l)
                            .map(|t| t.name.clone())
                            .unwrap_or_else(|| l.kind.label().to_string());
                        let color = p.kind(l, state.type_info(l).map(|t| t.name.as_str()));
                        starts.push((
                            format!(
                                "Line {} starts at {off:#x}: {what}",
                                fmt_count(*lix as u64 + 1)
                            ),
                            color,
                        ));
                    }
                    let rel = (off - l.start) as usize;
                    let in_content = off < l.content().end;
                    let field = fields.iter().position(|f| f.span.contains(&rel));
                    classes.push(byte_class(b, in_content, l.kind, field, lix % 2 == 1));
                    in_line.push(*lix == self.line);
                    in_field.push(*lix == self.line && field.is_some() && field == self.field);
                }
                None => {
                    classes.push(Class::Muted);
                    in_line.push(false);
                    in_field.push(false);
                }
            }
        }

        let mut hex_hl = Vec::new();
        let mut ascii_hl = Vec::new();
        for (i, class) in classes.iter().enumerate() {
            let mut style = p.class(*class);
            if in_line[i] {
                style.background_color = Some(theme.list_active);
            }
            if in_field[i] {
                style.background_color = Some(theme.selection);
            }
            if start + i as u64 == self.byte {
                style.background_color = Some(theme.primary);
                style.color = Some(theme.primary_foreground);
            }
            hex_hl.push((hex_range(i), style));
            ascii_hl.push((i..i + 1, style));
        }
        if let Some(needle) = &self.find {
            // Look a little past both ends so matches across rows show.
            let lo = start.saturating_sub(needle.len() as u64 - 1);
            let hi = (end + needle.len() as u64 - 1).min(self.data.len());
            let window = &self.bytes()[lo as usize..hi as usize];
            let mut hits = Vec::new();
            for r in find_all(window, needle) {
                for b in r {
                    let off = lo + b as u64;
                    if off >= start && off < end {
                        hits.push((off - start) as usize);
                    }
                }
            }
            let find_bg = Palette::bg(theme.yellow.opacity(0.35));
            let hex_hits: Vec<_> = hits.iter().map(|&i| (hex_range(i), find_bg)).collect();
            let ascii_hits: Vec<_> = hits.iter().map(|&i| (i..i + 1, find_bg)).collect();
            hex_hl = combine_highlights(hex_hl, hex_hits).collect();
            ascii_hl = combine_highlights(ascii_hl, ascii_hits).collect();
        }

        let n = bytes.len();
        let entity = cx.weak_entity();
        let hex = InteractiveText::new(
            ElementId::NamedInteger("hex".into(), row as u64),
            StyledText::new(hex_text(&bytes)).with_highlights(hex_hl),
        )
        .on_click((0..n).map(hex_range).collect(), move |i, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.select_byte(start + i as u64, Reveal::None, cx)
            });
        });
        let entity = cx.weak_entity();
        let ascii = InteractiveText::new(
            ElementId::NamedInteger("ascii".into(), row as u64),
            StyledText::new(ascii_text(&bytes)).with_highlights(ascii_hl),
        )
        .on_click((0..n).map(|i| i..i + 1).collect(), move |i, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.select_byte(start + i as u64, Reveal::None, cx)
            });
        });

        let w = offset_digits(self.data.len());
        let cursor_row = self.byte / HEX_WIDTH as u64 == row as u64;
        h_flex()
            .id(ElementId::NamedInteger("hex-row".into(), row as u64))
            .h(px(ROW_H))
            .w_full()
            .gap_3()
            .px_3()
            .whitespace_nowrap()
            .when(cursor_row, |d| d.bg(theme.list_hover))
            .child({
                // A line starting in this row colors the offset by its kind.
                let color = starts.first().map_or(theme.muted_foreground, |(_, c)| *c);
                let tip = starts
                    .iter()
                    .map(|(t, _)| t.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                div()
                    .id(ElementId::NamedInteger("hex-offset".into(), row as u64))
                    .text_color(color)
                    .child(format!("{start:0w$X}"))
                    .when(!tip.is_empty(), |d| {
                        d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
            })
            .child(hex)
            .child(
                h_flex()
                    .child(div().text_color(theme.muted_foreground).child("│"))
                    .child(ascii)
                    .child(div().text_color(theme.muted_foreground).child("│")),
            )
            .into_any_element()
    }

    fn render_scrollbar(
        &self,
        state: &IndexState,
        total: usize,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = &p.theme;
        let height = self.body.get().size.height.as_f32();
        let visible = self.visible_rows();
        let (thumb_top, thumb_h) = if total > visible && height > 0. {
            let h = (visible as f32 / total as f32 * height)
                .max(24.)
                .min(height);
            let max_top = (total - visible) as f64;
            let top = (self.top as f64 / max_top) as f32 * (height - h);
            (top, h)
        } else {
            (0., 0.)
        };

        // Where a line sits along the bar, 0–1.
        let len = self.data.len().max(1) as f64;
        let at = |ix: usize| -> Option<f32> {
            Some(match self.mode {
                Mode::Lines => ix as f64 / state.lines.max(1) as f64,
                Mode::Hex => state.line(ix)?.start as f64 / len,
            } as f32)
        };
        let step = state.warnings.len().div_ceil(MAX_TICKS).max(1);
        let ticks: Vec<_> = state
            .warnings
            .iter()
            .step_by(step)
            .filter_map(|&w| at(w))
            .map(|f| {
                div()
                    .absolute()
                    .left(px(2.))
                    .w(px(SCROLLBAR_W - 4.))
                    .h(px(2.))
                    .top(px(f * (height - 2.)))
                    .bg(theme.warning)
            })
            .collect();
        let cursor_tick = at(self.line).map(|f| {
            div()
                .absolute()
                .left_0()
                .w(px(SCROLLBAR_W))
                .h(px(2.))
                .top(px(f * (height - 2.)))
                .bg(theme.primary)
        });

        div()
            .id("debug-scrollbar")
            .absolute()
            .top_0()
            .right_0()
            .h_full()
            .w(px(SCROLLBAR_W))
            .bg(theme.scrollbar)
            .border_l_1()
            .border_color(theme.border)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.dragging_scrollbar = true;
                    this.scroll_to_fraction(e.position.y, cx);
                    cx.stop_propagation();
                }),
            )
            .when(thumb_h > 0., |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(2.))
                        .w(px(SCROLLBAR_W - 4.))
                        .top(px(thumb_top))
                        .h(px(thumb_h))
                        .rounded_sm()
                        .bg(theme.scrollbar_thumb),
                )
            })
            .children(ticks)
            .children(cursor_tick)
    }

    fn render_footer(&self, state: &IndexState, p: &Palette) -> impl IntoElement {
        let theme = &p.theme;
        let mut parts = Vec::new();
        if let Some(line) = state.line(self.line) {
            parts.push(format!(
                "Ln {} / {}",
                fmt_count(self.line as u64 + 1),
                fmt_count(state.lines as u64)
            ));
            if let Some(r) = state.record_number(self.line) {
                parts.push(format!("Rec #{}", fmt_count(r)));
            }
            parts.push(format!("Byte {:#x} ({})", self.byte, fmt_count(self.byte)));
            if let Some(t) = state.type_info(&line) {
                parts.push(t.name.clone());
            } else {
                parts.push(line.kind.label().to_string());
            }
            if let Some(f) = self.field {
                let name = state
                    .type_info(&line)
                    .and_then(|t| t.mapping.as_ref().ok())
                    .and_then(|m| m.columns.get(f).cloned());
                parts.push(match name {
                    Some(n) => format!("[{f}] {n}"),
                    None => format!("[{f}]"),
                });
            }
        }
        h_flex()
            .px_3()
            .py_1()
            .gap_4()
            .text_xs()
            .border_t_1()
            .border_color(theme.border)
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .flex_1()
                    .font_family(theme.mono_font_family.clone())
                    .child(parts.join("  ·  ")),
            )
            .child("⌘G jump · / find · F8 warning · [ ] same type · x hex · ⌘C copy")
    }

    // -----------------------------------------------------------------------
    // Inspector
    // -----------------------------------------------------------------------

    fn render_inspector(
        &mut self,
        state: &IndexState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = &p.theme;
        let content = match self.tab {
            Tab::Line => self.render_line_tab(state, p, cx),
            Tab::File => self.render_file_tab(state, p, cx),
        };
        v_flex()
            .w(px(INSPECTOR_W))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        ButtonGroup::new("inspector-tab")
                            .small()
                            .outline()
                            .child(
                                Button::new("tab-line")
                                    .label("Line")
                                    .selected(self.tab == Tab::Line),
                            )
                            .child(
                                Button::new("tab-file")
                                    .label("File")
                                    .selected(self.tab == Tab::File),
                            )
                            .on_click(cx.listener(|this, selected: &Vec<usize>, _, cx| {
                                this.tab = if selected.first() == Some(&1) {
                                    Tab::File
                                } else {
                                    Tab::Line
                                };
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id("inspector-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .gap_3()
                    .text_sm()
                    .children(content),
            )
    }

    fn render_line_tab(
        &mut self,
        state: &IndexState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = p.theme.clone();
        let Some(line) = state.line(self.line) else {
            return vec![muted("Nothing selected", &theme)];
        };
        let delimiter = Self::delimiter(state);
        let fields = self.fields(self.line, &line, delimiter);
        let content = self.content(&line).to_vec();
        let info = state.type_info(&line).cloned();
        let mapping = info.as_ref().and_then(|t| t.mapping.as_ref().ok()).cloned();
        let columns: &[String] = mapping.as_ref().map(|m| m.columns).unwrap_or_default();
        let mono = theme.mono_font_family.clone();
        let mut out = Vec::new();

        // Position
        let mut kv = Kv::new(&theme);
        kv.row(
            "Line",
            format!(
                "{} of {}",
                fmt_count(self.line as u64 + 1),
                fmt_count(state.lines as u64)
            ),
        );
        kv.row(
            "Record",
            match state.record_number(self.line) {
                Some(n) => format!(
                    "#{} {}",
                    fmt_count(n),
                    match n {
                        0 => "(header)",
                        1 => "(cover)",
                        _ => "",
                    }
                ),
                None => match line.kind {
                    Kind::Blank => "— (blank lines are skipped)".into(),
                    _ => "— (inside a text block)".into(),
                },
            },
        );
        kv.row("Kind", line.kind.label().to_string());
        kv.row(
            "Bytes",
            format!(
                "{:#x}–{:#x}  ({}–{})",
                line.start,
                line.end(),
                fmt_count(line.start),
                fmt_count(line.end())
            ),
        );
        kv.row(
            "Length",
            format!(
                "{} B + {} line ending",
                fmt_count(line.len as u64),
                line.eol.label()
            ),
        );
        out.push(section("Position", kv.render(), &theme));

        // Row type and mapping
        if let Some(info) = &info {
            let mut kv = Kv::new(&theme);
            let schedule = (line.kind == Kind::Record)
                .then(|| Schedule::of(&info.name))
                .flatten()
                .map(|s| s.label());
            kv.row(
                "Row type",
                match schedule {
                    Some(s) => format!("{}  ({s})", info.name),
                    None => info.name.clone(),
                },
            );
            kv.row(
                "Seen",
                format!(
                    "{} rows, first on line {}",
                    fmt_count(info.count as u64),
                    fmt_count(info.first_line as u64 + 1)
                ),
            );
            let version = state
                .facts
                .as_ref()
                .map(|f| f.version.clone())
                .unwrap_or_default();
            match &info.mapping {
                Ok(m) => {
                    kv.row("Form key", m.form_pattern.to_string());
                    kv.row(
                        "Version key",
                        format!("{}  (for v{version})", m.version_pattern),
                    );
                    let n = line.fields as usize;
                    let cols = m.columns.len();
                    kv.row(
                        "Fields",
                        match n.cmp(&cols) {
                            std::cmp::Ordering::Equal => format!("{n} of {cols} columns"),
                            std::cmp::Ordering::Less => {
                                format!("{n} of {cols} columns ({} trailing omitted)", cols - n)
                            }
                            std::cmp::Ordering::Greater => {
                                format!("{n} for {cols} columns ({} extra)", n - cols)
                            }
                        },
                    );
                }
                Err(e) => kv.row_colored("Mapping", e.clone(), theme.danger),
            }
            out.push(section("Mapping", kv.render(), &theme));
        }

        // Issues
        if !line.flags.is_empty() {
            let items: Vec<AnyElement> = line
                .flags
                .iter()
                .map(|(sev, label, about)| {
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(
                            div()
                                .mt(px(6.))
                                .w(px(6.))
                                .h(px(6.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(match sev {
                                    Severity::Warning => theme.warning,
                                    Severity::Notice => theme.muted_foreground,
                                }),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().font_weight(FontWeight::MEDIUM).child(label))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(about),
                                ),
                        )
                        .into_any_element()
                })
                .collect();
            out.push(section(
                "Issues",
                v_flex().gap_2().children(items).into_any_element(),
                &theme,
            ));
        }

        // Fields
        let field_rows: Vec<AnyElement> = fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let raw = &content[f.span.clone()];
                let value = match line.kind {
                    Kind::HeaderBlock => String::from_utf8_lossy(raw).trim().to_string(),
                    _ => field_value(raw, f.quote, delimiter),
                };
                let name = match line.kind {
                    Kind::HeaderBlock => if i == 0 { "key" } else { "value" }.to_string(),
                    Kind::Text => "text".to_string(),
                    _ => columns
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("extra_{i}")),
                };
                let extra =
                    !matches!(line.kind, Kind::HeaderBlock | Kind::Text) && i >= columns.len();
                let selected = self.field == Some(i);
                h_flex()
                    .id(ElementId::NamedInteger("field".into(), i as u64))
                    .px_1()
                    .py(px(2.))
                    .gap_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.selection))
                    .when(!selected, |d| d.hover(|d| d.bg(theme.list_hover)))
                    .child(
                        div()
                            .w(px(24.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_right()
                            .text_color(theme.muted_foreground)
                            .font_family(mono.clone())
                            .child(i.to_string()),
                    )
                    .child(
                        div()
                            .w(px(150.))
                            .flex_shrink_0()
                            .truncate()
                            .text_xs()
                            .text_color(if extra {
                                theme.danger
                            } else {
                                theme.muted_foreground
                            })
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono.clone())
                            .text_xs()
                            .when(value.is_empty(), |d| {
                                d.text_color(theme.muted_foreground.opacity(0.5))
                            })
                            .child(if value.is_empty() {
                                "∅".to_string()
                            } else {
                                value
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let ix = this.line;
                        this.select_line(ix, Some(i), Reveal::Nearest, cx);
                        this.reveal_field_x();
                    }))
                    .into_any_element()
            })
            .collect();
        if !field_rows.is_empty() {
            out.push(section(
                &format!("Fields ({})", fields.len()),
                v_flex().children(field_rows).into_any_element(),
                &theme,
            ));
        }

        // The selected field, byte by byte.
        if let Some(f) = self.field.and_then(|f| fields.get(f).map(|x| (f, x))) {
            let (i, field) = f;
            let raw = &content[field.span.clone()];
            let name = columns.get(i).cloned();
            let value = field_value(raw, field.quote, delimiter);
            let mut kv = Kv::new(&theme);
            kv.row("Column", name.clone().unwrap_or_else(|| "(none)".into()));
            kv.row(
                "Bytes",
                format!(
                    "{:#x}–{:#x}  (+{} in line, {} B)",
                    line.start + field.span.start as u64,
                    line.start + field.span.end as u64,
                    field.span.start,
                    raw.len()
                ),
            );
            kv.row(
                "Quoting",
                match field.quote {
                    Quote::None => "none",
                    Quote::Wrapped => "wrapped in quotes (stripped)",
                    Quote::Unterminated => "opening quote never closed",
                    Quote::Stray => "stray quote, kept as written",
                }
                .to_string(),
            );
            if let Some(n) = &name {
                if DATE_COLUMNS.contains(n) {
                    kv.row(
                        "As date",
                        read_date(value.trim()).unwrap_or_else(|| "not a CCYYMMDD date".into()),
                    );
                }
                if FLOAT_COLUMNS.contains(n) && !value.trim().is_empty() {
                    kv.row(
                        "As amount",
                        value
                            .trim()
                            .parse::<f64>()
                            .map(crate::cover::layout::format_usd)
                            .unwrap_or_else(|_| "not a number".into()),
                    );
                }
            }
            let hex: String = raw
                .iter()
                .take(48)
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let more = if raw.len() > 48 { " …" } else { "" };
            let body = v_flex()
                .gap_2()
                .child(kv.render())
                .child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.muted)
                        .font_family(mono.clone())
                        .text_xs()
                        .child(format!("{hex}{more}")),
                )
                .child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.muted)
                        .font_family(mono.clone())
                        .text_xs()
                        .child(format!("{value:?}")),
                )
                .into_any_element();
            out.push(section(&format!("Field {i}"), body, &theme));
        }

        // Copy helpers
        let json = self.line_json(state);
        out.push(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("copy-line")
                        .small()
                        .outline()
                        .label("Copy line")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.field = None;
                            this.copy_selection(cx);
                        })),
                )
                .child(
                    Button::new("copy-offset")
                        .small()
                        .outline()
                        .label("Copy offset")
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(format!(
                                "{:#x}",
                                this.byte
                            )));
                            this.notice = Some(("Copied the offset".into(), false));
                            cx.notify();
                        })),
                )
                .when_some(json, |d, json| {
                    d.child(
                        Button::new("copy-json")
                            .small()
                            .outline()
                            .label("Copy as JSON")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(json.clone()));
                                this.notice = Some(("Copied the line as JSON".into(), false));
                                cx.notify();
                            })),
                    )
                })
                .into_any_element(),
        );
        out
    }

    fn render_file_tab(
        &mut self,
        state: &IndexState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = p.theme.clone();
        let mono = theme.mono_font_family.clone();
        let mut out = Vec::new();

        let mut kv = Kv::new(&theme);
        kv.row("Size", format!("{} bytes", fmt_count(self.data.len())));
        kv.row(
            "Lines",
            format!(
                "{}{}",
                fmt_count(state.lines as u64),
                if state.done { "" } else { " so far" }
            ),
        );
        if let Some(f) = &state.facts {
            kv.row(
                "Header",
                match f.style {
                    HeaderStyle::Hdr => "HDR record",
                    HeaderStyle::LegacyBlock => "/* Header block (v1–2)",
                    HeaderStyle::Paper => "HDR record, paper filing",
                }
                .to_string(),
            );
            kv.row(
                "Version",
                if f.version.is_empty() {
                    "unknown".into()
                } else {
                    f.version.clone()
                },
            );
            kv.row(
                "Supported",
                if fec_parser::format::is_supported_version(&f.version) {
                    "yes"
                } else {
                    "no"
                }
                .to_string(),
            );
            kv.row(
                "Delimiter",
                match f.delimiter {
                    Delimiter::Fs => "0x1C (file separator)",
                    Delimiter::Comma => "comma, with quoting",
                }
                .to_string(),
            );
            kv.row(
                "BOM",
                if f.bom { "yes (skipped)" } else { "no" }.to_string(),
            );
        }
        let mut eols: Vec<_> = state
            .eols
            .iter()
            .filter(|(e, _)| **e != Eol::None)
            .collect();
        eols.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        kv.row(
            "Line endings",
            eols.iter()
                .map(|(e, n)| format!("{} {}", e.label(), fmt_count(**n as u64)))
                .collect::<Vec<_>>()
                .join(" · "),
        );
        out.push(section("File", kv.render(), &theme));

        // Kinds
        let mut kv = Kv::new(&theme);
        for kind in [
            Kind::HeaderBlock,
            Kind::Header,
            Kind::Cover,
            Kind::Record,
            Kind::TextBegin,
            Kind::Text,
            Kind::Blank,
        ] {
            if let Some(&n) = state.kinds.get(&kind).filter(|n| **n > 0) {
                kv.row(kind.label(), plural_lines(n));
            }
        }
        out.push(section("Line kinds", kv.render(), &theme));

        // Row types
        let mut types: Vec<_> = state.types.iter().enumerate().collect();
        types.sort_by_key(|(_, t)| t.first_line);
        let rows: Vec<AnyElement> = types
            .into_iter()
            .map(|(id, t)| {
                let first = t.first_line;
                let (status, color) = match &t.mapping {
                    Ok(m) => (format!("{} cols", m.columns.len()), theme.muted_foreground),
                    Err(_) => ("unmapped".into(), theme.danger),
                };
                h_flex()
                    .id(ElementId::NamedInteger("type".into(), id as u64))
                    .px_1()
                    .py(px(2.))
                    .gap_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|d| d.bg(theme.list_hover))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono.clone())
                            .child(t.name.clone()),
                    )
                    .child(div().text_xs().text_color(color).child(status))
                    .child(
                        div()
                            .w(px(70.))
                            .text_right()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(fmt_count(t.count as u64)),
                    )
                    .tooltip({
                        let tip = match &t.mapping {
                            Ok(m) => format!(
                                "{} · {} · first on line {}",
                                m.form_pattern,
                                m.version_pattern,
                                first + 1
                            ),
                            Err(e) => e.clone(),
                        };
                        move |window, cx| Tooltip::new(tip.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_line(first, None, Reveal::Center, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        out.push(section(
            "Row types",
            v_flex().children(rows).into_any_element(),
            &theme,
        ));

        // Issues
        let issue_rows: Vec<AnyElement> =
            Flags::ALL
                .iter()
                .zip(state.flag_counts)
                .filter(|(_, n)| *n > 0)
                .map(|((flag, sev, label, about), n)| {
                    let flag = *flag;
                    h_flex()
                        .id(SharedString::from(format!("issue-{label}")))
                        .px_1()
                        .py(px(2.))
                        .gap_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|d| d.bg(theme.list_hover))
                        .child(div().w(px(6.)).h(px(6.)).rounded_full().flex_shrink_0().bg(
                            match sev {
                                Severity::Warning => theme.warning,
                                Severity::Notice => theme.muted_foreground,
                            },
                        ))
                        .child(div().flex_1().child(*label))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(plural_lines(n)),
                        )
                        .tooltip({
                            let about = format!("{about} Click for the next one.");
                            move |window, cx| Tooltip::new(about.clone()).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.next_flag(flag, cx)))
                        .into_any_element()
                })
                .collect();
        if issue_rows.is_empty() {
            out.push(section("Issues", muted("None found", &theme), &theme));
        } else {
            out.push(section(
                "Issues",
                v_flex().children(issue_rows).into_any_element(),
                &theme,
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Small pieces
// ---------------------------------------------------------------------------

fn section(title: &str, body: AnyElement, theme: &Theme) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(title.to_uppercase()),
        )
        .child(body)
        .into_any_element()
}

fn muted(text: &str, theme: &Theme) -> AnyElement {
    div()
        .text_color(theme.muted_foreground)
        .child(text.to_string())
        .into_any_element()
}

/// Label / value rows.
struct Kv {
    rows: Vec<(String, String, Option<Hsla>)>,
    label: Hsla,
    mono: SharedString,
}

impl Kv {
    fn new(theme: &Theme) -> Self {
        Self {
            rows: Vec::new(),
            label: theme.muted_foreground,
            mono: theme.mono_font_family.clone(),
        }
    }

    fn row(&mut self, label: &str, value: String) {
        self.rows.push((label.to_string(), value, None));
    }

    fn row_colored(&mut self, label: &str, value: String, color: Hsla) {
        self.rows.push((label.to_string(), value, Some(color)));
    }

    fn render(self) -> AnyElement {
        let (label, mono) = (self.label, self.mono);
        v_flex()
            .gap(px(2.))
            .children(self.rows.into_iter().map(|(k, v, color)| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .w(px(96.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(label)
                            .child(k),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .font_family(mono.clone())
                            .when_some(color, |d, c| d.text_color(c))
                            .child(v),
                    )
            }))
            .into_any_element()
    }
}

fn plural_lines(n: usize) -> String {
    if n == 1 {
        "1 line".into()
    } else {
        format!("{} lines", fmt_count(n as u64))
    }
}
