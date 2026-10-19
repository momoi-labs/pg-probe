//! Live view of the queries running on a PostgreSQL server, in the spirit of
//! pg_activity.
//!
//! Unlike `capture`, this view shows query text, but only on the operator's
//! terminal: nothing is persisted. The session is opened read-only with a
//! statement timeout, and each refresh runs one constant `SELECT` over
//! `pg_stat_activity`. The blocking view adds `pg_blocking_pids`, which takes
//! lock-manager locks, so it only runs while that view is open.
//!
//! When the server runs on this host, host load and memory and per-backend
//! CPU and I/O come from the local `/proc`, which costs the server nothing.

mod explain;
mod function;
mod os;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use sqlformat::{Dialect, FormatOptions, QueryParams};
use thiserror::Error;

use crate::postgres::stats::capability::{has_backend_type, has_wait_event_type};
use crate::postgres::{
    AdapterError, ConnectedClient, ConnectionParams, EndpointIdentity, EndpointKind,
};
use os::{Host, ProcessMemory, ProcessSample};

const SOURCE: &str = "postgres.top";
const STATEMENT_TIMEOUT_MS: u64 = 5_000;
const PAGE: usize = 10;

const SERVER_SQL: &str = "\
SELECT current_setting('server_version_num'), current_setting('server_version'), \
    current_setting('max_connections'), current_user, current_database(), \
    current_setting('track_activity_query_size')";

/// Builds the `SELECT` that pairs each backend blocked on a lock with the
/// pids holding it. PG 9.6+ has `pg_blocking_pids`. Older servers match
/// ungranted locks against granted locks on the same object in `pg_locks`;
/// the CTE makes the server read `pg_locks` once instead of twice.
fn blocking_sql(server_version_num: i32) -> &'static str {
    if has_wait_event_type(server_version_num).is_supported() {
        "\
SELECT pid, array_to_string(pg_blocking_pids(pid), ',') \
FROM pg_stat_activity \
WHERE wait_event_type = 'Lock'"
    } else {
        "\
WITH l AS (SELECT * FROM pg_locks) \
SELECT w.pid, string_agg(DISTINCT h.pid::text, ',') \
FROM l w \
JOIN l h ON h.granted \
    AND h.pid <> w.pid \
    AND h.locktype = w.locktype \
    AND h.database IS NOT DISTINCT FROM w.database \
    AND h.relation IS NOT DISTINCT FROM w.relation \
    AND h.page IS NOT DISTINCT FROM w.page \
    AND h.tuple IS NOT DISTINCT FROM w.tuple \
    AND h.virtualxid IS NOT DISTINCT FROM w.virtualxid \
    AND h.transactionid IS NOT DISTINCT FROM w.transactionid \
    AND h.classid IS NOT DISTINCT FROM w.classid \
    AND h.objid IS NOT DISTINCT FROM w.objid \
    AND h.objsubid IS NOT DISTINCT FROM w.objsubid \
WHERE NOT w.granted \
GROUP BY w.pid"
    }
}

/// Builds the `SELECT` behind the details panel for one backend. It only runs
/// while the panel is open. Returns `None` unless `pid` is a plain number, so
/// nothing but digits is ever spliced into the statement.
fn details_sql(server_version_num: i32, pid: &str) -> Option<String> {
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let backend_type = if has_backend_type(server_version_num).is_supported() {
        "backend_type"
    } else {
        "'client backend'::text"
    };
    // backend_xid and backend_xmin arrived in PG 9.4.
    let (xid, xmin) = if server_version_num >= 90400 {
        ("backend_xid::text", "backend_xmin::text")
    } else {
        ("NULL::text", "NULL::text")
    };
    let age = |column: &str| {
        format!("round(EXTRACT(EPOCH FROM (clock_timestamp() - {column}))::numeric, 3)")
    };
    let started = |column: &str| format!("to_char({column}, 'YYYY-MM-DD HH24:MI:SS')");
    Some(format!(
        "\
SELECT application_name, client_addr::text, client_port, {backend_type}, \
    {}, {}, {}, {}, {}, {}, {xid}, {xmin}, query \
FROM pg_stat_activity \
WHERE pid = {pid}",
        started("backend_start"),
        age("backend_start"),
        started("xact_start"),
        age("xact_start"),
        age("query_start"),
        age("state_change"),
    ))
}

/// Builds the backend `SELECT` for the given server version. Idle backends
/// are kept for the session counts, but without their last query text.
/// Before PG 10 `pg_stat_activity` lists only client backends and has no
/// `backend_type`; before PG 9.6 a lock wait is only the `waiting` flag.
fn backends_sql(server_version_num: i32) -> String {
    let client_backends = if has_backend_type(server_version_num).is_supported() {
        "backend_type = 'client backend' AND "
    } else {
        ""
    };
    let wait = if has_wait_event_type(server_version_num).is_supported() {
        "wait_event_type, wait_event"
    } else {
        "CASE WHEN waiting THEN 'Lock' END AS wait_event_type, NULL::text AS wait_event"
    };
    format!(
        "\
SELECT \
    pid, datname, usename, state, {wait}, \
    round(EXTRACT(EPOCH FROM (clock_timestamp() - query_start))::numeric, 3) AS duration_s, \
    CASE WHEN state = 'idle' THEN NULL ELSE query END AS query \
FROM pg_stat_activity \
WHERE {client_backends}pid <> pg_backend_pid() \
ORDER BY query_start ASC NULLS LAST"
    )
}

#[derive(Debug, Error)]
pub enum TopError {
    #[error(transparent)]
    Postgres(#[from] AdapterError),
    #[error("terminal error: {0}")]
    Terminal(#[from] std::io::Error),
}

/// Facts about the server read once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub version_num: i32,
    pub version: String,
    pub max_connections: usize,
    pub user: String,
    pub database: String,
    pub endpoint: String,
    /// `track_activity_query_size`: the server cuts longer query text.
    pub query_size_limit: usize,
}

/// One client backend from `pg_stat_activity`.
#[derive(Debug, Clone, PartialEq)]
pub struct Backend {
    pub pid: String,
    pub database: String,
    pub user: String,
    pub state: String,
    pub wait_type: String,
    pub wait_event: String,
    pub duration_s: Option<f64>,
    pub query: String,
}

impl Backend {
    fn waits_on_lock(&self) -> bool {
        self.wait_type == "Lock"
    }

