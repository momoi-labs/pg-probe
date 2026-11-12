//! What the log lines add up to since `top` started: errors by user,
//! database and error, temp files by statement, and checkpoints.
//!
//! The counts are evidence. Nothing here says a count is high or low.

use std::collections::{BTreeMap, HashMap, VecDeque};

use chrono::NaiveDateTime;

use super::reader::COLUMNS;

/// The most groups a summary keeps. Lines past it count under no group.
const GROUPS_MAX: usize = 500;
/// The most checkpoints kept for the percentiles and the intervals.
const CHECKPOINTS_MAX: usize = 1000;
/// The cells a group's message or statement keeps.
const KEY_MAX: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Error,
    Temp,
    Checkpoint,
    Other,
}

/// One line of the log. The text stays in memory for the screen and is
/// never written anywhere.
#[derive(Debug, Clone)]
pub(super) struct Event {
    pub(super) id: u64,
    /// `log_time` as the server wrote it.
    pub(super) logged: String,
    pub(super) pid: String,
    pub(super) user: String,
    pub(super) database: String,
    pub(super) client: String,
    pub(super) session: String,
    pub(super) severity: String,
    pub(super) code: String,
    pub(super) message: String,
    pub(super) detail: String,
    pub(super) hint: String,
    pub(super) context: String,
    pub(super) statement: String,
    pub(super) application: String,
    pub(super) kind: Kind,
}

impl Event {
    /// A csvlog record as an event. `english` says whether the server
    /// writes its messages in English, which the temp file and checkpoint
    /// lines need to be recognized.
    pub(super) fn new(id: u64, mut fields: Vec<String>, english: bool) -> Self {
        fields.resize(COLUMNS, String::new());
        let mut take = |index: usize| std::mem::take(&mut fields[index]);
        let (code, message) = (take(12), take(13));
        // An error is a line with an error SQLSTATE. The severity is
        // written in the server's language, the code is not.
        let kind = if !matches!(code.get(..2), Some("00" | "01" | "02") | None) {
            Kind::Error
        } else if english && message.starts_with("temporary file: path ") {
            Kind::Temp
        } else if english && checkpoint_part(&message).is_some() {
            Kind::Checkpoint
        } else {
            Kind::Other
        };
        Self {
            id,
            logged: take(0),
            user: take(1),
            database: take(2),
            pid: take(3),
            client: take(4),
            session: take(5),
            severity: take(11),
            code,
            message,
            detail: take(14),
            hint: take(15),
            context: take(18),
            statement: take(19),
            application: take(22),
            kind,
        }
    }

    /// The time of day, to the second or to the millisecond.
    pub(super) fn time(&self, millis: bool) -> &str {
        let end = if millis { 23 } else { 19 };
        self.logged
            .get(11..end)
            .or_else(|| self.logged.get(11..19))
            .unwrap_or("—")
    }

    pub(super) fn who(&self) -> String {
        who(&self.user, &self.database)
    }

    fn seconds(&self) -> Option<f64> {
        let stamp = self.logged.get(..23)?;
        let time = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.3f").ok()?;
        Some(time.and_utc().timestamp_millis() as f64 / 1000.0)
    }
}

/// `user@database`, with `—` for a line of a server process.
pub(super) fn who(user: &str, database: &str) -> String {
    if user.is_empty() && database.is_empty() {
        "—".to_owned()
    } else {
        format!("{user}@{database}")
    }
}

/// `starting` or `complete` and the rest of a checkpoint or restartpoint
/// line.
fn checkpoint_part(message: &str) -> Option<(bool, &str)> {
    let rest = message
        .strip_prefix("checkpoint ")
        .or_else(|| message.strip_prefix("restartpoint "))?;
    if let Some(flags) = rest.strip_prefix("starting:") {
        Some((true, flags))
    } else {
        rest.strip_prefix("complete:").map(|rest| (false, rest))
    }
}

