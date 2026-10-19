//! The default capture progress on stderr.
//!
//! The library reports each collector outcome as an info event, and the start
//! of the capture and of each sample as debug events, which `--verbose` leaves
//! out. This subscriber turns them into a summary
//! instead of one line per outcome, which stays a `--verbose` feature.
//!
//! On a terminal it redraws a block in place: progress, collector counts per
//! domain, sample time and the issues so far. Elsewhere, such as a pipe, a file
//! or systemd, it only appends lines and only when something changes, plus one
//! progress line per minute.
//!
//! It also shows what pg-probe itself costs the host: CPU time, peak memory
//! and disk writes, from `getrusage`. The goal is a footprint near zero, so
//! the operator can see it.
//!
//! Everything comes from events already in memory and one system call.
//! Showing progress adds no work on the observed server (principle 1), and it
//! states facts without judging them (principle 2): no thresholds and no
//! health colors.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pg_probe::top::palette;
use ratatui::crossterm::style::{
    Attribute, Color as CColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use ratatui::crossterm::Command;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// Samples between percentile updates, so the values hold still on screen.
/// The first p95 and p99 appear once this many samples exist.
const PERCENTILE_STEP: usize = 10;
/// Time between progress lines when stderr is not a terminal.
const HEARTBEAT: Duration = Duration::from_secs(60);
/// Most error groups the live block lists; the rest are counted.
const MAX_ERROR_LINES: usize = 6;
const BAR_WIDTH: usize = 30;

/// Installs the progress subscriber as the global default and returns the
/// handle that ends the display once the capture returns.
/// `refresh` is the time between redraws on a terminal.
pub fn install(refresh: Duration) -> Progress {
    let live = std::io::stderr().is_terminal();
    let color = use_color();
    let state = Arc::new(Mutex::new(State::new(live, color)));
    let stop = Arc::new(AtomicBool::new(false));

    // The tracing global default can only be set once; a second install
    // would be a programming error, so the result is ignored like `init`.
    let _ = tracing::subscriber::set_global_default(ProgressSubscriber {
        state: Arc::clone(&state),
    });

    let ticker = live.then(|| {
        let state = Arc::clone(&state);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || loop {
            std::thread::park_timeout(refresh);
            if stop.load(Ordering::Relaxed) {
                break;
            }
            lock(&state).redraw();
        })
    });

    Progress {
        state,
        stop,
        ticker,
    }
}

/// Ends the display. The final block or summary line stays on stderr, and
/// later events print as plain lines below it.
pub struct Progress {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
}

impl Progress {
    /// Ends the display. `artifact` is the file the capture left: the
    /// published artifact, or the partial one when interrupted.
    pub fn finish(mut self, interrupted: bool, artifact: Option<&Path>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(ticker) = self.ticker.take() {
            ticker.thread().unpark();
            let _ = ticker.join();
        }
        lock(&self.state).finish(interrupted, artifact);
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ProgressSubscriber {
    state: Arc<Mutex<State>>,
}

impl Subscriber for ProgressSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::DEBUG && metadata.target().starts_with("pg_probe")
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::DEBUG)
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        lock(&self.state).event(*event.metadata().level(), fields);
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// The fields of one event. Known fields are kept by name; the rest are kept
/// in order so a message can still show them.
#[derive(Default)]
struct Fields {
    message: String,
    sample: Option<u64>,
    duration_s: Option<u64>,
    interval_s: Option<u64>,
    named: BTreeMap<&'static str, String>,
    other: Vec<(&'static str, String)>,
}

impl Fields {
    fn take(&mut self, name: &str) -> String {
        self.named.remove(name).unwrap_or_default()
    }

