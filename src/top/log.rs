//! The log view: the server's csvlog as it grows, like `tail -f`, with a
//! filter for errors, temp files and checkpoints and a summary of each
//! since `top` started.
//!
//! `top` reads the log file from the local disk, so the view works only on
//! the database host. Each refresh reads what the server appended, up to a
//! fixed amount. One `SELECT` on `pg_settings` runs when `top` starts and
//! when the view opens, to find the file and to know what the server logs.
//! The text of the log stays on the screen and is never written anywhere.
//! The view counts what the log holds. It does not say a count is too high.

mod reader;
mod sqlstate;
mod stats;

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use tui_kiso::{fit, Card, Columns, Found, Page, Scroll, Search, Theme};

use super::{
    code_block, fact_lines, fit_page, format_bytes, local_time, put, single_line,
    single_line_keep_indent, split_cells, text_of, PAGE, SOURCE,
};
use crate::postgres::{AdapterError, ConnectedClient};
use reader::{Problem, Tail};
use stats::{percentile, who, Event, Kind, Stats};

/// The narrowest screen that keeps the summary beside the log. Under it
/// the summary goes below the log.
const SIDE_MIN_WIDTH: u16 = 100;
/// The lines kept for the screen. The summaries count every line.
const EVENTS_MAX: usize = 1000;
/// The groups a summary shows.
const TOP: usize = 10;

/// The hints of the switches that are on: a space before the key and
/// after the label, which the footer fills.
const ERRORS_ON: (&str, &str) = (" e", "errors ");
const TEMP_ON: (&str, &str) = (" t", "temp files ");
const CHECKPOINTS_ON: (&str, &str) = (" c", "checkpoints ");
/// The narrowest the summary and the list get when `←` `→` move the rule
/// between them, and how far one press moves it.
const PANE_MIN_WIDTH: u16 = 30;
const LIST_MIN_WIDTH: u16 = 40;
const PANE_STEP: u16 = 8;
const DETAILS_ON: (&str, &str) = (" enter", "details ");

const SETTINGS_SQL: &str = "\
SELECT name, setting FROM pg_settings \
WHERE name IN ('logging_collector', 'log_destination', 'log_directory', 'data_directory', \
'lc_messages', 'log_temp_files', 'log_checkpoints')";

fn fetch_settings(client: &ConnectedClient) -> Result<HashMap<String, String>, AdapterError> {
    let rows = client.exec_bounded_rows(SETTINGS_SQL, SOURCE, &mut Vec::new())?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|row| Some((text_of(row.first()?), text_of(row.get(1)?))))
        .collect())
}

/// What the tail shows. `e`, `t` and `c` turn a filter on and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Errors,
    Temp,
    Checkpoints,
}

impl Filter {
    fn keeps(self, kind: Kind) -> bool {
        match self {
            Self::All => true,
            Self::Errors => kind == Kind::Error,
            Self::Temp => kind == Kind::Temp,
            Self::Checkpoints => kind == Kind::Checkpoint,
        }
    }
}

/// Why the log is not read.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Why {
    /// The server is on another host.
    Remote,
    /// The settings could not be read.
    Query,
    /// The role may not see where the log is.
    Hidden,
    /// A setting the view needs is off.
    Off { name: &'static str, value: String },
}

/// Where the csvlog files are, from the server's settings.
fn log_dir(settings: &HashMap<String, String>) -> Result<PathBuf, Why> {
    let get = |name: &str| settings.get(name).map(String::as_str);
    let off = |name: &'static str, value: &str| Why::Off {
        name,
        value: value.to_owned(),
    };
    match get("logging_collector") {
        Some("on") => {}
        Some(value) => return Err(off("logging_collector", value)),
        None => return Err(Why::Query),
    }
    let destination = get("log_destination").unwrap_or_default();
    if !destination.split(',').any(|part| part.trim() == "csvlog") {
        return Err(off("log_destination", destination));
    }
    // Only a superuser, or a member of pg_read_all_settings, sees these.
    let dir = PathBuf::from(get("log_directory").ok_or(Why::Hidden)?);
    if dir.is_absolute() {
        return Ok(dir);
    }
    Ok(PathBuf::from(get("data_directory").ok_or(Why::Hidden)?).join(dir))
}

/// True when the server writes its messages in English.
fn english(lc_messages: &str) -> bool {
    lc_messages.is_empty()
        || lc_messages.starts_with("en")
        || matches!(lc_messages.split('.').next(), Some("C" | "POSIX"))
}

/// What a key did in the view.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Pressed {
    Handled,
    /// `S`: open the settings view at this setting.
    Settings(&'static str),
    /// A key of every view, such as `q` or `1`.
    Pass,
}

pub(super) struct LogView {
    /// True when the server runs on this host.
    local: bool,
    /// When `top` started: the time of day, and the clock the last five
    /// minutes count on.
    started: String,
    start: Instant,
    why: Option<Why>,
    settings: HashMap<String, String>,
    tail: Option<Tail>,
    /// The last lines read, oldest first.
    events: VecDeque<Event>,
    next_id: u64,
    stats: Stats,
    filter: Filter,
    /// The selected line by its id, or `None` to follow the newest one.
    selected: Option<u64>,
    detail: bool,
    /// The width the person gave the summary beside the list, with `←`
    /// and `→`.
    pane: Option<u16>,
    /// The width of the summary on the last draw, or 0 when it was not
    /// beside the list.
    pane_shown: Cell<u16>,
    pub(super) search: Search,
    scroll: Scroll,
    detail_scroll: Scroll,
}

impl LogView {
    pub(super) fn new(local: bool) -> Self {
        Self {
            local,
            started: local_time().unwrap_or_else(|| "—".to_owned()),
            start: Instant::now(),
            why: Some(Why::Remote),
            settings: HashMap::new(),
            tail: None,
            events: VecDeque::new(),
            next_id: 0,
            stats: Stats::default(),
            filter: Filter::All,
            selected: None,
            detail: false,
            pane: None,
            pane_shown: Cell::new(0),
            search: Search::default(),
            scroll: Scroll::default(),
            detail_scroll: Scroll::default(),
        }
    }