    fn wait_label(&self) -> String {
        match (self.wait_type.as_str(), self.wait_event.as_str()) {
            ("", _) => String::new(),
            (kind, "") => kind.to_owned(),
            (kind, event) => format!("{kind}:{event}"),
        }
    }
}

/// What the details panel shows beyond the table row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    pub application: String,
    pub client: String,
    pub backend_type: String,
    pub backend_start: String,
    pub connection_age_s: Option<f64>,
    pub xact_start: String,
    pub xact_age_s: Option<f64>,
    pub query_age_s: Option<f64>,
    pub state_age_s: Option<f64>,
    pub xid: String,
    pub xmin: String,
    pub query: String,
    /// The query exactly as the server holds it, line breaks included. It
    /// is sent to `EXPLAIN`, and drawn only after formatting strips control
    /// characters.
    pub raw_query: String,
}

/// The details panel for one backend, opened with `v`.
struct Popup {
    pid: String,
    details: Option<Details>,
    memory: Option<ProcessMemory>,
    /// The backend is gone; the panel keeps its last known details.
    ended: bool,
    scroll: u16,
    /// The definitions of the functions the query calls, opened with `f`.
    function: Option<FunctionPanel>,
    /// The estimated plan of the query, opened with `e`.
    explain: Option<ExplainPanel>,
}

struct ExplainPanel {
    database: String,
    /// The plan lines, or why there is no plan.
    result: Result<Vec<String>, String>,
    scroll: u16,
}

struct FunctionPanel {
    database: String,
    /// The functions found, or why the lookup failed.
    result: Result<Vec<function::FunctionDef>, String>,
    index: usize,
    scroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Running,
    Waiting,
    Blocking,
}

impl View {
    const fn title(self) -> &'static str {
        match self {
            Self::Running => "RUNNING QUERIES",
            Self::Waiting => "WAITING QUERIES",
            Self::Blocking => "BLOCKING QUERIES",
        }
    }
}

struct App {
    server: Server,
    interval: Duration,
    view: View,
    paused: bool,
    /// The key list, opened with `?`.
    help: bool,
    popup: Option<Popup>,
    backends: Vec<Backend>,
    /// Blocker pid to the pids it blocks. Filled only in the blocking view.
    blocks: HashMap<String, Vec<String>>,
    /// Local `/proc` metrics, present only when the server runs on this host.
    os: Option<OsState>,
    table: TableState,
}

struct OsState {
    host: Host,
    ticks_per_second: f64,
    samples: HashMap<String, (Instant, ProcessSample)>,
    rates: HashMap<String, ProcessRates>,
}

/// Per-backend rates over the last refresh interval. They stay `None` until
/// a backend has two samples, or when its I/O counters are not readable.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct ProcessRates {
    cpu_percent: Option<f64>,
    read_per_s: Option<f64>,
    write_per_s: Option<f64>,
    io_wait: bool,
}

impl OsState {
    fn new() -> Self {
        Self {
            host: Host::default(),
            ticks_per_second: os::clock_ticks_per_second(),
            samples: HashMap::new(),
            rates: HashMap::new(),
        }
    }

    /// Samples the host and every non-idle backend, and turns the change
    /// since the previous sample into rates.
    fn refresh(&mut self, backends: &[Backend]) {
        let now = Instant::now();
        self.host = os::read_host();
        let mut samples = HashMap::new();
        let mut rates = HashMap::new();
        for backend in backends.iter().filter(|backend| backend.state != "idle") {
            let Some(sample) = os::read_process(&backend.pid) else {
                continue;
            };
            let mut rate = ProcessRates {
                io_wait: sample.io_wait,
                ..ProcessRates::default()
            };
            if let Some((then, previous)) = self.samples.get(&backend.pid) {
                let elapsed = now.duration_since(*then).as_secs_f64();
                if elapsed > 0.0 {
                    let ticks = sample.cpu_ticks.saturating_sub(previous.cpu_ticks) as f64;
                    rate.cpu_percent = Some(ticks / self.ticks_per_second / elapsed * 100.0);
                    if let (Some((read, written)), Some((read_before, written_before))) =
                        (sample.io, previous.io)
                    {
                        rate.read_per_s = Some(read.saturating_sub(read_before) as f64 / elapsed);
                        rate.write_per_s =
                            Some(written.saturating_sub(written_before) as f64 / elapsed);
                    }
                }
            }
            rates.insert(backend.pid.clone(), rate);
            samples.insert(backend.pid.clone(), (now, sample));
        }
        self.samples = samples;
        self.rates = rates;
    }

    fn io_denied(&self) -> bool {
        !self.samples.is_empty() && self.samples.values().all(|(_, sample)| sample.io.is_none())
    }
}

/// Opens a read-only session and runs the interactive view until the
/// operator quits or the server stops answering.
pub fn run(params: &ConnectionParams, interval: Duration) -> Result<(), TopError> {
    let client =
        ConnectedClient::open_read_only(params, STATEMENT_TIMEOUT_MS, SOURCE, &mut Vec::new())?;
    let server = read_server(&client)?;
    let sql = backends_sql(server.version_num);
    let local = is_local(client.endpoint());
    let mut app = App::new(server, interval, local);
    // The first fetch happens before raw mode, so a failure prints normally.
    app.refresh(&client, &sql)?;

    let mut terminal = ratatui::try_init()?;
    let result = app.event_loop(&client, &sql, &mut terminal);
    ratatui::restore();
    result
}

impl App {
    fn new(server: Server, interval: Duration, local: bool) -> Self {
        Self {
            server,
            interval,
            view: View::Running,
            paused: false,
            help: false,
            popup: None,
            backends: Vec::new(),
            blocks: HashMap::new(),
            os: local.then(OsState::new),
            table: TableState::default().with_selected(Some(0)),
        }
    }

