//! The settings view: every row of `pg_settings`, under its category, with
//! the details of the selected setting beside the list.
//!
//! One `SELECT` runs when the view opens and again on `r`, never per
//! refresh. The view shows what differs from the built-in default and where
//! each value comes from. It does not say what a value should be.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use tui_kiso::{fit, Card, Columns, Found, Page, Scroll, Search, Theme};

use super::{fact_lines, fit_page, put, split_cells, text_of, PAGE, SOURCE};
use crate::postgres::{AdapterError, ConnectedClient};

/// `pg_settings.pending_restart` arrived in PostgreSQL 9.5.
const PENDING_RESTART_MIN_VERSION: i32 = 90500;

/// The narrowest screen that keeps the details beside the list. Under it
/// they go below the list.
const SIDE_MIN_WIDTH: u16 = 100;

/// The hints of the filters while they are on. See `fill_on`.
const CHANGED_ON: (&str, &str) = (" d", "changed ");
const PENDING_ON: (&str, &str) = (" p", "pending ");

/// The narrowest list that has the default beside the value. Under it the
/// details alone show it.
const DEFAULT_MIN_WIDTH: usize = 48;

pub(super) const fn has_pending_restart(server_version_num: i32) -> bool {
    server_version_num >= PENDING_RESTART_MIN_VERSION
}

/// Builds the `SELECT` of the view. It reads `reset_val`, the value a new
/// session gets, because `setting` holds what this session changed for
/// itself. The columns exist since 9.3, except `pending_restart`.
fn sql(server_version_num: i32) -> String {
    let pending = if has_pending_restart(server_version_num) {
        "pending_restart"
    } else {
        "false"
    };
    format!(
        "\
SELECT name, reset_val, unit, category, short_desc, context, source, boot_val, {pending}, \
sourcefile, sourceline \
FROM pg_settings \
ORDER BY category, name"
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Guc {
    name: String,
    /// `reset_val`, in `unit`.
    value: String,
    unit: String,
    category: String,
    description: String,
    context: String,
    source: String,
    /// `boot_val`, the value built into the server, in `unit`.
    default: String,
    pending_restart: bool,
    /// The file and the line that set the value. Empty when no file set
    /// it, and when the role may not see it.
    file: String,
    line: String,
}

impl Guc {
    /// The value differs from the built-in default, and something set it:
    /// a file, a command line option, `ALTER SYSTEM`, a database or a role.
    /// A value the server works out by itself (`default`, `override`) or
    /// that this connection asked for (`client`) does not count.
    fn changed(&self) -> bool {
        self.value != self.default
            && !matches!(self.source.as_str(), "default" | "override" | "client")
    }

    /// How many times the value is more or less than the default, when
    /// both are positive amounts: `4× more`. Both are in the same unit.
    /// Only for a changed setting, so it agrees with the mark and the filter.
    fn ratio(&self) -> Option<String> {
        if !self.changed() {
            return None;
        }
        let value = self.value.parse::<f64>().ok()?;
        let default = self.default.parse::<f64>().ok()?;
        if !(value > 0.0 && default > 0.0 && value.is_finite() && default.is_finite())
            || value == default
        {
            return None;
        }
        let (times, way) = if value > default {
            (value / default, "more")
        } else {
            (default / value, "less")
        };
        let times = if times >= 10.0 {
            format!("{times:.0}")
        } else {
            format!("{times:.1}").trim_end_matches(".0").to_owned()
        };
        Some(format!("{times}× {way}"))
    }

    /// When a new value takes effect, from `pg_settings.context`.
    fn applies(&self) -> &'static str {
        match self.context.as_str() {
            "postmaster" => "on restart",
            "sighup" => "on reload",
            "superuser" | "user" => "in the session",
            "backend" | "superuser-backend" => "in new sessions",
            "internal" => "never, it is fixed",
            _ => "—",
        }
    }
}

fn fetch(client: &ConnectedClient, server_version_num: i32) -> Result<Vec<Guc>, AdapterError> {
    let rows = client.exec_bounded_rows(&sql(server_version_num), SOURCE, &mut Vec::new())?;
    Ok(rows
        .rows
        .iter()
        .map(|row| {
            let text = |index: usize| {
                row.get(index)
                    .map(|value| text_of(value))
                    .unwrap_or_default()
            };
            Guc {
                name: text(0),
                value: text(1),
                unit: text(2),
                category: text(3),
                description: text(4),
                context: text(5),
                source: text(6),
                default: text(7),
                pending_restart: text(8) == "t",
                file: text(9),
                line: text(10),
            }
        })
        .collect())
}

/// Memory and time units, each with its size in the smallest one.
const MEMORY: [(&str, i128); 5] = [
    ("B", 1),
    ("kB", 1 << 10),
    ("MB", 1 << 20),
    ("GB", 1 << 30),
    ("TB", 1 << 40),
];
const TIME: [(&str, i128); 6] = [
    ("us", 1),
    ("ms", 1_000),
    ("s", 1_000_000),
    ("min", 60_000_000),
    ("h", 3_600_000_000),
    ("d", 86_400_000_000),
];

/// A value of `pg_settings` with its unit, as `SHOW` prints it: in the
/// largest unit that keeps it a whole number, such as `128MB` for `16384`
/// in `8kB`. Zero and negative values are switches, so they stay as they
/// are. The names are PostgreSQL's, where `MB` is 1024 kB.
fn format_value(value: &str, unit: &str) -> String {
    if unit.is_empty() || value.is_empty() {
        return value.to_owned();
    }
    let digits = unit.len() - unit.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let (factor, base) = unit.split_at(digits);
    let raw = || {
        if factor.is_empty() {
            format!("{value}{unit}")
        } else {
            format!("{value} × {unit}")
        }
    };
    let Ok(number) = value.parse::<i128>() else {
        return raw();
    };
    if number <= 0 {
        return value.to_owned();
    }
    let factor = if factor.is_empty() {
        1
    } else {
        match factor.parse::<i128>() {
            Ok(factor) => factor,
            Err(_) => return raw(),
        }
    };
    for units in [&MEMORY[..], &TIME[..]] {
        let Some((_, size)) = units.iter().find(|(name, _)| *name == base) else {
            continue;
        };
        let total = number * factor * size;
        // The smallest unit is 1, so one always divides the total.
        if let Some((name, size)) = units.iter().rev().find(|(_, size)| total % size == 0) {
            return format!("{}{name}", total / size);
        }
    }
    raw()
}

/// What a key did in the view.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Pressed {
    Handled,
    /// `r`: read the settings again.
    Reload,
    /// A key of every view, such as `q` or `1`.
    Pass,
}