    fn set(&mut self, name: &'static str, value: String) {
        match name {
            "message" => self.message = value,
            "source" | "status" | "error_code" | "detail" | "artifact" => {
                self.named.insert(name, value);
            }
            _ => self.other.push((name, value)),
        }
    }
}

impl Visit for Fields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "sample" => self.sample = Some(value),
            "duration_s" => self.duration_s = Some(value),
            "interval_s" => self.interval_s = Some(value),
            name => self.set(name, value.to_string()),
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.set(field.name(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.set(field.name(), format!("{value:?}"));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    status: String,
    error_code: String,
    detail: String,
}

impl Outcome {
    fn kind(&self) -> Kind {
        match self.status.as_str() {
            "collected" => Kind::Collected,
            "unsupported" => Kind::Unsupported,
            _ => Kind::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Collected,
    Unsupported,
    Error,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    collected: usize,
    unsupported: usize,
    errors: usize,
}

/// Repeated failures of one source with the same status and code.
#[derive(Debug, Clone)]
struct ErrorGroup {
    times: u64,
    last_sample: u64,
    detail: String,
    /// Whether a later sample collected the source again.
    recovered: bool,
}

/// The error groups of one status and code, across sources.
#[derive(Debug, Default)]
struct SharedErrors {
    sources: Vec<String>,
    last_sample: u64,
    detail: String,
    /// Whether every source collected again after its last failure.
    recovered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    Running,
    Done,
    Interrupted,
}

struct State {
    live: bool,
    color: bool,
    started: Option<Instant>,
    artifact: Option<PathBuf>,
    duration: Option<Duration>,
    interval: Option<Duration>,
    sample: u64,
    sample_started: Option<Instant>,
    sampling_now: bool,
    /// This sample's outcome per source. A source can report more than once;
    /// the first failure wins over a success.
    current: BTreeMap<String, Outcome>,
    snapshot: Counts,
    snapshot_time: Option<Duration>,
    /// Per domain: counts of the last repeated sample, errors of all of them.
    latest: BTreeMap<&'static str, Counts>,
    repeated_errors: BTreeMap<&'static str, usize>,
    sample_times: Vec<Duration>,
    /// p95 and p99 of `sample_times`, updated every `PERCENTILE_STEP` samples.
    p95: Option<Duration>,
    p99: Option<Duration>,
    unsupported: BTreeMap<String, BTreeSet<String>>,
    errors: BTreeMap<(String, String, String), ErrorGroup>,
    failing: BTreeSet<String>,
    last_heartbeat: Duration,
    /// CPU time used when the capture started and when the snapshot ended,
    /// so the sampling phase can be measured on its own.
    cpu_at_start: Option<(Instant, Duration)>,
    cpu_at_sampling: Option<(Instant, Duration)>,
    finish: Finish,
    /// Whether the final block is on screen for good.
    frozen: bool,
    /// Lines of the live block now on screen, below which the cursor sits.
    drawn: usize,
}

impl State {
    fn new(live: bool, color: bool) -> Self {
        Self {
            live,
            color,
            started: None,
            artifact: None,
            duration: None,
            interval: None,
            sample: 0,
            sample_started: None,
            sampling_now: false,
            current: BTreeMap::new(),
            snapshot: Counts::default(),
            snapshot_time: None,
            latest: BTreeMap::new(),
            repeated_errors: BTreeMap::new(),
            sample_times: Vec::new(),
            p95: None,
            p99: None,
            unsupported: BTreeMap::new(),
            errors: BTreeMap::new(),
            failing: BTreeSet::new(),
            last_heartbeat: Duration::ZERO,
            cpu_at_start: None,
            cpu_at_sampling: None,
            finish: Finish::Running,
            frozen: false,
            drawn: 0,
        }
    }

    fn event(&mut self, level: Level, mut fields: Fields) {
        match fields.message.as_str() {
            "capture started" => self.capture_started(&mut fields),
            "sample started" => self.sample_started(fields.sample.unwrap_or(1)),
            "sample finished" => self.sample_finished(),
            "collector outcome" => {
                let source = fields.take("source");
                let outcome = Outcome {
                    status: fields.take("status"),
                    error_code: fields.take("error_code"),
                    detail: fields.take("detail"),
                };
                self.outcome(source, outcome);
            }
            _ => self.message(level, fields),
        }
    }

    fn capture_started(&mut self, fields: &mut Fields) {
        self.started = Some(Instant::now());
        self.cpu_at_start = usage().map(|usage| (Instant::now(), usage.cpu));
        self.artifact = Some(PathBuf::from(fields.take("artifact")));
        self.duration = fields.duration_s.map(Duration::from_secs);
        self.interval = fields.interval_s.map(Duration::from_secs);
        if !self.live {
            let mut line = String::from("capture started");
            if let (Some(duration), Some(interval)) = (self.duration, self.interval) {
                let _ = write!(
                    line,
                    " duration={} interval={}",
                    span(duration),
                    span(interval)
                );
            }
            if let Some(artifact) = &self.artifact {
                let _ = write!(line, " artifact={}", artifact.display());
            }
            self.print(&line);
        }
        self.redraw();
    }

    fn sample_started(&mut self, sample: u64) {
        self.sample = sample;
        self.sample_started = Some(Instant::now());
        self.sampling_now = true;
        self.current.clear();
    }

    fn outcome(&mut self, source: String, outcome: Outcome) {
        match self.current.get(&source) {
            Some(existing) if existing.kind() != Kind::Collected => {}
            _ => {
                self.current.insert(source, outcome);
            }
        }
    }

    fn sample_finished(&mut self) {
        let took = self
            .sample_started
            .map(|started| started.elapsed())
            .unwrap_or_default();
        self.sampling_now = false;
        let sample = self.sample;
        let current = std::mem::take(&mut self.current);

        let mut counts: BTreeMap<&'static str, Counts> = BTreeMap::new();
        let mut total = Counts::default();
        let mut new_unsupported: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut new_errors = Vec::new();
        let mut recovered = Vec::new();
        let mut failing = BTreeSet::new();

        for (source, outcome) in &current {
            let domain_counts = counts.entry(domain(source)).or_default();
            match outcome.kind() {
                Kind::Collected => {
                    domain_counts.collected += 1;
                    total.collected += 1;
                    if self.failing.contains(source) {
                        recovered.push(source.clone());
                    }
                    for ((group_source, _, _), group) in &mut self.errors {
                        if group_source == source {
                            group.recovered = true;
                        }
                    }
                }
                Kind::Unsupported => {
                    domain_counts.unsupported += 1;
                    total.unsupported += 1;
                    if self
                        .unsupported
                        .entry(outcome.error_code.clone())
                        .or_default()
                        .insert(source.clone())
                    {
                        new_unsupported
                            .entry(outcome.error_code.clone())
                            .or_default()
                            .push(source.clone());
                    }
                }
                Kind::Error => {
                    domain_counts.errors += 1;
                    total.errors += 1;
                    failing.insert(source.clone());
                    let key = (
                        source.clone(),
                        outcome.status.clone(),
                        outcome.error_code.clone(),
                    );
                    let group = self.errors.entry(key).or_insert(ErrorGroup {
                        times: 0,
                        last_sample: sample,
                        detail: String::new(),
                        recovered: false,
                    });
                    group.times += 1;
                    group.last_sample = sample;
                    group.detail = outcome.detail.clone();
                    group.recovered = false;
                    if !self.failing.contains(source) {
                        new_errors.push((source.clone(), outcome.clone()));
                    }
                }
            }
        }
        self.failing = failing;

        if sample <= 1 {
            self.snapshot = total;
            self.snapshot_time = Some(took);
            if self.duration.is_some() {
                self.cpu_at_sampling = usage().map(|usage| (Instant::now(), usage.cpu));
            }
        } else {
            self.sample_times.push(took);
            if self.sample_times.len() % PERCENTILE_STEP == 0 {
                self.p95 = percentile(&self.sample_times, 95);
                self.p99 = percentile(&self.sample_times, 99);
            }
            for (domain, domain_counts) in &counts {
                *self.repeated_errors.entry(domain).or_default() += domain_counts.errors;
            }
            self.latest = counts;
        }

        if !self.live {
            if sample <= 1 {
                self.print(&format!(
                    "snapshot collected={} unsupported={} errors={} time={}",
                    total.collected,
                    total.unsupported,
                    total.errors,
                    short(took)
                ));
            }
            for (code, sources) in &new_unsupported {
                self.print(&format!(
                    "unsupported sample={sample} reason={code} sources={}",
                    field(sources)
                ));
            }
            let mut shared: BTreeMap<(&str, &str, &str), Vec<String>> = BTreeMap::new();
            for (source, outcome) in &new_errors {
                shared
                    .entry((&outcome.status, &outcome.error_code, &outcome.detail))
                    .or_default()
                    .push(source.clone());
            }
            for ((status, code, detail), sources) in &shared {
                let mut line = format!("error sample={sample} status={status} code={code}");
                if !detail.is_empty() && *detail != "-" {
                    let _ = write!(line, " detail={detail:?}");
                }
                let _ = write!(line, " sources={}", field(sources));
                self.print(&line);
            }
            if !recovered.is_empty() {
                self.print(&format!(
                    "recovered sample={sample} sources={}",
                    field(&recovered)
                ));
            }
            let elapsed = self.elapsed();
            if sample > 1 && elapsed >= self.last_heartbeat + HEARTBEAT {
                self.last_heartbeat = elapsed;
                let line = format!("progress {}", self.summary());
                self.print(&line);
            }
        }
        self.redraw();
    }

    fn message(&mut self, level: Level, mut fields: Fields) {
        let mut line = match level {
            Level::ERROR => format!("error: {}", fields.message),
            Level::WARN => format!("warning: {}", fields.message),
            _ => fields.message.clone(),
        };
        if let Some(artifact) = fields.named.remove("artifact") {
            let _ = write!(line, " artifact={artifact}");
        }
        for (name, value) in &fields.other {
            let _ = write!(line, " {name}={value}");
        }
        self.print(&line);
    }

    fn finish(&mut self, interrupted: bool, artifact: Option<&Path>) {
        if self.started.is_none() {
            return;
        }
        if let Some(artifact) = artifact {
            self.artifact = Some(artifact.to_path_buf());
        }
        self.finish = if interrupted {
            Finish::Interrupted
        } else {
            Finish::Done
        };
        if self.live {
            self.redraw();
            // The block stays on screen; later lines print below it.
            self.frozen = true;
            self.drawn = 0;
        } else {
            let word = if interrupted {
                "capture interrupted"
            } else {
                "capture finished"
            };
            let line = format!("{word} {}", self.summary());
            self.print(&line);
        }
    }

    fn elapsed(&self) -> Duration {
        self.started
            .map(|started| started.elapsed())
            .unwrap_or_default()
    }

    /// The fields of a plain progress or finish line.
    fn summary(&self) -> String {
        let errors: u64 = self.errors.values().map(|group| group.times).sum();
        let mut line = format!(
            "samples={} elapsed={}",
            self.sample.max(1),
            clock(self.elapsed())
        );
        if !self.sample_times.is_empty() {
            let _ = write!(
                line,
                " sample_time_p95={} sample_time_p99={}",
                optional(self.p95),
                optional(self.p99),
            );
        }
        let _ = write!(line, " errors={errors}");
        if let Some(size) = self.artifact_size() {
            let _ = write!(line, " size={}", iec(size));
        }
        if let Some(footprint) = self.footprint() {
            if let Some(cpu) = footprint.cpu_sampling {
                let _ = write!(line, " cpu_sampling={}", percent(cpu));
            }
            if let Some(cpu) = footprint.cpu_overall {
                let _ = write!(line, " cpu_overall={}", percent(cpu));
            }
            let _ = write!(
                line,
                " peak_memory={} disk_written={}",
                iec(footprint.peak_memory).replace(' ', ""),
                iec(footprint.written).replace(' ', "")
            );
        }
        line
    }

    /// What pg-probe has cost the host so far. CPU is a share of one core.
    fn footprint(&self) -> Option<Footprint> {
        let now = usage()?;
        let share = |since: Option<(Instant, Duration)>| {
            let (at, cpu) = since?;
            let wall = at.elapsed().as_secs_f64();
            (wall >= 1.0).then(|| now.cpu.saturating_sub(cpu).as_secs_f64() / wall * 100.0)
        };
        Some(Footprint {
            cpu_overall: share(self.cpu_at_start),
            cpu_sampling: share(self.cpu_at_sampling),
            peak_memory: now.peak_memory,
            written: now.written,
        })
    }

    fn artifact_size(&self) -> Option<u64> {
        let path = self.artifact.as_ref()?;
        std::fs::metadata(path).ok().map(|metadata| metadata.len())
    }

    /// Prints one line: above the live block on a terminal, or appended with
    /// a timestamp elsewhere.
    fn print(&mut self, line: &str) {
        let mut out = std::io::stderr().lock();
        if self.live {
            // On the page, the level sits in the label column.
            let (label, style, text) = if let Some(text) = line.strip_prefix("error: ") {
                ("error", palette::danger(), text)
            } else if let Some(text) = line.strip_prefix("warning: ") {
                ("warning", palette::warning(), text)
            } else {
                ("info", palette::muted(), line)
            };
            let mut page = String::new();
            write_line(
                &mut page,
                &Line::from(vec![
                    Span::styled(format!(" {label:<16}"), style),
                    Span::raw(text.to_string()),
                ]),
                terminal_width(),
                self.color,
            );
            self.clear(&mut out);
            let _ = out.write_all(page.as_bytes());
            self.drawn = 0;
            let _ = out.flush();
            drop(out);
            self.redraw();
        } else {
            let stamp = pg_probe::contract::now_utc().format("%Y-%m-%dT%H:%M:%SZ");
            let _ = writeln!(out, "{stamp} {line}");
        }
    }

    fn clear(&self, out: &mut impl Write) {
        if self.drawn > 0 {
            let _ = write!(out, "\x1b[{}A\r\x1b[J", self.drawn);
        }
    }

    fn redraw(&mut self) {
        if !self.live || self.started.is_none() || self.frozen {
            return;
        }
        let width = terminal_width();
        let lines = self.block(width);
        // Overwrite the block in place instead of clearing it first, inside
        // a synchronized update where the terminal supports one, so it does
        // not flicker. Each line clears its own tail; whatever is left below
        // a shorter block is cleared at the end.
        let mut text = String::from("\x1b[?2026h");
        if self.drawn > 0 {
            let _ = write!(text, "\x1b[{}A\r", self.drawn);
        }
        for line in &lines {
            let mut row = String::new();
            write_line(&mut row, line, width, self.color);
            text.push_str(&row.replace('\n', "\x1b[K\n"));
        }
        text.push_str("\x1b[J\x1b[?2026l");
        let mut out = std::io::stderr().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
        self.drawn = lines.len();
    }

    /// The live block, drawn like a page of `pg-probe top`: the brand badge,
    /// full-width section bars and a label column.
    fn block(&self, width: usize) -> Vec<Line<'static>> {
        let label = |text: &str| Span::styled(format!(" {text:<16}"), palette::muted());
        let field = |name: &str, value: Vec<Span<'static>>| {
            let mut spans = vec![label(name)];
            spans.extend(value);
            Line::from(spans)
        };
        let section =
            |name: &str| Line::from(format!(" {name:<width$}")).style(palette::table_header());
        let strong = |text: String| Span::styled(text, palette::strong());
        let plain = |text: String| Span::raw(text);
        let muted = |text: String| Span::styled(text, palette::muted());

        let elapsed = self.elapsed();
        let mut lines = Vec::new();

        lines.push(section("Progress"));
        match self.duration {
            Some(duration) => {
                let fraction = (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0);
                let filled = (fraction * BAR_WIDTH as f64).round() as usize;
                lines.push(field("Samples", vec![strong(self.sample.to_string())]));
                lines.push(field(
                    "Elapsed",
                    vec![
                        strong(clock(elapsed)),
                        muted(format!(" of {}  ", clock(duration))),
                        Span::styled("█".repeat(filled), palette::link()),
                        Span::styled("░".repeat(BAR_WIDTH - filled), palette::subtle()),
                    ],
                ));
                if self.finish == Finish::Running {
                    let next = if self.sampling_now {
                        let running = self
                            .sample_started
                            .map(|started| started.elapsed())
                            .unwrap_or_default();
                        let mut text = format!("taking sample {}", self.sample);
                        if running >= Duration::from_secs(1) {
                            let _ = write!(text, " for {}s", running.as_secs());
                        }
                        vec![plain(text)]
                    } else {
                        match self.next_sample(elapsed) {
                            Some(next) => vec![
                                muted("in ".to_string()),
                                strong(format!("{}s", next.as_secs() + 1)),
                            ],
                            None => vec![muted("none, finishing".to_string())],
                        }
                    };
                    lines.push(field("Next sample", next));
                } else {
                    let status = if self.finish == Finish::Done {
                        "done"
                    } else {
                        "interrupted"
                    };
                    lines.push(field("Status", vec![strong(status.to_string())]));
                }
            }
            None => {
                let value = match (self.finish, self.snapshot_time) {
                    (Finish::Interrupted, _) => vec![plain("interrupted".to_string())],
                    (_, Some(took)) => vec![muted("taken in ".to_string()), strong(short(took))],
                    _ => vec![plain(format!("taking it · {}", clock(elapsed)))],
                };
                lines.push(field("Snapshot", value));
            }
        }
        if let Some(artifact) = &self.artifact {
            let mut value = vec![plain(artifact.display().to_string())];
            if let Some(size) = self.artifact_size() {
                value.push(muted(format!("  {}", iec(size))));
            }
            lines.push(field("Artifact", value));
        }
        lines.push(Line::default());

        lines.push(section("Collectors"));
        lines.push(field(
            "",
            vec![muted(format!("{:>6}{:>7}{:>8}", "ok", "unsup", "errors"))],
        ));
        let counts = |counts: Counts, errors: usize, note: &str| {
            let errors = if errors > 0 {
                Span::styled(format!("{errors:>8}"), palette::danger())
            } else {
                muted(format!("{errors:>8}"))
            };
            vec![
                strong(format!("{:>6}", counts.collected)),
                plain(format!("{:>7}", counts.unsupported)),
                errors,
                muted(note.to_string()),
            ]
        };
        if self.snapshot_time.is_some() {
            lines.push(field(
                "Snapshot",
                counts(self.snapshot, self.snapshot.errors, ""),
            ));
        }
        for (domain, domain_counts) in &self.latest {
            let errors = self.repeated_errors.get(domain).copied().unwrap_or(0);
            let name = if *domain == "postgres" {
                "PostgreSQL"
            } else {
                "Linux"
            };
            lines.push(field(name, counts(*domain_counts, errors, "  each sample")));
        }
        if self.duration.is_some() {
            lines.push(field(
                "",
                vec![muted(
                    "ok and unsup: last sample · errors: all samples".to_string(),
                )],
            ));
        }
        lines.push(Line::default());

        lines.push(section("Cost"));
        if let Some(last) = self.sample_times.last() {
            lines.push(field(
                "Sample time",
                vec![
                    strong(short(*last)),
                    muted(" last · p95 ".to_string()),
                    percentile_span(self.p95),
                    muted(" · p99 ".to_string()),
                    percentile_span(self.p99),
                    muted(format!(" · {} samples", self.sample_times.len())),
                ],
            ));
        }
        if let Some(footprint) = self.footprint() {
            let mut cpu = Vec::new();
            if let Some(share) = footprint.cpu_sampling {
                cpu.push(strong(percent(share)));
                cpu.push(muted(" sampling · ".to_string()));
            }
            match footprint.cpu_overall {
                Some(share) => {
                    cpu.push(strong(percent(share)));
                    cpu.push(muted(" overall".to_string()));
                }
                None => cpu.push(strong("—".to_string())),
            }
            lines.push(field("CPU", cpu));
            lines.push(field(
                "Memory",
                vec![
                    strong(iec(footprint.peak_memory)),
                    muted(" peak resident".to_string()),
                ],
            ));
            lines.push(field("Disk writes", vec![strong(iec(footprint.written))]));
        }

        if !self.errors.is_empty() || !self.unsupported.is_empty() {
            lines.push(Line::default());
            lines.push(section("Issues"));
            // Sources that fail the same way share one line.
            let mut shared: BTreeMap<(&str, &str), SharedErrors> = BTreeMap::new();
            for ((source, status, code), group) in &self.errors {
                let entry = shared.entry((status, code)).or_default();
                entry.sources.push(source.clone());
                entry.last_sample = entry.last_sample.max(group.last_sample);
                entry.recovered = group.recovered && (entry.sources.len() == 1 || entry.recovered);
                entry.detail.clone_from(&group.detail);
            }
            let mut shared: Vec<_> = shared.into_iter().collect();
            shared.sort_by_key(|(_, errors)| std::cmp::Reverse(errors.last_sample));
            for ((status, code), errors) in shared.iter().take(MAX_ERROR_LINES) {
                let mut spans = vec![
                    Span::styled(format!(" {:<16}", "error"), palette::danger()),
                    Span::styled(format!("{status}  {code}"), palette::danger()),
                    muted(format!("  last in sample {}", errors.last_sample)),
                ];
                if errors.recovered {
                    spans.push(muted(" · collected since".to_string()));
                }
                if !errors.detail.is_empty() && errors.detail != "-" {
                    spans.push(muted(format!(" · {}", errors.detail)));
                }
                spans.push(plain(format!("  {}", compact(&errors.sources))));
                lines.push(Line::from(spans));
            }
            if shared.len() > MAX_ERROR_LINES {
                lines.push(field(
                    "",
                    vec![muted(format!(
                        "+ {} more kinds of error",
                        shared.len() - MAX_ERROR_LINES
                    ))],
                ));
            }
            for (code, sources) in &self.unsupported {
                let sources: Vec<String> = sources.iter().cloned().collect();
                lines.push(field(
                    "unsupported",
                    vec![plain(compact(&sources)), muted(format!("  {code}"))],
                ));
            }
        }

        lines.push(Line::default());
        if self.finish == Finish::Running {
            lines.push(Line::from(muted(
                " Ctrl-C stops and keeps what was collected".to_string(),
            )));
        }
        lines
    }

    /// Time until the next sample, following the library's schedule: one
    /// tick per interval from the start, while the tick is within the
    /// duration.
    fn next_sample(&self, elapsed: Duration) -> Option<Duration> {
        let interval = self.interval?.as_secs().max(1);
        let duration = self.duration?.as_secs();
        let next = (elapsed.as_secs() / interval + 1) * interval;
        (next <= duration).then(|| Duration::from_secs(next).saturating_sub(elapsed))
    }
}

struct Footprint {
    cpu_overall: Option<f64>,
    cpu_sampling: Option<f64>,
    peak_memory: u64,
    written: u64,
}

struct Usage {
    cpu: Duration,
    peak_memory: u64,
    written: u64,
}

/// This process's CPU time, peak resident memory and bytes written to block
/// devices, from one `getrusage` call.
fn usage() -> Option<Usage> {
    let mut raw: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `raw` is a valid, writable `rusage` for the call to fill.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut raw) } != 0 {
        return None;
    }
    let time = |value: libc::timeval| {
        Duration::from_secs(u64::try_from(value.tv_sec).unwrap_or(0))
            + Duration::from_micros(u64::try_from(value.tv_usec).unwrap_or(0))
    };
    Some(Usage {
        cpu: time(raw.ru_utime) + time(raw.ru_stime),
        // Linux reports `ru_maxrss` in KiB and `ru_oublock` in 512-byte units.
        peak_memory: u64::try_from(raw.ru_maxrss).unwrap_or(0) * 1024,
        written: u64::try_from(raw.ru_oublock).unwrap_or(0) * 512,
    })
}

fn percent(value: f64) -> String {
    if value < 1.0 {
        format!("{value:.2}%")
    } else {
        format!("{value:.1}%")
    }
}

fn domain(source: &str) -> &'static str {
    if source.starts_with("postgres") {
        "postgres"
    } else {
        "linux"
    }
}

