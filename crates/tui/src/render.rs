use std::sync::LazyLock;

use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SynStyle, Theme, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};
use unicode_width::UnicodeWidthStr;

const CODE_BG: Color = Color::Rgb(40, 42, 54);
const INLINE_CODE_BG: Color = Color::Rgb(60, 60, 70);
const INLINE_CODE_FG: Color = Color::Rgb(229, 181, 103);
const QUOTE_COLOR: Color = Color::Rgb(150, 150, 150);
const QUOTE_BG: Color = Color::Rgb(50, 50, 58);
const QUOTE_FG: Color = Color::Rgb(200, 200, 200);
const LINK_COLOR: Color = Color::Rgb(120, 170, 255);
const RULE_COLOR: Color = Color::DarkGray;
const CODE_LANG_COLOR: Color = Color::Rgb(180, 180, 100);
const SYNTECT_THEME: &str = "base16-ocean.dark";
const QUOTE_PREFIX: &str = "▎ ";
const QUOTE_PREFIX_W: usize = 2;
const TABLE_MIN_COL: usize = 3;
const CODE_LEFT_PAD: usize = 2;

// Loading syntect's defaults costs tens of milliseconds; unlike mdview (which
// renders once at startup) we re-render on every repo navigation, so share
// them process-wide.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<Theme> =
    LazyLock::new(|| ThemeSet::load_defaults().themes[SYNTECT_THEME].clone());

pub fn render(source: &str, width: u16) -> Text<'static> {
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_SMART_PUNCTUATION,
    );
    let mut r = Renderer::new(width);
    for ev in parser {
        r.event(ev);
    }
    r.finish()
}

struct Renderer {
    width: u16,
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    style: Style,
    style_stack: Vec<Style>,
    lists: Vec<Option<u64>>,
    quote_depth: u16,
    quote_starts: Vec<usize>,
    code: Option<CodeCtx>,
    table: Option<TableCtx>,
    item_pending: bool,
}

struct CodeCtx {
    lang: String,
    body: String,
}

struct TableCtx {
    _alignments: Vec<Alignment>,
    rows: Vec<Vec<Vec<Span<'static>>>>,
    cur_row: Vec<Vec<Span<'static>>>,
    cur_cell: Vec<Span<'static>>,
}

impl Renderer {
    fn new(width: u16) -> Self {
        Self {
            width,
            lines: vec![],
            cur: vec![],
            style: Style::default(),
            style_stack: vec![],
            lists: vec![],
            quote_depth: 0,
            quote_starts: vec![],
            code: None,
            table: None,
            item_pending: false,
        }
    }