#[derive(Default)]
pub(super) struct GucView {
    /// `None` while the settings are being read.
    result: Option<Result<Vec<Guc>, String>>,
    pub(super) search: Search,
    changed_only: bool,
    pending_only: bool,
    selected: usize,
    scroll: Scroll,
}

impl GucView {
    /// Shows the loading state until `load` returns.
    pub(super) fn loading(&mut self) {
        self.result = None;
    }

    /// Reads the settings. A failure shows in the view, never ends `top`.
    pub(super) fn load(&mut self, client: &ConnectedClient, server_version_num: i32) {
        self.result = Some(fetch(client, server_version_num).map_err(|_| {
            "Could not read pg_settings. The query failed or passed the statement timeout."
                .to_owned()
        }));
        self.move_by(0);
    }

    fn gucs(&self) -> &[Guc] {
        match &self.result {
            Some(Ok(gucs)) => gucs,
            _ => &[],
        }
    }

    fn filtered(&self) -> bool {
        !self.search.query.is_empty() || self.changed_only || self.pending_only
    }

    /// The settings the search and the filters keep.
    fn shown(&self) -> Vec<&Guc> {
        self.gucs()
            .iter()
            .filter(|guc| !self.changed_only || guc.changed())
            .filter(|guc| !self.pending_only || guc.pending_restart)
            .filter(|guc| self.search.hit(&[&guc.name, &guc.category]))
            .collect()
    }

    /// The count of the view's tab: every setting, or "3 of 5" while
    /// something filters. `None` until the settings are read.
    pub(super) fn count(&self) -> Option<String> {
        let total = self.gucs().len();
        match &self.result {
            Some(Ok(_)) if self.filtered() => Some(format!("{} of {total}", self.shown().len())),
            Some(Ok(_)) => Some(total.to_string()),
            _ => None,
        }
    }

