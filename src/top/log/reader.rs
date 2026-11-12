//! Follows the server's csvlog files as they grow.
//!
//! Each tick reads what the server appended since the last one, from the
//! local file, up to a fixed amount. It starts at the end of the newest
//! file, so it never reads the log from before `top` started. It moves to
//! the next file when the server rotates the log.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// The most one tick reads. What is left waits for the next tick.
const TICK_MAX: usize = 8 << 20;
const CHUNK: usize = 64 << 10;
/// How often the directory is listed while it does not change. A server
/// that rotates to a file that exists, as `postgresql-%a` does each week,
/// leaves the directory as it was.
const RELIST: Duration = Duration::from_secs(10);
/// A record that grows past this without ending is dropped.
const RECORD_MAX: usize = 8 << 20;
/// The most a field keeps. A statement longer than this is cut.
const FIELD_MAX: usize = 4 << 10;
/// csvlog has 23 columns in 9.3, and more from 13 on.
pub(super) const COLUMNS: usize = 23;

/// Splits the bytes of a csvlog file into records. A quoted field can
/// hold line breaks, and `""` is a quote inside it.
#[derive(Debug, Default)]
pub(super) struct Parser {
    pending: Vec<u8>,
    /// True when `pending` starts at the start of a record.
    synced: bool,
    /// Records dropped: too long, or not a csvlog line.
    pub(super) skipped: u64,
}

impl Parser {
    /// The next bytes come from the start of a file.
    fn at_start(&mut self) {
        self.pending.clear();
        self.synced = true;
    }

    /// The next bytes come from the middle of a file, so the parser waits
    /// for a line that starts with a timestamp.
    fn at_middle(&mut self) {
        self.pending.clear();
        self.synced = false;
    }

    pub(super) fn feed(&mut self, bytes: &[u8], records: &mut Vec<Vec<String>>) {
        self.pending.extend_from_slice(bytes);
        let mut start = 0;
        loop {
            if !self.synced {
                match record_start(&self.pending[start..]) {
                    Ok(offset) => {
                        start += offset;
                        self.synced = true;
                    }
                    Err(discard) => {
                        start += discard;
                        break;
                    }
                }
            }
            let Some(end) = record_end(&self.pending[start..]) else {
                break;
            };
            let record = &self.pending[start..start + end];
            let fields = fields(record);
            if fields.len() >= COLUMNS && is_stamp(record) {
                records.push(fields);
            } else if !record.is_empty() {
                self.skipped += 1;
            }
            start += end + 1;
        }
        self.pending.drain(..start);
        if self.pending.len() > RECORD_MAX {
            self.skipped += 1;
            self.at_middle();
        }
    }
}

/// True when `bytes` start as a csvlog record does: `2026-11-12 14:02:11`.
fn is_stamp(bytes: &[u8]) -> bool {
    const SHAPE: &[u8] = b"dddd-dd-dd dd:dd:dd";
    bytes.len() >= SHAPE.len()
        && SHAPE.iter().zip(bytes).all(|(shape, byte)| match shape {
            b'd' => byte.is_ascii_digit(),
            other => byte == other,
        })
}

/// Where the first line that starts with a timestamp begins, or how many
/// bytes hold none and can go.
fn record_start(bytes: &[u8]) -> Result<usize, usize> {
    let mut at = 0;
    loop {
        if is_stamp(&bytes[at..]) {
            return Ok(at);
        }
        match bytes[at..].iter().position(|byte| *byte == b'\n') {
            Some(end) => at += end + 1,
            // The last line may be the start of a timestamp.
            None if bytes.len() - at < 19 => return Err(at),
            None => return Err(bytes.len()),
        }
    }
}

/// The line break that ends the first record, outside quotes.
fn record_end(bytes: &[u8]) -> Option<usize> {
    let mut quoted = false;
    bytes.iter().position(|byte| {
        if *byte == b'"' {
            quoted = !quoted;
        }
        *byte == b'\n' && !quoted
    })
}

