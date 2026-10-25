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
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Widget;
use ratatui::{DefaultTerminal, Frame};
use serde::{Deserialize, Serialize};
use sqlformat::{Dialect, FormatOptions, QueryParams};
use thiserror::Error;
use tui_kiso::{
    braille, fit, set_background, set_foreground, Columns, KeyHints, Mark, Modal, Page, Scroll,
    Tabs, Theme, Title,
};

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
    #[error("the PostgreSQL connection failed: {0}")]
    Connection(String),
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
    /// Whether the running view lists the backend: any state but idle.
    fn is_running(&self) -> bool {
        !self.state.is_empty() && self.state != "idle"
    }

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
            _ => "—".to_owned(),
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
    scroll: Scroll,
    /// The definitions of the functions the query calls, opened with `f`.
    function: Option<FunctionPanel>,
    /// The estimated plan of the query, opened with `e`.
    explain: Option<ExplainPanel>,
}

struct ExplainPanel {
    database: String,
    /// The plan lines, or why there is no plan.
    result: Result<Vec<String>, String>,
    scroll: Scroll,
}

struct FunctionPanel {
    database: String,
    /// The functions found, or why the lookup failed.
    result: Result<Vec<function::FunctionDef>, String>,
    index: usize,
    scroll: Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Running,
    Waiting,
    Blocking,
    Vacuum,
}

impl View {
    const ALL: [Self; 4] = [Self::Running, Self::Waiting, Self::Blocking, Self::Vacuum];

    const fn name(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Waiting => "Waiting",
            Self::Blocking => "Blocking",
            Self::Vacuum => "Vacuum",
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
    theme: Theme,
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
    /// The selected row of the list, and where the list is scrolled.
    selected: usize,
    list: Scroll,
    /// Refreshes so far: the spinner turns once per refresh.
    tick: u32,
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
/// Runs the view. With `verbose`, the steps of the connection go to stderr,
/// before the view takes the screen.
pub fn run(params: &ConnectionParams, interval: Duration, verbose: bool) -> Result<(), TopError> {
    let client = if verbose {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_target(false)
            .with_writer(io::stderr)
            .finish();
        tracing::subscriber::with_default(subscriber, || connect(params))?
    } else {
        connect(params)?
    };
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
    // A panic skips the end of `run`, so the panic hook puts the colors
    // back too, before ratatui's hook leaves the alternate screen.
    restore_terminal_on_panic(io::stdout);
    // The view leaves its cells unpainted, so the terminal's own colors
    // become the theme's for the run, and come back when it ends. With
    // NO_COLOR, or off a terminal, they stay as they are.
    if takes_terminal_colors(
        io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    ) {
        paint_terminal(&mut io::stdout(), app.theme.tokens());
    }
    let result = app.event_loop(&client, &sql, &mut terminal);
    restore_terminal(&mut io::stdout());
    ratatui::restore();
    result
}

/// Opens the session, and on failure says why when the reason is safe to
/// show: the server's message or the operating system's.
fn connect(params: &ConnectionParams) -> Result<ConnectedClient, TopError> {
    let mut outcomes = Vec::new();
    ConnectedClient::open_read_only(params, STATEMENT_TIMEOUT_MS, SOURCE, &mut outcomes).map_err(
        |error| match (&error, outcomes.pop().and_then(|outcome| outcome.detail)) {
            (AdapterError::ConnectionFailed, Some(detail)) => TopError::Connection(detail),
            _ => TopError::Postgres(error),
        },
    )
}

/// The Kiso theme of the terminal views: the dark theme with the gold
/// accent the console uses. It sends 24-bit color only when `COLORTERM`
/// says the terminal takes it, and the nearest xterm-256 colors otherwise.
pub fn theme() -> Theme {
    Theme::new(kiso_tokens::GOLD_DARK)
}

/// Sets the terminal's background and text color to the theme's with OSC 11
/// and OSC 10, or puts the terminal's own colors back with `None`.
fn terminal_colors(out: &mut impl Write, tokens: Option<&kiso_tokens::Theme>) -> io::Result<()> {
    set_background(out, tokens.map(|tokens| tokens.background))?;
    set_foreground(out, tokens.map(|tokens| tokens.foreground))
}

/// Whether a view may set the terminal's colors: only on a terminal, and
/// not when `NO_COLOR` is set.
pub fn takes_terminal_colors(is_terminal: bool, no_color: bool) -> bool {
    is_terminal && !no_color
}

/// Whether the terminal's colors are Kiso's now. `top` and the capture
/// progress set them and put them back only through it, on the normal and
/// the panic paths, so each happens at most once per process.
static TERMINAL_COLORS: AtomicBool = AtomicBool::new(false);

/// Sets the terminal's background and text color to `tokens`, unless they
/// are set already.
pub fn paint_terminal(out: &mut impl Write, tokens: &kiso_tokens::Theme) {
    if !TERMINAL_COLORS.swap(true, Ordering::SeqCst) {
        let _ = terminal_colors(out, Some(tokens));
    }
}

/// Puts the terminal's own colors back, if `paint_terminal` changed them
/// and nothing has put them back since.
pub fn restore_terminal(out: &mut impl Write) {
    if TERMINAL_COLORS.swap(false, Ordering::SeqCst) {
        let _ = terminal_colors(out, None);
    }
}

/// Chains a panic hook before the current one that puts the terminal's
/// colors back on the writer `out` returns.
pub fn restore_terminal_on_panic<W: Write + 'static>(out: fn() -> W) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal(&mut out());
        previous(info);
    }));
}