    /// Shows only the settings with `name` in their name, as a search for
    /// it does. The filters go off, since they may hide the setting.
    pub(super) fn find(&mut self, name: &str) {
        self.changed_only = false;
        self.pending_only = false;
        self.search.clear();
        self.search.query = name.to_owned();
        self.first();
    }

    fn first(&mut self) {
        self.selected = 0;
        self.scroll.top();
    }

    fn move_by(&mut self, delta: isize) {
        let last = self.shown().len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.scroll.follow();
    }

    pub(super) fn key(&mut self, key: KeyEvent, pending: bool) -> Pressed {
        // While the person types, every key is text, `q` too.
        if self.search.typing {
            match self.search.key(key) {
                Found::Changed => self.first(),
                Found::Up => self.move_by(-1),
                Found::Down => self.move_by(1),
                Found::Ignored => {}
            }
            return Pressed::Handled;
        }
        match key.code {
            KeyCode::Char('/') => self.search.start(),
            KeyCode::Char('d') => {
                self.changed_only = !self.changed_only;
                self.first();
            }
            KeyCode::Char('p') if pending => {
                self.pending_only = !self.pending_only;
                self.first();
            }
            KeyCode::Char('r') => return Pressed::Reload,
            // Esc clears the search first, then the filters.
            KeyCode::Esc => {
                if self.search.is_active() {
                    self.search.clear();
                } else {
                    self.changed_only = false;
                    self.pending_only = false;
                }
                self.first();
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::PageDown => self.move_by(PAGE as isize),
            KeyCode::PageUp => self.move_by(-(PAGE as isize)),
            KeyCode::Home => self.first(),
            KeyCode::End => self.move_by(isize::MAX / 2),
            KeyCode::Char('q' | ' ' | '?' | 'h' | '1'..='6') | KeyCode::F(_) => {
                return Pressed::Pass
            }
            // The keys of the session lists do nothing here.
            _ => {}
        }
        Pressed::Handled
    }

    /// The footer's keys, and whether something filters, so esc clears it.
    pub(super) fn hints(&self, pending: bool) -> Vec<(&'static str, &'static str)> {
        if self.search.typing {
            return vec![("enter", "keep"), ("esc", "clear")];
        }
        let mut hints = vec![
            ("1-6", "view"),
            ("↑↓", "move"),
            ("/", "search"),
            if self.changed_only {
                CHANGED_ON
            } else {
                ("d", "changed")
            },
        ];
        if pending {
            hints.push(if self.pending_only {
                PENDING_ON
            } else {
                ("p", "pending")
            });
        }
        hints.push(("r", "reload"));
        if self.filtered() {
            hints.push(("esc", "clear"));
        }
        hints.extend([("?", "help"), ("q", "quit")]);
        hints
    }

    /// How many settings the search keeps, for the footer's prompt.
    pub(super) fn matches(&self) -> usize {
        self.shown().len()
    }

    /// The column titles on the first row of `area`, then the list, with
    /// the details of the selected setting beside it or, on a narrow
    /// screen, under it.
    pub(super) fn draw(&self, t: &Theme, buf: &mut Buffer, area: Rect, pending: bool) {
        let message = |buf: &mut Buffer, lines: Vec<Line<'static>>| {
            let top = area.y + area.height.saturating_sub(lines.len() as u16) / 2;
            for (index, line) in lines.into_iter().enumerate() {
                let row = Rect {
                    y: top + index as u16,
                    height: 1,
                    ..area
                }
                .intersection(area);
                line.centered().render(row, buf);
            }
        };
        let gucs = match &self.result {
            None => return message(buf, vec![Line::styled("Reading the settings…", t.muted())]),
            Some(Err(reason)) => {
                let mut lines: Vec<Line> = split_cells(reason, usize::from(area.width))
                    .into_iter()
                    .map(|line| Line::styled(line, t.danger()))
                    .collect();
                lines.push(Line::styled("Press r to read them again.", t.muted()));
                return message(buf, lines);
            }
            Some(Ok(gucs)) => gucs,
        };
        let shown = self.shown();
        let selected = self.selected.min(shown.len().saturating_sub(1));
        let body = Rect {
            y: area.y.saturating_add(1).min(area.bottom()),
            height: area.height.saturating_sub(1),
            ..area
        };
        // Beside the list, the details are a card. The scrollbar of the list
        // takes the column after it, then one is free, and the card's rail
        // takes the one before the card.
        let mut beside = false;
        let (list, pane) = match shown.get(selected) {
            None => (body, None),
            Some(guc) if area.width >= SIDE_MIN_WIDTH => {
                let width = (body.width * 2 / 5).min(56);
                let list = Rect {
                    width: body.width - width - 4,
                    ..body
                };
                beside = true;
                let card = Card::new(t);
                let surface = Rect {
                    x: list.right() + 4,
                    width,
                    ..area
                };
                let lines = self.details(
                    t,
                    guc,
                    usize::from(card.inner(surface).width),
                    pending,
                    true,
                );
                card.render(surface, buf);
                (list, Some((Card::new(t).inner(surface), lines)))
            }
            Some(guc) => {
                let mut lines = vec![Line::default()];
                lines.extend(self.details(t, guc, usize::from(body.width), pending, false));
                // The list keeps at least two fifths of the rows.
                let height = (lines.len() as u16).min(body.height * 3 / 5);
                let list = Rect {
                    height: body.height - height,
                    ..body
                };
                let pane = Rect {
                    y: list.bottom(),
                    height,
                    ..body
                };
                (list, Some((pane, lines)))
            }
        };
        if let Some((pane, lines)) = pane {
            for (index, line) in lines.into_iter().enumerate() {
                put(
                    buf,
                    pane,
                    index as u16,
                    Line::from(fit(line.spans, usize::from(pane.width))),
                );
            }
        }
        let build = |width| self.page(t, gucs, &shown, width, pending);
        // Beside the details, the list keeps its width and the scrollbar
        // goes in the column after it.
        let (rows, (head, page)) = if beside {
            (list, build(usize::from(list.width)))
        } else {
            fit_page(list, build, |(_, page)| page.len())
        };
        put(
            buf,
            Rect {
                width: rows.width,
                ..area
            },
            0,
            head,
        );
        // The rows on screen change with the screen size, so every draw
        // scrolls the selection back into view.
        self.scroll.follow();
        let view = page
            .view(t, &self.scroll)
            .selected((!shown.is_empty()).then_some(selected));
        view.render(rows, buf);
    }