    fn event_loop(
        &mut self,
        client: &ConnectedClient,
        sql: &str,
        terminal: &mut DefaultTerminal,
    ) -> Result<(), TopError> {
        let mut next_refresh = Instant::now() + self.interval;
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            let timeout = next_refresh.saturating_duration_since(Instant::now());
            if event::poll(timeout)? {
                let Event::Key(key) = event::read()? else {
                    continue;
                };
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                if self.help {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        _ => self.help = false,
                    }
                    continue;
                }
                if let Some(panel) = self
                    .popup
                    .as_mut()
                    .and_then(|popup| popup.function.as_mut())
                {
                    let count = panel.result.as_ref().map_or(0, Vec::len);
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('f') | KeyCode::Esc => {
                            if let Some(popup) = &mut self.popup {
                                popup.function = None;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            panel.scroll = panel.scroll.saturating_add(1)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            panel.scroll = panel.scroll.saturating_sub(1)
                        }
                        KeyCode::PageDown => {
                            panel.scroll = panel.scroll.saturating_add(PAGE as u16)
                        }
                        KeyCode::PageUp => panel.scroll = panel.scroll.saturating_sub(PAGE as u16),
                        KeyCode::Right | KeyCode::Char('l') if panel.index + 1 < count => {
                            panel.index += 1;
                            panel.scroll = 0;
                        }
                        KeyCode::Left | KeyCode::Char('h') if panel.index > 0 => {
                            panel.index -= 1;
                            panel.scroll = 0;
                        }
                        _ => {}
                    }
                    continue;
                }
                if let Some(panel) = self.popup.as_mut().and_then(|popup| popup.explain.as_mut()) {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('e') | KeyCode::Esc => {
                            if let Some(popup) = &mut self.popup {
                                popup.explain = None;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            panel.scroll = panel.scroll.saturating_add(1)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            panel.scroll = panel.scroll.saturating_sub(1)
                        }
                        KeyCode::PageDown => {
                            panel.scroll = panel.scroll.saturating_add(PAGE as u16)
                        }
                        KeyCode::PageUp => panel.scroll = panel.scroll.saturating_sub(PAGE as u16),
                        _ => {}
                    }
                    continue;
                }
                if key.code == KeyCode::Char('f') && self.popup.is_some() {
                    self.open_function(client);
                    continue;
                }
                if key.code == KeyCode::Char('e') && self.popup.is_some() {
                    self.open_explain(client);
                    continue;
                }
                if let Some(popup) = &mut self.popup {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('v') | KeyCode::Esc | KeyCode::Enter => self.popup = None,
                        KeyCode::Char(' ') => self.paused = !self.paused,
                        KeyCode::Down | KeyCode::Char('j') => {
                            popup.scroll = popup.scroll.saturating_add(1)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            popup.scroll = popup.scroll.saturating_sub(1)
                        }
                        KeyCode::PageDown => {
                            popup.scroll = popup.scroll.saturating_add(PAGE as u16)
                        }
                        KeyCode::PageUp => popup.scroll = popup.scroll.saturating_sub(PAGE as u16),
                        _ => {}
                    }
                    continue;
                }
                let view = self.view;
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char(' ') => self.paused = !self.paused,
                    KeyCode::Char('?') => self.help = true,
                    KeyCode::Char('v') | KeyCode::Enter => self.open_popup(client)?,
                    KeyCode::F(1) | KeyCode::Char('1') => self.view = View::Running,
                    KeyCode::F(2) | KeyCode::Char('2') => self.view = View::Waiting,
                    KeyCode::F(3) | KeyCode::Char('3') => self.view = View::Blocking,
                    KeyCode::Down | KeyCode::Char('j') => self.select_by(1),
                    KeyCode::Up | KeyCode::Char('k') => self.select_by(-1),
                    KeyCode::PageDown => self.select_by(PAGE as isize),
                    KeyCode::PageUp => self.select_by(-(PAGE as isize)),
                    KeyCode::Home => self.table.select(Some(0)),
                    KeyCode::End => self.select_by(isize::MAX / 2),
                    _ => {}
                }
                if self.view != view {
                    self.table.select(Some(0));
                    if !self.paused {
                        self.refresh(client, sql)?;
                        next_refresh = Instant::now() + self.interval;
                    }
                }
            } else {
                if !self.paused {
                    self.refresh(client, sql)?;
                }
                next_refresh = Instant::now() + self.interval;
            }
        }
    }

    fn refresh(&mut self, client: &ConnectedClient, sql: &str) -> Result<(), TopError> {
        self.backends = fetch_backends(client, sql)?;
        if let Some(os) = &mut self.os {
            os.refresh(&self.backends);
        }
        self.refresh_popup(client)?;
        self.blocks = if self.view == View::Blocking {
            fetch_blocks(client, blocking_sql(self.server.version_num))?
        } else {
            HashMap::new()
        };
        self.select_by(0);
        Ok(())
    }

    fn open_popup(&mut self, client: &ConnectedClient) -> Result<(), TopError> {
        let selected = self.table.selected().unwrap_or(0);
        let Some(pid) = self
            .visible()
            .get(selected)
            .map(|backend| backend.pid.clone())
        else {
            return Ok(());
        };
        self.popup = Some(Popup {
            pid,
            details: None,
            memory: None,
            ended: false,
            scroll: 0,
            function: None,
            explain: None,
        });
        self.refresh_popup(client)
    }

    /// Looks up the functions the popup's query calls, in the database the
    /// query runs in. A lookup failure is shown in the panel, never fatal.
    fn open_function(&mut self, client: &ConnectedClient) {
        let Some(popup) = &self.popup else {
            return;
        };
        let backend = self
            .backends
            .iter()
            .find(|backend| backend.pid == popup.pid);
        let query = popup
            .details
            .as_ref()
            .map(|details| details.query.clone())
            .or_else(|| backend.map(|backend| backend.query.clone()))
            .unwrap_or_default();
        let database = backend
            .map(|backend| backend.database.clone())
            .unwrap_or_default();
        let result = self.in_database(client, &database, |session| {
            function::fetch(session, &query).map_err(|_| "The catalog query failed.".to_owned())
        });
        if let Some(popup) = &mut self.popup {
            popup.function = Some(FunctionPanel {
                database,
                result,
                index: 0,
                scroll: 0,
            });
        }
    }

    /// Plans the popup's query with `EXPLAIN`, without `ANALYZE`, in the
    /// database the query runs in. Refusals and failures show in the popup.
    fn open_explain(&mut self, client: &ConnectedClient) {
        let Some(popup) = &self.popup else {
            return;
        };
        let database = self
            .backends
            .iter()
            .find(|backend| backend.pid == popup.pid)
            .map(|backend| backend.database.clone())
            .unwrap_or_default();
        let raw_query = popup
            .details
            .as_ref()
            .map(|details| details.raw_query.clone())
            .unwrap_or_default();
        let result =
            explain::statement(&raw_query, self.server.query_size_limit).and_then(|statement| {
                self.in_database(client, &database, |session| {
                    explain::fetch(session, &statement)
                })
            });
        if let Some(popup) = &mut self.popup {
            popup.explain = Some(ExplainPanel {
                database,
                result,
                scroll: 0,
            });
        }
    }

    /// Runs `work` in `database`. Catalogs such as `pg_proc` and the tables a
    /// query names are per database, so another database gets a short
    /// read-only session that closes when `work` returns.
    fn in_database<T>(
        &self,
        client: &ConnectedClient,
        database: &str,
        work: impl FnOnce(&ConnectedClient) -> Result<T, String>,
    ) -> Result<T, String> {
        if database.is_empty() {
            return Err("The backend has ended, so its database is unknown.".to_owned());
        }
        if database == self.server.database {
            return work(client);
        }
        match client.for_database(database, SOURCE, &mut Vec::new()) {
            Ok(session) => work(&session),
            Err(_) => Err(format!("Could not connect to database {database}.")),
        }
    }

    fn refresh_popup(&mut self, client: &ConnectedClient) -> Result<(), TopError> {
        let Some(popup) = &mut self.popup else {
            return Ok(());
        };
        match fetch_details(client, self.server.version_num, &popup.pid)? {
            Some(details) => popup.details = Some(details),
            None => popup.ended = true,
        }
        if self.os.is_some() && !popup.ended {
            popup.memory = os::read_memory(&popup.pid);
        }
        Ok(())
    }

    fn visible(&self) -> Vec<&Backend> {
        self.backends
            .iter()
            .filter(|backend| match self.view {
                View::Running => !backend.state.is_empty() && backend.state != "idle",
                View::Waiting => backend.waits_on_lock(),
                View::Blocking => self.blocks.contains_key(&backend.pid),
            })
            .collect()
    }

    fn select_by(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.table.select(Some(0));
            return;
        }
        let current = self.table.selected().unwrap_or(0) as isize;
        let next = current.saturating_add(delta).clamp(0, len as isize - 1);
        self.table.select(Some(next as usize));
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [header, title, body, footer] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        frame.render_widget(Paragraph::new(self.header_lines(header.width)), header);
        let count = self.visible().len();
        frame.render_widget(
            Line::from(format!("{} ({count})", self.view.title()))
                .bold()
                .green()
                .alignment(Alignment::Center),
            title,
        );
        self.draw_table(frame, body);
        frame.render_widget(footer_line(self.view), footer);
        if self.popup.is_some() {
            self.draw_popup(frame);
            self.draw_function_panel(frame);
            self.draw_explain_panel(frame);
        }
        if self.help {
            draw_help(frame);
        }
    }

    fn header_lines(&self, width: u16) -> Vec<Line<'static>> {
        let server = &self.server;
        let mut info = vec![
            Span::styled(
                concat!(" pg-probe ", env!("CARGO_PKG_VERSION"), " "),
                Style::new().black().on_green().bold(),
            ),
            Span::raw(" "),
            Span::styled(
                format!("PostgreSQL {}", server.version),
                Style::new().bold(),
            ),
            Span::raw(" - "),
            Span::styled(
                format!("{}@{}/{}", server.user, server.endpoint, server.database),
                Style::new().cyan(),
            ),
            Span::raw(format!(" - Ref.: {}s", self.interval.as_secs())),
        ];
        if self.paused {
            info.push(Span::styled(" - PAUSED", Style::new().bold().red()));
        }

        let counts = SessionCounts::of(&self.backends);
        let number = |value: usize| Span::styled(value.to_string(), Style::new().bold().green());
        let dot = || Span::raw(" · ");
        let sessions = Line::from(vec![
            Span::raw(" * Sessions: "),
            number(counts.total),
            Span::raw(format!("/{} total", server.max_connections)),
            dot(),
            number(counts.active),
            Span::raw(" active"),
            dot(),
            number(counts.idle),
            Span::raw(" idle"),
            dot(),
            number(counts.idle_in_transaction),
            Span::raw(" idle in txn"),
            dot(),
            number(counts.idle_in_transaction_aborted),
            Span::raw(" idle in txn abrt"),
            dot(),
            number(counts.waiting),
            Span::raw(" waiting"),
        ]);

        vec![
            Line::from(info),
            sessions,
            self.os_line(),
            session_bar(&counts, server.max_connections, usize::from(width)),
        ]
    }

    fn os_line(&self) -> Line<'static> {
        let Some(os) = &self.os else {
            return Line::from(Span::styled(
                " * OS: not shown, the server is not on this host",
                Style::new().dark_gray(),
            ));
        };
        let value = |text: String| Span::styled(text, Style::new().bold().green());
        let dot = || Span::raw(" · ");
        let mut spans = vec![Span::raw(" * Load: ")];
        match &os.host.load {
            Some(load) => spans.push(value(format!(
                "{:.2} {:.2} {:.2}",
                load.load_1m, load.load_5m, load.load_15m
            ))),
            None => spans.push(Span::raw("-")),
        }
        if let Some(memory) = os.host.memory {
            spans.extend([
                dot(),
                Span::raw("Mem: "),
                value(format_bytes(memory.total as f64)),
                Span::raw(" total "),
                value(format_bytes(memory.available as f64)),
                Span::raw(" avail "),
                value(format_bytes(memory.buff_cached as f64)),
                Span::raw(" buff+cached"),
                dot(),
                Span::raw("Swap: "),
                value(format_bytes(memory.swap_total as f64)),
                Span::raw(" total "),
                value(format_bytes(
                    memory.swap_total.saturating_sub(memory.swap_free) as f64,
                )),
                Span::raw(" used"),
            ]);
        }
        if os.io_denied() {
            spans.push(Span::styled(
                " · per-backend I/O needs root or the postgres user",
                Style::new().yellow(),
            ));
        }
        Line::from(spans)
    }

    fn draw_table(&mut self, frame: &mut Frame, area: Rect) {
        let last_column = if self.view == View::Blocking {
            "BLOCKS"
        } else {
            "WAITING"
        };
        let mut titles = vec!["PID", "DATABASE", "USER"];
        let mut widths = vec![
            Constraint::Length(7),
            Constraint::Length(16),
            Constraint::Length(12),
        ];
        if self.os.is_some() {
            titles.extend(["CPU%", "READ/s", "WRITE/s", "IOW"]);
            widths.extend([
                Constraint::Length(6),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(3),
            ]);
        }
        titles.extend(["STATE", "TIME+", last_column, "QUERY"]);
        widths.extend([
            Constraint::Length(20),
            Constraint::Length(10),
            Constraint::Length(22),
            Constraint::Fill(1),
        ]);
        let header = Row::new(titles.into_iter().map(Cell::from))
            .style(Style::new().black().on_green().bold());

        let rows: Vec<Row> = self
            .visible()
            .into_iter()
            .map(|backend| {
                let wait = if self.view == View::Blocking {
                    Span::styled(
                        self.blocks
                            .get(&backend.pid)
                            .map(|pids| pids.join(","))
                            .unwrap_or_default(),
                        Style::new().red().bold(),
                    )
                } else {
                    let style = if backend.waits_on_lock() {
                        Style::new().red().bold()
                    } else {
                        Style::new().yellow()
                    };
                    Span::styled(backend.wait_label(), style)
                };
                let mut cells = vec![
                    Cell::from(Span::styled(backend.pid.clone(), Style::new().cyan())),
                    Cell::from(Span::styled(
                        backend.database.clone(),
                        Style::new().dark_gray().bold(),
                    )),
                    Cell::from(backend.user.clone()),
                ];
                if let Some(os) = &self.os {
                    let rate = os.rates.get(&backend.pid);
                    let right =
                        |text: String| Cell::from(Line::from(text).alignment(Alignment::Right));
                    let bytes = |value: Option<f64>| {
                        value.map(format_bytes).unwrap_or_else(|| "-".to_owned())
                    };
                    cells.extend([
                        right(
                            rate.and_then(|rate| rate.cpu_percent)
                                .map(|cpu| format!("{cpu:.1}"))
                                .unwrap_or_else(|| "-".to_owned()),
                        ),
                        right(bytes(rate.and_then(|rate| rate.read_per_s))),
                        right(bytes(rate.and_then(|rate| rate.write_per_s))),
                        match rate.map(|rate| rate.io_wait) {
                            Some(true) => Cell::from(Span::styled(" Y", Style::new().red().bold())),
                            Some(false) => Cell::from(Span::styled(" N", Style::new().green())),
                            None => Cell::from(" -"),
                        },
                    ]);
                }
                cells.extend([
                    Cell::from(Span::styled(
                        backend.state.clone(),
                        state_style(&backend.state),
                    )),
                    Cell::from(
                        Line::from(Span::styled(
                            backend.duration_s.map(format_duration).unwrap_or_default(),
                            duration_style(backend.duration_s),
                        ))
                        .alignment(Alignment::Right),
                    ),
                    Cell::from(wait),
                    Cell::from(backend.query.clone()),
                ]);
                Row::new(cells)
            })
            .collect();

        let table = Table::new(rows, widths)
            .header(header)
            .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(table, area, &mut self.table);
    }

    fn draw_popup(&self, frame: &mut Frame) {
        let Some(popup) = &self.popup else {
            return;
        };
        let backend = self
            .backends
            .iter()
            .find(|backend| backend.pid == popup.pid);
        let details = popup.details.clone().unwrap_or_default();
        let label = |text: &str| Span::styled(format!("{text:<16}"), Style::new().cyan().bold());
        let field = |name: &str, value: Vec<Span<'static>>| {
            let mut spans = vec![label(name)];
            spans.extend(value);
            Line::from(spans)
        };
        let plain = |text: String| vec![Span::raw(text)];
        let age = |seconds: Option<f64>, since: &str| {
            let mut spans = vec![Span::styled(
                seconds
                    .map(format_duration)
                    .unwrap_or_else(|| "-".to_owned()),
                duration_style(seconds).bold(),
            )];
            if !since.is_empty() {
                spans.push(Span::styled(
                    format!("  (since {since})"),
                    Style::new().dark_gray(),
                ));
            }
            spans
        };

        let mut lines = Vec::new();
        if popup.ended {
            lines.push(Line::from(Span::styled(
                "This backend has ended. Showing its last known details.",
                Style::new().yellow().bold(),
            )));
            lines.push(Line::default());
        }
        let state = backend
            .map(|backend| backend.state.clone())
            .unwrap_or_default();
        lines.extend([
            field(
                "Database",
                plain(backend.map(|b| b.database.clone()).unwrap_or_default()),
            ),
            field(
                "User",
                plain(backend.map(|b| b.user.clone()).unwrap_or_default()),
            ),
            field("Application", plain(details.application.clone())),
            field(
                "Client",
                plain(if details.client.is_empty() {
                    "local socket".to_owned()
                } else {
                    details.client.clone()
                }),
            ),
            field("Backend type", plain(details.backend_type.clone())),
            field(
                "State",
                vec![Span::styled(state.clone(), state_style(&state))],
            ),
            field(
                "Waiting",
                vec![Span::styled(
                    backend.map(Backend::wait_label).unwrap_or_default(),
                    Style::new().red().bold(),
                )],
            ),
            Line::default(),
            field(
                "Connection age",
                age(details.connection_age_s, &details.backend_start),
            ),
            field(
                "Transaction age",
                age(details.xact_age_s, &details.xact_start),
            ),
            field("Query duration", age(details.query_age_s, "")),
            field("In this state", age(details.state_age_s, "")),
        ]);
        if !details.xid.is_empty() || !details.xmin.is_empty() {
            lines.push(field(
                "xid / xmin",
                plain(format!(
                    "{} / {}",
                    or_dash(&details.xid),
                    or_dash(&details.xmin)
                )),
            ));
        }

        if let Some(os) = &self.os {
            let rate = os.rates.get(&popup.pid);
            let sample = os.samples.get(&popup.pid).map(|(_, sample)| sample);
            lines.push(Line::default());
            lines.push(field(
                "CPU",
                plain(format!(
                    "{} now · {} total",
                    rate.and_then(|rate| rate.cpu_percent)
                        .map(|cpu| format!("{cpu:.1}%"))
                        .unwrap_or_else(|| "-".to_owned()),
                    sample
                        .map(|sample| format_duration(
                            sample.cpu_ticks as f64 / os.ticks_per_second
                        ))
                        .unwrap_or_else(|| "-".to_owned()),
                )),
            ));
            lines.push(field(
                "Memory",
                plain(match popup.memory {
                    Some(memory) => match memory.anonymous {
                        Some(anonymous) => format!(
                            "{} private · {} resident (includes shared buffers)",
                            format_bytes(anonymous as f64),
                            format_bytes(memory.rss as f64)
                        ),
                        None => format!(
                            "{} resident (includes shared buffers)",
                            format_bytes(memory.rss as f64)
                        ),
                    },
                    None => "-".to_owned(),
                }),
            ));
            let per_second = |value: Option<f64>| {
                value
                    .map(|value| format!("{}/s", format_bytes(value)))
                    .unwrap_or_else(|| "-".to_owned())
            };
            lines.push(field(
                "I/O now",
                plain(format!(
                    "read {} · write {}",
                    per_second(rate.and_then(|rate| rate.read_per_s)),
                    per_second(rate.and_then(|rate| rate.write_per_s)),
                )),
            ));
            lines.push(field(
                "I/O total",
                plain(match sample.and_then(|sample| sample.io) {
                    Some((read, written)) => format!(
                        "read {} · write {}",
                        format_bytes(read as f64),
                        format_bytes(written as f64)
                    ),
                    None => "-".to_owned(),
                }),
            ));
            lines.push(field(
                "I/O wait",
                match rate.map(|rate| rate.io_wait) {
                    Some(true) => vec![Span::styled("yes", Style::new().red().bold())],
                    Some(false) => plain("no".to_owned()),
                    None => plain("-".to_owned()),
                },
            ));
        }

        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Query",
            Style::new().cyan().bold(),
        )));
        lines.extend(
            query_lines(&details, self.server.query_size_limit)
                .into_iter()
                .map(Line::from),
        );

        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((popup.scroll, 0))
                .block(
                    Block::bordered()
                        .title(Line::from(vec![
                            Span::styled(
                                concat!(" pg-probe ", env!("CARGO_PKG_VERSION"), " "),
                                Style::new().black().on_green().bold(),
                            ),
                            Span::styled(
                                format!(" query details · PID {} ", popup.pid),
                                Style::new().bold().green(),
                            ),
                        ]))
                        .title_bottom(
                            Line::from(" v/Esc back · f function · e explain · ↑↓ scroll ")
                                .dark_gray(),
                        ),
                ),
            area,
        );
    }
}