/// Nearest-rank percentile, or `None` without values.
fn percentile(values: &[Duration], p: usize) -> Option<Duration> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    Some(sorted[rank - 1])
}

fn percentile_span(value: Option<Duration>) -> Span<'static> {
    match value {
        Some(value) => Span::styled(short(value), palette::strong()),
        None => Span::styled("—", palette::muted()),
    }
}

fn optional(value: Option<Duration>) -> String {
    value.map_or_else(|| "—".to_string(), short)
}

/// Lists sources sharing a prefix once: `proc.pressure.{cpu,io}`.
fn compact(sources: &[String]) -> String {
    let mut by_prefix: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for source in sources {
        let (prefix, leaf) = source.rsplit_once('.').unwrap_or(("", source));
        by_prefix.entry(prefix).or_default().push(leaf);
    }
    by_prefix
        .iter()
        .map(
            |(prefix, leaves)| match (prefix.is_empty(), leaves.as_slice()) {
                (true, _) => leaves.join(", "),
                (false, [leaf]) => format!("{prefix}.{leaf}"),
                (false, _) => format!("{prefix}.{{{}}}", leaves.join(",")),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}

/// [`compact`] as one `key=value` field, without spaces.
fn field(sources: &[String]) -> String {
    compact(sources).replace(", ", ",")
}

fn short(duration: Duration) -> String {
    let micros = duration.as_micros();
    if micros < 1_000 {
        format!("{micros}µs")
    } else if micros < 1_000_000 {
        format!("{:.1}ms", micros as f64 / 1_000.0)
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    }
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!(
            "{}h{:02}m{:02}s",
            seconds / 3600,
            seconds % 3600 / 60,
            seconds % 60
        )
    } else {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    }
}

/// A whole duration as the CLI takes it: `30m`, `1h`, `90s`.
fn span(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds > 0 && seconds % 3600 == 0 {
        format!("{}h", seconds / 3600)
    } else if seconds > 0 && seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn iec(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Whether stderr is a terminal that takes color.
fn use_color() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// Columns for a page, one short of the terminal so a full line never wraps.
fn terminal_width() -> usize {
    ratatui::crossterm::terminal::size()
        .map(|(columns, _)| usize::from(columns))
        .unwrap_or(80)
        .saturating_sub(1)
        .max(40)
}

/// Prints the page title once, when stderr is a terminal. The live block and
/// any messages before it are sections below it.
pub fn header(sampling: Option<(Duration, Duration)>) {
    if !std::io::stderr().is_terminal() {
        return;
    }
    let width = terminal_width();
    let color = use_color();
    let title = Line::from(vec![
        Span::styled(
            concat!(" pg-probe ", env!("CARGO_PKG_VERSION"), " "),
            palette::brand(),
        ),
        Span::styled(" Capture", palette::strong()),
        Span::styled(
            match sampling {
                Some((duration, interval)) => {
                    format!(" · {} every {}", span(duration), span(interval))
                }
                None => " · single snapshot".to_string(),
            },
            palette::muted(),
        ),
    ]);
    let mut text = String::new();
    write_line(&mut text, &title, width, color);
    write_line(&mut text, &Line::default(), width, color);
    let mut out = std::io::stderr().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// Writes one line cut to `width` characters, so it never wraps and the block
/// keeps its height. With color, the canvas is painted to the edge, as `top`
/// paints its screen.
fn write_line(out: &mut String, line: &Line<'_>, width: usize, color: bool) {
    let base = palette::base().patch(line.style);
    let mut left = width;
    for span in &line.spans {
        if left == 0 {
            break;
        }
        let text: String = span.content.chars().take(left).collect();
        left -= text.chars().count();
        write_styled(out, base.patch(span.style), &text, color);
    }
    if color && left > 0 {
        write_styled(out, base, &" ".repeat(left), color);
    }
    out.push('\n');
}

fn write_styled(out: &mut String, style: Style, text: &str, color: bool) {
    if !color {
        out.push_str(text);
        return;
    }
    let fg = CColor::from(style.fg.unwrap_or(Color::Reset));
    let bg = CColor::from(style.bg.unwrap_or(Color::Reset));
    let _ = SetForegroundColor(fg).write_ansi(out);
    let _ = SetBackgroundColor(bg).write_ansi(out);
    if style.add_modifier.contains(Modifier::BOLD) {
        let _ = SetAttribute(Attribute::Bold).write_ansi(out);
    }
    out.push_str(text);
    let _ = SetAttribute(Attribute::Reset).write_ansi(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(values: impl IntoIterator<Item = u64>) -> Vec<Duration> {
        values.into_iter().map(Duration::from_millis).collect()
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        assert_eq!(percentile(&[], 95), None);
        assert_eq!(
            percentile(&millis(1..=20), 95),
            Some(Duration::from_millis(19))
        );
        assert_eq!(
            percentile(&millis(1..=200), 99),
            Some(Duration::from_millis(198))
        );
    }

    #[test]
    fn percentiles_appear_and_update_every_ten_samples() {
        let mut state = State::new(false, false);
        state.duration = Some(Duration::from_secs(600));
        for sample in 1..=25 {
            state.sample_started(sample);
            state.sample_finished();
            let repeated = state.sample_times.len();
            assert_eq!(
                state.p95.is_some(),
                repeated >= 10,
                "after {repeated} samples"
            );
        }
        assert_eq!(state.p95, percentile(&state.sample_times[..20], 95));
    }

    #[test]
    fn a_failure_wins_over_a_success_of_the_same_source() {
        let mut state = State::new(false, false);
        let collected = Outcome {
            status: "collected".into(),
            error_code: String::new(),
            detail: String::new(),
        };
        let failed = Outcome {
            status: "io_error".into(),
            error_code: "statement_timeout".into(),
            detail: "-".into(),
        };
        state.outcome("postgres.stats.database".into(), failed.clone());
        state.outcome("postgres.stats.database".into(), collected);
        assert_eq!(state.current["postgres.stats.database"], failed);
    }

    #[test]
    fn compact_groups_sources_by_prefix() {
        let sources = ["proc.pressure.cpu", "proc.pressure.io", "proc.cgroup.io"].map(String::from);
        assert_eq!(compact(&sources), "proc.cgroup.io, proc.pressure.{cpu,io}");
    }
}