impl App {
    fn new(server: Server, interval: Duration, local: bool) -> Self {
        let list = Scroll::default();
        list.top();
        Self {
            theme: theme(),
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
            selected: 0,
            list,
            tick: 0,
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
                // Every layer closes with q or Esc; only the list quits.
                if self.help {
                    self.help = false;
                    continue;
                }
                if let Some(panel) = self
                    .popup
                    .as_mut()
                    .and_then(|popup| popup.function.as_mut())
                {
                    let count = panel.result.as_ref().map_or(0, Vec::len);
                    match key.code {
                        KeyCode::Char('f' | 'q') | KeyCode::Esc => {
                            if let Some(popup) = &mut self.popup {
                                popup.function = None;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => panel.scroll.by(1),
                        KeyCode::Up | KeyCode::Char('k') => panel.scroll.by(-1),
                        KeyCode::PageDown => panel.scroll.by(PAGE as isize),
                        KeyCode::PageUp => panel.scroll.by(-(PAGE as isize)),
                        KeyCode::Right | KeyCode::Char('l') if panel.index + 1 < count => {
                            panel.index += 1;
                            panel.scroll.top();
                        }
                        KeyCode::Left | KeyCode::Char('h') if panel.index > 0 => {
                            panel.index -= 1;
                            panel.scroll.top();
                        }
                        _ => {}
                    }
                    continue;
                }
                if let Some(panel) = self.popup.as_mut().and_then(|popup| popup.explain.as_mut()) {
                    match key.code {
                        KeyCode::Char('e' | 'q') | KeyCode::Esc => {
                            if let Some(popup) = &mut self.popup {
                                popup.explain = None;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => panel.scroll.by(1),
                        KeyCode::Up | KeyCode::Char('k') => panel.scroll.by(-1),
                        KeyCode::PageDown => panel.scroll.by(PAGE as isize),
                        KeyCode::PageUp => panel.scroll.by(-(PAGE as isize)),
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
                        KeyCode::Char('v' | 'q') | KeyCode::Esc | KeyCode::Enter => {
                            self.popup = None
                        }
                        KeyCode::Char(' ') => self.paused = !self.paused,
                        KeyCode::Down | KeyCode::Char('j') => popup.scroll.by(1),
                        KeyCode::Up | KeyCode::Char('k') => popup.scroll.by(-1),
                        KeyCode::PageDown => popup.scroll.by(PAGE as isize),
                        KeyCode::PageUp => popup.scroll.by(-(PAGE as isize)),
                        _ => {}
                    }
                    continue;
                }
                let view = self.view;
                let before = self.settings();
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char(' ') => self.paused = !self.paused,
                    KeyCode::Char('?') => self.help = true,
                    KeyCode::Tab => self.preview = !self.preview,
                    KeyCode::Char('c') => self.compact = !self.compact,
                    KeyCode::Char('h') => self.compact_header = !self.compact_header,
                    KeyCode::Char('g') => {
                        self.group = self.group.next();
                        self.select_first();
                    }
                    KeyCode::Char('t') => {
                        self.tree = !self.tree;
                        self.select_first();
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
                    KeyCode::Home => self.select_first(),
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
                    self.select_first();
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
        self.tick = self.tick.wrapping_add(1);
        self.blocks = if self.view == View::Blocking {
            fetch_blocks(client, blocking_sql(self.server.version_num))?
        } else {
            HashMap::new()
        };
        self.select_by(0);
        Ok(())
    }

    fn open_popup(&mut self, client: &ConnectedClient) -> Result<(), TopError> {
        let Some(pid) = self
            .visible()
            .get(self.selected)
            .map(|backend| backend.pid.clone())
        else {
            return Ok(());
        };
        self.popup = Some(Popup {
            pid,
            details: None,
            memory: None,
            ended: false,
            scroll: Scroll::default(),
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
                scroll: Scroll::default(),
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
                scroll: Scroll::default(),
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
                View::Running => backend.is_running(),
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
        let last = self.visible().len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.list.follow();
    }

    /// Back to the first row, with the list at the top.
    fn select_first(&mut self) {
        self.selected = 0;
        self.list.top();
    }

    fn draw(&mut self, frame: &mut Frame) {
        let screen = frame.area();
        let buf = frame.buffer_mut();
        // Every row counts: the page starts on the first row and the keys
        // take the last one, with two columns free at each side. The cells
        // stay unpainted, so the terminal's background, set with OSC 11 in
        // `run`, shows through.
        let body = Rect::new(
            screen.x.saturating_add(2),
            screen.y,
            screen.width.saturating_sub(4),
            screen.height.saturating_sub(1),
        );
        let foot = Rect::new(body.x, screen.bottom().saturating_sub(1), body.width, 1)
            .intersection(screen);
        // The scrollbar takes the second column right of the page.
        let page = Rect {
            width: body.width.saturating_sub(2),
            ..body
        };
        match &self.popup {
            Some(popup) => self.draw_details(buf, page, popup),
            None => self.draw_list(buf, page),
        }
        self.draw_hints(buf, foot);
        if let Some(popup) = &self.popup {
            self.draw_function_panel(buf, screen, popup);
            self.draw_explain_panel(buf, screen, popup);
        }
        if self.help {
            draw_help(buf, &self.theme, screen);
        }
    }

    /// A categorical series: active, idle in transaction, waiting.
    fn series(&self, slot: usize) -> Style {
        let tokens = self.theme.tokens();
        self.theme
            .fg([tokens.chart_1, tokens.chart_2, tokens.chart_3][slot])
    }

    /// The brand and the server, then `extra`, with the refresh on the
    /// right: a spinner that turns on each refresh, or "paused".
    fn server_row(&self, buf: &mut Buffer, area: Rect, extra: Vec<Span<'static>>, spinner: bool) {
        let t = &self.theme;
        let server = &self.server;
        let refresh = if self.paused {
            // The spinner stops on a full braille cell.
            Line::from(Span::styled(
                "⠿ paused",
                t.link().add_modifier(Modifier::BOLD),
            ))
        } else if spinner {
            Line::from(vec![
                Span::styled(braille(self.tick), t.fg(t.tokens().primary)),
                Span::styled(format!(" every {} s", self.interval.as_secs()), t.muted()),
            ])
        } else {
            Line::default()
        };
        let mut spans = vec![
            Span::styled(
                concat!(" pg-probe ", env!("CARGO_PKG_VERSION"), " "),
                t.brand(),
            ),
            Span::raw(" "),
            Span::styled(format!("PostgreSQL {}", server.version), t.strong()),
            Span::styled(" · ", t.subtle()),
            Span::styled(
                format!("{}@{}/{}", server.user, server.endpoint, server.database),
                t.link(),
            ),
        ];
        spans.extend(extra);
        let width = usize::from(area.width);
        let right = refresh.width();
        let room = if right == 0 {
            width
        } else {
            width.saturating_sub(right + 1)
        };
        put(buf, area, 0, Line::from(fit(spans, room)));
        if right > 0 && right <= width {
            let x = area.right() - right as u16;
            put(buf, Rect { x, ..area }, 0, refresh);
        }
    }

    /// The session counts. On a narrow screen the states take the short
    /// names the help screen spells out.
    fn sessions_line(&self, width: usize) -> Line<'static> {
        let t = &self.theme;
        let counts = SessionCounts::of(&self.backends);
        let build = |short: bool| {
            let mut spans = vec![
                Span::styled("Sessions  ", t.muted()),
                Span::styled(
                    format!("{}/{}", counts.total, self.server.max_connections),
                    t.strong(),
                ),
            ];
            for (mark, style, count, long, brief) in [
                ("●", self.series(0), counts.active, "active", "act"),
                ("○", t.subtle(), counts.idle, "idle", "idle"),
                (
                    "●",
                    self.series(1),
                    counts.idle_in_transaction,
                    "idle in txn",
                    "txn",
                ),
                (
                    "●",
                    t.subtle(),
                    counts.idle_in_transaction_aborted,
                    "aborted",
                    "abrt",
                ),
                ("●", self.series(2), counts.waiting, "waiting", "waiting"),
            ] {
                spans.extend([
                    Span::styled(" · ", t.subtle()),
                    Span::styled(format!("{mark} "), style),
                    Span::styled(count.to_string(), t.strong()),
                    Span::styled(format!(" {}", if short { brief } else { long }), t.muted()),
                ]);
            }
            Line::from(spans)
        };
        let line = build(false);
        if line.width() <= width {
            line
        } else {
            build(true)
        }
    }

    /// The connection slots as a meter `width` cells wide: waiting, other
    /// active, idle in a transaction and idle, each in its series, then the
    /// free slots.
    fn meter(&self, width: usize) -> Line<'static> {
        let t = &self.theme;
        let counts = SessionCounts::of(&self.backends);
        let max = self.server.max_connections.max(counts.total).max(1);
        let cells = |count: usize| count * width / max;
        let parts = [
            (cells(counts.waiting), self.series(2)),
            (
                cells(counts.active.saturating_sub(counts.waiting)),
                self.series(0),
            ),
            (
                cells(counts.idle_in_transaction + counts.idle_in_transaction_aborted),
                self.series(1),
            ),
            (cells(counts.idle), t.subtle()),
        ];
        let mut used = 0;
        let mut spans = Vec::new();
        for (count, style) in parts {
            let count = count.min(width - used);
            spans.push(Span::styled("━".repeat(count), style));
            used += count;
        }
        spans.push(Span::styled("─".repeat(width - used), t.border()));
        Line::from(spans)
    }

    /// The compact header's session counts, with the active and waiting
    /// counts of the last refreshes.
    fn history_line(&self) -> Line<'static> {
        let t = &self.theme;
        let counts = SessionCounts::of(&self.backends);
        let active: Vec<usize> = self.history.iter().map(|(active, _)| *active).collect();
        let waiting: Vec<usize> = self.history.iter().map(|(_, waiting)| *waiting).collect();
        let dot = || Span::styled(" · ", t.subtle());
        let label = |text: &str| Span::styled(text.to_owned(), t.muted());
        let value = |count: usize| Span::styled(count.to_string(), t.strong());
        let series = |count: usize, slot: usize| {
            Span::styled(
                count.to_string(),
                self.series(slot).add_modifier(Modifier::BOLD),
            )
        };
        Line::from(vec![
            label("Sessions  "),
            Span::styled(
                format!("{}/{}", counts.total, self.server.max_connections),
                t.strong(),
            ),
            dot(),
            label("active "),
            series(counts.active, 0),
            Span::styled(format!(" {}", sparkline(&active)), self.series(0)),
            dot(),
            label("idle "),
            value(counts.idle),
            dot(),
            label("idle in txn "),
            series(counts.idle_in_transaction, 1),
            dot(),
            label("abrt "),
            value(counts.idle_in_transaction_aborted),
            dot(),
            label("waiting "),
            series(counts.waiting, 2),
            Span::styled(format!(" {}", sparkline(&waiting)), self.series(2)),
        ])
    }

    /// Host load and memory, from the local `/proc`. `short` keeps the
    /// figures of the compact header.
    fn host_spans(&self, os: &OsState, short: bool) -> Vec<Span<'static>> {
        let t = &self.theme;
        let value = |text: String| Span::styled(text, t.text());
        let label = |text: &str| Span::styled(text.to_owned(), t.muted());
        let dot = || Span::styled(" · ", t.subtle());
        let mut spans = vec![label("load ")];
        match &os.host.load {
            Some(load) if short => spans.push(value(format!("{:.2}", load.load_1m))),
            Some(load) => spans.push(value(format!(
                "{:.2} {:.2} {:.2}",
                load.load_1m, load.load_5m, load.load_15m
            ))),
            None => spans.push(label("—")),
        }
        if let Some(memory) = os.host.memory {
            let swap_used = memory.swap_total.saturating_sub(memory.swap_free) as f64;
            if short {
                spans.extend([
                    dot(),
                    label("mem "),
                    value(format_bytes(memory.available as f64)),
                    label(&format!("/{} avail", format_bytes(memory.total as f64))),
                    dot(),
                    label("swap "),
                    value(format_bytes(swap_used)),
                    label(" used"),
                ]);
            } else {
                spans.extend([
                    dot(),
                    label("mem "),
                    value(format_bytes(memory.total as f64)),
                    label(" total "),
                    value(format_bytes(memory.available as f64)),
                    label(" avail "),
                    value(format_bytes(memory.buff_cached as f64)),
                    label(" buff+cached"),
                    dot(),
                    label("swap "),
                    value(format_bytes(memory.swap_total as f64)),
                    label(" total "),
                    value(format_bytes(swap_used)),
                    label(" used"),
                ]);
            }
        }
        if os.io_denied() {
            spans.push(Span::styled(
                " · per-backend I/O needs root or the postgres user",
                t.warning(),
            ));
        }
        spans
    }

    /// The host row: its figures in full when they fit, else the short ones.
    fn host_line(&self, width: usize) -> Line<'static> {
        let t = &self.theme;
        let label = Span::styled("Host      ", t.muted());
        let Some(os) = &self.os else {
            return Line::from(vec![
                label,
                Span::styled("not shown, the server is not on this host", t.muted()),
            ]);
        };
        let line = |short: bool| {
            let mut spans = vec![label.clone()];
            spans.extend(self.host_spans(os, short));
            Line::from(spans)
        };
        let full = line(false);
        if full.width() <= width {
            full
        } else {
            line(true)
        }
    }

    /// The views, with the session count of each one this refresh read.
    /// The blocking and vacuum views send their own query, so they have a
    /// count only while open.
    fn tabs_line(&self, width: usize) -> Line<'static> {
        let labels = View::ALL.map(|view| {
            let count = if view == self.view {
                Some(self.visible().len())
            } else {
                match view {
                    View::Running => Some(self.backends.iter().filter(|b| b.is_running()).count()),
                    View::Waiting => {
                        Some(self.backends.iter().filter(|b| b.waits_on_lock()).count())
                    }
                    View::Blocking | View::Vacuum => None,
                }
            };
            let mut label = view.name().to_owned();
            if let Some(count) = count {
                label.push_str(&format!(" {count}"));
            }
            if view == self.view && self.shown_group() != Group::None {
                label.push_str(&format!(" · grouped by {}", self.shown_group().name()));
            }
            label
        });
        let current = View::ALL
            .iter()
            .position(|view| *view == self.view)
            .unwrap_or(0);
        Tabs::new(&self.theme, labels, current).line(width)
    }

    /// The header, the views, the column titles, then the rows, with the
    /// preview of the selected query under them when it is open.
    fn draw_list(&self, buf: &mut Buffer, area: Rect) {
        let t = &self.theme;
        let width = usize::from(area.width);
        let mut row = 0;
        if self.compact_header {
            let extra = match &self.os {
                Some(os) => {
                    let mut spans = vec![Span::styled(" · ", t.subtle())];
                    spans.extend(self.host_spans(os, true));
                    spans
                }
                None => vec![
                    Span::styled(" · ", t.subtle()),
                    Span::styled("OS not on this host", t.muted()),
                ],
            };
            self.server_row(buf, area, extra, true);
            put(
                buf,
                area,
                1,
                Line::from(fit(self.history_line().spans, width)),
            );
            row = 2;
        } else if area.height > 0 {
            self.server_row(buf, area, Vec::new(), true);
            let sessions = self.sessions_line(width);
            // The meter takes what the counts leave of the row, if enough.
            let room = width.saturating_sub(sessions.width() + 3).min(30);
            put(buf, area, 1, Line::from(fit(sessions.spans, width)));
            if room >= 10 {
                let x = area.right() - room as u16;
                put(buf, Rect { x, ..area }, 1, self.meter(room));
            }
            put(
                buf,
                area,
                2,
                Line::from(fit(self.host_line(width).spans, width)),
            );
            row = 3;
        }
        put(buf, area, row, self.tabs_line(width));
        let (head, page) = self.list_page(width);
        put(buf, area, row + 1, head);
        let top = area.y.saturating_add(row + 2).min(area.bottom());
        let mut rows = Rect {
            y: top,
            height: area.bottom() - top,
            ..area
        };
        if self.preview {
            let preview = self.preview_page(width);
            let height = (preview.len() as u16).min(rows.height / 2);
            rows.height -= height;
            let lower = Rect {
                y: rows.bottom(),
                height,
                ..rows
            };
            preview
                .view(t, &Scroll::default())
                .mark(Mark::Dim)
                .render(lower, buf);
        }
        // The rows on screen change with the preview, the screen size and
        // the layout, so every draw scrolls the selection back into view.
        self.list.follow();
        page.view(t, &self.list)
            .selected(Some(self.selected))
            .render(rows, buf);
    }

    /// The column titles and the sessions of the list, under a section per
    /// group when the list is grouped.
    fn list_page(&self, width: usize) -> (Line<'static>, Page) {
        if self.view == View::Vacuum {
            return self.vacuum_page(width);
        }
        let t = &self.theme;
        let tree = self.view == View::Blocking && self.tree;
        let mut spec = vec![if tree {
            Column::text("PID", 16, 10)
        } else {
            Column::text("PID", 7, 7)
        }];
        if self.compact {
            spec.push(Column::text("User@database", 24, 4));
        } else {
            spec.extend([Column::text("Database", 16, 4), Column::text("User", 12, 4)]);
        }
        if self.os.is_some() {
            spec.extend([
                Column::number("CPU%", 5, 5),
                Column::number("Read/s", 10, 9),
                Column::number("Write/s", 10, 9),
                Column::text("IOW", 3, 3),
            ]);
        }
        spec.extend([
            Column::text("State", if self.compact { 4 } else { 20 }, 4),
            Column::number("Time", 10, 8),
            Column::text(
                if self.view == View::Blocking && !tree {
                    "Blocks"
                } else {
                    "Waiting"
                },
                22,
                4,
            )
            .on_second_row(),
        ]);
        let layout = Layout::new(width, &spec);

        let entries = self.rows();
        let group = self.shown_group();
        let mut page = Page::default();
        for (index, (branch, backend)) in entries.iter().enumerate() {
            let key = group.key(backend);
            if key.is_some() && (index == 0 || group.key(entries[index - 1].1) != key) {
                let count = entries
                    .iter()
                    .filter(|(_, other)| group.key(other) == key)
                    .count();
                page.section(t, width, or_dash(key.unwrap_or_default()), Some(count));
            }
            let mut cells: Vec<Vec<Span<'static>>> = vec![vec![
                Span::styled(branch.clone(), t.subtle()),
                Span::styled(backend.pid.clone(), t.info()),
            ]];
            if self.compact {
                cells.push(vec![
                    Span::styled(format!("{}@", backend.user), t.muted()),
                    Span::styled(backend.database.clone(), t.text()),
                ]);
            } else {
                cells.push(vec![Span::styled(backend.database.clone(), t.text())]);
                cells.push(vec![Span::styled(backend.user.clone(), t.muted())]);
            }
            let right = |text: String, width: usize| {
                vec![Span::styled(format!("{text:>width$}"), t.text())]
            };
            if let Some(os) = &self.os {
                let rate = os.rates.get(&backend.pid);
                let bytes =
                    |value: Option<f64>| value.map(format_bytes).unwrap_or_else(|| "—".to_owned());
                let cpu = rate
                    .and_then(|rate| rate.cpu_percent)
                    .map(|cpu| format!("{cpu:.1}"))
                    .unwrap_or_else(|| "—".to_owned());
                cells.push(right(cpu, layout.width(cells.len())));
                let read = bytes(rate.and_then(|rate| rate.read_per_s));
                cells.push(right(read, layout.width(cells.len())));
                let write = bytes(rate.and_then(|rate| rate.write_per_s));
                cells.push(right(write, layout.width(cells.len())));
                cells.push(vec![Span::styled(
                    match rate.map(|rate| rate.io_wait) {
                        Some(true) => " Y",
                        Some(false) => " N",
                        None => " —",
                    },
                    t.text(),
                )]);
            }
            let state = if self.compact {
                short_state(&backend.state).to_owned()
            } else {
                backend.state.clone()
            };
            cells.push(vec![Span::styled(state, t.text())]);
            let time = backend.duration_s.map(format_duration).unwrap_or_default();
            cells.push(right(time, layout.width(cells.len())));
            let wait = if self.view == View::Blocking && !tree {
                self.blocks
                    .get(&backend.pid)
                    .map(|pids| pids.join(","))
                    .unwrap_or_default()
            } else {
                backend.wait_label()
            };
            cells.push(vec![Span::styled(wait, t.text())]);
            page.item(layout.lines(t, &spec, cells, &backend.query, width));
        }
        if entries.is_empty() {
            page.line(Line::styled("  Nothing here right now.", t.muted()));
        }
        (layout.head(t, &spec), page)
    }