    /// The column titles and the settings, each category a section. A
    /// category with no setting left has no section.
    fn page(
        &self,
        t: &Theme,
        gucs: &[Guc],
        shown: &[&Guc],
        width: usize,
        pending: bool,
    ) -> (Line<'static>, Page) {
        let default = width >= DEFAULT_MIN_WIDTH;
        let value = if default {
            (width / 5).clamp(12, 20)
        } else {
            (width / 3).clamp(10, 28)
        };
        let mut spec = vec![1, 0, value];
        let mut titles = vec!["", "Name", "Value"];
        if default {
            spec.push(value);
            titles.push("Default");
        }
        let columns = Columns::with_gap(width, &spec, 1);
        let head = columns.head(t, &titles);
        let mut page = Page::default();
        if shown.is_empty() {
            page.line(if self.search.query.is_empty() {
                Line::styled("  No setting matches the filters.", t.muted())
            } else {
                self.search.nothing(t)
            });
            return (head, page);
        }
        // The query sorts by category, so a category's rows are together.
        for (index, group) in shown.chunk_by(|a, b| a.category == b.category).enumerate() {
            let category = &group[0].category;
            let total = gucs.iter().filter(|guc| &guc.category == category).count();
            let meta = if self.filtered() {
                format!("{} of {total}", group.len())
            } else {
                total.to_string()
            };
            if index > 0 {
                page.blank();
            }
            // A long category ends in an ellipsis, so the count stays.
            let room = width.saturating_sub(meta.chars().count() + 3);
            let title: String = if category.chars().count() > room {
                fit(vec![Span::raw(category.clone())], room)
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            } else {
                category.clone()
            };
            page.section_meta(t, width, &title, &meta);
            for guc in group {
                let changed = guc.changed();
                let mark = if pending && guc.pending_restart {
                    "!"
                } else if changed {
                    "●"
                } else {
                    ""
                };
                let name = self
                    .search
                    .mark(t, Line::styled(guc.name.clone(), t.text()));
                // An empty value as PostgreSQL writes it.
                let shown = |raw: &str| {
                    if raw.is_empty() {
                        "''".to_owned()
                    } else {
                        format_value(raw, &guc.unit)
                    }
                };
                let value = Span::styled(
                    shown(&guc.value),
                    if changed { t.strong() } else { t.text() },
                );
                let mut cells = vec![
                    vec![Span::styled(mark, t.strong())],
                    name.spans,
                    vec![value],
                ];
                // The default muted and the value strong, as a value
                // before and after a change.
                if default {
                    cells.push(vec![Span::styled(shown(&guc.default), t.muted())]);
                }
                page.item([columns.row(cells)]);
            }
        }
        (head, page)
    }