    fn finish(mut self) -> Text<'static> {
        if !self.cur.is_empty() {
            self.flush_line();
        }
        Text::from(self.lines)
    }

    fn push_style(&mut self, add: Style) {
        self.style_stack.push(self.style);
        self.style = self.style.patch(add);
    }

    fn pop_style(&mut self) {
        if let Some(s) = self.style_stack.pop() {
            self.style = s;
        }
    }

    fn push_span(&mut self, span: Span<'static>) {
        if let Some(t) = self.table.as_mut() {
            t.cur_cell.push(span);
            return;
        }
        self.cur.push(span);
    }

    fn push_text(&mut self, t: &str) {
        let span = Span::styled(t.to_string(), self.style);
        self.push_span(span);
    }

    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.cur);
        self.lines.push(Line::from(spans));
    }

    fn ensure_line_break(&mut self) {
        if !self.cur.is_empty() {
            self.flush_line();
        }
    }

    fn blank_line(&mut self) {
        self.ensure_line_break();
        let last_blank = self
            .lines
            .last()
            .map(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
            .unwrap_or(true);
        if last_blank {
            return;
        }
        self.lines.push(Line::default());
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(c) => self.inline_code(&c),
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(name) => {
                self.push_text(&format!("[^{name}]"));
            }
            Event::SoftBreak => {
                if self.code.is_some() {
                    self.text("\n");
                } else {
                    self.push_text(" ");
                }
            }
            Event::HardBreak => {
                if self.code.is_some() {
                    self.text("\n");
                } else {
                    self.flush_line();
                }
            }
            Event::Rule => {
                self.ensure_line_break();
                let rule = "─".repeat(self.width as usize);
                self.lines.push(Line::from(Span::styled(
                    rule,
                    Style::default().fg(RULE_COLOR),
                )));
            }
            Event::TaskListMarker(b) => {
                let mark = if b { "[x] " } else { "[ ] " };
                self.push_text(mark);
            }
            _ => {}
        }
    }

    fn text(&mut self, t: &str) {
        if let Some(c) = self.code.as_mut() {
            c.body.push_str(t);
            return;
        }
        self.push_text(t);
    }

    fn inline_code(&mut self, c: &str) {
        let style = Style::default().bg(INLINE_CODE_BG).fg(INLINE_CODE_FG);
        self.push_span(Span::styled(c.to_string(), style));
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.item_pending {
                    self.item_pending = false;
                } else {
                    self.ensure_line_break();
                }
            }
            Tag::Heading { level, .. } => {
                self.blank_line();
                self.push_style(heading_style(level));
            }
            Tag::BlockQuote(_) => {
                self.blank_line();
                self.quote_depth += 1;
                self.quote_starts.push(self.lines.len());
            }
            Tag::CodeBlock(kind) => {
                self.ensure_line_break();
                self.blank_line();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some(CodeCtx {
                    lang,
                    body: String::new(),
                });
            }
            Tag::List(start) => {
                if self.lists.is_empty() {
                    self.ensure_line_break();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.ensure_line_break();
                let depth = self.lists.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{indent}{n}. ");
                        *n += 1;
                        m
                    }
                    _ => format!("{indent}• "),
                };
                self.cur
                    .push(Span::styled(marker, Style::default().fg(Color::Cyan)));
                self.item_pending = true;
            }
            Tag::Emphasis => {
                self.push_style(Style::default().add_modifier(Modifier::ITALIC))
            }
            Tag::Strong => self.push_style(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => {
                self.push_style(Style::default().add_modifier(Modifier::CROSSED_OUT))
            }
            Tag::Link { .. } => self.push_style(
                Style::default()
                    .fg(LINK_COLOR)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Tag::Image { dest_url, .. } => {
                self.push_text(&format!("[image: {dest_url}]"));
                self.push_style(Style::default().fg(Color::DarkGray));
            }
            Tag::Table(aligns) => {
                self.ensure_line_break();
                self.blank_line();
                self.table = Some(TableCtx {
                    _alignments: aligns,
                    rows: vec![],
                    cur_row: vec![],
                    cur_cell: vec![],
                });
            }
            Tag::TableHead | Tag::TableRow | Tag::TableCell => {}
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.ensure_line_break();
                self.blank_line();
            }
            TagEnd::Heading(level) => {
                let text_width: usize = self
                    .cur
                    .iter()
                    .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                    .sum();
                self.pop_style();
                self.ensure_line_break();
                let bar_width = text_width.min(self.width as usize);
                if bar_width > 0 {
                    let (ch, color) = match level {
                        HeadingLevel::H1 => ("═", Color::Magenta),
                        HeadingLevel::H2 => ("─", Color::Cyan),
                        _ => ("", Color::Reset),
                    };
                    if !ch.is_empty() {
                        let bar = ch.repeat(bar_width);
                        self.lines
                            .push(Line::from(Span::styled(bar, Style::default().fg(color))));
                    }
                }
                self.blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.ensure_line_break();
                self.quote_depth = self.quote_depth.saturating_sub(1);
                let start = self.quote_starts.pop().unwrap_or(self.lines.len());
                if self.quote_depth == 0 {
                    self.emit_blockquote(start);
                }
                self.blank_line();
            }
            TagEnd::CodeBlock => {
                if let Some(ctx) = self.code.take() {
                    self.emit_code_block(&ctx);
                }
                self.blank_line();
            }
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.ensure_line_break();
                    self.blank_line();
                }
            }
            TagEnd::Item => self.ensure_line_break(),
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Link => self.pop_style(),
            TagEnd::Image => self.pop_style(),
            TagEnd::Table => {
                if let Some(ctx) = self.table.take() {
                    self.emit_table(&ctx);
                }
                self.blank_line();
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.cur_row);
                    if !row.is_empty() {
                        t.rows.push(row);
                    }
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.cur_row);
                    if !row.is_empty() {
                        t.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                if let Some(t) = self.table.as_mut() {
                    let cell = std::mem::take(&mut t.cur_cell);
                    t.cur_row.push(cell);
                }
            }
            _ => {}
        }
    }

    fn emit_code_block(&mut self, ctx: &CodeCtx) {
        let syntax = if ctx.lang.is_empty() {
            SYNTAX_SET.find_syntax_plain_text()
        } else {
            SYNTAX_SET
                .find_syntax_by_token(&ctx.lang)
                .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text())
        };
        let mut hl = HighlightLines::new(syntax, &THEME);
        let width = self.width as usize;
        let bg = Style::default().bg(CODE_BG);
        let left_pad = Span::styled(" ".repeat(CODE_LEFT_PAD), bg);

        if !ctx.lang.is_empty() {
            let label = format!(" {} ", ctx.lang);
            let pad = width.saturating_sub(UnicodeWidthStr::width(label.as_str()));
            self.lines.push(Line::from(vec![
                Span::styled(
                    label,
                    bg.fg(CODE_LANG_COLOR).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ".repeat(pad), bg),
            ]));
        } else {
            self.lines.push(Line::from(Span::styled(" ".repeat(width), bg)));
        }

        let body = if ctx.body.is_empty() { "\n" } else { &ctx.body };
        for raw in LinesWithEndings::from(body) {
            let line = raw.trim_end_matches('\n');
            let ranges = hl
                .highlight_line(line, &SYNTAX_SET)
                .unwrap_or_else(|_| vec![(SynStyle::default(), line)]);
            let mut spans: Vec<Span<'static>> = vec![left_pad.clone()];
            spans.extend(ranges.into_iter().map(|(s, t)| {
                Span::styled(t.to_string(), syn_to_rt_style(s).bg(CODE_BG))
            }));
            let content_w: usize = spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            if content_w < width {
                spans.push(Span::styled(" ".repeat(width - content_w), bg));
            }
            self.lines.push(Line::from(spans));
        }
        self.lines.push(Line::from(Span::styled(" ".repeat(width), bg)));
        self.lines.push(Line::default());
    }

    fn emit_table(&mut self, ctx: &TableCtx) {
        if ctx.rows.is_empty() {
            return;
        }
        let cols = ctx.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if cols == 0 {
            return;
        }

        let mut natural: Vec<usize> = vec![0; cols];
        for row in &ctx.rows {
            for (i, cell) in row.iter().enumerate() {
                let w = cell_width(cell);
                natural[i] = natural[i].max(w);
            }
        }

        let max_total = self.width as usize;
        let frame_overhead = cols + 1 + cols * 2;
        let avail = max_total.saturating_sub(frame_overhead);
        let mins: Vec<usize> = (0..cols)
            .map(|ci| {
                let header_first = ctx
                    .rows
                    .first()
                    .and_then(|r| r.get(ci))
                    .map(|cell| first_word_width(cell))
                    .unwrap_or(0);
                if header_first > 0 {
                    header_first + 2
                } else {
                    TABLE_MIN_COL
                }
            })
            .collect();
        let col_w = fit_cols(&natural, avail, &mins);

        let border = Style::default().fg(RULE_COLOR);
        self.lines
            .push(make_table_border("┌", "┬", "┐", "─", &col_w, border));

        for (ri, row) in ctx.rows.iter().enumerate() {
            let wrapped: Vec<Vec<Vec<Span<'static>>>> = (0..cols)
                .map(|ci| {
                    let empty: Vec<Span<'static>> = vec![];
                    let cell = row.get(ci).unwrap_or(&empty);
                    wrap_spans(cell, col_w[ci])
                })
                .collect();
            let h = wrapped.iter().map(|c| c.len()).max().unwrap_or(1).max(1);

            for vi in 0..h {
                let mut spans: Vec<Span<'static>> =
                    vec![Span::styled("│".to_string(), border)];
                for ci in 0..cols {
                    let target = col_w[ci];
                    spans.push(Span::raw(" ".to_string()));
                    let empty_line: Vec<Span<'static>> = vec![];
                    let line = wrapped[ci].get(vi).unwrap_or(&empty_line);
                    let line_w = cell_width(line);
                    for s in line {
                        let mut s = s.clone();
                        if ri == 0 {
                            s.style = s.style.add_modifier(Modifier::BOLD);
                        }
                        spans.push(s);
                    }
                    let pad = target.saturating_sub(line_w) + 1;
                    spans.push(Span::raw(" ".repeat(pad)));
                    spans.push(Span::styled("│".to_string(), border));
                }
                self.lines.push(Line::from(spans));
            }

            if ri == 0 {
                self.lines
                    .push(make_table_border("╞", "╪", "╡", "═", &col_w, border));
            } else if ri + 1 < ctx.rows.len() {
                self.lines
                    .push(make_table_border("├", "┼", "┤", "─", &col_w, border));
            }
        }

        self.lines
            .push(make_table_border("└", "┴", "┘", "─", &col_w, border));
    }

    fn emit_blockquote(&mut self, start: usize) {
        if start >= self.lines.len() {
            return;
        }
        let inner: Vec<Line<'static>> = self.lines.drain(start..).collect();
        let total_w = self.width as usize;
        let avail = total_w.saturating_sub(QUOTE_PREFIX_W);
        let bg = Style::default().bg(QUOTE_BG);
        let bar_style = Style::default().fg(QUOTE_COLOR).bg(QUOTE_BG);
        let pad_row = || -> Line<'static> {
            Line::from(vec![
                Span::styled(QUOTE_PREFIX.to_string(), bar_style),
                Span::styled(" ".repeat(avail), bg),
            ])
        };

        let mut content_rows: Vec<Line<'static>> = vec![];
        let mut emitted_any = false;
        for line in &inner {
            let is_blank = line
                .spans
                .iter()
                .all(|s| s.content.trim().is_empty());
            if is_blank {
                if emitted_any {
                    content_rows.push(pad_row());
                }
                continue;
            }
            let visual = wrap_spans(&line.spans, avail);
            for v in visual {
                let content_w = cell_width(&v);
                let mut row: Vec<Span<'static>> = vec![Span::styled(
                    QUOTE_PREFIX.to_string(),
                    bar_style,
                )];
                for s in v {
                    let fg = s.style.fg.unwrap_or(QUOTE_FG);
                    row.push(Span::styled(
                        s.content.into_owned(),
                        s.style.fg(fg).bg(QUOTE_BG),
                    ));
                }
                let pad = avail.saturating_sub(content_w);
                if pad > 0 {
                    row.push(Span::styled(" ".repeat(pad), bg));
                }
                content_rows.push(Line::from(row));
                emitted_any = true;
            }
        }

        while content_rows
            .last()
            .map(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
            .unwrap_or(false)
        {
            content_rows.pop();
        }

        if !emitted_any {
            return;
        }
        self.lines.push(pad_row());
        self.lines.extend(content_rows);
    }
}