    /// The vacuums in progress. Before 9.6 there is no phase or progress,
    /// so those columns go and the query gets their room.
    fn vacuum_page(&self, width: usize) -> (Line<'static>, Page) {
        let t = &self.theme;
        let progress = has_vacuum_progress(self.server.version_num);
        let mut spec = vec![Column::text("PID", 7, 7)];
        if self.compact {
            spec.extend([
                Column::text("Table@database", 36, 8),
                Column::text("Type", 4, 4),
            ]);
        } else {
            spec.extend([
                Column::text("Database", 16, 4),
                Column::text("Table", 32, 4),
                Column::text("Type", 10, 4),
            ]);
        }
        if progress {
            spec.push(Column::text("Phase", 24, 4).on_second_row());
        }
        if self.os.is_some() {
            spec.extend([
                Column::number("CPU%", 5, 5),
                Column::number("Read/s", 10, 9),
                Column::number("Write/s", 10, 9),
            ]);
        }
        if progress {
            spec.extend([Column::number("Scanned", 7, 6), Column::number("IDX", 3, 3)]);
        }
        spec.extend([
            Column::number("Time", 10, 8),
            Column::text("Waiting", 22, 4).on_second_row(),
        ]);
        let layout = Layout::new(width, &spec);

        let mut page = Page::default();
        for vacuum in &self.vacuums {
            let backend = &vacuum.backend;
            let right = |text: String, width: usize| {
                vec![Span::styled(format!("{text:>width$}"), t.text())]
            };
            let mut cells: Vec<Vec<Span<'static>>> =
                vec![vec![Span::styled(backend.pid.clone(), t.info())]];
            if self.compact {
                cells.push(vec![
                    Span::styled(format!("{}@", or_dash(&vacuum.table)), t.text()),
                    Span::styled(backend.database.clone(), t.muted()),
                ]);
                cells.push(vec![Span::styled(
                    short_kind(&vacuum.kind).to_owned(),
                    t.text(),
                )]);
            } else {
                cells.push(vec![Span::styled(backend.database.clone(), t.text())]);
                cells.push(vec![Span::styled(
                    or_dash(&vacuum.table).to_owned(),
                    t.text(),
                )]);
                cells.push(vec![Span::styled(vacuum.kind.clone(), t.text())]);
            }
            if progress {
                cells.push(vec![Span::styled(
                    or_dash(&vacuum.phase).to_owned(),
                    t.text(),
                )]);
            }
            if let Some(os) = &self.os {
                let rate = os.rates.get(&backend.pid);
                let bytes =
                    |value: Option<f64>| value.map(format_bytes).unwrap_or_else(|| "—".to_owned());
                let cpu = rate
                    .and_then(|rate| rate.cpu_percent)
                    .map(|cpu| format!("{cpu:.1}"))
                    .unwrap_or_else(|| "—".to_owned());
                cells.push(right(cpu, layout.width(cells.len())));
                let read = bytes(rate.and_then(|rate| rate.read_per_s));
                cells.push(right(read, layout.width(cells.len())));
                let write = bytes(rate.and_then(|rate| rate.write_per_s));
                cells.push(right(write, layout.width(cells.len())));
            }
            if progress {
                cells.push(right(vacuum.scanned(), layout.width(cells.len())));
                let passes = or_dash(&vacuum.index_passes).to_owned();
                cells.push(right(passes, layout.width(cells.len())));
            }
            let time = backend.duration_s.map(format_duration).unwrap_or_default();
            cells.push(right(time, layout.width(cells.len())));
            cells.push(vec![Span::styled(backend.wait_label(), t.text())]);
            page.item(layout.lines(t, &spec, cells, &backend.query, width));
        }
        if self.vacuums.is_empty() {
            page.line(Line::styled("  Nothing here right now.", t.muted()));
        }
        (layout.head(t, &spec), page)
    }

