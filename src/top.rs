//! Live view of the queries running on a PostgreSQL server, in the spirit of
//! pg_activity.
//!
//! Unlike `capture`, this view shows query text, but only on the operator's
//! terminal: nothing is persisted. The session is opened read-only with a
//! statement timeout, and each refresh runs one constant `SELECT` over
//! `pg_stat_activity`. The blocking view adds `pg_blocking_pids`, which takes
//! lock-manager locks, so it only runs while that view is open. The vacuum
//! view reads `pg_stat_progress_vacuum`, also only while it is open.
//!
//! When the server runs on this host, host load and memory and per-backend
//! CPU and I/O come from the local `/proc`, which costs the server nothing.

mod explain;
mod function;
mod os;
mod settings;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use serde::{Deserialize, Serialize};
use sqlformat::{Dialect, FormatOptions, QueryParams};
use thiserror::Error;

use settings::Settings;

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

/// Whether `pg_stat_progress_vacuum` exists (PG 9.6+).
const fn has_vacuum_progress(server_version_num: i32) -> bool {
    server_version_num >= 90600
}

/// Builds the vacuum view's `SELECT`: every manual and autovacuum run in
/// progress. The table name resolves only in the connected database, since
/// `regclass` reads the local catalog; other databases show the table OID.
/// Before PG 9.6 there is no progress view, so the sessions come from
/// `pg_stat_activity` without phase or progress.
fn vacuum_sql(server_version_num: i32) -> String {
    let duration = "round(EXTRACT(EPOCH FROM (clock_timestamp() - a.query_start))::numeric, 3)";
    let autovacuum = if has_backend_type(server_version_num).is_supported() {
        "a.backend_type = 'autovacuum worker'"
    } else {
        "a.query LIKE 'autovacuum:%'"
    };
    let kind = format!(
        "CASE WHEN {autovacuum} THEN \
            CASE WHEN a.query LIKE '%(to prevent wraparound)' THEN 'wraparound' ELSE 'auto' END \
        ELSE 'manual' END"
    );
    if has_vacuum_progress(server_version_num) {
        format!(
            "\
SELECT a.pid, a.datname, a.usename, a.state, a.wait_event_type, a.wait_event, \
    {duration}, a.query, \
    CASE WHEN p.datname = current_database() THEN p.relid::regclass::text \
        ELSE p.relid::text END, \
    {kind}, p.phase, p.heap_blks_scanned, p.heap_blks_total, p.index_vacuum_count \
FROM pg_stat_progress_vacuum p \
JOIN pg_stat_activity a ON a.pid = p.pid \
ORDER BY a.query_start ASC NULLS LAST"
        )
    } else {
        format!(
            "\
SELECT a.pid, a.datname, a.usename, a.state, \
    CASE WHEN a.waiting THEN 'Lock' END, NULL::text, \
    {duration}, a.query, NULL::text, {kind}, NULL::text, NULL, NULL, NULL \
FROM pg_stat_activity a \
WHERE a.query LIKE 'autovacuum:%' OR a.query ~* '^[[:space:]]*vacuum' \
ORDER BY a.query_start ASC NULLS LAST"
        )
    }
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Backend {
    pub pid: String,
    pub database: String,
    pub user: String,
    pub state: String,
    pub wait_type: String,
    pub wait_event: String,
    pub duration_s: Option<f64>,
    pub query: String,
    /// The query with its line breaks, for the preview panel.
    pub raw_query: String,
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

/// One vacuum in progress. Its session fields fill `backend`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vacuum {
    pub backend: Backend,
    /// The table name in the connected database, its OID in any other.
    pub table: String,
    /// `auto`, `wraparound`, or `manual`.
    pub kind: String,
    pub phase: String,
    pub heap_scanned: Option<f64>,
    pub heap_total: Option<f64>,
    pub index_passes: String,
}

impl Vacuum {
    /// The share of heap blocks scanned, as `pg_stat_progress_vacuum`
    /// counts them: `heap_blks_scanned / heap_blks_total`.
    fn scanned(&self) -> String {
        match (self.heap_scanned, self.heap_total) {
            (Some(scanned), Some(total)) if total > 0.0 => {
                format!("{:.1}%", scanned / total * 100.0)
            }
            _ => "-".to_owned(),
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
    Vacuum,
}

impl View {
    const fn title(self) -> &'static str {
        match self {
            Self::Running => "RUNNING QUERIES",
            Self::Waiting => "WAITING QUERIES",
            Self::Blocking => "BLOCKING QUERIES",
            Self::Vacuum => "VACUUM",
        }
    }
}

/// What the list groups sessions by, cycled with `g`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Group {
    #[default]
    None,
    State,
    Database,
    User,
}

impl Group {
    const fn next(self) -> Self {
        match self {
            Self::None => Self::State,
            Self::State => Self::Database,
            Self::Database => Self::User,
            Self::User => Self::None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::None => "",
            Self::State => "state",
            Self::Database => "database",
            Self::User => "user",
        }
    }

    fn key(self, backend: &Backend) -> Option<&str> {
        match self {
            Self::None => None,
            Self::State => Some(&backend.state),
            Self::Database => Some(&backend.database),
            Self::User => Some(&backend.user),
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
    /// The selected query, formatted under the list. Toggled with Tab.
    preview: bool,
    /// The blocking view as a tree of who waits on whom. Toggled with `t`.
    tree: bool,
    /// USER and DATABASE in one column and short states, so the query gets
    /// more room. Toggled with `c`.
    compact: bool,
    group: Group,
    /// A two-line header with session history. Toggled with `h`.
    compact_header: bool,
    /// Active and waiting counts of the last refreshes, oldest first. Kept
    /// in memory only, for the compact header.
    history: VecDeque<(usize, usize)>,
    /// Where display options are saved when they change. `None` keeps them
    /// for this run only.
    settings_path: Option<PathBuf>,
    popup: Option<Popup>,
    backends: Vec<Backend>,
    /// Blocker pid to the pids it blocks. Filled only in the blocking view.
    blocks: HashMap<String, Vec<String>>,
    /// Vacuums in progress. Filled only in the vacuum view.
    vacuums: Vec<Vacuum>,
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
    let settings_path = settings::path();
    if let Some(path) = &settings_path {
        app.apply(settings::load(path));
    }
    app.settings_path = settings_path;
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
            preview: false,
            tree: false,
            compact: false,
            group: Group::None,
            compact_header: false,
            history: VecDeque::new(),
            settings_path: None,
            popup: None,
            backends: Vec::new(),
            blocks: HashMap::new(),
            vacuums: Vec::new(),
            os: local.then(OsState::new),
            table: TableState::default().with_selected(Some(0)),
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            preview: self.preview,
            tree: self.tree,
            compact: self.compact,
            group: self.group,
            compact_header: self.compact_header,
        }
    }

    fn apply(&mut self, settings: Settings) {
        self.preview = settings.preview;
        self.tree = settings.tree;
        self.compact = settings.compact;
        self.group = settings.group;
        self.compact_header = settings.compact_header;
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
                let before = self.settings();
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char(' ') => self.paused = !self.paused,
                    KeyCode::Char('?') => self.help = true,
                    KeyCode::Tab => self.preview = !self.preview,
                    KeyCode::Char('c') => self.compact = !self.compact,
                    KeyCode::Char('h') => self.compact_header = !self.compact_header,
                    KeyCode::Char('g') => {
                        self.group = self.group.next();
                        self.table.select(Some(0));
                    }
                    KeyCode::Char('t') => {
                        self.tree = !self.tree;
                        self.table.select(Some(0));
                    }
                    KeyCode::Char('v') | KeyCode::Enter => self.open_popup(client)?,
                    KeyCode::F(1) | KeyCode::Char('1') => self.view = View::Running,
                    KeyCode::F(2) | KeyCode::Char('2') => self.view = View::Waiting,
                    KeyCode::F(3) | KeyCode::Char('3') => self.view = View::Blocking,
                    KeyCode::F(4) | KeyCode::Char('4') => self.view = View::Vacuum,
                    KeyCode::Down | KeyCode::Char('j') => self.select_by(1),
                    KeyCode::Up | KeyCode::Char('k') => self.select_by(-1),
                    KeyCode::PageDown => self.select_by(PAGE as isize),
                    KeyCode::PageUp => self.select_by(-(PAGE as isize)),
                    KeyCode::Home => self.table.select(Some(0)),
                    KeyCode::End => self.select_by(isize::MAX / 2),
                    _ => {}
                }
                let after = self.settings();
                if after != before {
                    if let Some(path) = &self.settings_path {
                        // A read-only home must not stop the live view.
                        let _ = settings::save(path, &after);
                    }
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
        let counts = SessionCounts::of(&self.backends);
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((counts.active, counts.waiting));
        self.vacuums = if self.view == View::Vacuum {
            fetch_vacuums(client, &vacuum_sql(self.server.version_num))?
        } else {
            Vec::new()
        };
        if let Some(os) = &mut self.os {
            // Autovacuum workers are not client backends, so they are not
            // in the backend list.
            let observed: Vec<Backend> = self
                .backends
                .iter()
                .chain(self.vacuums.iter().map(|vacuum| &vacuum.backend))
                .cloned()
                .collect();
            os.refresh(&observed);
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
        let backend = self.backend_by_pid(&popup.pid);
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
            .backend_by_pid(&popup.pid)
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
        self.rows()
            .into_iter()
            .map(|(_, backend)| backend)
            .collect()
    }

    /// A backend of the list or a vacuum session, which may be an
    /// autovacuum worker outside the list.
    fn backend_by_pid(&self, pid: &str) -> Option<&Backend> {
        self.backends
            .iter()
            .chain(self.vacuums.iter().map(|vacuum| &vacuum.backend))
            .find(|backend| backend.pid == pid)
    }

    /// The grouping in effect. The blocking tree and the vacuum view have
    /// their own order.
    fn shown_group(&self) -> Group {
        if self.view == View::Vacuum || self.view == View::Blocking && self.tree {
            Group::None
        } else {
            self.group
        }
    }

    /// The list rows, each with the tree branch drawn before its PID.
    fn rows(&self) -> Vec<(String, &Backend)> {
        if self.view == View::Blocking && self.tree {
            return self.blocking_tree();
        }
        if self.view == View::Vacuum {
            return self
                .vacuums
                .iter()
                .map(|vacuum| (String::new(), &vacuum.backend))
                .collect();
        }
        let mut rows = self
            .backends
            .iter()
            .filter(|backend| match self.view {
                View::Running => !backend.state.is_empty() && backend.state != "idle",
                View::Waiting => backend.waits_on_lock(),
                View::Blocking => self.blocks.contains_key(&backend.pid),
                View::Vacuum => false,
            })
            .map(|backend| (String::new(), backend))
            .collect::<Vec<_>>();
        // The sort is stable, so each group keeps the oldest query first.
        rows.sort_by_key(|(_, backend)| self.shown_group().key(backend));
        rows
    }

    /// Blockers that wait on no one are roots, and each session hangs under
    /// the sessions it waits on. A session with two blockers shows under
    /// both. In a deadlock every blocker also waits, so they become roots.
    fn blocking_tree(&self) -> Vec<(String, &Backend)> {
        let waiting: HashSet<&str> = self.blocks.values().flatten().map(String::as_str).collect();
        let blockers = || {
            self.backends
                .iter()
                .filter(|backend| self.blocks.contains_key(&backend.pid))
        };
        let mut rows = Vec::new();
        for root in blockers().filter(|backend| !waiting.contains(backend.pid.as_str())) {
            self.add_branch(root, "", "", &mut vec![root.pid.as_str()], &mut rows);
        }
        for blocker in blockers() {
            if !rows.iter().any(|(_, backend)| backend.pid == blocker.pid) {
                self.add_branch(blocker, "", "", &mut vec![blocker.pid.as_str()], &mut rows);
            }
        }
        rows
    }

    fn add_branch<'a>(
        &'a self,
        backend: &'a Backend,
        branch: &str,
        indent: &str,
        path: &mut Vec<&'a str>,
        rows: &mut Vec<(String, &'a Backend)>,
    ) {
        rows.push((branch.to_owned(), backend));
        let children: Vec<&Backend> = self
            .blocks
            .get(&backend.pid)
            .into_iter()
            .flatten()
            .filter(|pid| !path.contains(&pid.as_str()))
            .filter_map(|pid| self.backends.iter().find(|backend| &backend.pid == pid))
            .collect();
        for (index, child) in children.iter().enumerate() {
            let last = index + 1 == children.len();
            let branch = format!("{indent}{}", if last { "└─ " } else { "├─ " });
            let indent = format!("{indent}{}", if last { "   " } else { "│  " });
            path.push(&child.pid);
            self.add_branch(child, &branch, &indent, path, rows);
            path.pop();
        }
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
            Constraint::Length(if self.compact_header { 2 } else { 4 }),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let lines = if self.compact_header {
            self.compact_header_lines()
        } else {
            self.header_lines(header.width)
        };
        frame.render_widget(Paragraph::new(lines), header);
        let count = self.visible().len();
        frame.render_widget(
            Line::from(match self.shown_group() {
                Group::None => format!("{} ({count})", self.view.title()),
                group => format!(
                    "{} ({count}) · GROUPED BY {}",
                    self.view.title(),
                    group.name().to_uppercase()
                ),
            })
            .bold()
            .green()
            .alignment(Alignment::Center),
            title,
        );
        let body = if self.compact {
            let [body, legend] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(body);
            let text = if self.view == View::Vacuum {
                " TYPE: wrap = to prevent wraparound · man = manual"
            } else {
                " STATE: act = active · txn = idle in transaction · \
                 abrt = idle in transaction (aborted)"
            };
            frame.render_widget(Line::from(text).dark_gray(), legend);
            body
        } else {
            body
        };
        if self.preview {
            let lines = self.preview_lines();
            let height = (lines.len() as u16 + 2).min(body.height / 2);
            let [table, preview] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(height)]).areas(body);
            self.draw_table(frame, table);
            self.draw_preview(frame, preview, lines);
        } else {
            self.draw_table(frame, body);
        }
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

    fn preview_lines(&self) -> Vec<String> {
        let selected = self.table.selected().unwrap_or(0);
        match self.visible().get(selected) {
            Some(backend) => query_lines(
                &backend.raw_query,
                &backend.query,
                self.server.query_size_limit,
            ),
            None => vec!["No query selected.".to_owned()],
        }
    }

    fn draw_preview(&self, frame: &mut Frame, area: Rect, lines: Vec<String>) {
        let selected = self.table.selected().unwrap_or(0);
        let title = self
            .visible()
            .get(selected)
            .map(|backend| {
                format!(
                    " PID {} · {}@{} · {} ",
                    backend.pid,
                    backend.user,
                    backend.database,
                    backend.duration_s.map(format_duration).unwrap_or_default()
                )
            })
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()).block(
                Block::bordered()
                    .border_style(Style::new().cyan())
                    .title(Line::from(title).bold())
                    .title_bottom(Line::from(" Tab hides ").dark_gray()),
            ),
            area,
        );
    }

    fn compact_header_lines(&self) -> Vec<Line<'static>> {
        let server = &self.server;
        let value = |text: String| Span::styled(text, Style::new().bold().green());
        let bar = || Span::styled(" │ ", Style::new().dark_gray());
        let mut info = vec![
            Span::styled(
                concat!(" pg-probe ", env!("CARGO_PKG_VERSION"), " "),
                Style::new().black().on_green().bold(),
            ),
            Span::styled(format!(" PG {} ", server.version), Style::new().bold()),
            Span::styled(
                format!("{}@{}/{}", server.user, server.endpoint, server.database),
                Style::new().cyan(),
            ),
            Span::raw(format!(" {}s", self.interval.as_secs())),
        ];
        if self.paused {
            info.push(Span::styled(" PAUSED", Style::new().bold().red()));
        }
        match self.os.as_ref().map(|os| &os.host) {
            Some(host) => {
                if let Some(load) = &host.load {
                    info.extend([
                        bar(),
                        Span::raw("load "),
                        value(format!("{:.2}", load.load_1m)),
                    ]);
                }
                if let Some(memory) = host.memory {
                    info.extend([
                        bar(),
                        Span::raw("mem "),
                        value(format_bytes(memory.available as f64)),
                        Span::raw(format!("/{} avail", format_bytes(memory.total as f64))),
                        bar(),
                        Span::raw("swap "),
                        value(format_bytes(
                            memory.swap_total.saturating_sub(memory.swap_free) as f64,
                        )),
                        Span::raw(" used"),
                    ]);
                }
            }
            None => info.extend([
                bar(),
                Span::styled("OS not on this host", Style::new().dark_gray()),
            ]),
        }

        let counts = SessionCounts::of(&self.backends);
        let active: Vec<usize> = self.history.iter().map(|(active, _)| *active).collect();
        let waiting: Vec<usize> = self.history.iter().map(|(_, waiting)| *waiting).collect();
        let sessions = Line::from(vec![
            Span::raw(" sessions "),
            value(counts.total.to_string()),
            Span::raw(format!("/{}", server.max_connections)),
            bar(),
            Span::raw("active "),
            value(counts.active.to_string()),
            Span::styled(format!(" {}", sparkline(&active)), Style::new().green()),
            bar(),
            Span::raw("idle "),
            value(counts.idle.to_string()),
            bar(),
            Span::raw("idle in txn "),
            value(counts.idle_in_transaction.to_string()),
            bar(),
            Span::raw("abrt "),
            value(counts.idle_in_transaction_aborted.to_string()),
            bar(),
            Span::raw("waiting "),
            value(counts.waiting.to_string()),
            Span::styled(format!(" {}", sparkline(&waiting)), Style::new().red()),
        ]);
        vec![Line::from(info), sessions]
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
        if self.view == View::Vacuum {
            self.draw_vacuum_table(frame, area);
            return;
        }
        let tree = self.view == View::Blocking && self.tree;
        let last_column = if self.view == View::Blocking && !tree {
            "BLOCKS"
        } else {
            "WAITING"
        };
        let mut titles = vec!["PID"];
        let mut widths = vec![Constraint::Length(if tree { 16 } else { 7 })];
        if self.compact {
            titles.push("USER@DATABASE");
            widths.push(Constraint::Length(24));
        } else {
            titles.extend(["DATABASE", "USER"]);
            widths.extend([Constraint::Length(16), Constraint::Length(12)]);
        }
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
            Constraint::Length(if self.compact { 5 } else { 20 }),
            Constraint::Length(10),
            Constraint::Length(22),
            Constraint::Fill(1),
        ]);
        let header = Row::new(titles.into_iter().map(Cell::from))
            .style(Style::new().black().on_green().bold());

        let entries = self.rows();
        let sessions: Vec<Row> = entries
            .iter()
            .map(|(branch, backend)| {
                let wait = if self.view == View::Blocking && !tree {
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
                let mut cells = vec![Cell::from(Line::from(vec![
                    Span::styled(branch.clone(), Style::new().dark_gray()),
                    Span::styled(backend.pid.clone(), Style::new().cyan()),
                ]))];
                let database =
                    Span::styled(backend.database.clone(), Style::new().dark_gray().bold());
                if self.compact {
                    cells.push(Cell::from(Line::from(vec![
                        Span::raw(format!("{}@", backend.user)),
                        database,
                    ])));
                } else {
                    cells.extend([Cell::from(database), Cell::from(backend.user.clone())]);
                }
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
                        if self.compact {
                            short_state(&backend.state).to_owned()
                        } else {
                            backend.state.clone()
                        },
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

        // A group title is an empty table row, painted over full width
        // after the table is drawn, since table cells cannot span columns.
        let group = self.shown_group();
        let mut rows = Vec::new();
        let mut titles = Vec::new();
        let mut positions = Vec::new();
        for (index, row) in sessions.into_iter().enumerate() {
            let key = group.key(entries[index].1);
            if key.is_some() && (index == 0 || group.key(entries[index - 1].1) != key) {
                let count = entries
                    .iter()
                    .filter(|(_, backend)| group.key(backend) == key)
                    .count();
                let key = key.unwrap_or_default();
                let style = if group == Group::State {
                    state_style(key)
                } else {
                    Style::new().cyan()
                };
                titles.push((rows.len(), format!("▾ {} ({count})", or_dash(key)), style));
                rows.push(Row::new(Vec::<Cell>::new()));
            }
            positions.push(rows.len());
            rows.push(row);
        }

        let table = Table::new(rows, widths)
            .header(header)
            .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        let selected = self
            .table
            .selected()
            .and_then(|index| positions.get(index).copied());
        let mut state = TableState::default()
            .with_offset(self.table.offset())
            .with_selected(selected);
        frame.render_stateful_widget(table, area, &mut state);
        *self.table.offset_mut() = state.offset();
        for (position, title, style) in titles {
            let Some(line) = position.checked_sub(state.offset()) else {
                continue;
            };
            if line + 1 < usize::from(area.height) {
                let row = Rect::new(area.x, area.y + 1 + line as u16, area.width, 1);
                frame.render_widget(Line::from(title).style(style.bold()), row);
            }
        }
    }

    fn draw_vacuum_table(&mut self, frame: &mut Frame, area: Rect) {
        // Before 9.6 there is no phase or progress, so those columns go
        // and the query gets their room.
        let progress = has_vacuum_progress(self.server.version_num);
        let mut titles = vec!["PID"];
        let mut widths = vec![Constraint::Length(7)];
        if self.compact {
            titles.extend(["TABLE@DATABASE", "TYPE"]);
            widths.extend([Constraint::Length(36), Constraint::Length(4)]);
        } else {
            titles.extend(["DATABASE", "TABLE", "TYPE"]);
            widths.extend([
                Constraint::Length(16),
                Constraint::Length(32),
                Constraint::Length(10),
            ]);
        }
        if progress {
            titles.push("PHASE");
            widths.push(Constraint::Length(24));
        }
        if self.os.is_some() {
            titles.extend(["CPU%", "READ/s", "WRITE/s"]);
            widths.extend([
                Constraint::Length(6),
                Constraint::Length(9),
                Constraint::Length(9),
            ]);
        }
        if progress {
            titles.extend(["SCANNED", "IDX"]);
            widths.extend([Constraint::Length(7), Constraint::Length(3)]);
        }
        titles.extend(["TIME+", "WAITING", "QUERY"]);
        widths.extend([
            Constraint::Length(10),
            Constraint::Length(22),
            Constraint::Fill(1),
        ]);
        let header = Row::new(titles.into_iter().map(Cell::from))
            .style(Style::new().black().on_green().bold());
        let right = |text: String| Cell::from(Line::from(text).alignment(Alignment::Right));
        let rows: Vec<Row> = self
            .vacuums
            .iter()
            .map(|vacuum| {
                let backend = &vacuum.backend;
                let kind_style = match vacuum.kind.as_str() {
                    "wraparound" => Style::new().red().bold(),
                    "auto" => Style::new().yellow(),
                    _ => Style::new(),
                };
                let mut cells = vec![Cell::from(Span::styled(
                    backend.pid.clone(),
                    Style::new().cyan(),
                ))];
                let database =
                    Span::styled(backend.database.clone(), Style::new().dark_gray().bold());
                if self.compact {
                    cells.extend([
                        Cell::from(Line::from(vec![
                            Span::raw(format!("{}@", or_dash(&vacuum.table))),
                            database,
                        ])),
                        Cell::from(Span::styled(short_kind(&vacuum.kind), kind_style)),
                    ]);
                } else {
                    cells.extend([
                        Cell::from(database),
                        Cell::from(or_dash(&vacuum.table).to_owned()),
                        Cell::from(Span::styled(vacuum.kind.clone(), kind_style)),
                    ]);
                }
                if progress {
                    cells.push(Cell::from(or_dash(&vacuum.phase).to_owned()));
                }
                if let Some(os) = &self.os {
                    let rate = os.rates.get(&backend.pid);
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
                    ]);
                }
                if progress {
                    cells.extend([
                        right(vacuum.scanned()),
                        right(or_dash(&vacuum.index_passes).to_owned()),
                    ]);
                }
                cells.extend([
                    Cell::from(
                        Line::from(Span::styled(
                            backend.duration_s.map(format_duration).unwrap_or_default(),
                            duration_style(backend.duration_s),
                        ))
                        .alignment(Alignment::Right),
                    ),
                    Cell::from(Span::styled(backend.wait_label(), Style::new().yellow())),
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
        let backend = self.backend_by_pid(&popup.pid);
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
            query_lines(
                &details.raw_query,
                &details.query,
                self.server.query_size_limit,
            )
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
            ("F4 4", "Vacuum in progress, manual and autovacuum"),
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
            ("Tab", "Show or hide the selected query, formatted"),
            ("t", "Blocking view as a list or a tree"),
            ("c", "Compact columns, more room for the query"),
            ("g", "Group by state, database, user, or none"),
            ("h", "Two-line header with session history"),
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
        ("F4", "Vacuum", Some(View::Vacuum)),
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

/// Refreshes the compact header's sparklines cover.
const HISTORY: usize = 24;

/// One block per value, scaled so the largest value fills the cell.
fn sparkline(values: &[usize]) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    values
        .iter()
        .map(|value| BLOCKS[value * (BLOCKS.len() - 1) / max])
        .collect()
}

/// The compact list's states. The legend under the list spells them out.
fn short_state(state: &str) -> &str {
    match state {
        "active" => "act",
        "idle in transaction" => "txn",
        "idle in transaction (aborted)" => "abrt",
        other => other,
    }
}

/// The compact vacuum view's types. The legend under the list spells them
/// out.
fn short_kind(kind: &str) -> &str {
    match kind {
        "wraparound" => "wrap",
        "manual" => "man",
        other => other,
    }
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
    Ok(rows.rows.iter().map(|row| backend_of(row)).collect())
}

/// Decodes the first eight columns, which the backend and vacuum `SELECT`s
/// share.
fn backend_of(row: &[Vec<u8>]) -> Backend {
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
        raw_query: row
            .get(7)
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .unwrap_or_default(),
    }
}

/// The table an autovacuum worker names in its query, such as `public.t`
/// in `autovacuum: VACUUM ANALYZE public.t (to prevent wraparound)`. The
/// name may hold spaces inside quotes, so it is everything after the
/// keywords.
fn autovacuum_table(query: &str) -> Option<String> {
    let mut rest = query.strip_prefix("autovacuum: ")?;
    rest = rest
        .strip_suffix(" (to prevent wraparound)")
        .unwrap_or(rest);
    for keyword in ["VACUUM ", "ANALYZE "] {
        rest = rest.strip_prefix(keyword).unwrap_or(rest);
    }
    let name = rest.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn fetch_vacuums(client: &ConnectedClient, sql: &str) -> Result<Vec<Vacuum>, AdapterError> {
    let rows = client.exec_bounded_rows(sql, SOURCE, &mut Vec::new())?;
    Ok(rows
        .rows
        .iter()
        .map(|row| {
            let text = |index: usize| {
                row.get(index)
                    .map(|value| text_of(value))
                    .unwrap_or_default()
            };
            let backend = backend_of(row);
            // The server resolves names only in the connected database and
            // has none before 9.6; an autovacuum's query names its table.
            let mut table = text(8);
            if table.is_empty() || table.bytes().all(|byte| byte.is_ascii_digit()) {
                if let Some(name) = autovacuum_table(&backend.query) {
                    table = name;
                }
            }
            Vacuum {
                backend,
                table,
                kind: text(9),
                phase: text(10),
                heap_scanned: text(11).parse().ok(),
                heap_total: text(12).parse().ok(),
                index_passes: text(13),
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

/// Lays the query out one clause per line. Text the server cut at
/// `track_activity_query_size` stays as `flat`, since the formatter would
/// indent a broken statement.
fn query_lines(raw: &str, flat: &str, size_limit: usize) -> Vec<String> {
    if raw.trim().is_empty() || (size_limit > 0 && raw.len() + 1 >= size_limit) {
        return vec![flat.to_owned()];
    }
    let options = FormatOptions {
        dialect: Dialect::PostgreSql,
        ..FormatOptions::default()
    };
    sqlformat::format(raw, &QueryParams::None, &options)
        .lines()
        .map(single_line_keep_indent)
        .collect()
}

/// Decodes a server value for display: whitespace collapses to single spaces
/// and control characters are dropped, so text from the server can neither
/// break the layout nor send escape sequences to the terminal.
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
            raw_query: query.to_owned(),
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
        let lines = query_lines(&details.raw_query, &details.query, 1024);
        assert_eq!(lines[0], "select");
        assert!(lines.contains(&"  a".to_owned()), "{lines:?}");
        assert!(lines.iter().all(|line| !line.contains('\x1b')));
        assert_eq!(
            query_lines(&details.raw_query, &details.query, 32),
            vec![details.query.clone()]
        );
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
    fn preview_formats_the_selected_query_under_the_list() {
        let mut app = app(View::Running);
        app.backends[0].raw_query = "select a, b from t".to_owned();
        assert!(!screen(&mut app).contains("PID 101 ·"));
        app.preview = true;
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>() + "\n")
            .collect();
        assert!(screen.contains("PID 101 ·"), "{screen}");
        assert!(screen.contains("│from "), "{screen}");
    }

    #[test]
    fn blocking_tree_hangs_waiters_under_their_blockers() {
        let mut app = app(View::Blocking);
        app.backends = ["10", "20", "30", "40", "50", "51"]
            .iter()
            .map(|pid| backend(pid, "active", "", ""))
            .collect();
        app.blocks = HashMap::from([
            ("10".to_owned(), vec!["20".to_owned(), "40".to_owned()]),
            ("20".to_owned(), vec!["30".to_owned()]),
            ("50".to_owned(), vec!["51".to_owned()]),
            ("51".to_owned(), vec!["50".to_owned()]),
        ]);
        assert_eq!(app.visible().len(), 4);
        app.tree = true;
        let rows: Vec<String> = app
            .rows()
            .iter()
            .map(|(branch, backend)| format!("{branch}{}", backend.pid))
            .collect();
        assert_eq!(rows, ["10", "├─ 20", "│  └─ 30", "└─ 40", "50", "└─ 51"]);
    }

    #[test]
    fn compact_columns_merge_user_and_database() {
        let mut app = app(View::Running);
        app.backends[0].state = "idle in transaction".to_owned();
        app.compact = true;
        let screen = screen(&mut app);
        assert!(screen.contains("USER@DATABASE"), "{screen}");
        assert!(screen.contains("postgres@bench"), "{screen}");
        assert!(screen.contains(" txn "), "{screen}");
        assert!(screen.contains("txn = idle in transaction"), "{screen}");
    }

    #[test]
    fn grouping_sorts_sessions_under_a_title_per_group() {
        let mut app = app(View::Running);
        app.backends[0].database = "reports".to_owned();
        app.group = Group::Database;
        let pids: Vec<&str> = app.visible().iter().map(|b| b.pid.as_str()).collect();
        assert_eq!(pids, ["102", "101"]);
        app.table.select(Some(1));
        let screen = screen(&mut app);
        assert!(screen.contains("GROUPED BY DATABASE"), "{screen}");
        assert!(screen.contains("▾ bench (1)"), "{screen}");
        assert!(screen.contains("▾ reports (1)"), "{screen}");
        let lines: Vec<&str> = screen.lines().collect();
        let reports = lines.iter().position(|l| l.contains("▾ reports")).unwrap();
        assert!(lines[reports + 1].starts_with("101"), "{screen}");
    }

    #[test]
    fn compact_header_fits_two_lines_with_history() {
        assert_eq!(sparkline(&[0, 1, 2, 4]), "▁▂▄█");
        assert_eq!(sparkline(&[0, 0]), "▁▁");
        let mut app = app(View::Running);
        app.history = VecDeque::from([(1, 0), (2, 1)]);
        app.compact_header = true;
        let screen = screen(&mut app);
        let lines: Vec<&str> = screen.lines().collect();
        assert!(lines[1].contains("active 2 ▄█"), "{screen}");
        assert!(lines[1].contains("waiting 1 ▁█"), "{screen}");
        assert!(lines[2].contains("RUNNING QUERIES"), "{screen}");
    }

    #[test]
    fn vacuum_sql_follows_the_server_version() {
        let legacy = vacuum_sql(90325);
        assert!(!legacy.contains("pg_stat_progress_vacuum"));
        assert!(legacy.contains("a.query LIKE 'autovacuum:%'"));
        let progress = vacuum_sql(90600);
        assert!(progress.contains("pg_stat_progress_vacuum"));
        assert!(!progress.contains("backend_type"));
        assert!(vacuum_sql(170000).contains("a.backend_type = 'autovacuum worker'"));
    }

    #[test]
    fn autovacuum_queries_name_their_table() {
        assert_eq!(
            autovacuum_table("autovacuum: VACUUM public.t (to prevent wraparound)").as_deref(),
            Some("public.t")
        );
        assert_eq!(
            autovacuum_table("autovacuum: VACUUM ANALYZE public.\"my t\"").as_deref(),
            Some("public.\"my t\"")
        );
        assert_eq!(autovacuum_table("VACUUM;"), None);
    }

    #[test]
    fn vacuum_view_drops_progress_columns_before_9_6() {
        let mut app = app(View::Vacuum);
        app.vacuums = vec![Vacuum {
            backend: Backend {
                pid: "900".to_owned(),
                query: "autovacuum: VACUUM public.t (to prevent wraparound)".to_owned(),
                ..Backend::default()
            },
            table: "public.t".to_owned(),
            kind: "wraparound".to_owned(),
            ..Vacuum::default()
        }];
        let screen = screen(&mut app);
        assert!(!screen.contains("PHASE"), "{screen}");
        assert!(screen.contains("public.t "), "{screen}");
    }

    #[test]
    fn compact_vacuum_view_merges_table_and_database() {
        let mut app = app(View::Vacuum);
        app.compact = true;
        app.vacuums = vec![Vacuum {
            backend: Backend {
                pid: "900".to_owned(),
                database: "app".to_owned(),
                ..Backend::default()
            },
            table: "public.t".to_owned(),
            kind: "wraparound".to_owned(),
            ..Vacuum::default()
        }];
        let screen = screen(&mut app);
        assert!(screen.contains("TABLE@DATABASE"), "{screen}");
        assert!(screen.contains("public.t@app"), "{screen}");
        assert!(screen.contains(" wrap "), "{screen}");
        assert!(screen.contains("wrap = to prevent wraparound"), "{screen}");
    }

    #[test]
    fn vacuum_view_lists_progress_and_finds_autovacuum_workers() {
        let mut app = app(View::Vacuum);
        app.server.version_num = 170000;
        app.vacuums = vec![Vacuum {
            backend: Backend {
                pid: "900".to_owned(),
                database: "bench".to_owned(),
                query: "autovacuum: VACUUM public.accounts".to_owned(),
                ..Backend::default()
            },
            table: "accounts".to_owned(),
            kind: "auto".to_owned(),
            phase: "scanning heap".to_owned(),
            heap_scanned: Some(250.0),
            heap_total: Some(1000.0),
            index_passes: "0".to_owned(),
        }];
        assert_eq!(app.visible().len(), 1);
        assert!(app.backend_by_pid("900").is_some());
        let screen = screen(&mut app);
        assert!(screen.contains("VACUUM (1)"), "{screen}");
        assert!(screen.contains("scanning heap"), "{screen}");
        assert!(screen.contains("25.0%"), "{screen}");
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