fn heading_style(level: HeadingLevel) -> Style {
    let base = Style::default().add_modifier(Modifier::BOLD);
    match level {
        HeadingLevel::H1 => base.fg(Color::Magenta),
        HeadingLevel::H2 => base.fg(Color::Cyan),
        HeadingLevel::H3 => base.fg(Color::Yellow),
        HeadingLevel::H4 => base.fg(Color::Green),
        HeadingLevel::H5 => base.fg(Color::Blue),
        HeadingLevel::H6 => base.fg(Color::Gray),
    }
}

fn syn_to_rt_style(s: SynStyle) -> Style {
    Style::default().fg(Color::Rgb(
        s.foreground.r,
        s.foreground.g,
        s.foreground.b,
    ))
}

fn cell_width(cell: &[Span<'static>]) -> usize {
    cell.iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

fn fit_cols(natural: &[usize], avail: usize, mins: &[usize]) -> Vec<usize> {
    let n = natural.len();
    if n == 0 {
        return vec![];
    }
    if natural.iter().sum::<usize>() <= avail {
        return natural.to_vec();
    }

    let mut alloc = vec![0usize; n];
    let mut locked = vec![false; n];
    let mut remaining = avail;
    let mut pending = n;

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| natural[i]);
    for &i in &order {
        if pending == 0 {
            break;
        }
        let fair = remaining / pending;
        if natural[i] <= fair {
            alloc[i] = natural[i].max(mins[i].min(fair));
            remaining = remaining.saturating_sub(alloc[i]);
            locked[i] = true;
            pending -= 1;
        }
    }

    if pending > 0 {
        let unlocked_natural_sum: usize = (0..n)
            .filter(|&i| !locked[i])
            .map(|i| natural[i])
            .sum();
        for i in 0..n {
            if locked[i] {
                continue;
            }
            let share = if unlocked_natural_sum > 0 {
                ((natural[i] as f64) * (remaining as f64) / (unlocked_natural_sum as f64))
                    .floor() as usize
            } else {
                remaining / pending
            };
            alloc[i] = share.max(mins[i].min(remaining));
        }
    }

    let total: usize = alloc.iter().sum();
    if total > avail {
        let mut over = total - avail;
        let mut idxs: Vec<usize> = (0..n).collect();
        idxs.sort_by_key(|&i| std::cmp::Reverse(alloc[i]));
        for i in idxs {
            if over == 0 {
                break;
            }
            let slack = alloc[i].saturating_sub(mins[i]);
            let take = slack.min(over);
            alloc[i] -= take;
            over -= take;
        }
    }

    alloc
}

fn first_word_width(cell: &[Span<'static>]) -> usize {
    let mut w = 0usize;
    for s in cell {
        for ch in s.content.chars() {
            if ch.is_whitespace() {
                if w > 0 {
                    return w;
                }
                continue;
            }
            w += UnicodeWidthStr::width(ch.to_string().as_str());
        }
    }
    w
}

fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![spans.to_vec()];
    }

    struct Tok {
        text: String,
        style: Style,
        is_ws: bool,
        w: usize,
    }
    let mut toks: Vec<Tok> = vec![];
    for s in spans {
        let mut buf = String::new();
        let mut buf_ws: Option<bool> = None;
        for ch in s.content.chars() {
            let is_ws = ch.is_whitespace();
            match buf_ws {
                Some(w) if w != is_ws => {
                    let cw = UnicodeWidthStr::width(buf.as_str());
                    toks.push(Tok {
                        text: std::mem::take(&mut buf),
                        style: s.style,
                        is_ws: w,
                        w: cw,
                    });
                    buf_ws = Some(is_ws);
                }
                None => buf_ws = Some(is_ws),
                _ => {}
            }
            buf.push(ch);
        }
        if let Some(w) = buf_ws {
            let cw = UnicodeWidthStr::width(buf.as_str());
            toks.push(Tok {
                text: buf,
                style: s.style,
                is_ws: w,
                w: cw,
            });
        }
    }

    let mut lines: Vec<Vec<Span<'static>>> = vec![];
    let mut cur: Vec<Span<'static>> = vec![];
    let mut cur_w = 0usize;
    for t in toks {
        if t.is_ws && cur_w == 0 {
            continue;
        }
        if t.is_ws {
            if cur_w + t.w > width {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
                continue;
            }
            cur.push(Span::styled(t.text, t.style));
            cur_w += t.w;
            continue;
        }
        if t.w > width {
            let mut buf = String::new();
            let mut bw = 0usize;
            for ch in t.text.chars() {
                let cw = UnicodeWidthStr::width(ch.to_string().as_str());
                if cur_w + bw + cw > width {
                    if !buf.is_empty() {
                        cur.push(Span::styled(std::mem::take(&mut buf), t.style));
                        cur_w += bw;
                        bw = 0;
                    }
                    if !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                }
                buf.push(ch);
                bw += cw;
            }
            if !buf.is_empty() {
                cur.push(Span::styled(buf, t.style));
                cur_w += bw;
            }
            continue;
        }
        if cur_w + t.w > width && cur_w > 0 {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(Span::styled(t.text, t.style));
        cur_w += t.w;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(vec![]);
    }
    lines
}

fn make_table_border(
    left: &str,
    mid: &str,
    right: &str,
    fill: &str,
    col_w: &[usize],
    style: Style,
) -> Line<'static> {
    let mut s = String::from(left);
    for (i, w) in col_w.iter().enumerate() {
        for _ in 0..w + 2 {
            s.push_str(fill);
        }
        if i + 1 < col_w.len() {
            s.push_str(mid);
        }
    }
    s.push_str(right);
    Line::from(Span::styled(s, style))
}