fn fields(record: &[u8]) -> Vec<String> {
    let mut fields = Vec::with_capacity(COLUMNS + 3);
    let mut field: Vec<u8> = Vec::new();
    let mut quoted = false;
    let mut index = 0;
    let keep = |field: &mut Vec<u8>, byte: u8| {
        if field.len() < FIELD_MAX {
            field.push(byte);
        }
    };
    while index < record.len() {
        let byte = record[index];
        match byte {
            b'"' if quoted && record.get(index + 1) == Some(&b'"') => {
                keep(&mut field, b'"');
                index += 1;
            }
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                fields.push(String::from_utf8_lossy(&field).into_owned());
                field.clear();
            }
            _ => keep(&mut field, byte),
        }
        index += 1;
    }
    fields.push(String::from_utf8_lossy(&field).into_owned());
    fields
}

/// Why a tick read nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Problem {
    /// The directory has no `.csv` file.
    NoFile,
    /// The directory or the file could not be read, and why.
    Unreadable(PathBuf, String),
}

struct Open {
    path: PathBuf,
    file: File,
    inode: u64,
    offset: u64,
}

pub(super) struct Tail {
    dir: PathBuf,
    open: Option<Open>,
    /// False until the first file opens. That one is read from its end.
    started: bool,
    /// The directory's change time when it was last listed.
    listed: Option<SystemTime>,
    listed_at: Option<Instant>,
    pub(super) parser: Parser,
    /// Bytes read since `top` started.
    pub(super) read: u64,
    /// Bytes of the file the last tick left for the next one.
    pub(super) behind: u64,
    pub(super) problem: Option<Problem>,
}

impl Tail {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            open: None,
            started: false,
            listed: None,
            listed_at: None,
            parser: Parser::default(),
            read: 0,
            behind: 0,
            problem: None,
        }
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file being followed.
    pub(super) fn path(&self) -> Option<&Path> {
        self.open.as_ref().map(|open| open.path.as_path())
    }

    /// Reads what the server appended since the last tick.
    pub(super) fn tick(&mut self) -> Vec<Vec<String>> {
        let mut records = Vec::new();
        self.problem = None;
        if self.open.is_none() {
            let newest = csv_files(&self.dir).map(|files| {
                files
                    .into_iter()
                    .max_by_key(|(_, modified)| *modified)
                    .map(|(path, _)| path)
            });
            match newest {
                Ok(Some(path)) => {
                    if let Err(problem) = self.start(path) {
                        self.problem = Some(problem);
                        return records;
                    }
                }
                Ok(None) => {
                    self.problem = Some(Problem::NoFile);
                    return records;
                }
                Err(error) => {
                    self.problem = Some(unreadable(&self.dir, &error));
                    return records;
                }
            }
        }
        let Some(open) = &mut self.open else {
            return records;
        };
        let mut chunk = vec![0; CHUNK];
        let mut budget = TICK_MAX;
        while budget > 0 {
            let want = chunk.len().min(budget);
            match open.file.read(&mut chunk[..want]) {
                Ok(0) => break,
                Ok(count) => {
                    self.parser.feed(&chunk[..count], &mut records);
                    open.offset += count as u64;
                    self.read += count as u64;
                    budget -= count;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    self.problem = Some(unreadable(&open.path, &error));
                    self.open = None;
                    return records;
                }
            }
        }
        match fs::metadata(&open.path) {
            Ok(meta) if meta.ino() == open.inode => {
                if meta.len() < open.offset {
                    // The server truncated the file to write it again.
                    if open.file.seek(SeekFrom::Start(0)).is_ok() {
                        open.offset = 0;
                        self.parser.at_start();
                    }
                }
                self.behind = meta.len().saturating_sub(open.offset);
                if self.behind == 0 {
                    let current = (open.path.clone(), meta.modified().ok());
                    self.rotate(&current.0, current.1);
                }
            }
            // The file is gone or another took its name.
            _ => {
                self.open = None;
                self.behind = 0;
            }
        }
        records
    }

    /// Opens `path`: at its end the first time, so nothing from before
    /// `top` started is read, and from its start afterwards.
    fn start(&mut self, path: PathBuf) -> Result<(), Problem> {
        let opened = File::open(&path).and_then(|mut file| {
            let meta = file.metadata()?;
            let offset = if self.started {
                0
            } else {
                file.seek(SeekFrom::End(0))?
            };
            Ok((file, meta.ino(), offset))
        });
        let (file, inode, offset) = opened.map_err(|error| unreadable(&path, &error))?;
        if offset == 0 {
            self.parser.at_start();
        } else {
            self.parser.at_middle();
        }
        self.started = true;
        self.open = Some(Open {
            path,
            file,
            inode,
            offset,
        });
        Ok(())
    }

    /// Moves to the file the server opened after `current`, if there is
    /// one. The directory is listed when it changed, and every `RELIST`.
    fn rotate(&mut self, current: &Path, modified: Option<SystemTime>) {
        let changed = fs::metadata(&self.dir)
            .and_then(|meta| meta.modified())
            .ok();
        let fresh = self.listed_at.is_some_and(|at| at.elapsed() < RELIST);
        if changed.is_some() && changed == self.listed && fresh {
            return;
        }
        let Ok(files) = csv_files(&self.dir) else {
            return;
        };
        self.listed = changed;
        self.listed_at = Some(Instant::now());
        let next = files
            .into_iter()
            .filter(|(path, written)| path != current && Some(*written) > modified)
            .min_by_key(|(_, written)| *written)
            .map(|(path, _)| path);
        if let Some(path) = next {
            // The next tick reads it. A file that cannot be opened shows
            // as the problem then.
            self.open = None;
            if let Err(problem) = self.start(path) {
                self.problem = Some(problem);
            }
        }
    }
}