    /// Reads the settings that say where the log is and what the server
    /// logs. A failure shows in the view, never ends `top`.
    pub(super) fn open(&mut self, client: &ConnectedClient) {
        if !self.local {
            return;
        }
        match fetch_settings(client) {
            Ok(settings) => self.apply(settings),
            Err(_) if self.tail.is_none() => self.why = Some(Why::Query),
            // The log is being read: keep the settings from before.
            Err(_) => {}
        }
    }

    fn apply(&mut self, settings: HashMap<String, String>) {
        self.settings = settings;
        match log_dir(&self.settings) {
            Ok(dir) => {
                self.why = None;
                if self.tail.as_ref().is_none_or(|tail| tail.dir() != dir) {
                    self.tail = Some(Tail::new(dir));
                }
            }
            Err(why) => {
                self.why = Some(why);
                self.tail = None;
            }
        }
    }

    /// Reads what the server appended to the log since the last refresh.
    pub(super) fn tick(&mut self) {
        let records = match &mut self.tail {
            Some(tail) => tail.tick(),
            None => return,
        };
        for fields in records {
            self.push(fields);
        }
    }

    fn push(&mut self, fields: Vec<String>) {
        let event = Event::new(self.next_id, fields, english(self.setting("lc_messages")));
        self.next_id += 1;
        self.stats.add(&event, self.slot());
        if self.events.len() == EVENTS_MAX {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// The ten seconds since `top` started that now is in.
    fn slot(&self) -> u64 {
        self.start.elapsed().as_secs() / 10
    }

    fn setting(&self, name: &str) -> &str {
        self.settings.get(name).map_or("", String::as_str)
    }

    /// The lines the filter and the search keep, oldest first.
    fn shown(&self) -> Vec<&Event> {
        self.events
            .iter()
            .filter(|event| self.filter.keeps(event.kind))
            .filter(|event| {
                self.search.hit(&[
                    &event.message,
                    &event.statement,
                    &event.user,
                    &event.database,
                    &event.code,
                    &event.pid,
                ])
            })
            .collect()
    }

    /// The selected line among `shown`: the newest one while following.
    fn position(&self, shown: &[&Event]) -> Option<usize> {
        let last = shown.len().checked_sub(1)?;
        Some(match self.selected {
            None => last,
            Some(id) => shown.partition_point(|event| event.id < id).min(last),
        })
    }

    /// Turns `filter` on, or off when it is the one that is on.
    fn toggle(&mut self, filter: Filter) {
        self.filter = if self.filter == filter {
            Filter::All
        } else {
            filter
        };
        self.follow();
    }

    /// Changes the width of the summary beside the list. The draw keeps
    /// the width inside what the screen has.
    fn resize(&mut self, to: fn(u16) -> u16) {
        let shown = self.pane_shown.get();
        if shown > 0 {
            self.pane = Some(to(shown));
        }
    }

    fn follow(&mut self) {
        self.selected = None;
        self.detail = false;
    }

    fn move_by(&mut self, delta: isize) {
        let shown = self.shown();
        if let Some(position) = self.position(&shown) {
            let next = position.saturating_add_signed(delta).min(shown.len() - 1);
            self.selected = Some(shown[next].id);
            self.detail_scroll.top();
        }
    }

    /// How many lines the search keeps, for the footer's prompt.
    pub(super) fn matches(&self) -> usize {
        self.shown().len()
    }

    /// The setting that is off and limits what the screen shows.
    fn missing(&self) -> Option<&'static str> {
        match (&self.why, self.filter) {
            (Some(Why::Off { name, .. }), _) => Some(name),
            (Some(_), _) => None,
            (None, Filter::Temp) if self.setting("log_temp_files") == "-1" => {
                Some("log_temp_files")
            }
            (None, Filter::Checkpoints) if self.setting("log_checkpoints") == "off" => {
                Some("log_checkpoints")
            }
            (None, _) => None,
        }
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Pressed {
        // While the person types, every key is text, `q` too.
        if self.search.typing {
            match self.search.key(key) {
                Found::Changed => self.follow(),
                Found::Up => self.move_by(-1),
                Found::Down => self.move_by(1),
                Found::Ignored => {}
            }
            return Pressed::Handled;
        }
        match key.code {
            KeyCode::Char('/') => self.search.start(),
            KeyCode::Char('e') => self.toggle(Filter::Errors),
            KeyCode::Char('t') => self.toggle(Filter::Temp),
            KeyCode::Char('c') => self.toggle(Filter::Checkpoints),
            // The arrow moves the rule between the list and the summary.
            KeyCode::Left => self.resize(|width| width.saturating_add(PANE_STEP)),
            KeyCode::Right => self.resize(|width| width.saturating_sub(PANE_STEP)),
            KeyCode::Enter | KeyCode::Char('v') => {
                // The details stay on one line while new ones arrive.
                self.move_by(0);
                self.detail = !self.detail && self.selected.is_some();
            }
            // Esc closes the details first, then clears the search, then
            // the filter.
            KeyCode::Esc if self.detail => self.detail = false,
            KeyCode::Esc if self.search.is_active() => {
                self.search.clear();
                self.follow();
            }
            KeyCode::Esc => self.toggle(self.filter),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::PageDown if self.detail => self.detail_scroll.by(self.detail_scroll.half()),
            KeyCode::PageUp if self.detail => self.detail_scroll.by(-self.detail_scroll.half()),
            KeyCode::PageDown => self.move_by(PAGE as isize),
            KeyCode::PageUp => self.move_by(-(PAGE as isize)),
            KeyCode::Home => self.move_by(isize::MIN / 2),
            KeyCode::End => self.follow(),
            KeyCode::Char('s' | 'S') => {
                if let Some(name) = self.missing() {
                    return Pressed::Settings(name);
                }
            }
            KeyCode::Char('q' | ' ' | '?' | 'h' | '1'..='6') | KeyCode::F(_) => {
                return Pressed::Pass
            }
            // The keys of the session lists do nothing here.
            _ => {}
        }
        Pressed::Handled
    }

    /// The footer's keys.
    pub(super) fn hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.search.typing {
            return vec![("enter", "keep"), ("esc", "clear")];
        }
        let mut hints = vec![("1-6", "view")];
        for (filter, off, on) in [
            (Filter::Errors, ("e", "errors"), ERRORS_ON),
            (Filter::Temp, ("t", "temp files"), TEMP_ON),
            (Filter::Checkpoints, ("c", "checkpoints"), CHECKPOINTS_ON),
        ] {
            hints.push(if self.filter == filter { on } else { off });
        }
        hints.push(("↑↓", "move"));
        if self.pane_shown.get() > 0 {
            hints.push(("←→", "resize"));
        }
        if self.detail {
            hints.extend([DETAILS_ON, ("esc", "close"), ("PgUp PgDn", "scroll")]);
        } else {
            hints.extend([("enter", "details"), ("/", "search"), ("end", "follow")]);
            if self.search.is_active() || self.filter != Filter::All {
                hints.push(("esc", "clear"));
            }
        }
        if self.missing().is_some() {
            hints.push(("S", "settings"));
        }
        hints.extend([("?", "help"), ("q", "quit")]);
        hints
    }

    /// The file and how far the reading is, on the first row of the view.
    fn source_line(&self, t: &Theme, shown: &[&Event]) -> Vec<Span<'static>> {
        let mut spans = vec![Span::styled("Log  ", t.muted())];
        let dot = || Span::styled(" · ", t.subtle());
        let Some(tail) = self.tail.as_ref().filter(|_| self.why.is_none()) else {
            spans.push(Span::styled("not read", t.muted()));
            return spans;
        };
        let Some(path) = tail.path() else {
            spans.push(Span::styled(tail.dir().display().to_string(), t.text()));
            spans.extend([dot(), Span::styled("not read", t.muted())]);
            return spans;
        };
        spans.push(Span::styled(path.display().to_string(), t.text()));
        spans.push(dot());
        match (self.selected, self.position(shown)) {
            (Some(_), Some(position)) => {
                let new = shown.len() - 1 - position;
                spans.push(Span::styled(
                    format!("paused · {} newer", group(new as u64)),
                    t.strong(),
                ));
            }
            _ => spans.push(Span::styled("following", t.muted())),
        }
        let mut rest = format!(" · {} read", format_bytes(tail.read as f64));
        if tail.behind > 0 {
            rest.push_str(&format!(" · {} behind", format_bytes(tail.behind as f64)));
        }
        if tail.parser.skipped > 0 {
            rest.push_str(&format!(" · {} skipped", group(tail.parser.skipped)));
        }
        spans.push(Span::styled(rest, t.muted()));
        spans
    }