impl App {
    fn draw_function_panel(&self, frame: &mut Frame) {
        let Some(panel) = self
            .popup
            .as_ref()
            .and_then(|popup| popup.function.as_ref())
        else {
            return;
        };
        let (lines, position) = match &panel.result {
            Err(message) => (
                vec![Line::from(Span::styled(
                    message.clone(),
                    Style::new().red(),
                ))],
                String::new(),
            ),
            Ok(functions) if functions.is_empty() => (
                vec![Line::from("No user function call found in this query.")],
                String::new(),
            ),
            Ok(functions) => (
                function::lines(&functions[panel.index]),
                format!(
                    " · ←→ other matches ({}/{})",
                    panel.index + 1,
                    functions.len()
                ),
            ),
        };
        let area = centered(frame.area(), 90, 80);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((panel.scroll, 0))
                .block(
                    Block::bordered()
                        .title(
                            Line::from(format!(" function in {} ", panel.database))
                                .bold()
                                .green(),
                        )
                        .title_bottom(
                            Line::from(format!(" f/Esc back · ↑↓ scroll{position} ")).dark_gray(),
                        ),
                ),
            area,
        );
    }
}

impl App {
    fn draw_explain_panel(&self, frame: &mut Frame) {
        let Some(panel) = self.popup.as_ref().and_then(|popup| popup.explain.as_ref()) else {
            return;
        };
        let lines: Vec<Line> = match &panel.result {
            Err(message) => vec![Line::from(Span::styled(
                message.clone(),
                Style::new().red(),
            ))],
            Ok(plan) => {
                let mut lines = vec![
                    Line::from(Span::styled(
                        "Estimated plan (EXPLAIN without ANALYZE). The query was not run, \
                         and the plan may differ from the one the backend is running.",
                        Style::new().yellow(),
                    )),
                    Line::default(),
                ];
                lines.extend(
                    plan.iter()
                        .map(|line| Line::from(single_line_keep_indent(line))),
                );
                lines
            }
        };
        let area = centered(frame.area(), 90, 80);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((panel.scroll, 0))
                .block(
                    Block::bordered()
                        .title(
                            Line::from(format!(" explain in {} ", panel.database))
                                .bold()
                                .green(),
                        )
                        .title_bottom(Line::from(" e/Esc back · ↑↓ scroll ").dark_gray()),
                ),
            area,
        );
    }
}