    /// The selected query, formatted, as a section under the list.
    fn preview_page(&self, width: usize) -> Page {
        let t = &self.theme;
        let mut page = Page::default();
        let Some(backend) = self.visible().get(self.selected).copied() else {
            page.section(t, width, "Preview", None);
            page.line(Line::styled("  No query selected.", t.muted()));
            return page;
        };
        let title = format!(
            "PID {} · {}@{} · {}",
            backend.pid,
            backend.user,
            backend.database,
            backend.duration_s.map(format_duration).unwrap_or_default()
        );
        page.section(t, width, &title, None);
        code_block(
            t,
            &mut page,
            &query_lines(
                &backend.raw_query,
                &backend.query,
                self.server.query_size_limit,
            ),
            width,
        );
        page
    }

    /// The details of one backend, as a page in place of the list.
    fn draw_details(&self, buf: &mut Buffer, area: Rect, popup: &Popup) {
        let t = &self.theme;
        let width = usize::from(area.width);
        if area.height == 0 {
            return;
        }
        // The list's first row, without the spinner: this page is about one
        // backend, not the refresh.
        self.server_row(buf, area, Vec::new(), false);
        put(
            buf,
            area,
            1,
            Line::from(Title::new(t, format!("PID {}", popup.pid)).spans(width)),
        );
        let dot = || Span::styled(" · ", t.subtle());
        let summary = match self.backend_by_pid(&popup.pid) {
            Some(backend) => Line::from(vec![
                Span::styled(backend.database.clone(), t.text()),
                dot(),
                Span::styled(backend.user.clone(), t.muted()),
                dot(),
                Span::styled(or_dash(&backend.state).to_owned(), t.text()),
                dot(),
                Span::styled(
                    backend
                        .duration_s
                        .map(format_duration)
                        .unwrap_or_else(|| "—".to_owned()),
                    t.text(),
                ),
            ]),
            None => Line::styled("ended", t.muted()),
        };
        put(buf, area, 2, Line::from(fit(summary.spans, width)));
        let top = area.y.saturating_add(4).min(area.bottom());
        let view = Rect {
            y: top,
            height: area.bottom() - top,
            ..area
        };
        self.details_page(popup, width)
            .view(t, &popup.scroll)
            .mark(Mark::Dim)
            .render(view, buf);
    }