/// `text` with its numbers and its quoted strings as `?`, and each run of
/// spaces as one, so lines that differ only by values group together.
/// Names in double quotes stay.
pub(super) fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    let mut cells = 0;
    while let Some(c) = chars.next() {
        if cells >= KEY_MAX {
            break;
        }
        cells += 1;
        let after_word = out
            .chars()
            .next_back()
            .is_some_and(|last| last.is_alphanumeric() || matches!(last, '_' | '$' | '"'));
        match c {
            '\'' => {
                // A doubled quote is a quote inside the string.
                while let Some(next) = chars.next() {
                    if next == '\'' && chars.next_if_eq(&'\'').is_none() {
                        break;
                    }
                }
                out.push('?');
            }
            '"' => {
                out.push(c);
                for next in chars.by_ref() {
                    out.push(next);
                    if next == '"' {
                        break;
                    }
                }
            }
            c if c.is_ascii_digit() && !after_word => {
                while chars
                    .next_if(|next| next.is_ascii_digit() || *next == '.')
                    .is_some()
                {}
                out.push('?');
            }
            c if c.is_whitespace() => {
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
    }
    out.trim_end().to_owned()
}

/// Counts over the last five minutes, in thirty buckets of ten seconds.
/// A slot is the number of whole ten seconds since `top` started.
#[derive(Debug, Clone, Default)]
pub(super) struct Recent {
    buckets: [u32; 30],
    last: u64,
}

impl Recent {
    fn add(&mut self, slot: u64) {
        if slot > self.last {
            if slot - self.last >= 30 {
                self.buckets = [0; 30];
            } else {
                for passed in self.last + 1..=slot {
                    self.buckets[(passed % 30) as usize] = 0;
                }
            }
            self.last = slot;
        }
        self.buckets[(self.last % 30) as usize] += 1;
    }

    pub(super) fn sum(&self, now: u64) -> u64 {
        (0..30)
            .filter_map(|back| self.last.checked_sub(back))
            .filter(|slot| slot + 30 > now)
            .map(|slot| u64::from(self.buckets[(slot % 30) as usize]))
            .sum()
    }
}

/// Sizes in buckets a sixteenth of a power of two wide, so a percentile
/// is within 3% of the real value however many sizes come in.
#[derive(Debug, Clone, Default)]
pub(super) struct Histogram {
    buckets: BTreeMap<u16, u64>,
    count: u64,
    max: u64,
}

impl Histogram {
    fn add(&mut self, value: u64) {
        let bucket = if value < 32 {
            value as u16
        } else {
            let power = 63 - value.leading_zeros();
            (power * 16 + ((value >> (power - 4)) & 15) as u32) as u16
        };
        *self.buckets.entry(bucket).or_default() += 1;
        self.count += 1;
        self.max = self.max.max(value);
    }

    /// The middle of the bucket that holds the `fraction` percentile, or the
    /// largest value when that is smaller.
    pub(super) fn percentile(&self, fraction: f64) -> Option<f64> {
        let rank = (fraction * self.count as f64).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (bucket, count) in &self.buckets {
            seen += count;
            if seen >= rank {
                let bucket = u32::from(*bucket);
                if bucket < 80 {
                    // Buckets under 32 hold one value each.
                    return Some(f64::from(bucket.min(31)));
                }
                let (power, part) = (bucket / 16, bucket % 16);
                let width = (1u64 << (power - 4)) as f64;
                let low = (1u64 << power) as f64 + f64::from(part) * width;
                return Some((low + width / 2.0).min(self.max as f64));
            }
        }
        None
    }
}

/// The exact `fraction` percentile of `values`.
pub(super) fn percentile(values: impl Iterator<Item = f64>, fraction: f64) -> Option<f64> {
    let mut values: Vec<f64> = values.collect();
    values.sort_by(f64::total_cmp);
    let rank = (fraction * values.len() as f64).ceil().max(1.0) as usize;
    values.get(rank.saturating_sub(1)).copied()
}

#[derive(Debug, Clone, Default)]
pub(super) struct ErrorGroup {
    pub(super) count: u64,
    pub(super) recent: Recent,
    /// The time of day of the last line.
    pub(super) last: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TempGroup {
    pub(super) files: u64,
    pub(super) bytes: u64,
    pub(super) recent: Recent,
    pub(super) sizes: Histogram,
}

/// A checkpoint that completed.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Checkpoint {
    /// The time of day it started, or completed when the start was not
    /// seen.
    pub(super) time: String,
    /// What started it, as the server wrote it: `time`, `xlog`, `wal`.
    pub(super) by: String,
    pub(super) buffers: f64,
    pub(super) percent: f64,
    pub(super) write: f64,
    pub(super) sync: f64,
    pub(super) total: f64,
}

#[derive(Debug, Default)]
pub(super) struct Checkpoints {
    /// How many started, by what started them, in the order first seen.
    pub(super) by: Vec<(String, u64)>,
    /// The start that has not completed yet: time of day, cause, seconds.
    started: Option<(String, String)>,
    last_start: Option<f64>,
    /// Seconds between one start and the next.
    pub(super) intervals: VecDeque<f64>,
    pub(super) done: VecDeque<Checkpoint>,
    pub(super) completed: u64,
    pub(super) added: u64,
    pub(super) removed: u64,
    pub(super) recycled: u64,
}

impl Checkpoints {
    fn add(&mut self, event: &Event) {
        let Some((starting, rest)) = checkpoint_part(&event.message) else {
            return;
        };
        if starting {
            const CAUSES: [&str; 7] = [
                "time",
                "xlog",
                "wal",
                "shutdown",
                "end-of-recovery",
                "immediate",
                "force",
            ];
            let flags: Vec<&str> = rest.split_whitespace().collect();
            let by = CAUSES
                .iter()
                .find(|cause| flags.contains(cause))
                .or(flags.first())
                .map_or("—", |cause| *cause)
                .to_owned();
            match self.by.iter_mut().find(|(known, _)| *known == by) {
                Some((_, count)) => *count += 1,
                None => self.by.push((by.clone(), 1)),
            }
            if let Some(now) = event.seconds() {
                if let Some(before) = self.last_start.replace(now) {
                    push_capped(&mut self.intervals, now - before);
                }
            }
            self.started = Some((event.time(false).to_owned(), by));
            return;
        }
        let (time, by) = self
            .started
            .take()
            .unwrap_or_else(|| (event.time(false).to_owned(), "—".to_owned()));
        let files = |name: &str| {
            rest.split([';', ','])
                .find(|part| part.trim_end().ends_with(name))
                .and_then(|part| part.split_whitespace().next()?.parse::<u64>().ok())
                .unwrap_or(0)
        };
        self.completed += 1;
        self.added += files("added");
        self.removed += files("removed");
        self.recycled += files("recycled");
        push_capped(
            &mut self.done,
            Checkpoint {
                time,
                by,
                buffers: number_after(rest, "wrote ").unwrap_or(0.0),
                percent: number_after(rest, "buffers (").unwrap_or(0.0),
                write: number_after(rest, "write=").unwrap_or(0.0),
                sync: number_after(rest, "sync=").unwrap_or(0.0),
                total: number_after(rest, "total=").unwrap_or(0.0),
            },
        );
    }
}

fn push_capped<T>(queue: &mut VecDeque<T>, value: T) {
    if queue.len() == CHECKPOINTS_MAX {
        queue.pop_front();
    }
    queue.push_back(value);
}

/// The number right after `marker` in `text`.
fn number_after(text: &str, marker: &str) -> Option<f64> {
    let rest = &text[text.find(marker)? + marker.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The size of a `temporary file: path "…", size N` line.
fn temp_size(message: &str) -> Option<u64> {
    let size = &message[message.rfind(", size ")? + 7..];
    size.trim().parse().ok()
}

/// The user, the database, the SQLSTATE and the normalized message.
pub(super) type ErrorKey = (String, String, String, String);
/// The user, the database and the normalized statement.
pub(super) type TempKey = (String, String, String);

#[derive(Debug, Default)]
pub(super) struct Stats {
    pub(super) lines: u64,
    pub(super) errors: u64,
    pub(super) errors_recent: Recent,
    pub(super) error_groups: HashMap<ErrorKey, ErrorGroup>,
    pub(super) temp_files: u64,
    pub(super) temp_bytes: u64,
    pub(super) temp_recent: Recent,
    pub(super) temp_sizes: Histogram,
    pub(super) temp_groups: HashMap<TempKey, TempGroup>,
    pub(super) checkpoints: Checkpoints,
}

impl Stats {
    /// Counts `event`, received in `slot`.
    pub(super) fn add(&mut self, event: &Event, slot: u64) {
        self.lines += 1;
        match event.kind {
            Kind::Error => {
                self.errors += 1;
                self.errors_recent.add(slot);
                let key = (
                    event.user.clone(),
                    event.database.clone(),
                    event.code.clone(),
                    normalize(&event.message),
                );
                if let Some(group) = entry(&mut self.error_groups, key) {
                    group.count += 1;
                    group.recent.add(slot);
                    group.last = event.time(false).to_owned();
                }
            }
            Kind::Temp => {
                let size = temp_size(&event.message).unwrap_or(0);
                self.temp_files += 1;
                self.temp_bytes += size;
                self.temp_recent.add(slot);
                self.temp_sizes.add(size);
                let key = (
                    event.user.clone(),
                    event.database.clone(),
                    normalize(&event.statement),
                );
                if let Some(group) = entry(&mut self.temp_groups, key) {
                    group.files += 1;
                    group.bytes += size;
                    group.recent.add(slot);
                    group.sizes.add(size);
                }
            }
            Kind::Checkpoint => self.checkpoints.add(event),
            Kind::Other => {}
        }
    }
}

/// The group of `key`, or `None` when the summary is full and the key is
/// new. The totals still count the line.
fn entry<K: std::hash::Hash + Eq, V: Default>(
    groups: &mut HashMap<K, V>,
    key: K,
) -> Option<&mut V> {
    if groups.len() >= GROUPS_MAX && !groups.contains_key(&key) {
        return None;
    }
    Some(groups.entry(key).or_default())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A csvlog record with the columns the view reads.
    pub(crate) fn record(time: &str, code: &str, message: &str, statement: &str) -> Vec<String> {
        let mut fields = vec![String::new(); COLUMNS];
        fields[0] = format!("2026-11-12 {time} UTC");
        fields[1] = "app".to_owned();
        fields[2] = "shop".to_owned();
        fields[3] = "4242".to_owned();
        fields[11] = if code == "00000" { "LOG" } else { "ERROR" }.to_owned();
        fields[12] = code.to_owned();
        fields[13] = message.to_owned();
        fields[19] = statement.to_owned();
        fields
    }

    fn stats(records: Vec<Vec<String>>) -> Stats {
        let mut stats = Stats::default();
        for (id, fields) in records.into_iter().enumerate() {
            stats.add(&Event::new(id as u64, fields, true), 0);
        }
        stats
    }

    #[test]
    fn values_become_placeholders_and_names_stay() {
        assert_eq!(
            normalize("duplicate key value violates unique constraint \"orders_2024_key\""),
            "duplicate key value violates unique constraint \"orders_2024_key\""
        );
        assert_eq!(
            normalize("SELECT * FROM t1\n  WHERE id = 42 AND name = 'it''s' AND x = $1"),
            "SELECT * FROM t1 WHERE id = ? AND name = ? AND x = $1"
        );
        assert_eq!(
            normalize("invalid input syntax for integer: \"abc\""),
            "invalid input syntax for integer: \"abc\""
        );
    }

    #[test]
    fn errors_group_by_user_database_code_and_message() {
        let stats = stats(vec![
            record("14:00:01.000", "23505", "duplicate key 1", ""),
            record("14:00:02.000", "23505", "duplicate key 2", ""),
            record("14:00:03.000", "57014", "canceling statement", ""),
            record("14:00:04.000", "00000", "connection received", ""),
        ]);
        assert_eq!((stats.lines, stats.errors), (4, 3));
        let key = (
            "app".to_owned(),
            "shop".to_owned(),
            "23505".to_owned(),
            "duplicate key ?".to_owned(),
        );
        let group = &stats.error_groups[&key];
        assert_eq!((group.count, group.last.as_str()), (2, "14:00:02"));
        assert_eq!(stats.error_groups.len(), 2);
    }

    #[test]
    fn an_error_is_known_by_its_code_in_any_language() {
        let mut fields = record("14:00:01.000", "23505", "duplicar valor da chave", "");
        fields[11] = "ERRO".to_owned();
        assert_eq!(Event::new(0, fields, false).kind, Kind::Error);
        // Without English messages a temp file line is not recognized.
        let fields = record(
            "14:00:01.000",
            "00000",
            "temporary file: path \"x\", size 1",
            "",
        );
        assert_eq!(Event::new(0, fields, false).kind, Kind::Other);
    }

    #[test]
    fn temp_files_add_up_by_statement() {
        let temp = |size: u64, id: u32| {
            record(
                "14:00:01.000",
                "00000",
                &format!("temporary file: path \"base/pgsql_tmp/pgsql_tmp77.{id}\", size {size}"),
                &format!("SELECT * FROM orders ORDER BY {id}"),
            )
        };
        let stats = stats(vec![temp(1 << 20, 1), temp(3 << 20, 2)]);
        assert_eq!((stats.temp_files, stats.temp_bytes), (2, 4 << 20));
        let group = stats.temp_groups.values().next().unwrap();
        assert_eq!((stats.temp_groups.len(), group.files), (1, 2));
        let p95 = stats.temp_sizes.percentile(0.95).unwrap();
        assert!((p95 / f64::from(3 << 20) - 1.0).abs() < 0.04, "{p95}");
    }

    #[test]
    fn percentiles_stay_within_three_percent() {
        let mut sizes = Histogram::default();
        for value in 1..=10_000u64 {
            sizes.add(value * 1000);
        }
        for (fraction, real) in [(0.5, 5.0e6), (0.95, 9.5e6), (0.99, 9.9e6)] {
            let got = sizes.percentile(fraction).unwrap();
            assert!((got / real - 1.0).abs() < 0.035, "{fraction}: {got}");
        }
        assert_eq!(percentile([3.0, 1.0, 2.0].into_iter(), 0.95), Some(3.0));
        assert_eq!(percentile(std::iter::empty(), 0.95), None);
    }

    #[test]
    fn the_last_five_minutes_forget_older_slots() {
        let mut recent = Recent::default();
        recent.add(0);
        recent.add(0);
        recent.add(29);
        assert_eq!(recent.sum(29), 3);
        assert_eq!(recent.sum(30), 1);
        recent.add(70);
        assert_eq!(recent.sum(70), 1);
        assert_eq!(recent.sum(200), 0);
    }

    #[test]
    fn checkpoints_of_9_3_and_of_newer_servers_are_read() {
        let stats =
            stats(vec![
            record("14:00:00.000", "00000", "checkpoint starting: time", ""),
            record(
                "14:00:28.600",
                "00000",
                "checkpoint complete: wrote 4102 buffers (12.5%); 0 transaction log file(s) \
added, 3 removed, 14 recycled; write=28.100 s, sync=0.410 s, total=28.600 s; sync files=52, \
longest=0.120 s, average=0.007 s",
                "",
            ),
            record("14:05:00.000", "00000", "checkpoint starting: immediate force wait", ""),
            record(
                "14:05:01.000",
                "00000",
                "checkpoint complete: wrote 10 buffers (0.1%); 1 WAL file(s) added, 0 removed, \
2 recycled; write=0.900 s, sync=0.050 s, total=1.000 s; sync files=3, longest=0.010 s, \
average=0.003 s; distance=100 kB, estimate=200 kB",
                "",
            ),
            record("14:10:00.000", "00000", "restartpoint starting: xlog", ""),
        ]);
        let checkpoints = &stats.checkpoints;
        assert_eq!(
            checkpoints.by,
            [("time", 1), ("immediate", 1), ("xlog", 1)].map(|(by, count)| (by.to_owned(), count))
        );
        assert_eq!(checkpoints.completed, 2);
        assert_eq!(
            (checkpoints.added, checkpoints.removed, checkpoints.recycled),
            (1, 3, 16)
        );
        assert_eq!(
            checkpoints.done[0],
            Checkpoint {
                time: "14:00:00".to_owned(),
                by: "time".to_owned(),
                buffers: 4102.0,
                percent: 12.5,
                write: 28.1,
                sync: 0.41,
                total: 28.6,
            }
        );
        assert_eq!(checkpoints.intervals, [300.0, 300.0]);
    }
}