/// Every key, grouped by where it works. The footer has room for a few.
const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Views",
        &[
            ("F1 1", "Running queries"),
            ("F2 2", "Waiting queries"),
            ("F3 3", "Blocking queries"),
        ],
    ),
    (
        "List",
        &[
            ("↑ ↓ j k", "Move the selection"),
            ("PgUp PgDn", "Move one page"),
            ("Home End", "First or last row"),
            ("v Enter", "Query details"),
            ("Space", "Pause or resume refreshes"),
            ("?", "This help"),
            ("q Esc", "Quit"),
        ],
    ),
    (
        "Query details",
        &[
            ("f", "Functions the query calls"),
            ("e", "Estimated plan (EXPLAIN)"),
            ("↑ ↓ PgUp PgDn", "Scroll"),
            ("v Esc Enter", "Back to the list"),
        ],
    ),
];

fn draw_help(frame: &mut Frame) {
    let mut lines = Vec::new();
    for (section, keys) in HELP {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            *section,
            Style::new().cyan().bold(),
        )));
        for (key, action) in *keys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<16}"), Style::new().bold()),
                Span::raw(*action),
            ]));
        }
    }
    let area = centered(frame.area(), 60, 80);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(Line::from(" keys ").bold().green())
                .title_bottom(Line::from(" any key closes ").dark_gray()),
        ),
        area,
    );
}