    fn details_page(&self, popup: &Popup, width: usize) -> Page {
        let t = &self.theme;
        let backend = self.backend_by_pid(&popup.pid);
        let details = popup.details.clone().unwrap_or_default();
        let mut page = Page::default();
        let fact = |page: &mut Page, key: &str, value: Vec<Span<'static>>| {
            for line in fact_lines(t, width, 2, 17, key, value) {
                page.line(line);
            }
        };
        let plain = |text: String| vec![Span::styled(text, t.text())];
        let age = |seconds: Option<f64>, since: &str| {
            let mut spans = vec![Span::styled(
                seconds
                    .map(format_duration)
                    .unwrap_or_else(|| "—".to_owned()),
                t.strong(),
            )];
            if !since.is_empty() {
                spans.push(Span::styled(format!("  (since {since})"), t.muted()));
            }
            spans
        };

        if popup.ended {
            page.line(Line::styled(
                "  This backend has ended. Showing its last known details.",
                t.warning(),
            ));
            page.blank();
        }
        page.section(t, width, "Session", None);
        let backend_text = |field: fn(&Backend) -> String| backend.map(field).unwrap_or_default();
        fact(
            &mut page,
            "Database",
            plain(backend_text(|b| b.database.clone())),
        );
        fact(&mut page, "User", plain(backend_text(|b| b.user.clone())));
        fact(&mut page, "Application", plain(details.application.clone()));
        let client = if details.client.is_empty() {
            "local socket".to_owned()
        } else {
            details.client.clone()
        };
        fact(&mut page, "Client", plain(client));
        fact(
            &mut page,
            "Backend type",
            plain(details.backend_type.clone()),
        );
        fact(&mut page, "State", plain(backend_text(|b| b.state.clone())));
        fact(
            &mut page,
            "Waiting",
            plain(backend_text(Backend::wait_label)),
        );
        page.blank();

        page.section(t, width, "Timing", None);
        fact(
            &mut page,
            "Connection age",
            age(details.connection_age_s, &details.backend_start),
        );
        fact(
            &mut page,
            "Transaction age",
            age(details.xact_age_s, &details.xact_start),
        );
        fact(&mut page, "Query duration", age(details.query_age_s, ""));
        fact(&mut page, "In this state", age(details.state_age_s, ""));
        if !details.xid.is_empty() || !details.xmin.is_empty() {
            fact(
                &mut page,
                "xid / xmin",
                plain(format!(
                    "{} / {}",
                    or_dash(&details.xid),
                    or_dash(&details.xmin)
                )),
            );
        }

        if let Some(os) = &self.os {
            let rate = os.rates.get(&popup.pid);
            let sample = os.samples.get(&popup.pid).map(|(_, sample)| sample);
            let dot = || Span::styled(" · ", t.subtle());
            page.blank();
            page.section(t, width, "Resources", None);
            let cpu = rate
                .and_then(|rate| rate.cpu_percent)
                .map(|cpu| format!("{cpu:.1}%"))
                .unwrap_or_else(|| "—".to_owned());
            let total = sample
                .map(|sample| format_duration(sample.cpu_ticks as f64 / os.ticks_per_second))
                .unwrap_or_else(|| "—".to_owned());
            fact(
                &mut page,
                "CPU",
                vec![
                    Span::styled(format!("{cpu} now"), t.text()),
                    dot(),
                    Span::styled(format!("{total} total"), t.text()),
                ],
            );
            let resident = |rss: u64| {
                [
                    Span::styled(format!("{} resident", format_bytes(rss as f64)), t.text()),
                    Span::styled(" (includes shared buffers)", t.muted()),
                ]
            };
            let memory = match popup.memory {
                Some(memory) => {
                    let mut spans = Vec::new();
                    if let Some(anonymous) = memory.anonymous {
                        spans.push(Span::styled(
                            format!("{} private", format_bytes(anonymous as f64)),
                            t.text(),
                        ));
                        spans.push(dot());
                    }
                    spans.extend(resident(memory.rss));
                    spans
                }
                None => plain("—".to_owned()),
            };
            fact(&mut page, "Memory", memory);
            let per_second = |value: Option<f64>| {
                value
                    .map(|value| format!("{}/s", format_bytes(value)))
                    .unwrap_or_else(|| "—".to_owned())
            };
            fact(
                &mut page,
                "I/O now",
                vec![
                    Span::styled(
                        format!("read {}", per_second(rate.and_then(|rate| rate.read_per_s))),
                        t.text(),
                    ),
                    dot(),
                    Span::styled(
                        format!(
                            "write {}",
                            per_second(rate.and_then(|rate| rate.write_per_s))
                        ),
                        t.text(),
                    ),
                ],
            );
            let io_total = match sample.and_then(|sample| sample.io) {
                Some((read, written)) => vec![
                    Span::styled(format!("read {}", format_bytes(read as f64)), t.text()),
                    dot(),
                    Span::styled(format!("write {}", format_bytes(written as f64)), t.text()),
                ],
                None => plain("—".to_owned()),
            };
            fact(&mut page, "I/O total", io_total);
            let io_wait = match rate.map(|rate| rate.io_wait) {
                Some(true) => "yes",
                Some(false) => "no",
                None => "—",
            };
            fact(&mut page, "I/O wait", plain(io_wait.to_owned()));
        }

        page.blank();
        page.section(t, width, "Query", None);
        code_block(
            t,
            &mut page,
            &query_lines(
                &details.raw_query,
                &details.query,
                self.server.query_size_limit,
            ),
            width,
        );
        page
    }

    fn draw_function_panel(&self, buf: &mut Buffer, screen: Rect, popup: &Popup) {
        let Some(panel) = &popup.function else {
            return;
        };
        let t = &self.theme;
        let count = panel.result.as_ref().map_or(0, Vec::len);
        let position = format!("other matches ({}/{count})", panel.index + 1);
        let mut hints = vec![("↑↓", "scroll")];
        if count > 1 {
            hints.push(("←→", position.as_str()));
        }
        hints.push(("esc", "back"));
        let inner = Modal::new(t, format!("Function in {}", panel.database))
            .width(screen.width * 9 / 10)
            .height(screen.height * 8 / 10)
            .hints(hints)
            .draw(screen, buf);
        let width = usize::from(inner.width);
        let page = match &panel.result {
            Err(message) => message_page(width, message, t.danger()),
            Ok(functions) if functions.is_empty() => message_page(
                width,
                "No user function call found in this query.",
                t.text(),
            ),
            Ok(functions) => function::page(t, &functions[panel.index], width),
        };
        page.view(t, &panel.scroll)
            .mark(Mark::Dim)
            .on_border(inner.right().saturating_add(2))
            .render(inner, buf);
    }

    fn draw_explain_panel(&self, buf: &mut Buffer, screen: Rect, popup: &Popup) {
        let Some(panel) = &popup.explain else {
            return;
        };
        let t = &self.theme;
        let inner = Modal::new(t, format!("Explain in {}", panel.database))
            .width(screen.width * 9 / 10)
            .height(screen.height * 8 / 10)
            .hints([("↑↓", "scroll"), ("esc", "back")])
            .draw(screen, buf);
        let width = usize::from(inner.width);
        let page = match &panel.result {
            Err(message) => message_page(width, message, t.danger()),
            Ok(plan) => {
                let mut page = message_page(
                    width,
                    "Estimated plan (EXPLAIN without ANALYZE). The query was not run, \
                     and the plan may differ from the one the backend is running.",
                    t.info(),
                );
                page.blank();
                let lines: Vec<String> = plan
                    .iter()
                    .map(|line| single_line_keep_indent(line))
                    .collect();
                code_block(t, &mut page, &lines, width);
                page
            }
        };
        page.view(t, &panel.scroll)
            .mark(Mark::Dim)
            .on_border(inner.right().saturating_add(2))
            .render(inner, buf);
    }

    /// The keys of the top layer, in the order the help screen gives them.
    fn draw_hints(&self, buf: &mut Buffer, area: Rect) {
        let pause = if self.paused { "resume" } else { "pause" };
        let overlay = self
            .popup
            .as_ref()
            .is_some_and(|popup| popup.function.is_some() || popup.explain.is_some());
        let hints = if self.help {
            vec![("any key", "close")]
        } else if overlay {
            vec![("↑↓", "scroll"), ("esc", "back")]
        } else if self.popup.is_some() {
            vec![
                ("↑↓", "scroll"),
                ("f", "functions"),
                ("e", "explain"),
                ("space", pause),
                ("esc", "back"),
            ]
        } else {
            vec![
                ("1-4", "view"),
                ("↑↓", "move"),
                ("enter", "details"),
                ("tab", "preview"),
                ("g", "group"),
                ("space", pause),
                ("?", "help"),
                ("q", "quit"),
            ]
        };
        KeyHints::new(&self.theme, hints).render(area, buf);
    }
}