    /// What the filter adds up to since `top` started.
    fn summary_line(&self, t: &Theme) -> Vec<Span<'static>> {
        if self.why.is_some() {
            return Vec::new();
        }
        let stats = &self.stats;
        let since = format!("since top started at {}", self.started);
        let recent = |count: u64| format!(" · {} in the last 5 min", group(count));
        let (count, rest) = match self.filter {
            Filter::All => {
                let mut rest = format!(" {} {since}", plural(stats.lines, "line"));
                for (filter, count, noun) in [
                    (Filter::Errors, stats.errors, "error"),
                    (Filter::Temp, stats.temp_files, "temp file"),
                    (
                        Filter::Checkpoints,
                        stats.checkpoints.completed,
                        "checkpoint",
                    ),
                ] {
                    // A source that is off has nothing to count.
                    if count > 0 || self.limit(filter).is_none() {
                        rest.push_str(&format!(" · {} {}", group(count), plural(count, noun)));
                    }
                }
                (stats.lines, rest)
            }
            Filter::Errors => (
                stats.errors,
                format!(
                    " {} {since} · {} distinct{}",
                    plural(stats.errors, "error"),
                    group(stats.error_groups.len() as u64),
                    recent(stats.errors_recent.sum(self.slot()))
                ),
            ),
            Filter::Temp => {
                let mut rest = format!(
                    " {} {since} · {} written",
                    plural(stats.temp_files, "temp file"),
                    format_bytes(stats.temp_bytes as f64)
                );
                for (name, fraction) in [("p95", 0.95), ("p99", 0.99)] {
                    if let Some(size) = stats.temp_sizes.percentile(fraction) {
                        rest.push_str(&format!(" · {name} {}", format_bytes(size)));
                    }
                }
                rest.push_str(&recent(stats.temp_recent.sum(self.slot())));
                // The setting is the smallest file logged, in kB.
                if let Some(least) = self
                    .setting("log_temp_files")
                    .parse::<f64>()
                    .ok()
                    .filter(|least| *least > 0.0)
                {
                    rest.push_str(&format!(
                        " · only files of {} or more",
                        format_bytes(least * 1024.0)
                    ));
                }
                (stats.temp_files, rest)
            }
            Filter::Checkpoints => {
                let checkpoints = &stats.checkpoints;
                let mut rest = format!(
                    " {} completed {since}",
                    plural(checkpoints.completed, "checkpoint")
                );
                for (by, count) in &checkpoints.by {
                    rest.push_str(&format!(" · {} by {by}", group(*count)));
                }
                if let Some(total) = percentile(checkpoints.done.iter().map(|c| c.total), 0.95) {
                    rest.push_str(&format!(" · total p95 {}", seconds(total)));
                }
                (checkpoints.completed, rest)
            }
        };
        vec![
            Span::styled(format!("  {}", group(count)), t.strong()),
            Span::styled(rest, t.muted()),
        ]
    }

    /// Why `filter` may have nothing to show: what is off, and what
    /// turning it on gives.
    fn limit(&self, filter: Filter) -> Option<[String; 2]> {
        let lc_messages = self.setting("lc_messages");
        let (what, name, value, gives) = match filter {
            Filter::Temp => (
                "temp files",
                "log_temp_files",
                "-1",
                "Set it to 0 to log every temp file with its size.",
            ),
            Filter::Checkpoints => (
                "checkpoints",
                "log_checkpoints",
                "off",
                "With it on, each checkpoint logs the buffers it wrote and its write and sync time.",
            ),
            Filter::All | Filter::Errors => return None,
        };
        if !english(lc_messages) {
            return Some([
                format!("No {what} are counted: lc_messages is {lc_messages}."),
                "top knows these lines only when the server writes its messages in English."
                    .to_owned(),
            ]);
        }
        (self.setting(name) == value).then(|| {
            [
                format!("No {what} in the log. {name} is {value}."),
                gives.to_owned(),
            ]
        })
    }

    /// The lines that say why the log is not read, if it is not.
    fn unread(&self) -> Option<Vec<String>> {
        let lines = match &self.why {
            Some(Why::Remote) => vec![
                "The log is not read: top reads the log file only on the database host.".to_owned(),
            ],
            Some(Why::Query) => vec![
                "The log is not read: the settings that say where it is could not be read."
                    .to_owned(),
            ],
            Some(Why::Hidden) => vec![
                "The log is not read: this role may not see log_directory.".to_owned(),
                "A superuser sees it, and from PostgreSQL 10 a member of pg_read_all_settings."
                    .to_owned(),
            ],
            Some(Why::Off { name, value }) => vec![
                format!("The log is not read: {name} is {value}."),
                "The view reads the csvlog files, which the server writes with \
logging_collector on and csvlog in log_destination."
                    .to_owned(),
                "S opens Settings at this setting.".to_owned(),
            ],
            None => match self.tail.as_ref()?.problem.as_ref()? {
                Problem::NoFile => vec![format!(
                    "No .csv file in {}.",
                    self.tail.as_ref()?.dir().display()
                )],
                Problem::Unreadable(path, reason) => vec![
                    format!("Could not read {}: {reason}.", path.display()),
                    "The log belongs to the server's user. Run top as that user or as root."
                        .to_owned(),
                ],
            },
        };
        Some(lines)
    }

    /// The source on the first row, the summary of the filter, the lines
    /// of the log with the summary pane beside or under them, and the
    /// filters on the last row. The details of the selected line take the
    /// place of the pane while they are open.
    pub(super) fn draw(&self, t: &Theme, buf: &mut Buffer, area: Rect) {
        let width = usize::from(area.width);
        let shown = self.shown();
        put(
            buf,
            area,
            0,
            Line::from(fit(self.source_line(t, &shown), width)),
        );
        if area.height < 4 {
            return;
        }
        put(buf, area, 1, Line::from(fit(self.summary_line(t), width)));
        let body = Rect {
            y: area.y + 2,
            height: area.height - 2,
            ..area
        };
        let message = |buf: &mut Buffer, room: Rect, lines: Vec<Line<'static>>| {
            let top = room.y + room.height.saturating_sub(lines.len() as u16) / 2;
            for (index, line) in lines.into_iter().enumerate() {
                let row = Rect {
                    y: top + index as u16,
                    height: 1,
                    ..room
                }
                .intersection(room);
                line.centered().render(row, buf);
            }
        };
        let wrapped = |room: Rect, lines: Vec<String>| -> Vec<Line<'static>> {
            let width = usize::from(room.width).saturating_sub(4).max(1);
            lines
                .iter()
                .flat_map(|line| split_cells(line, width))
                .map(|line| Line::styled(line, t.muted()))
                .collect()
        };
        if let Some(lines) = self.unread() {
            return message(buf, body, wrapped(body, lines));
        }
        let position = self.position(&shown);
        let summary = self.filter != Filter::All;
        self.pane_shown.set(0);
        // Beside the lines, the summary is a card. The scrollbar of the
        // lines takes the column after them, then one is free, and the
        // card's rail takes the one before the card.
        let mut beside = false;
        let mut list = body;
        if summary && area.width >= SIDE_MIN_WIDTH {
            let widest = (body.width - 3 - LIST_MIN_WIDTH).max(PANE_MIN_WIDTH);
            let pane_width = self
                .pane
                .unwrap_or((body.width * 2 / 5).min(64))
                .clamp(PANE_MIN_WIDTH, widest);
            self.pane_shown.set(pane_width);
            list.width = body.width - pane_width - 4;
            beside = true;
            let card = Card::new(t);
            let surface = Rect {
                x: list.right() + 4,
                width: pane_width,
                ..body
            };
            let pane = card.inner(surface);
            // The summary does not scroll: it shows what fits.
            let page = self.pane_page(t, usize::from(pane.width), usize::from(pane.height));
            card.render(surface, buf);
            page.view(t, &Scroll::default())
                .render(Card::new(t).inner(surface), buf);
        }
        if let (true, Some(position)) = (self.detail, position) {
            // Under the lines, and beside the summary on a wide screen. A
            // narrow screen has no room for the three, so the details
            // take the place of the summary.
            let build = |width| self.detail_page(t, shown[position], width);
            let room = Rect {
                height: body.height * 3 / 5,
                ..list
            };
            let (room, page) = fit_page(room, build, Page::len);
            let height = (page.len() as u16).min(room.height);
            list.height = body.height - height;
            let under = Rect {
                y: list.bottom(),
                height,
                ..room
            };
            page.view(t, &self.detail_scroll).render(under, buf);
        } else if summary && !beside {
            let room = body.height / 2;
            let page = self.pane_page(t, width, usize::from(room));
            let height = (page.len() as u16).min(room);
            list.height = body.height - height;
            let under = Rect {
                y: list.bottom(),
                height,
                ..body
            };
            page.view(t, &Scroll::default()).render(under, buf);
        }
        let Some(position) = position else {
            // The summary counts since top started, so it stays while the
            // list has no line to show.
            let lines = if !self.search.query.is_empty() {
                vec![self.search.nothing(t)]
            } else if let Some(limit) = self.limit(self.filter) {
                let mut lines = limit.to_vec();
                if self.missing().is_some() {
                    lines.push("S opens Settings at this setting.".to_owned());
                }
                wrapped(list, lines)
            } else if self.events.is_empty() {
                wrapped(
                    list,
                    vec![format!(
                        "Nothing in the log since top started at {}.",
                        self.started
                    )],
                )
            } else {
                let what = match self.filter {
                    Filter::Errors => "errors",
                    Filter::Temp => "temp files",
                    Filter::Checkpoints => "checkpoints",
                    Filter::All => "lines",
                };
                wrapped(
                    list,
                    vec![format!(
                        "No {what} among the last {} lines.",
                        group(self.events.len() as u64)
                    )],
                )
            };
            return message(buf, list, lines);
        };
        let rows = Rect {
            y: list.y.saturating_add(1).min(list.bottom()),
            height: list.height.saturating_sub(1),
            ..list
        };
        let build = |width| self.tail_page(t, &shown, width);
        // Beside the summary, the lines keep their width and the scrollbar
        // goes in the column after them.
        let (rows, (head, page)) = if beside {
            (rows, build(usize::from(rows.width)))
        } else {
            fit_page(rows, build, |(_, page)| page.len())
        };
        put(
            buf,
            Rect {
                width: rows.width,
                ..list
            },
            0,
            head,
        );
        // The rows on screen change with the pane and the screen size, so
        // every draw scrolls the selection back into view.
        self.scroll.follow();
        let view = page.view(t, &self.scroll).selected(Some(position));
        view.render(rows, buf);
    }

    /// The column titles and the lines of the log. A wide list also has
    /// the kind of each line and who wrote it.
    fn tail_page(&self, t: &Theme, shown: &[&Event], width: usize) -> (Line<'static>, Page) {
        let wide = width >= usize::from(SIDE_MIN_WIDTH);
        let (spec, titles): (&[usize], &[&str]) = if wide {
            (
                &[12, 7, 10, 5, 18, 0],
                &[
                    "Time",
                    "    PID",
                    "Kind",
                    "Code",
                    "User@database",
                    "Message",
                ],
            )
        } else {
            (&[8, 7, 5, 0], &["Time", "    PID", "Code", "Message"])
        };
        let columns = Columns::with_gap(width, spec, 1);
        let mut page = Page::default();
        for event in shown {
            let muted = |text: String| vec![Span::styled(text, t.muted())];
            let mut cells = vec![
                muted(event.time(wide).to_owned()),
                muted(format!("{:>7}", event.pid)),
            ];
            if wide {
                let kind = match event.kind {
                    Kind::Temp => Span::styled("temp file", t.text()),
                    Kind::Checkpoint => Span::styled("checkpoint", t.text()),
                    Kind::Error => Span::styled(event.severity.clone(), t.strong()),
                    Kind::Other => Span::styled(event.severity.clone(), t.muted()),
                };
                cells.push(vec![kind]);
            }
            cells.push(muted(event.code.clone()));
            if wide {
                cells.push(muted(event.who()));
            }
            cells.push(vec![Span::styled(single_line(&event.message), t.text())]);
            page.item([self.search.mark(t, columns.row(cells))]);
        }
        (columns.head(t, titles), page)
    }

    /// The summary of the filter, `width` wide, with the groups that fit
    /// in `height` rows.
    fn pane_page(&self, t: &Theme, width: usize, height: usize) -> Page {
        match self.filter {
            Filter::Errors => self.errors_page(t, width, height),
            Filter::Temp => self.temp_page(t, width, height),
            Filter::Checkpoints | Filter::All => self.checkpoints_page(t, width),
        }
    }

    /// The errors with the most lines, each under who got it: the
    /// condition name of its SQLSTATE and its message without the values.
    fn errors_page(&self, t: &Theme, width: usize, height: usize) -> Page {
        let mut groups: Vec<_> = self.stats.error_groups.iter().collect();
        groups.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));
        let top = top_that_fits(groups.len(), height);
        let mut page = Page::default();
        page.heading(
            t,
            width,
            "Errors by user, database and error",
            &format!("top {top} of {}", group(groups.len() as u64)),
        );
        let columns = Columns::new(width, &[7, 7, 8, 0]);
        page.line(columns.head(t, &["  Count", "Last 5m", "Last", "User@database"]));
        for ((user, database, code, message), found) in groups.into_iter().take(top) {
            page.line(columns.row(vec![
                vec![Span::styled(format!("{:>7}", group(found.count)), t.text())],
                vec![Span::styled(
                    format!("{:>7}", group(found.recent.sum(self.slot()))),
                    t.text(),
                )],
                vec![Span::styled(found.last.clone(), t.muted())],
                vec![Span::styled(who(user, database), t.text())],
            ]));
            let name = sqlstate::name(code).unwrap_or(code);
            page.line(second_row(vec![
                Span::styled(format!("{name} "), t.strong()),
                Span::styled(single_line(message), t.muted()),
            ]));
        }
        page
    }

    /// The statements that wrote the most temp files, by bytes written.
    fn temp_page(&self, t: &Theme, width: usize, height: usize) -> Page {
        let mut groups: Vec<_> = self.stats.temp_groups.iter().collect();
        groups.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes).then_with(|| a.0.cmp(b.0)));
        let top = top_that_fits(groups.len(), height);
        let mut page = Page::default();
        page.heading(
            t,
            width,
            "Temp files by statement",
            &format!("top {top} of {}", group(groups.len() as u64)),
        );
        let columns = Columns::new(width, &[6, 10, 10, 0]);
        page.line(columns.head(t, &[" Files", "   Written", "       p95", "User@database"]));
        for ((user, database, statement), found) in groups.into_iter().take(top) {
            let p95 = found
                .sizes
                .percentile(0.95)
                .map_or("—".to_owned(), format_bytes);
            page.line(columns.row(vec![
                vec![Span::styled(format!("{:>6}", group(found.files)), t.text())],
                vec![Span::styled(
                    format!("{:>10}", format_bytes(found.bytes as f64)),
                    t.text(),
                )],
                vec![Span::styled(format!("{p95:>10}"), t.text())],
                vec![Span::styled(who(user, database), t.text())],
            ]));
            let statement = if statement.is_empty() {
                "— the line has no statement".to_owned()
            } else {
                statement.clone()
            };
            page.line(second_row(vec![Span::styled(statement, t.muted())]));
        }
        page
    }

    /// What started the checkpoints, the percentiles of what they wrote
    /// and how long they took, and the last five.
    fn checkpoints_page(&self, t: &Theme, width: usize) -> Page {
        let checkpoints = &self.stats.checkpoints;
        let mut page = Page::default();
        page.heading(
            t,
            width,
            "Checkpoints",
            &format!("{} completed", group(checkpoints.completed)),
        );
        let mut fact = |key: &str, value: String| {
            for line in fact_lines(t, width, 2, 12, key, vec![Span::styled(value, t.text())]) {
                page.line(line);
            }
        };
        let by: Vec<String> = checkpoints
            .by
            .iter()
            .map(|(by, count)| format!("{by} {}", group(*count)))
            .collect();
        fact("Started by", or_dash(by.join(" · ")));
        let mut intervals: Vec<f64> = checkpoints.intervals.iter().copied().collect();
        intervals.sort_by(f64::total_cmp);
        fact(
            "Interval",
            intervals
                .get(intervals.len().saturating_sub(1) / 2)
                .map_or("—".to_owned(), |median| {
                    format!("median {}", span(*median))
                }),
        );
        let pair = |value: fn(&stats::Checkpoint) -> f64, show: fn(f64) -> String| {
            let of = |fraction| percentile(checkpoints.done.iter().map(value), fraction).map(show);
            match (of(0.95), of(0.99)) {
                (Some(p95), Some(p99)) => format!("p95 {p95} · p99 {p99}"),
                _ => "—".to_owned(),
            }
        };
        fact("Buffers", pair(|c| c.buffers, |v| group(v as u64)));
        fact("Buffers %", pair(|c| c.percent, |v| format!("{v:.1}%")));
        fact("Write", pair(|c| c.write, seconds));
        fact("Sync", pair(|c| c.sync, seconds));
        fact("Total", pair(|c| c.total, seconds));
        fact(
            "WAL files",
            format!(
                "added {} · removed {} · recycled {}",
                group(checkpoints.added),
                group(checkpoints.removed),
                group(checkpoints.recycled)
            ),
        );
        if checkpoints.done.is_empty() {
            return page;
        }
        page.blank();
        let columns = Columns::with_gap(width, &[8, 0, 7, 7, 7, 7], 1);
        page.line(columns.head(
            t,
            &["Started", "By", "Buffers", "  Write", "   Sync", "  Total"],
        ));
        for checkpoint in checkpoints.done.iter().rev().take(5) {
            let number = |text: String, width: usize| {
                vec![Span::styled(format!("{text:>width$}"), t.text())]
            };
            page.line(columns.row(vec![
                vec![Span::styled(checkpoint.time.clone(), t.muted())],
                vec![Span::styled(checkpoint.by.clone(), t.text())],
                number(group(checkpoint.buffers as u64), 7),
                number(seconds(checkpoint.write), 7),
                number(seconds(checkpoint.sync), 7),
                number(seconds(checkpoint.total), 7),
            ]));
        }
        page
    }

    /// Every field of one line, with its statement whole.
    fn detail_page(&self, t: &Theme, event: &Event, width: usize) -> Page {
        let mut page = Page::default();
        page.section_meta(t, width, "Log line", &event.logged);
        let mut fact = |key: &str, value: String| {
            if value.is_empty() {
                return;
            }
            let value = vec![Span::styled(single_line(&value), t.text())];
            for line in fact_lines(t, width, 2, 11, key, value) {
                page.line(line);
            }
        };
        let code = match sqlstate::name(&event.code) {
            Some(name) => format!("{} {name}", event.code),
            None => event.code.clone(),
        };
        fact("Severity", format!("{} · {code}", event.severity));
        let session = [
            event.who(),
            format!("pid {}", event.pid),
            event.application.clone(),
            event.client.clone(),
            format!("session {}", event.session),
        ];
        fact(
            "Session",
            session
                .iter()
                .filter(|part| !part.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · "),
        );
        fact("Message", event.message.clone());
        fact("Detail", event.detail.clone());
        fact("Hint", event.hint.clone());
        fact("Context", event.context.clone());
        if !event.statement.is_empty() {
            page.blank();
            page.section(t, width, "Statement", None);
            let lines: Vec<String> = event
                .statement
                .lines()
                .map(single_line_keep_indent)
                .collect();
            code_block(t, &mut page, &lines, width);
        }
        page
    }
}