/// Drops control characters from one plan line but keeps its leading
/// spaces, which carry the plan's tree shape.
fn single_line_keep_indent(line: &str) -> String {
    line.chars().filter(|c| !c.is_control()).collect()
}

fn or_dash(value: &str) -> &str {
    if value.is_empty() {
        "-"
    } else {
        value
    }
}

/// A rectangle of the given percentages, centered in `area`.
fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let [_, middle, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Fill(1),
    ])
    .areas(middle);
    center
}

fn is_local(endpoint: &EndpointIdentity) -> bool {
    endpoint.endpoint_kind == EndpointKind::UnixSocket
        || matches!(endpoint.host.as_str(), "localhost" | "127.0.0.1" | "::1")
}

/// Bytes with a binary unit suffix, as top tools print them.
fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0}B")
    } else {
        format!("{value:.2}{}", UNITS[unit])
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SessionCounts {
    total: usize,
    active: usize,
    idle: usize,
    idle_in_transaction: usize,
    idle_in_transaction_aborted: usize,
    waiting: usize,
}

impl SessionCounts {
    fn of(backends: &[Backend]) -> Self {
        let mut counts = Self {
            total: backends.len(),
            ..Self::default()
        };
        for backend in backends {
            match backend.state.as_str() {
                "active" => counts.active += 1,
                "idle" => counts.idle += 1,
                "idle in transaction" => counts.idle_in_transaction += 1,
                "idle in transaction (aborted)" => counts.idle_in_transaction_aborted += 1,
                _ => {}
            }
            if backend.waits_on_lock() {
                counts.waiting += 1;
            }
        }
        counts
    }
}

/// An htop-style meter of connection slots: waiting backends in red, other
/// active ones in green, idle in transaction in yellow, idle in gray.
fn session_bar(counts: &SessionCounts, max_connections: usize, width: usize) -> Line<'static> {
    let label = " Conn[";
    let suffix = format!("{}/{}]", counts.total, max_connections);
    let inner = width.saturating_sub(label.len() + suffix.len() + 1);
    let max = max_connections.max(counts.total).max(1);
    let cells = |count: usize| count * inner / max;

    let waiting = cells(counts.waiting);
    let active = cells(counts.active.saturating_sub(counts.waiting));
    let in_transaction = cells(counts.idle_in_transaction + counts.idle_in_transaction_aborted);
    let idle = cells(counts.idle);
    let used = (waiting + active + in_transaction + idle).min(inner);

    Line::from(vec![
        Span::styled(label, Style::new().cyan().bold()),
        Span::styled("|".repeat(waiting), Style::new().red()),
        Span::styled("|".repeat(active), Style::new().green()),
        Span::styled("|".repeat(in_transaction), Style::new().yellow()),
        Span::styled("|".repeat(idle), Style::new().dark_gray()),
        Span::raw(" ".repeat(inner - used)),
        Span::styled(suffix, Style::new().dark_gray()),
    ])
}