/// A column of a list: its title, its width and the narrowest it may get.
/// A numeric column aligns right, title included.
struct Column {
    title: &'static str,
    width: usize,
    narrowest: usize,
    numeric: bool,
    /// Goes to the query's row when the query moves under the figures.
    second_row: bool,
}

impl Column {
    const fn text(title: &'static str, width: usize, narrowest: usize) -> Self {
        Self {
            title,
            width,
            narrowest,
            numeric: false,
            second_row: false,
        }
    }

    const fn number(title: &'static str, width: usize, narrowest: usize) -> Self {
        Self {
            title,
            width,
            narrowest,
            numeric: true,
            second_row: false,
        }
    }

    const fn on_second_row(self) -> Self {
        Self {
            second_row: true,
            ..self
        }
    }
}

/// The fewest cells the query keeps at the end of a row: a fifth of the
/// row, and at least this.
const QUERY_MIN: usize = 16;
/// Cells between two columns.
const GAP: usize = 1;

/// The widths of a list's columns. The query, last, takes what the others
/// leave. When that is too little, the widest text columns give up cells,
/// down to their narrowest. When that is still not enough, as on a narrow
/// screen with the host's figures, the query moves to a row of its own
/// under the figures, with the columns marked for it.
struct Layout {
    columns: Columns,
    /// Each column's width, in the order of the spec. A column on the
    /// second row has none.
    widths: Vec<usize>,
    query_row: bool,
}

impl Layout {
    fn new(width: usize, spec: &[Column]) -> Self {
        let query_min = (width / 5).max(QUERY_MIN);
        // A row is indented two cells, and each column has a gap after it.
        let used = |widths: &[usize]| 2 + widths.iter().map(|w| w + GAP).sum::<usize>();
        let shrink = |columns: &[&Column], room: usize| {
            let mut widths: Vec<usize> = columns.iter().map(|column| column.width).collect();
            while used(&widths) > room {
                let widest = (0..widths.len())
                    .filter(|&i| widths[i] > columns[i].narrowest)
                    .max_by_key(|&i| widths[i]);
                let Some(widest) = widest else {
                    break;
                };
                widths[widest] -= 1;
            }
            widths
        };
        let all: Vec<&Column> = spec.iter().collect();
        let mut widths = shrink(&all, width.saturating_sub(query_min));
        if used(&widths) + query_min <= width {
            let by_column = widths.clone();
            widths.push(width - used(&widths));
            return Self {
                columns: Columns::with_gap(width, &widths, GAP),
                widths: by_column,
                query_row: false,
            };
        }
        // Without the query, the last column has no gap after it.
        let first: Vec<&Column> = spec.iter().filter(|column| !column.second_row).collect();
        let mut kept = shrink(&first, width + GAP).into_iter();
        let widths = spec
            .iter()
            .map(|column| {
                if column.second_row {
                    0
                } else {
                    kept.next().unwrap_or(0)
                }
            })
            .collect::<Vec<_>>();
        let row: Vec<usize> = spec
            .iter()
            .zip(&widths)
            .filter(|(column, _)| !column.second_row)
            .map(|(_, width)| *width)
            .collect();
        Self {
            columns: Columns::with_gap(width, &row, GAP),
            widths,
            query_row: true,
        }
    }

    /// The width of the column at `index` in the spec.
    fn width(&self, index: usize) -> usize {
        self.widths.get(index).copied().unwrap_or(0)
    }

    /// The column titles in `subtle-foreground`.
    fn head(&self, t: &Theme, spec: &[Column]) -> Line<'static> {
        let mut titles: Vec<Vec<Span<'static>>> = spec
            .iter()
            .zip(&self.widths)
            .filter(|(column, _)| !self.query_row || !column.second_row)
            .map(|(column, width)| {
                let title = if column.numeric {
                    format!("{:>width$}", column.title)
                } else {
                    column.title.to_owned()
                };
                vec![Span::styled(title, t.subtle())]
            })
            .collect();
        if !self.query_row {
            titles.push(vec![Span::styled("Query", t.subtle())]);
        }
        self.columns.row(titles)
    }

    /// The rows of one item: the cells and the query, on one row or two.
    fn lines(
        &self,
        t: &Theme,
        spec: &[Column],
        cells: Vec<Vec<Span<'static>>>,
        query: &str,
        width: usize,
    ) -> Vec<Line<'static>> {
        let query = Span::styled(query.to_owned(), t.muted());
        if !self.query_row {
            let mut cells = cells;
            cells.push(vec![query]);
            return vec![self.columns.row(cells)];
        }
        // A cell moved to the query's row keeps its title before it.
        let mut first = Vec::new();
        let mut second = Vec::new();
        for (column, cell) in spec.iter().zip(cells) {
            if !column.second_row {
                first.push(cell);
            } else if cell.iter().any(|span| !span.content.is_empty()) {
                second.push(Span::styled(format!("{} ", column.title), t.subtle()));
                second.extend(cell);
                second.push(Span::styled(" · ", t.subtle()));
            }
        }
        second.push(query);
        let mut row = vec![Span::raw("    ")];
        row.extend(fit(second, width.saturating_sub(4)));
        vec![self.columns.row(first), Line::from(row)]
    }
}

/// Draws `line` on row `dy` of `area`, if the row is on screen.
fn put(buf: &mut Buffer, area: Rect, dy: u16, line: Line<'_>) {
    let area = area.intersection(buf.area);
    if dy < area.height {
        buf.set_line(area.x, area.y + dy, &line, area.width);
    }
}

/// A fact of a record: the key in `muted-foreground` in a column
/// `key_width` cells wide after `indent` cells, then the value. A value too
/// wide for the page goes on more rows under the first, at whole spans
/// where it can, and a span wider than a row is split.
fn fact_lines(
    t: &Theme,
    width: usize,
    indent: usize,
    key_width: usize,
    key: &str,
    value: Vec<Span<'static>>,
) -> Vec<Line<'static>> {
    let room = width.saturating_sub(indent + key_width).max(1);
    let mut rows: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used = 0;
    for span in value {
        let pieces = if span.width() > room {
            split_cells(&span.content, room)
        } else {
            vec![span.content.to_string()]
        };
        for piece in pieces {
            let mut piece = Span::styled(piece, span.style);
            if used > 0 && used + piece.width() > room {
                rows.push(Vec::new());
                used = 0;
                piece.content = piece.content.trim_start().to_owned().into();
            }
            used += piece.width();
            if let Some(row) = rows.last_mut() {
                row.push(piece);
            }
        }
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, value)| {
            let key = if index == 0 { key } else { "" };
            let mut spans = vec![Span::styled(
                format!("{}{key:<key_width$}", " ".repeat(indent)),
                t.muted(),
            )];
            spans.extend(value);
            Line::from(spans)
        })
        .collect()
}