/// The second row of a group, under its figures.
fn second_row(spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("    ")];
    row.extend(spans);
    Line::from(row)
}

/// How many of `groups` a pane `height` rows high shows: ten at most, two
/// rows each under the three of the title and the column titles.
fn top_that_fits(groups: usize, height: usize) -> usize {
    groups.min(TOP).min((height.saturating_sub(3) / 2).max(1))
}

fn or_dash(value: String) -> String {
    if value.is_empty() {
        "—".to_owned()
    } else {
        value
    }
}

/// `noun`, with an `s` unless `count` is one.
fn plural(count: u64, noun: &str) -> String {
    if count == 1 {
        noun.to_owned()
    } else {
        format!("{noun}s")
    }
}

/// A count with a comma between each three digits.
fn group(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A duration the server wrote in seconds.
fn seconds(value: f64) -> String {
    if value < 10.0 {
        format!("{value:.2} s")
    } else {
        format!("{value:.1} s")
    }
}

/// The time between two checkpoints.
fn span(value: f64) -> String {
    let whole = value.max(0.0).round() as u64;
    match (whole / 3600, whole / 60 % 60, whole % 60) {
        (0, 0, _) => seconds(value),
        (0, minutes, secs) => format!("{minutes}m {secs:02}s"),
        (hours, minutes, _) => format!("{hours}h {minutes:02}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::stats::tests::record;
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    const TEMP: &str = "temporary file: path \"base/pgsql_tmp/pgsql_tmp7.0\", size 1048576";
    const DONE: &str = "checkpoint complete: wrote 5408 buffers (33.0%); \
0 transaction log file(s) added, 0 removed, 8 recycled; write=0.150 s, sync=0.002 s, \
total=0.153 s; sync files=11, longest=0.001 s, average=0.000 s";

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// A view that reads a log with errors, temp files and a checkpoint.
    fn view() -> LogView {
        let mut view = LogView::new(true);
        view.why = None;
        view.settings = settings(&[
            ("lc_messages", "en_US.utf8"),
            ("log_temp_files", "0"),
            ("log_checkpoints", "on"),
        ]);
        for second in 0..3 {
            let time = format!("14:06:3{second}.000");
            view.push(record(
                &time,
                "23505",
                "duplicate key value violates unique constraint \"orders_pkey\"",
                &format!("insert into orders values ({second}, 'x')"),
            ));
            view.push(record(
                &time,
                "00000",
                TEMP,
                "select k from facts order by k",
            ));
        }
        view.push(record(
            "14:06:33.000",
            "00000",
            "checkpoint starting: xlog",
            "",
        ));
        view.push(record("14:06:34.000", "00000", DONE, ""));
        view
    }

    fn press(view: &mut LogView, code: KeyCode) -> Pressed {
        view.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn rows(view: &LogView, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.draw(&super::super::theme(), &mut buf, area);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn the_lines_follow_the_newest_one_with_the_count_of_each_filter() {
        let view = view();
        let rows = rows(&view, 120, 20);
        assert!(rows[1].starts_with("  8 lines since top started at"));
        assert!(
            rows[1].ends_with("· 3 errors · 3 temp files · 1 checkpoint"),
            "{:?}",
            rows[1]
        );
        assert!(rows[2].contains("Kind"), "{:?}", rows[2]);
        // The newest line is the last one and has the selection.
        assert!(rows[10].starts_with("▌ 14:06:34.000"), "{:?}", rows[10]);
        // No filter is on, so no summary shows.
        assert!(!view
            .hints()
            .iter()
            .any(|(_, label)| label.contains("summary")));
    }

    #[test]
    fn a_filter_shows_its_summary_beside_the_lines() {
        let mut view = view();
        press(&mut view, KeyCode::Char('e'));
        let screen = rows(&view, 120, 20);
        assert!(screen[1].starts_with("  3 errors since top started"));
        assert!(screen[3].contains("      Errors by user, database and error"));
        let text = screen.join("\n");
        assert!(text.contains("3        3  14:06:32  app@shop"), "{text}");
        assert!(text.contains("unique_violation duplicate key"), "{text}");
        assert!(!text.contains("temporary file"), "{text}");

        press(&mut view, KeyCode::Char('t'));
        let text = rows(&view, 120, 20).join("\n");
        assert!(text.contains("Temp files by statement"), "{text}");
        assert!(
            text.contains("3     3.0 MiB     1.0 MiB  app@shop"),
            "{text}"
        );
        assert!(text.contains("select k from facts order by k"), "{text}");

        press(&mut view, KeyCode::Char('c'));
        let text = rows(&view, 120, 24).join("\n");
        assert!(text.contains("Started by  xlog 1"), "{text}");
        assert!(text.contains("Buffers     p95 5,408 · p99 5,408"), "{text}");
        assert!(text.contains("Sync        p95 0.00 s"), "{text}");
        assert!(text.contains("recycled 8"), "{text}");
        assert!(text.contains("14:06:33 xlog"), "{text}");

        // The key of the filter that is on turns it off, and so does esc.
        press(&mut view, KeyCode::Char('c'));
        assert_eq!(view.filter, Filter::All);
        press(&mut view, KeyCode::Char('e'));
        assert!(view.hints().contains(&ERRORS_ON));
        press(&mut view, KeyCode::Esc);
        assert_eq!(view.filter, Filter::All);
    }

    #[test]
    fn the_arrows_resize_the_summary_beside_the_lines() {
        let mut view = view();
        let rule = |view: &LogView, width| {
            rows(view, width, 20)[2]
                .chars()
                .position(|cell| cell == '▄')
        };
        // No summary beside the list, so nothing to resize.
        press(&mut view, KeyCode::Left);
        assert_eq!(view.pane, None);
        press(&mut view, KeyCode::Char('e'));
        let start = rule(&view, 120).expect("the card");
        assert!(view.hints().contains(&("←→", "resize")));
        press(&mut view, KeyCode::Left);
        assert_eq!(rule(&view, 120), Some(start - usize::from(PANE_STEP)));
        // The list and the summary keep a least width.
        for _ in 0..20 {
            press(&mut view, KeyCode::Left);
            rows(&view, 120, 20);
        }
        assert_eq!(rule(&view, 120), Some(usize::from(LIST_MIN_WIDTH) + 3));
        press(&mut view, KeyCode::Right);
        assert_eq!(
            rule(&view, 120),
            Some(usize::from(LIST_MIN_WIDTH + PANE_STEP) + 3)
        );
        for _ in 0..20 {
            press(&mut view, KeyCode::Right);
            rows(&view, 120, 20);
        }
        assert_eq!(rule(&view, 120), Some(120 - usize::from(PANE_MIN_WIDTH)));
        // Under the list there is no rule to move.
        rows(&view, 80, 20);
        assert!(!view.hints().contains(&("←→", "resize")));
    }

    #[test]
    fn the_summary_stays_when_the_list_has_no_line_of_the_filter() {
        let mut view = view();
        for _ in 0..EVENTS_MAX {
            view.push(record("14:07:00.000", "00000", "statement: select 1", ""));
        }
        press(&mut view, KeyCode::Char('e'));
        for width in [80, 120] {
            let text = rows(&view, width, 24).join("\n");
            assert!(
                text.contains("No errors among the last 1,000 lines."),
                "{text}"
            );
            assert!(
                text.contains("Errors by user, database and error"),
                "{text}"
            );
            assert!(text.contains("unique_violation"), "{text}");
        }
    }

    #[test]
    fn a_narrow_screen_puts_the_summary_under_the_lines() {
        let mut view = view();
        press(&mut view, KeyCode::Char('e'));
        let rows = rows(&view, 80, 20);
        assert!(rows.iter().all(|row| row.chars().count() <= 80));
        let title = rows
            .iter()
            .position(|row| row.starts_with("Errors by user, database and error"))
            .expect("the summary");
        assert!(title > 3, "{rows:?}");
        assert!(rows[5].starts_with("▌ 14:06:32"), "{rows:?}");
        assert!(view.hints().contains(&ERRORS_ON));
    }

    #[test]
    fn enter_shows_the_details_of_the_selected_line() {
        let mut view = view();
        press(&mut view, KeyCode::Char('e'));
        press(&mut view, KeyCode::Up);
        press(&mut view, KeyCode::Enter);
        for width in [80, 120] {
            let text = rows(&view, width, 24).join("\n");
            assert!(text.contains("Log line"), "{text}");
            assert!(text.contains("ERROR · 23505 unique_violation"), "{text}");
            assert!(text.contains("app@shop · pid 4242"), "{text}");
            assert!(
                text.contains("insert into orders values (1, 'x')"),
                "{text}"
            );
            // A wide screen keeps the summary beside the details. On a
            // narrow one the details take its place.
            assert_eq!(text.contains("Errors by user"), width == 120, "{text}");
        }
        assert!(view.hints().contains(&DETAILS_ON));
        assert!(view.hints().contains(&("esc", "close")));
        // The details follow the selection.
        press(&mut view, KeyCode::Down);
        assert!(rows(&view, 120, 24).join("\n").contains("values (2, 'x')"));
        press(&mut view, KeyCode::Esc);
        assert!(rows(&view, 120, 24).join("\n").contains("Errors by user"));
    }

    #[test]
    fn moving_pauses_the_follow_and_end_resumes_it() {
        let mut view = view();
        assert_eq!(view.selected, None);
        press(&mut view, KeyCode::Up);
        let paused = view.selected;
        assert!(paused.is_some());
        view.push(record("14:06:40.000", "22012", "division by zero", ""));
        // The selection stays on its line while new ones arrive.
        assert_eq!(view.selected, paused);
        let dir = tempfile::tempdir().expect("a directory");
        std::fs::write(dir.path().join("a.csv"), "").expect("a log file");
        let mut tail = Tail::new(dir.path().to_owned());
        tail.tick();
        view.tail = Some(tail);
        let source = &rows(&view, 120, 20)[0];
        assert!(source.contains("a.csv · paused · 2 newer"), "{source}");
        press(&mut view, KeyCode::End);
        assert_eq!(view.selected, None);
    }

    #[test]
    fn the_search_keeps_the_lines_with_the_text() {
        let mut view = view();
        press(&mut view, KeyCode::Char('/'));
        for letter in "facts".chars() {
            assert_eq!(press(&mut view, KeyCode::Char(letter)), Pressed::Handled);
        }
        press(&mut view, KeyCode::Enter);
        assert_eq!(view.matches(), 3);
        press(&mut view, KeyCode::Esc);
        assert_eq!(view.matches(), 8);
        assert_eq!(press(&mut view, KeyCode::Char('q')), Pressed::Pass);
        assert_eq!(press(&mut view, KeyCode::Char('5')), Pressed::Pass);
    }

    #[test]
    fn a_setting_that_is_off_says_what_is_missing() {
        let mut view = LogView::new(true);
        view.apply(settings(&[("logging_collector", "off")]));
        let text = rows(&view, 120, 20).join("\n");
        assert!(text.contains("The log is not read: logging_collector is off."));
        assert_eq!(
            press(&mut view, KeyCode::Char('S')),
            Pressed::Settings("logging_collector")
        );

        view.apply(settings(&[
            ("logging_collector", "on"),
            ("log_destination", "stderr"),
        ]));
        assert!(rows(&view, 120, 20)
            .join("\n")
            .contains("log_destination is stderr"));

        // The role does not see where the log is.
        view.apply(settings(&[
            ("logging_collector", "on"),
            ("log_destination", "stderr,csvlog"),
        ]));
        assert_eq!(view.why, Some(Why::Hidden));
        assert_eq!(press(&mut view, KeyCode::Char('S')), Pressed::Handled);

        let remote = LogView::new(false);
        assert!(rows(&remote, 120, 20)
            .join("\n")
            .contains("top reads the log file only on the database host"));
    }

    #[test]
    fn a_filter_without_its_setting_says_so() {
        let mut view = LogView::new(true);
        view.why = None;
        view.settings = settings(&[("log_temp_files", "-1"), ("log_checkpoints", "off")]);
        view.push(record("14:06:30.000", "22012", "division by zero", ""));
        press(&mut view, KeyCode::Char('t'));
        let text = rows(&view, 120, 20).join("\n");
        assert!(text.contains("No temp files in the log. log_temp_files is -1."));
        assert_eq!(
            press(&mut view, KeyCode::Char('S')),
            Pressed::Settings("log_temp_files")
        );
        press(&mut view, KeyCode::Char('c'));
        assert!(rows(&view, 120, 20)
            .join("\n")
            .contains("No checkpoints in the log. log_checkpoints is off."));

        view.settings = settings(&[("lc_messages", "pt_BR.UTF-8")]);
        assert!(rows(&view, 120, 20)
            .join("\n")
            .contains("No checkpoints are counted: lc_messages is pt_BR.UTF-8."));
    }

    #[test]
    fn the_log_directory_comes_from_the_settings() {
        let found = |log_directory: &str| {
            log_dir(&settings(&[
                ("logging_collector", "on"),
                ("log_destination", "csvlog"),
                ("log_directory", log_directory),
                ("data_directory", "/data"),
            ]))
        };
        assert_eq!(found("pg_log"), Ok(PathBuf::from("/data/pg_log")));
        assert_eq!(found("/var/log/pg"), Ok(PathBuf::from("/var/log/pg")));
        assert!(english("en_US.utf8") && english("C") && english("C.UTF-8") && english(""));
        assert!(!english("pt_BR.UTF-8"));
    }

    #[test]
    fn counts_have_a_comma_each_three_digits() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(48_210), "48,210");
        assert_eq!(group(1_000_000), "1,000,000");
        assert_eq!(span(0.89), "0.89 s");
        assert_eq!(span(301.0), "5m 01s");
        assert_eq!(span(3720.0), "1h 02m");
    }
}