fn unreadable(path: &Path, error: &io::Error) -> Problem {
    Problem::Unreadable(path.to_owned(), error.kind().to_string())
}

/// The `.csv` files of `dir`, each with the time of its last write.
fn csv_files(dir: &Path) -> io::Result<Vec<(PathBuf, SystemTime)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "csv") {
            if let Ok(modified) = fs::metadata(&path).and_then(|meta| meta.modified()) {
                files.push((path, modified));
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn line(time: &str, code: &str, message: &str, query: &str) -> String {
        let quote = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
        format!(
            "2026-11-12 {time} UTC,\"app\",\"shop\",4242,\"10.0.0.7:5432\",6551a.1092,1,\"INSERT\",\
2026-11-12 14:00:00 UTC,3/17,0,ERROR,{code},{},,,,,,{},,,\"psql\"\n",
            quote(message),
            quote(query)
        )
    }

    fn parse(parser: &mut Parser, text: &str) -> Vec<Vec<String>> {
        let mut records = Vec::new();
        parser.feed(text.as_bytes(), &mut records);
        records
    }

    #[test]
    fn a_quoted_field_keeps_its_line_breaks_commas_and_quotes() {
        let mut parser = Parser::default();
        parser.at_start();
        let text = line(
            "14:02:11.120",
            "23505",
            "duplicate key, \"orders_ref_key\"",
            "INSERT INTO orders\nVALUES (1, 'a')",
        );
        let records = parse(&mut parser, &text);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].len(), COLUMNS);
        assert_eq!(records[0][13], "duplicate key, \"orders_ref_key\"");
        assert_eq!(records[0][19], "INSERT INTO orders\nVALUES (1, 'a')");
        assert_eq!(records[0][22], "psql");
    }

    #[test]
    fn a_record_cut_by_a_read_waits_for_its_end() {
        let mut parser = Parser::default();
        parser.at_start();
        let text = line("14:02:11.120", "42601", "syntax error", "SELEC\n1");
        let (first, rest) = text.split_at(text.len() - 20);
        assert!(parse(&mut parser, first).is_empty());
        assert_eq!(parse(&mut parser, rest).len(), 1);
    }

    #[test]
    fn a_read_from_the_middle_starts_at_the_next_record() {
        let mut parser = Parser::default();
        parser.at_middle();
        let text = format!(
            "of a statement\"\n{}",
            line("14:02:12.000", "57014", "canceling statement", "")
        );
        let records = parse(&mut parser, &text);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0][12], "57014");
    }

    #[test]
    fn a_line_that_is_not_csvlog_is_counted_and_skipped() {
        let mut parser = Parser::default();
        parser.at_start();
        let text = format!(
            "not a log line\n{}",
            line("14:02:12.000", "57014", "canceling", "")
        );
        assert_eq!(parse(&mut parser, &text).len(), 1);
        assert_eq!(parser.skipped, 1);
    }

    #[test]
    fn a_long_field_is_cut() {
        let mut parser = Parser::default();
        parser.at_start();
        let text = line("14:02:12.000", "57014", "x", &"a".repeat(3 * FIELD_MAX));
        assert_eq!(parse(&mut parser, &text)[0][19].len(), FIELD_MAX);
    }

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn the_tail_starts_at_the_end_and_follows_the_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("postgresql-1.csv");
        append(&first, &line("14:00:00.000", "42601", "before top", ""));
        let mut tail = Tail::new(dir.path().to_owned());
        assert!(tail.tick().is_empty(), "nothing from before the start");
        assert_eq!(tail.path(), Some(first.as_path()));

        append(&first, &line("14:00:01.000", "42601", "one", ""));
        let records = tail.tick();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0][13], "one");

        // The server rotates: the next file is read from its start.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = dir.path().join("postgresql-2.csv");
        append(&second, &line("14:00:02.000", "42601", "two", ""));
        let mut records = tail.tick();
        records.extend(tail.tick());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0][13], "two");
        assert_eq!(tail.path(), Some(second.as_path()));
    }

    #[test]
    fn a_rotation_to_a_file_that_exists_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let monday = dir.path().join("postgresql-Mon.csv");
        append(&monday, &line("14:00:00.000", "42601", "last week", ""));
        std::thread::sleep(std::time::Duration::from_millis(20));
        let sunday = dir.path().join("postgresql-Sun.csv");
        append(&sunday, &line("23:59:59.000", "42601", "before top", ""));
        let mut tail = Tail::new(dir.path().to_owned());
        tail.tick();
        assert_eq!(tail.path(), Some(sunday.as_path()));

        // The server truncates Monday's file and writes to it. The
        // directory does not change.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&monday, line("00:00:01.000", "42601", "monday", "")).unwrap();
        tail.tick();
        assert_eq!(tail.path(), Some(sunday.as_path()));
        tail.listed_at = None;
        let mut records = tail.tick();
        records.extend(tail.tick());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0][13], "monday");
        assert_eq!(tail.path(), Some(monday.as_path()));
    }

    #[test]
    fn a_truncated_file_is_read_again_from_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postgresql-Mon.csv");
        append(&path, &line("14:00:00.000", "42601", "last week", ""));
        append(&path, &line("14:00:00.500", "42601", "last week", ""));
        let mut tail = Tail::new(dir.path().to_owned());
        tail.tick();
        fs::write(&path, line("14:00:01.000", "42601", "new", "")).unwrap();
        let mut records = tail.tick();
        records.extend(tail.tick());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0][13], "new");
    }

    #[test]
    fn a_directory_without_a_log_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut tail = Tail::new(dir.path().to_owned());
        tail.tick();
        assert_eq!(tail.problem, Some(Problem::NoFile));
        let mut tail = Tail::new(dir.path().join("missing"));
        tail.tick();
        assert!(matches!(tail.problem, Some(Problem::Unreadable(..))));
    }
}