/// Text of the product, such as a notice or a server error, wrapped to the
/// page. Server text is flattened first.
fn message_page(width: usize, message: &str, style: Style) -> Page {
    let mut page = Page::default();
    for line in split_cells(&single_line(message), width) {
        page.line(Line::styled(line, style));
    }
    page
}

/// `lines` as a code block, after a section title or a blank row, which
/// becomes its half row of fill above, with the blank row of fill below.
/// A line too wide for the block wraps under its own indent.
fn code_block(t: &Theme, page: &mut Page, lines: &[String], width: usize) {
    // The block's text starts four cells in and ends two before the edge.
    let room = width.saturating_sub(6).max(1);
    for line in lines {
        for row in wrap_keep_indent(line, room) {
            page.code(Line::styled(row, t.text()));
        }
    }
    page.blank();
}

/// `line` in rows at most `width` cells wide, broken at the last space
/// that fits, each row keeping the line's indent. The spaces inside a row
/// stay as they are, and a word wider than a row is split.
fn wrap_keep_indent(line: &str, width: usize) -> Vec<String> {
    if Span::raw(line).width() <= width {
        return vec![line.to_owned()];
    }
    let text = line.trim_start();
    let indent = (line.len() - text.len()).min(width / 2);
    let pad = " ".repeat(indent);
    split_cells(text, width.saturating_sub(indent))
        .into_iter()
        .map(|row| format!("{pad}{row}"))
        .collect()
}

/// `text` in pieces at most `room` terminal cells wide, so a CJK character
/// or an emoji counts as two. A piece ends at the last space that fits, or
/// inside a word wider than `room`. Graphemes stay whole, and the spaces at
/// a break are dropped.
fn split_cells(text: &str, room: usize) -> Vec<String> {
    let room = room.max(1);
    let span = Span::raw(text);
    let graphemes: Vec<(&str, usize)> = span
        .styled_graphemes(Style::new())
        .map(|grapheme| (grapheme.symbol, Span::raw(grapheme.symbol).width()))
        .collect();
    let piece = |range: std::ops::Range<usize>| {
        graphemes[range]
            .iter()
            .map(|(symbol, _)| *symbol)
            .collect::<String>()
            .trim_end()
            .to_owned()
    };
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < graphemes.len() {
        let (mut end, mut used, mut space) = (start, 0, None);
        while end < graphemes.len() && used + graphemes[end].1 <= room {
            if graphemes[end].0 == " " {
                space = Some(end);
            }
            used += graphemes[end].1;
            end += 1;
        }
        if end == graphemes.len() {
            pieces.push(piece(start..end));
            break;
        }
        // A grapheme wider than the room still takes a piece of its own.
        let cut = match space {
            Some(at) if at > start => at,
            _ => end.max(start + 1),
        };
        pieces.push(piece(start..cut));
        start = cut;
        while start < graphemes.len() && graphemes[start].0 == " " {
            start += 1;
        }
    }
    pieces
}

/// Every key, grouped by where it works. The footer has room for a few.
const HELP: &[(&str, &[(&str, &str)])] = &[
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
            ("q", "Quit"),
            ("Ctrl+C", "Quit from anywhere"),
        ],
    ),
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
        "Compact columns",
        &[
            ("act", "active"),
            ("txn", "idle in transaction"),
            ("abrt", "idle in transaction (aborted)"),
            ("wrap", "vacuum to prevent wraparound"),
            ("man", "manual vacuum"),
        ],
    ),
    (
        "Query details",
        &[
            ("f", "Functions the query calls"),
            ("e", "Estimated plan (EXPLAIN)"),
            ("↑ ↓ PgUp PgDn", "Scroll"),
            ("v q Esc Enter", "Back to the list"),
        ],
    ),
];

type HelpSections = [(&'static str, &'static [(&'static str, &'static str)])];

/// The key column's width and the widest action of `sections`.
fn help_widths(sections: &HelpSections) -> (usize, usize) {
    let keys = || sections.iter().flat_map(|(_, keys)| keys.iter());
    let key = keys()
        .map(|(key, _)| Span::raw(*key).width())
        .max()
        .unwrap_or(0)
        + 2;
    let action = keys()
        .map(|(_, action)| Span::raw(*action).width())
        .max()
        .unwrap_or(0);
    (key, action)
}

/// One column of the help, `width` cells wide: each section a title and a
/// rule, then its keys. An action too long for the column wraps.
fn help_column(t: &Theme, sections: &HelpSections, width: usize) -> Vec<Line<'static>> {
    let (key_width, _) = help_widths(sections);
    let room = width.saturating_sub(key_width).max(1);
    let mut lines = Vec::new();
    for (title, keys) in sections {
        let mut rule = vec![Span::styled(format!("{title} "), t.muted())];
        rule.extend(
            t.look()
                .rule
                .spans(t, width.saturating_sub(title.len() + 1)),
        );
        lines.push(Line::from(rule));
        for (key, action) in *keys {
            for (index, part) in tui_kiso::wrap(action, room).into_iter().enumerate() {
                let key = if index == 0 { *key } else { "" };
                lines.push(Line::from(vec![
                    Span::styled(format!("{key:<key_width$}"), t.text()),
                    Span::styled(part, t.muted()),
                ]));
            }
        }
    }
    lines
}

/// The help lists the list keys on the left and the rest on the right, so
/// it fits a 24-line terminal. On a narrow one the actions wrap.
fn draw_help(buf: &mut Buffer, t: &Theme, screen: Rect) {
    let (left, right) = (&HELP[..1], &HELP[1..]);
    let natural = |sections: &HelpSections| {
        let (key, action) = help_widths(sections);
        key + action
    };
    // Two columns stay free at each side of the dialog, and its frame and
    // padding take six more.
    let inside = usize::from(screen.width).saturating_sub(10);
    let (left_width, right_width) = if natural(left) + 2 + natural(right) <= inside {
        (natural(left), natural(right))
    } else {
        let half = inside.saturating_sub(2) / 2;
        (half, half)
    };
    let left = help_column(t, left, left_width);
    let right = help_column(t, right, right_width);
    let rows: Vec<Line<'static>> = (0..left.len().max(right.len()))
        .map(|index| {
            let mut spans = fit(
                left.get(index).cloned().unwrap_or_default().spans,
                left_width,
            );
            spans.push(Span::raw("  "));
            spans.extend(right.get(index).cloned().unwrap_or_default().spans);
            Line::from(spans)
        })
        .collect();
    Modal::new(t, "Help")
        .body(Text::from(rows))
        .hints([("any key", "close")])
        .render(screen, buf);
}

/// Drops control characters from one plan line but keeps its leading
/// spaces, which carry the plan's tree shape.
fn single_line_keep_indent(line: &str) -> String {
    line.chars().filter(|c| !c.is_control()).collect()
}

fn or_dash(value: &str) -> &str {
    if value.is_empty() {
        "—"
    } else {
        value
    }
}

fn is_local(endpoint: &EndpointIdentity) -> bool {
    endpoint.endpoint_kind == EndpointKind::UnixSocket
        || matches!(endpoint.host.as_str(), "localhost" | "127.0.0.1" | "::1")
}

/// Bytes with a binary unit suffix, as top tools print them.
fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
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

/// The compact list's states. The help screen spells them out.
fn short_state(state: &str) -> &str {
    match state {
        "active" => "act",
        "idle in transaction" => "txn",
        "idle in transaction (aborted)" => "abrt",
        other => other,
    }
}