    /// Everything the view knows about one setting. `spaced` puts a blank
    /// row around the description, where the rows allow it.
    fn details(
        &self,
        t: &Theme,
        guc: &Guc,
        width: usize,
        pending: bool,
        spaced: bool,
    ) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(Span::styled(guc.name.clone(), t.strong()))];
        if spaced {
            lines.push(Line::default());
        }
        for row in tui_kiso::wrap(&guc.description, width.max(1)) {
            lines.push(Line::from(Span::styled(row, t.muted())));
        }
        if spaced {
            lines.push(Line::default());
        }
        let mut fact = |key: &str, value: Vec<Span<'static>>| {
            lines.extend(fact_lines(t, width, 0, 9, key, value));
        };
        let amount = |raw: &str, style| {
            if raw.is_empty() {
                return vec![Span::styled("(empty)", t.muted())];
            }
            let shown = format_value(raw, &guc.unit);
            let mut spans = vec![Span::styled(shown.clone(), style)];
            // The server's own figure, when the view converted it.
            let unit = &guc.unit;
            let stored = if unit.starts_with(|c: char| c.is_ascii_digit()) {
                format!("{raw} × {unit}")
            } else {
                format!("{raw}{unit}")
            };
            if stored != shown && !unit.is_empty() && shown != raw {
                spans.push(Span::styled(format!("  ({stored})"), t.muted()));
            }
            spans
        };
        fact("Value", amount(&guc.value, t.strong()));
        fact("Default", amount(&guc.default, t.text()));
        if let Some(ratio) = guc.ratio() {
            fact("Change", vec![Span::styled(ratio, t.text())]);
        }
        let mut source = vec![Span::styled(guc.source.clone(), t.text())];
        if guc.source == "client" {
            source.push(Span::styled("  (this connection asked for it)", t.muted()));
        }
        fact("Source", source);
        if !guc.file.is_empty() {
            let place = if guc.line.is_empty() {
                guc.file.clone()
            } else {
                format!("{}:{}", guc.file, guc.line)
            };
            fact("File", vec![Span::styled(place, t.text())]);
        } else if guc.source == "configuration file" {
            // PostgreSQL shows the file to a superuser and, from version
            // 10, to a member of pg_read_all_settings.
            fact(
                "File",
                vec![Span::styled(
                    "not visible to this role",
                    t.muted().add_modifier(Modifier::ITALIC),
                )],
            );
        }
        fact("Context", vec![Span::styled(guc.context.clone(), t.text())]);
        fact(
            "Applies",
            vec![Span::styled(guc.applies().to_owned(), t.text())],
        );
        let restart = if !pending {
            Span::styled(
                "not reported before PostgreSQL 9.5",
                t.muted().add_modifier(Modifier::ITALIC),
            )
        } else if guc.pending_restart {
            Span::styled("waits for a restart", t.strong())
        } else {
            Span::styled("no", t.text())
        };
        fact("Pending", vec![restart]);
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guc(name: &str, category: &str, value: &str, default: &str) -> Guc {
        Guc {
            name: name.to_owned(),
            value: value.to_owned(),
            category: category.to_owned(),
            description: format!("Sets {name}."),
            context: "sighup".to_owned(),
            source: if value == default {
                "default"
            } else {
                "configuration file"
            }
            .to_owned(),
            default: default.to_owned(),
            ..Guc::default()
        }
    }

    fn view(gucs: Vec<Guc>) -> GucView {
        GucView {
            result: Some(Ok(gucs)),
            ..GucView::default()
        }
    }

    fn failed(reason: &str) -> GucView {
        GucView {
            result: Some(Err(reason.to_owned())),
            ..GucView::default()
        }
    }

    fn press(view: &mut GucView, code: KeyCode) -> Pressed {
        view.key(KeyEvent::from(code), true)
    }

    #[test]
    fn values_take_the_largest_whole_unit() {
        assert_eq!(format_value("16384", "8kB"), "128MB");
        assert_eq!(format_value("4096", "kB"), "4MB");
        assert_eq!(format_value("1048576", "kB"), "1GB");
        assert_eq!(format_value("1000", "kB"), "1000kB");
        assert_eq!(format_value("64", "16MB"), "1GB");
        assert_eq!(format_value("300", "s"), "5min");
        assert_eq!(format_value("90", "s"), "90s");
        assert_eq!(format_value("60000", "ms"), "1min");
        assert_eq!(format_value("1440", "min"), "1d");
        assert_eq!(format_value("200", "ms"), "200ms");
    }

    #[test]
    fn a_narrow_list_drops_the_default() {
        let mut long = sample();
        if let Some(Ok(gucs)) = &mut long.result {
            gucs[0].category =
                "Client Connection Defaults / Locale and Formatting of Every Session".to_owned();
        }
        let rows = screen(&long, 100, 12, true);
        // The list ends where the top half row of the card starts.
        let edge = rows[0].chars().position(|cell| cell == '▄').unwrap();
        let list = |row: &str| row.chars().take(edge).collect::<String>();
        let head = list(&rows[0]);
        assert!(
            head.contains("Default") && !head.contains("Change"),
            "{head}"
        );
        let section = list(&rows[1]);
        assert!(
            section.contains('…') && section.trim_end().ends_with(" 1"),
            "{section}"
        );
        let head = screen(&sample(), 44, 19, true).remove(0);
        assert!(
            head.contains("Value") && !head.contains("Default"),
            "{head}"
        );
    }

    #[test]
    fn the_ratio_compares_positive_amounts_only() {
        let ratio = |value, default| guc("x", "c", value, default).ratio();
        assert_eq!(ratio("32768", "16384").as_deref(), Some("2× more"));
        assert_eq!(ratio("6", "3").as_deref(), Some("2× more"));
        assert_eq!(ratio("10", "20").as_deref(), Some("2× less"));
        assert_eq!(ratio("0.05", "0.2").as_deref(), Some("4× less"));
        assert_eq!(ratio("150", "100").as_deref(), Some("1.5× more"));
        assert_eq!(ratio("1600", "100").as_deref(), Some("16× more"));
        assert_eq!(ratio("100", "100"), None);
        assert_eq!(ratio("60000", "-1"), None);
        assert_eq!(ratio("30", "0"), None);
        assert_eq!(ratio("on", "off"), None);
        let mut computed = guc("max_stack_depth", "c", "2048", "100");
        computed.source = "default".to_owned();
        assert_eq!(computed.ratio(), None);
    }

    #[test]
    fn switches_reals_and_unknown_units_stay_as_they_are() {
        assert_eq!(format_value("-1", "kB"), "-1");
        assert_eq!(format_value("0", "ms"), "0");
        assert_eq!(format_value("on", ""), "on");
        assert_eq!(format_value("", "kB"), "");
        assert_eq!(format_value("0.5", "ms"), "0.5ms");
        assert_eq!(format_value("3", "xyz"), "3xyz");
    }

    #[test]
    fn changed_needs_another_value_and_someone_who_set_it() {
        let mut work_mem = guc("work_mem", "Memory", "65536", "4096");
        assert!(work_mem.changed());
        // The same value written in a file is not a change.
        work_mem.value = "4096".to_owned();
        assert!(!work_mem.changed());
        // The server and this connection set these, not the operator.
        for source in ["default", "override", "client"] {
            let mut own = guc("statement_timeout", "Client", "5000", "0");
            own.source = source.to_owned();
            assert!(!own.changed(), "{source}");
        }
    }

    #[test]
    fn pending_restart_is_read_from_9_5_on() {
        assert!(sql(90500).contains(", pending_restart, sourcefile"));
        assert!(sql(180000).contains(", pending_restart, sourcefile"));
        assert!(sql(90325).contains(", false, sourcefile"));
        assert!(sql(90400).contains(", false, sourcefile"));
    }

    #[test]
    fn the_search_and_the_filters_narrow_the_list() {
        let mut pending = guc("max_connections", "Connections", "200", "100");
        pending.pending_restart = true;
        let mut view = view(vec![
            pending,
            guc("port", "Connections", "5432", "5432"),
            guc("work_mem", "Memory", "65536", "4096"),
        ]);
        assert_eq!(view.count().as_deref(), Some("3"));
        press(&mut view, KeyCode::Char('d'));
        assert_eq!(view.count().as_deref(), Some("2 of 3"));
        press(&mut view, KeyCode::Char('p'));
        assert_eq!(view.shown()[0].name, "max_connections");
        assert_eq!(view.shown().len(), 1);
        // Esc clears the search first, then the filters.
        press(&mut view, KeyCode::Char('/'));
        for letter in "MEM".chars() {
            press(&mut view, KeyCode::Char(letter));
        }
        assert!(view.shown().is_empty());
        press(&mut view, KeyCode::Enter);
        press(&mut view, KeyCode::Esc);
        assert_eq!(view.shown().len(), 1);
        press(&mut view, KeyCode::Esc);
        assert_eq!(view.count().as_deref(), Some("3"));
        // The search also reads the category.
        press(&mut view, KeyCode::Char('/'));
        for letter in "memory".chars() {
            press(&mut view, KeyCode::Char(letter));
        }
        assert_eq!(view.shown()[0].name, "work_mem");
    }

    #[test]
    fn while_typing_q_is_text_and_the_view_keys_wait() {
        let mut view = view(vec![guc("quote_all_identifiers", "Compat", "off", "off")]);
        assert_eq!(press(&mut view, KeyCode::Char('q')), Pressed::Pass);
        press(&mut view, KeyCode::Char('/'));
        assert_eq!(press(&mut view, KeyCode::Char('q')), Pressed::Handled);
        assert_eq!(press(&mut view, KeyCode::Char('1')), Pressed::Handled);
        assert_eq!(view.search.query, "q1");
        press(&mut view, KeyCode::Enter);
        assert_eq!(press(&mut view, KeyCode::Char('r')), Pressed::Reload);
    }

    #[test]
    fn p_does_nothing_where_the_server_does_not_report_it() {
        let mut view = view(vec![guc("port", "Connections", "5432", "5432")]);
        view.key(KeyEvent::from(KeyCode::Char('p')), false);
        assert_eq!(view.count().as_deref(), Some("1"));
        assert!(!view.hints(false).contains(&("p", "pending")));
        assert!(view.hints(true).contains(&("p", "pending")));
        press(&mut view, KeyCode::Char('d'));
        press(&mut view, KeyCode::Char('p'));
        let hints = view.hints(true);
        assert!(hints.contains(&CHANGED_ON) && hints.contains(&PENDING_ON));
        let t = super::super::theme();
        let mut spans = tui_kiso::KeyHints::new(&t, hints.clone()).spans(120);
        super::super::fill_on(&t, &mut spans, &hints);
        let filled: String = spans
            .iter()
            .filter(|span| span.style == t.brand())
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(filled, " d changed  p pending ");
    }

    fn screen(view: &GucView, width: u16, height: u16, pending: bool) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.draw(&super::super::theme(), &mut buf, area, pending);
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    fn sample() -> GucView {
        let mut buffers = guc("shared_buffers", "Memory", "32768", "16384");
        buffers.unit = "8kB".to_owned();
        buffers.context = "postmaster".to_owned();
        buffers.pending_restart = true;
        buffers.file = "/etc/pg/postgresql.conf".to_owned();
        buffers.line = "42".to_owned();
        view(vec![
            guc("port", "Connections", "5432", "5432"),
            buffers,
            guc("work_mem", "Memory", "65536", "4096"),
        ])
    }

    #[test]
    fn a_wide_screen_has_the_details_beside_the_list() {
        let mut view = sample();
        press(&mut view, KeyCode::Down);
        let rows = screen(&view, 120, 14, true);
        let text = rows.join("\n");
        assert!(
            rows[0].contains("Name") && rows[1].contains(" 1      shared_buffers"),
            "{text}"
        );
        assert!(rows[1].starts_with("Connections ─"), "{text}");
        assert!(rows[3].starts_with("    port "), "{text}");
        assert!(
            rows[5].starts_with("Memory ─") && rows[5].contains(" 2      Value"),
            "{text}"
        );
        assert!(rows[7].starts_with("▌ ! shared_buffers "), "{text}");
        assert!(rows[8].starts_with("  ● work_mem "), "{text}");
        assert!(text.contains("Value    256MB  (32768 × 8kB)"), "{text}");
        assert!(text.contains("Change   2× more"), "{text}");
        assert!(
            text.contains("File     /etc/pg/postgresql.conf:42"),
            "{text}"
        );
        assert!(
            rows[0].contains("Value") && rows[0].contains("Default"),
            "{text}"
        );
        let edge = rows[0].chars().position(|cell| cell == '▄').unwrap();
        let list: String = rows[8].chars().take(edge).collect();
        let cells: Vec<&str> = list.split_whitespace().collect();
        assert_eq!(cells, ["●", "work_mem", "65536", "4096"], "{text}");
        assert!(text.contains("Default  128MB  (16384 × 8kB)"), "{text}");
        assert!(text.contains("Source   configuration file"), "{text}");
        assert!(text.contains("Applies  on restart"), "{text}");
        assert!(text.contains("Pending  waits for a restart"), "{text}");
        // A file set it, but the role may not see which.
        press(&mut view, KeyCode::Down);
        let text = screen(&view, 120, 16, true).join("\n");
        assert!(text.contains("Change   16× more"), "{text}");
        assert!(text.contains("File     not visible to this role"), "{text}");
    }

    #[test]
    fn a_narrow_screen_has_the_details_under_the_list() {
        let rows = screen(&sample(), 80, 19, true);
        let text = rows.join("\n");
        assert!(rows.iter().all(|row| row.chars().count() == 80), "{text}");
        assert!(!rows[0].contains('▄'), "{text}");
        assert!(rows[3].starts_with("▌   port "), "{text}");
        assert!(text.contains("\nport "), "{text}");
        assert!(text.contains("\nPending  no"), "{text}");
    }

    #[test]
    fn a_server_without_pending_restart_says_so_and_has_no_mark() {
        let rows = screen(&sample(), 120, 14, false);
        let text = rows.join("\n");
        assert!(text.contains("  ● shared_buffers "), "{text}");
        assert!(!text.contains("! shared_buffers"), "{text}");
        let mut view = sample();
        press(&mut view, KeyCode::Down);
        let text = screen(&view, 120, 14, false).join("\n");
        assert!(
            text.contains("Pending  not reported before PostgreSQL 9.5"),
            "{text}"
        );
    }

    #[test]
    fn a_filter_hides_the_categories_with_nothing_left() {
        let mut view = sample();
        press(&mut view, KeyCode::Char('d'));
        let text = screen(&view, 120, 12, true).join("\n");
        assert!(!text.contains("Connections"), "{text}");
        assert!(text.contains(" 2 of 2 "), "{text}");
        press(&mut view, KeyCode::Char('/'));
        press(&mut view, KeyCode::Char('x'));
        let text = screen(&view, 120, 12, true).join("\n");
        assert!(text.contains("Nothing matches \"x\"."), "{text}");
    }

    #[test]
    fn loading_and_a_failed_read_show_in_the_view() {
        let text = screen(&GucView::default(), 80, 9, true).join("\n");
        assert!(text.contains("Reading the settings…"), "{text}");
        let text = screen(&failed("Could not read pg_settings."), 80, 9, true).join("\n");
        assert!(text.contains("Could not read pg_settings."), "{text}");
        assert!(text.contains("Press r to read them again."), "{text}");
    }
}