fn footer_line(view: View) -> Line<'static> {
    let keys = [
        ("F1", "Running queries", Some(View::Running)),
        ("F2", "Waiting queries", Some(View::Waiting)),
        ("F3", "Blocking queries", Some(View::Blocking)),
        ("v", "Query details", None),
        ("Space", "Pause/unpause", None),
        ("?", "Help", None),
        ("q", "Quit", None),
    ];
    let mut spans = Vec::new();
    for (key, label, target) in keys {
        spans.push(Span::raw(key));
        let mut style = Style::new().black().on_cyan();
        if target == Some(view) {
            style = style.bold();
        }
        spans.push(Span::styled(format!("{label:<18}"), style));
    }
    Line::from(spans)
}

fn state_style(state: &str) -> Style {
    match state {
        "active" => Style::new().green(),
        "idle in transaction" => Style::new().yellow(),
        "idle in transaction (aborted)" => Style::new().red(),
        _ => Style::new(),
    }
}

fn duration_style(duration_s: Option<f64>) -> Style {
    match duration_s {
        Some(seconds) if seconds >= 3.0 => Style::new().red(),
        Some(seconds) if seconds >= 1.0 => Style::new().yellow(),
        _ => Style::new().green(),
    }
}

/// Seconds below a minute, `m:ss` below an hour, then `h:mm:ss`.
fn format_duration(seconds: f64) -> String {
    if seconds < 60.0 {
        return format!("{:.3}", seconds.max(0.0));
    }
    let whole = seconds as u64;
    let (hours, minutes, secs) = (whole / 3600, whole / 60 % 60, whole % 60);
    if hours == 0 {
        format!("{minutes}:{secs:02}")
    } else {
        format!("{hours}:{minutes:02}:{secs:02}")
    }
}

fn read_server(client: &ConnectedClient) -> Result<Server, AdapterError> {
    let row = client.exec_bounded_row(SERVER_SQL, SOURCE, &mut Vec::new())?;
    let text = |index: usize| {
        row.get(index)
            .map(|value| text_of(value))
            .unwrap_or_default()
    };
    let version_num = text(0)
        .parse::<i32>()
        .map_err(|_| AdapterError::ColumnInvalid)?;
    let endpoint = client.endpoint();
    let endpoint = match (endpoint.endpoint_kind, endpoint.port) {
        (EndpointKind::Tcp, Some(port)) => format!("{}:{port}", endpoint.host),
        _ => endpoint.host.clone(),
    };
    Ok(Server {
        version_num,
        version: text(1),
        max_connections: text(2).parse().unwrap_or(0),
        query_size_limit: text(5).parse().unwrap_or(0),
        user: text(3),
        database: text(4),
        endpoint,
    })
}

fn fetch_backends(client: &ConnectedClient, sql: &str) -> Result<Vec<Backend>, AdapterError> {
    let rows = client.exec_bounded_rows(sql, SOURCE, &mut Vec::new())?;
    Ok(rows
        .rows
        .into_iter()
        .map(|row| {
            let text = |index: usize| {
                row.get(index)
                    .map(|value| text_of(value))
                    .unwrap_or_default()
            };
            Backend {
                pid: text(0),
                database: text(1),
                user: text(2),
                state: text(3),
                wait_type: text(4),
                wait_event: text(5),
                duration_s: text(6).parse().ok(),
                query: text(7),
            }
        })
        .collect())
}

fn fetch_details(
    client: &ConnectedClient,
    server_version_num: i32,
    pid: &str,
) -> Result<Option<Details>, AdapterError> {
    let Some(sql) = details_sql(server_version_num, pid) else {
        return Ok(None);
    };
    let rows = client.exec_bounded_rows(&sql, SOURCE, &mut Vec::new())?;
    Ok(rows.rows.into_iter().next().map(|row| {
        let text = |index: usize| {
            row.get(index)
                .map(|value| text_of(value))
                .unwrap_or_default()
        };
        let client = match (text(1), text(2)) {
            (address, _) if address.is_empty() => String::new(),
            (address, port) if port.is_empty() || port == "-1" => address,
            (address, port) => format!("{address}:{port}"),
        };
        Details {
            application: text(0),
            client,
            backend_type: text(3),
            backend_start: text(4),
            connection_age_s: text(5).parse().ok(),
            xact_start: text(6),
            xact_age_s: text(7).parse().ok(),
            query_age_s: text(8).parse().ok(),
            state_age_s: text(9).parse().ok(),
            xid: text(10),
            xmin: text(11),
            query: text(12),
            raw_query: row
                .get(12)
                .map(|value| String::from_utf8_lossy(value).into_owned())
                .unwrap_or_default(),
        }
    }))
}

fn fetch_blocks(
    client: &ConnectedClient,
    sql: &str,
) -> Result<HashMap<String, Vec<String>>, AdapterError> {
    let rows = client.exec_bounded_rows(sql, SOURCE, &mut Vec::new())?;
    let mut blocks: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows.rows {
        let blocked = row.first().map(|value| text_of(value)).unwrap_or_default();
        let blockers = row.get(1).map(|value| text_of(value)).unwrap_or_default();
        for blocker in blockers.split(',').filter(|pid| !pid.is_empty()) {
            blocks
                .entry(blocker.to_owned())
                .or_default()
                .push(blocked.clone());
        }
    }
    Ok(blocks)
}

/// Decodes a server value for display: whitespace collapses to single spaces
/// and control characters are dropped, so text from the server can neither
/// break the layout nor send escape sequences to the terminal.
/// Lays the query out one clause per line for the details panel. Text the
/// server cut at `track_activity_query_size` stays on one line, since the
/// formatter would indent a broken statement.
fn query_lines(details: &Details, size_limit: usize) -> Vec<String> {
    let raw = &details.raw_query;
    if raw.trim().is_empty() || (size_limit > 0 && raw.len() + 1 >= size_limit) {
        return vec![details.query.clone()];
    }
    let options = FormatOptions {
        dialect: Dialect::PostgreSql,
        ..FormatOptions::default()
    };
    sqlformat::format(raw, &QueryParams::None, &options)
        .lines()
        .map(|line| line.chars().filter(|c| !c.is_control()).collect())
        .collect()
}