/// The compact vacuum view's types. The help screen spells them out.
fn short_kind(kind: &str) -> &str {
    match kind {
        "wraparound" => "wrap",
        "manual" => "man",
        other => other,
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

    fn buffer(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn rows(buffer: &Buffer) -> Vec<String> {
        let area = buffer.area;
        (0..area.height)
            .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn the_terminal_colors_are_set_and_put_back() {
        let mut out = Vec::new();
        terminal_colors(&mut out, Some(&kiso_tokens::GOLD_DARK)).unwrap();
        let [r, g, b] = kiso_tokens::GOLD_DARK.background;
        let set = String::from_utf8_lossy(&out).into_owned();
        assert!(
            set.starts_with(&format!("\x1b]11;#{r:02x}{g:02x}{b:02x}\x1b\\\x1b]10;#")),
            "{set:?}"
        );
        out.clear();
        terminal_colors(&mut out, None).unwrap();
        assert_eq!(out, b"\x1b]111\x1b\\\x1b]110\x1b\\");
    }

    #[test]
    fn terminal_colors_need_a_terminal_without_no_color() {
        assert!(takes_terminal_colors(true, false));
        assert!(!takes_terminal_colors(true, true));
        assert!(!takes_terminal_colors(false, false));
    }

    static PANIC_OUT: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

    struct PanicOut;

    impl Write for PanicOut {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            PANIC_OUT.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn chained_panic_hooks_put_the_colors_back_once() {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        // The hooks of top and of the capture progress, chained.
        restore_terminal_on_panic(|| PanicOut);
        restore_terminal_on_panic(|| PanicOut);
        paint_terminal(&mut Vec::new(), &kiso_tokens::GOLD_DARK);
        let _ = std::panic::catch_unwind(|| panic!("a panic in a view"));
        std::panic::set_hook(original);
        assert_eq!(*PANIC_OUT.lock().unwrap(), b"\x1b]111\x1b\\\x1b]110\x1b\\");
    }

    #[test]
    fn every_layer_fits_80_by_24_inside_the_side_margins() {
        let mut list = app(View::Running);
        list.os = Some(OsState::new());
        list.backends[0].query = format!("SELECT {} FROM t", "column_name, ".repeat(20));
        let mut help = app(View::Running);
        help.help = true;
        let mut details = app(View::Running);
        details.popup = Some(Popup {
            pid: "101".to_owned(),
            details: Some(Details::default()),
            memory: None,
            ended: false,
            scroll: Scroll::default(),
            function: None,
            explain: None,
        });
        for (name, app) in [
            ("list", &mut list),
            ("help", &mut help),
            ("details", &mut details),
        ] {
            let rows = rows(&buffer(app, 80, 24));
            for row in &rows {
                let cells: Vec<char> = row.chars().collect();
                let margins = [cells[0], cells[1], cells[78], cells[79]];
                assert!(margins.iter().all(|c| *c == ' '), "{name}: {row:?}");
            }
            if name == "help" {
                let screen = rows.join("\n");
                assert!(screen.contains("Quit from anywhere"), "{screen}");
                assert!(screen.contains("Back to the list"), "{screen}");
                assert!(screen.contains("╰─ any key close"), "{screen}");
            }
        }
        // The query does not fit after the host's figures, so it moves under
        // them, with the wait and its title.
        let screen = rows(&buffer(&mut list, 80, 24)).join("\n");
        assert!(screen.contains("  ▌   SELECT column_name,"), "{screen}");
        assert!(screen.contains("      Waiting Lock · UPDATE t"), "{screen}");
    }

    #[test]
    fn the_selection_stays_on_screen_when_the_preview_opens() {
        let mut app = app(View::Running);
        app.backends = (100..130)
            .map(|pid| backend(&pid.to_string(), "active", "", "SELECT 1"))
            .collect();
        app.paused = true;
        app.selected = 25;
        assert!(rows(&buffer(&mut app, 160, 24))
            .iter()
            .any(|row| row.contains("▌ 125")));
        app.preview = true;
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen = rows(terminal.backend().buffer()).join("\n");
        assert!(screen.contains("▌ 125"), "{screen}");
    }

    #[test]
    fn a_long_fact_wraps_without_losing_text() {
        let theme = Theme::truecolor(kiso_tokens::GOLD_DARK);
        let value = format!("{} {}", "a".repeat(50), "b".repeat(10));
        let lines = fact_lines(&theme, 40, 2, 17, "Application", vec![Span::raw(value)]);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.width() <= 40), "{lines:?}");
        let text: String = lines.iter().map(|line| line.to_string()).collect();
        assert_eq!(text.matches('a').count(), 50 + 1, "{text}");
        assert_eq!(text.matches('b').count(), 10);
    }

    #[test]
    fn wide_characters_wrap_by_terminal_cells() {
        let line = format!("  SELECT '{}'", "中".repeat(40));
        let rows = wrap_keep_indent(&line, 68);
        assert!(rows.len() > 1, "{rows:?}");
        assert!(
            rows.iter().all(|row| Span::raw(row.as_str()).width() <= 68),
            "{rows:?}"
        );
        assert!(rows.iter().all(|row| row.starts_with("  ")));
        let text: String = rows
            .iter()
            .map(|row| row.trim_start())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text.matches('中').count(), 40);
    }

    #[test]
    fn the_selected_row_has_the_mark_and_the_selected_fill() {
        let mut app = app(View::Running);
        app.theme = Theme::truecolor(kiso_tokens::GOLD_DARK);
        app.selected = 1;
        let buffer = buffer(&mut app, 160, 13);
        let rows = rows(&buffer);
        let row = |pid: &str| rows.iter().position(|row| row.contains(pid)).unwrap() as u16;
        let selected = buffer[(2, row("102"))].clone();
        assert_eq!(selected.symbol(), "▌");
        assert_eq!(selected.bg, app.theme.selected_color());
        assert_eq!(buffer[(2, row("101"))].symbol(), " ");
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
        assert!(screen.contains(" Running 2 "), "{screen}");
        assert!(screen.contains("SELECT pg_sleep(5)"));
        assert!(!screen.contains("103"));
        assert!(screen.contains("Sessions  3/100"));
        assert!(screen.contains(concat!(" pg-probe ", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn waiting_view_lists_only_lock_waits() {
        let screen = screen(&mut app(View::Waiting));
        assert!(screen.contains(" Waiting 1 ━"), "{screen}");
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
        assert!(screen.contains("2.0 KiB"));
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
        assert_eq!(format_bytes(512.0), "512 B");
        assert_eq!(format_bytes(378_572.8), "369.7 KiB");
        assert_eq!(format_bytes(6_098_812_928.0), "5.7 GiB");
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
        assert!(screen.lines().any(|l| l.trim() == "from"), "{screen}");
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
        assert!(screen.contains("User@database"), "{screen}");
        assert!(screen.contains("postgres@bench"), "{screen}");
        assert!(screen.contains(" txn "), "{screen}");
    }

    #[test]
    fn grouping_sorts_sessions_under_a_title_per_group() {
        let mut app = app(View::Running);
        app.backends[0].database = "reports".to_owned();
        app.group = Group::Database;
        let pids: Vec<&str> = app.visible().iter().map(|b| b.pid.as_str()).collect();
        assert_eq!(pids, ["102", "101"]);
        app.selected = 1;
        let screen = screen(&mut app);
        assert!(
            screen.contains("Running 2 · grouped by database"),
            "{screen}"
        );
        let lines: Vec<&str> = screen.lines().collect();
        let section = |name: &str| {
            lines.iter().position(|l| {
                l.starts_with(&format!("  {name} ─")) && l.trim_end().ends_with("─ 1")
            })
        };
        assert!(section("bench").is_some(), "{screen}");
        let reports = section("reports").expect("a section per group");
        assert!(lines[reports + 2].starts_with("  ▌ 101"), "{screen}");
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
        assert!(lines[2].contains(" Running 2 "), "{screen}");
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
        assert!(!screen.contains("Phase"), "{screen}");
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
        assert!(screen.contains("Table@database"), "{screen}");
        assert!(screen.contains("public.t@app"), "{screen}");
        assert!(screen.contains(" wrap "), "{screen}");
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
        assert!(screen.contains(" Vacuum 1 "), "{screen}");
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
            scroll: Scroll::default(),
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