fn text_of(value: &[u8]) -> String {
    single_line(&String::from_utf8_lossy(value))
}

fn single_line(value: &str) -> String {
    value
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn backend(pid: &str, state: &str, wait_type: &str, query: &str) -> Backend {
        Backend {
            pid: pid.to_owned(),
            database: "bench".to_owned(),
            user: "postgres".to_owned(),
            state: state.to_owned(),
            wait_type: wait_type.to_owned(),
            wait_event: String::new(),
            duration_s: Some(0.115),
            query: query.to_owned(),
        }
    }

    fn app(view: View) -> App {
        let server = Server {
            version_num: 90325,
            version: "9.3.25".to_owned(),
            max_connections: 100,
            user: "postgres".to_owned(),
            database: "postgres".to_owned(),
            endpoint: "127.0.0.1:5432".to_owned(),
            query_size_limit: 1024,
        };
        let mut app = App::new(server, Duration::from_secs(2), false);
        app.view = view;
        app.backends = vec![
            backend("101", "active", "", "SELECT pg_sleep(5)"),
            backend("102", "active", "Lock", "UPDATE t SET id = 1"),
            backend("103", "idle", "", ""),
        ];
        app
    }

    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 13)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn query_text_is_flattened_and_stripped_of_escape_sequences() {
        assert_eq!(
            text_of(b"UPDATE t\n  SET x = 1\x1b[2J"),
            "UPDATE t SET x = 1[2J"
        );
    }

    #[test]
    fn details_query_is_formatted_unless_the_server_cut_it() {
        let details = Details {
            query: "SELECT a FROM t WHERE b = 1".to_owned(),
            raw_query: "select a from t where b = 1\x1b[2J".to_owned(),
            ..Details::default()
        };
        let lines = query_lines(&details, 1024);
        assert_eq!(lines[0], "select");
        assert!(lines.contains(&"  a".to_owned()), "{lines:?}");
        assert!(lines.iter().all(|line| !line.contains('\x1b')));
        assert_eq!(query_lines(&details, 32), vec![details.query.clone()]);
    }

    #[test]
    fn older_servers_get_a_query_without_newer_columns() {
        let legacy = backends_sql(90325);
        assert!(!legacy.contains("backend_type"));
        assert!(legacy.contains("CASE WHEN waiting"));
        let modern = backends_sql(140005);
        assert!(modern.contains("backend_type = 'client backend'"));
        assert!(!modern.contains("waiting"));
    }

    #[test]
    fn durations_switch_to_clock_format_after_a_minute() {
        assert_eq!(format_duration(0.1146), "0.115");
        assert_eq!(format_duration(125.0), "2:05");
        assert_eq!(format_duration(3_725.0), "1:02:05");
    }

    #[test]
    fn running_view_hides_idle_backends_and_counts_sessions() {
        let screen = screen(&mut app(View::Running));
        assert!(screen.contains("RUNNING QUERIES (2)"), "{screen}");
        assert!(screen.contains("SELECT pg_sleep(5)"));
        assert!(!screen.contains("103"));
        assert!(screen.contains("3/100 total"));
        assert!(screen.contains(concat!(" pg-probe ", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn waiting_view_lists_only_lock_waits() {
        let screen = screen(&mut app(View::Waiting));
        assert!(screen.contains("WAITING QUERIES (1)"), "{screen}");
        assert!(screen.contains("UPDATE t SET id = 1"));
        assert!(!screen.contains("pg_sleep"));
    }

    #[test]
    fn local_servers_show_host_and_per_backend_columns() {
        let mut app = app(View::Running);
        let mut os = OsState::new();
        os.rates.insert(
            "101".to_owned(),
            ProcessRates {
                cpu_percent: Some(12.5),
                read_per_s: Some(2048.0),
                write_per_s: Some(0.0),
                io_wait: true,
            },
        );
        app.os = Some(os);
        let screen = screen(&mut app);
        assert!(screen.contains("CPU%"), "{screen}");
        assert!(screen.contains("12.5"));
        assert!(screen.contains("2.00K"));
        assert!(screen.contains(" Y"));
    }

    #[test]
    fn remote_servers_hide_os_columns() {
        let screen = screen(&mut app(View::Running));
        assert!(!screen.contains("CPU%"), "{screen}");
        assert!(screen.contains("server is not on this host"));
    }

    #[test]
    fn bytes_use_binary_units() {
        assert_eq!(format_bytes(512.0), "512B");
        assert_eq!(format_bytes(378_572.8), "369.70K");
        assert_eq!(format_bytes(6_098_812_928.0), "5.68G");
    }

    #[test]
    fn blocking_view_uses_pg_locks_before_9_6() {
        assert!(blocking_sql(90325).contains("FROM l w"));
        assert!(!blocking_sql(90325).contains("pg_blocking_pids"));
        assert!(blocking_sql(90600).contains("pg_blocking_pids"));
    }

    #[test]
    fn details_sql_only_accepts_numeric_pids() {
        assert!(details_sql(90325, "1; DROP TABLE t").is_none());
        let legacy = details_sql(90325, "4242").unwrap();
        assert!(legacy.ends_with("WHERE pid = 4242"));
        assert!(!legacy.contains("backend_xid"));
        assert!(details_sql(140005, "4242").unwrap().contains("backend_xid"));
    }

    #[test]
    fn help_lists_every_key() {
        let mut app = app(View::Running);
        app.help = true;
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>() + "\n")
            .collect();
        assert!(screen.contains("Blocking queries"), "{screen}");
        assert!(screen.contains("Estimated plan (EXPLAIN)"), "{screen}");
    }

    #[test]
    fn details_panel_shows_the_whole_query_and_ages() {
        let mut app = app(View::Running);
        let long = format!("SELECT {} FROM t", "column_name, ".repeat(20));
        app.backends[0].query = long.clone();
        app.popup = Some(Popup {
            pid: "101".to_owned(),
            details: Some(Details {
                application: "psql".to_owned(),
                connection_age_s: Some(3_725.0),
                backend_start: "2026-10-18 20:01:02".to_owned(),
                query: long,
                ..Details::default()
            }),
            memory: None,
            ended: false,
            scroll: 0,
            function: None,
            explain: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>() + "\n")
            .collect();
        assert!(screen.contains("PID 101"), "{screen}");
        assert!(screen.contains("1:02:05"));
        assert!(screen.contains("since 2026-10-18 20:01:02"));
        assert!(screen.contains("FROM t"));
    }
}
