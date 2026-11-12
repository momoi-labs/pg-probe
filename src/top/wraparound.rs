//! What should be vacuumed against wraparound and is not: the tables and
//! databases past `autovacuum_freeze_max_age` or
//! `autovacuum_multixact_freeze_max_age`, and how full the multixact member
//! space is.
//!
//! The vacuum view reads this only while it is open, and only every
//! [`EVERY`], since it lists a directory and may open a short session per
//! database past a limit. A failed autovacuum never moves
//! `autovacuum_count`, so the view counts the workers it sees start on each
//! table and compares them with the vacuums that finished.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::postgres::ConnectedClient;

use super::{autovacuum_table, local_time, text_of, Vacuum, SOURCE};

/// How often the view reads the catalogs and the member files.
pub const EVERY: Duration = Duration::from_secs(30);

/// The most tables kept per database.
const TABLES: usize = 50;
/// Members in one segment file of `pg_multixact/members`: 32 pages of 1636.
const SEGMENT_MEMBERS: u64 = 52_352;
/// Member offsets wrap at 2^32 before PostgreSQL 19.
const MEMBER_SPACE: f64 = 4_294_967_296.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Members {
    /// Not read yet, or the read failed.
    Unknown,
    /// The role may not list `pg_multixact/members`.
    NeedsSuperuser,
    /// The share of the member space in use, from the number of segment
    /// files.
    Used { fraction: f64 },
    /// PostgreSQL 19 widened member offsets to 64 bits.
    NoLimit,
}

/// A table, or a database whose tables could not be read, past a freeze
/// limit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Behind {
    pub database: String,
    /// The table to show, the owner for a TOAST table. Empty for a
    /// database row.
    pub table: String,
    /// The name autovacuum reports, `schema.relname`.
    pub relation: String,
    pub toast: bool,
    /// Why the database has no table rows, such as "no connections".
    pub note: String,
    /// Age as a percent of the limit.
    pub xid_percent: Option<f64>,
    pub mxid_percent: Option<f64>,
    pub since_autovacuum_s: Option<f64>,
    pub autovacuum_count: Option<i64>,
}

impl Behind {
    /// The larger of the two ages, with its kind, such as `mxid 104%`.
    pub fn over(&self) -> String {
        match (self.xid_percent, self.mxid_percent) {
            (Some(xid), Some(mxid)) if mxid > xid => format!("mxid {mxid:.0}%"),
            (Some(xid), _) => format!("xid {xid:.0}%"),
            (None, Some(mxid)) => format!("mxid {mxid:.0}%"),
            (None, None) => "—".to_owned(),
        }
    }

    fn worst(&self) -> f64 {
        self.xid_percent
            .unwrap_or(0.0)
            .max(self.mxid_percent.unwrap_or(0.0))
    }
}

#[derive(Debug, Default)]
pub struct Wraparound {
    pub members: Option<Members>,
    /// The database with the oldest multixact, from 9.5.
    pub oldest: Option<String>,
    pub behind: Vec<Behind>,
    /// Why the catalogs were not read.
    pub error: Option<String>,
    /// The local time of the last read.
    pub read_time: Option<String>,
    read_at: Option<Instant>,
    /// The autovacuum workers seen on each (database, relation).
    tries: HashMap<(String, String), HashSet<String>>,
    /// `autovacuum_count` of each (database, relation) when first read.
    baseline: HashMap<(String, String), i64>,
}

impl Wraparound {
    pub fn due(&self) -> bool {
        self.read_at.is_none_or(|at| at.elapsed() >= EVERY)
    }

    /// Notes each autovacuum worker in progress. Workers that end between
    /// two refreshes are missed, so the count is a minimum.
    pub fn observe(&mut self, vacuums: &[Vacuum]) {
        for vacuum in vacuums {
            let backend = &vacuum.backend;
            let Some(relation) = autovacuum_table(&backend.query) else {
                continue;
            };
            self.tries
                .entry((backend.database.clone(), relation))
                .or_default()
                .insert(backend.pid.clone());
        }
    }

    /// Workers seen on the row's table, or in its database for a database
    /// row.
    pub fn tries(&self, row: &Behind) -> usize {
        if row.table.is_empty() {
            self.tries
                .iter()
                .filter(|((database, _), _)| *database == row.database)
                .map(|(_, pids)| pids.len())
                .sum()
        } else {
            self.tries
                .get(&(row.database.clone(), row.relation.clone()))
                .map_or(0, HashSet::len)
        }
    }

    /// Autovacuums that finished on the row's table since the first read.
    pub fn finished(&self, row: &Behind) -> Option<i64> {
        let count = row.autovacuum_count?;
        let first = self
            .baseline
            .get(&(row.database.clone(), row.relation.clone()))?;
        Some(count - first)
    }

    /// Reads the member space, the databases and the tables past a limit.
    /// A failure shows in the view, never stops it.
    pub fn read(&mut self, client: &ConnectedClient, version: i32, current: &str) {
        self.read_at = Some(Instant::now());
        self.read_time = local_time();
        self.members = Some(read_members(client, version));
        match read_databases(client, version) {
            Ok((databases, limits)) => {
                self.error = None;
                self.oldest = databases
                    .iter()
                    .filter(|database| database.mxid_age.is_some_and(|age| age > 0))
                    .max_by_key(|database| database.mxid_age)
                    .map(|database| database.name.clone());
                self.behind = behind(client, version, current, &databases, limits);
            }
            Err(error) => {
                self.error = Some(error);
                self.oldest = None;
                self.behind.clear();
            }
        }
        for row in &self.behind {
            if let Some(count) = row.autovacuum_count {
                self.baseline
                    .entry((row.database.clone(), row.relation.clone()))
                    .or_insert(count);
            }
        }
    }
}

fn behind(
    client: &ConnectedClient,
    version: i32,
    current: &str,
    databases: &[Database],
    limits: Limits,
) -> Vec<Behind> {
    let mut rows = Vec::new();
    for database in databases {
        let row = Behind {
            database: database.name.clone(),
            xid_percent: limits.xid_percent(database.xid_age),
            mxid_percent: limits.mxid_percent(database.mxid_age),
            ..Behind::default()
        };
        if row.worst() < 100.0 {
            continue;
        }
        if !database.allows_connections {
            rows.push(Behind {
                note: "no connections".to_owned(),
                ..row
            });
            continue;
        }
        let shared = database.name == current;
        let sql = tables_sql(version, limits, shared);
        let tables = if shared {
            fetch_tables(client, &sql, &database.name, limits)
        } else {
            client
                .for_database(&database.name, SOURCE, &mut Vec::new())
                .map_err(|_| "could not connect".to_owned())
                .and_then(|session| fetch_tables(&session, &sql, &database.name, limits))
        };
        match tables {
            Ok(tables) if tables.is_empty() => rows.push(row),
            Ok(tables) => rows.extend(tables),
            Err(note) => rows.push(Behind { note, ..row }),
        }
    }
    rows.sort_by(|a, b| b.worst().total_cmp(&a.worst()));
    rows
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Limits {
    xid: Option<i64>,
    mxid: Option<i64>,
}

impl Limits {
    fn xid_percent(&self, age: Option<i64>) -> Option<f64> {
        percent(age, self.xid)
    }

    fn mxid_percent(&self, age: Option<i64>) -> Option<f64> {
        percent(age, self.mxid)
    }
}

fn percent(age: Option<i64>, limit: Option<i64>) -> Option<f64> {
    let limit = limit.filter(|limit| *limit > 0)?;
    Some(age? as f64 * 100.0 / limit as f64)
}

struct Database {
    name: String,
    allows_connections: bool,
    xid_age: Option<i64>,
    mxid_age: Option<i64>,
}

/// `mxid_age` arrived in 9.5.
pub const fn has_mxid_age(version: i32) -> bool {
    version >= 90500
}

/// The limits come from `pg_settings`, since
/// `autovacuum_multixact_freeze_max_age` is missing before 9.3.3.
fn databases_sql(version: i32) -> String {
    let mxid = if has_mxid_age(version) {
        "mxid_age(d.datminmxid)"
    } else {
        "NULL::integer"
    };
    format!(
        "\
SELECT d.datname, d.datallowconn, age(d.datfrozenxid), {mxid}, \
    (SELECT setting FROM pg_settings WHERE name = 'autovacuum_freeze_max_age'), \
    (SELECT setting FROM pg_settings WHERE name = 'autovacuum_multixact_freeze_max_age') \
FROM pg_database d"
    )
}

fn read_databases(
    client: &ConnectedClient,
    version: i32,
) -> Result<(Vec<Database>, Limits), String> {
    let rows = client
        .exec_bounded_rows(&databases_sql(version), SOURCE, &mut Vec::new())
        .map_err(|_| "the pg_database query failed".to_owned())?;
    let mut limits = Limits {
        xid: None,
        mxid: None,
    };
    let databases = rows
        .rows
        .iter()
        .map(|row| {
            let text = |index: usize| row.get(index).map(|value| text_of(value));
            let number = |index: usize| text(index).and_then(|value| value.parse().ok());
            limits = Limits {
                xid: number(4),
                mxid: number(5),
            };
            Database {
                name: text(0).unwrap_or_default(),
                allows_connections: text(1).as_deref() == Some("t"),
                xid_age: number(2),
                mxid_age: number(3),
            }
        })
        .collect();
    Ok((databases, limits))
}

/// The tables of one database past a limit, worst first. A TOAST table
/// shows as its owner. Every database lists the shared catalogs, so only
/// the connected one keeps them.
fn tables_sql(version: i32, limits: Limits, shared: bool) -> String {
    let shared = if shared { "" } else { "AND NOT c.relisshared " };
    let mxid = if has_mxid_age(version) {
        "mxid_age(c.relminmxid)"
    } else {
        "NULL::integer"
    };
    // An absent limit never matches.
    let xid_limit = limits.xid.unwrap_or(i64::MAX);
    let mxid_limit = limits.mxid.unwrap_or(i64::MAX);
    format!(
        "\
SELECT n.nspname || '.' || c.relname, \
    CASE WHEN c.relkind = 't' THEN o.nspname || '.' || o.relname END, \
    age(c.relfrozenxid), {mxid}, s.autovacuum_count, \
    round(EXTRACT(EPOCH FROM (now() - s.last_autovacuum))) \
FROM pg_class c \
JOIN pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN (SELECT t.reltoastrelid, tn.nspname, t.relname FROM pg_class t \
    JOIN pg_namespace tn ON tn.oid = t.relnamespace WHERE t.reltoastrelid <> 0) o \
    ON c.relkind = 't' AND o.reltoastrelid = c.oid \
LEFT JOIN pg_stat_all_tables s ON s.relid = c.oid \
WHERE c.relkind IN ('r', 'm', 't') {shared}\
    AND (age(c.relfrozenxid) >= {xid_limit} OR COALESCE({mxid} >= {mxid_limit}, false)) \
ORDER BY greatest(age(c.relfrozenxid)::float8 / {xid_limit}, \
    COALESCE({mxid}::float8 / {mxid_limit}, 0)) DESC \
LIMIT {TABLES}"
    )
}

fn fetch_tables(
    client: &ConnectedClient,
    sql: &str,
    database: &str,
    limits: Limits,
) -> Result<Vec<Behind>, String> {
    let rows = client
        .exec_bounded_rows(sql, SOURCE, &mut Vec::new())
        .map_err(|_| "the pg_class query failed".to_owned())?;
    Ok(rows
        .rows
        .iter()
        .map(|row| {
            let text = |index: usize| row.get(index).map(|value| text_of(value));
            let number = |index: usize| text(index).and_then(|value| value.parse::<f64>().ok());
            let relation = text(0).unwrap_or_default();
            let owner = text(1).filter(|owner| !owner.is_empty());
            Behind {
                database: database.to_owned(),
                table: owner.clone().unwrap_or_else(|| relation.clone()),
                relation,
                toast: owner.is_some(),
                note: String::new(),
                xid_percent: limits.xid_percent(number(2).map(|age| age as i64)),
                mxid_percent: limits.mxid_percent(number(3).map(|age| age as i64)),
                autovacuum_count: number(4).map(|count| count as i64),
                since_autovacuum_s: number(5),
            }
        })
        .collect())
}

/// Lists `pg_multixact/members` only when the role may, so a refusal never
/// lands in the server log. Before 11 `pg_ls_dir` checks for a superuser
/// in its code, whatever the grants say.
fn may_list_sql(version: i32) -> &'static str {
    if version >= 110000 {
        "SELECT current_setting('is_superuser') = 'on' \
            OR has_function_privilege('pg_ls_dir(text)', 'EXECUTE')"
    } else {
        "SELECT current_setting('is_superuser') = 'on'"
    }
}

/// The members in use, as whole segment files, the same count the capture
/// records. The offset of the last checkpoint is not used: a segment made
/// after that checkpoint would read as a full member space.
fn members_sql(version: i32) -> Option<String> {
    (version < 190000).then(|| {
        format!(
            "SELECT count(*) * {SEGMENT_MEMBERS} FROM pg_ls_dir('pg_multixact/members') f \
WHERE f ~ '^[0-9A-F]+$'"
        )
    })
}

fn read_members(client: &ConnectedClient, version: i32) -> Members {
    let Some(sql) = members_sql(version) else {
        return Members::NoLimit;
    };
    let scalar = |sql: &str| {
        client
            .exec_bounded_row(sql, SOURCE, &mut Vec::new())
            .ok()
            .and_then(|row| row.first().map(|value| text_of(value)))
    };
    match scalar(may_list_sql(version)).as_deref() {
        Some("t") => {}
        Some(_) => return Members::NeedsSuperuser,
        None => return Members::Unknown,
    }
    match scalar(&sql) {
        Some(used) => Members::Used {
            fraction: used.parse::<f64>().unwrap_or(0.0) / MEMBER_SPACE,
        },
        None => Members::Unknown,
    }
}

/// Elapsed time in its largest unit, such as `20 d`.
pub fn format_ago(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3_600 => format!("{} min", seconds / 60),
        3_600..86_400 => format!("{} h", seconds / 3_600),
        _ => format!("{} d", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::super::Backend;
    use super::*;

    fn worker(pid: &str, database: &str, query: &str) -> Vacuum {
        Vacuum {
            backend: Backend {
                pid: pid.to_owned(),
                database: database.to_owned(),
                query: query.to_owned(),
                ..Backend::default()
            },
            ..Vacuum::default()
        }
    }

    #[test]
    fn each_worker_on_a_table_counts_once() {
        let mut state = Wraparound::default();
        let query = "autovacuum: VACUUM public.orders (to prevent wraparound)";
        state.observe(&[worker("10", "app", query), worker("11", "app", "VACUUM t")]);
        state.observe(&[worker("10", "app", query)]);
        state.observe(&[worker("12", "app", query)]);
        let table = Behind {
            database: "app".to_owned(),
            table: "public.orders".to_owned(),
            relation: "public.orders".to_owned(),
            ..Behind::default()
        };
        assert_eq!(state.tries(&table), 2);
        let database = Behind {
            database: "app".to_owned(),
            ..Behind::default()
        };
        assert_eq!(state.tries(&database), 2);
    }

    #[test]
    fn the_worse_age_names_the_limit() {
        let row = Behind {
            xid_percent: Some(90.0),
            mxid_percent: Some(104.4),
            ..Behind::default()
        };
        assert_eq!(row.over(), "mxid 104%");
        let limits = Limits {
            xid: Some(200_000_000),
            mxid: None,
        };
        assert_eq!(limits.xid_percent(Some(224_000_000)), Some(112.0));
        assert_eq!(limits.mxid_percent(Some(1)), None);
    }

    #[test]
    fn queries_follow_the_server_version() {
        assert!(!databases_sql(90325).contains("mxid_age"));
        assert!(databases_sql(90500).contains("mxid_age(d.datminmxid)"));
        let limits = Limits {
            xid: Some(200_000_000),
            mxid: Some(400_000_000),
        };
        assert!(!tables_sql(90325, limits, true).contains("mxid_age"));
        assert!(tables_sql(170000, limits, true).contains("mxid_age(c.relminmxid) >= 400000000"));
        assert!(!tables_sql(170000, limits, true).contains("relisshared"));
        assert!(tables_sql(170000, limits, false).contains("NOT c.relisshared"));
        assert!(members_sql(90325).unwrap().contains("count(*)"));
        assert!(members_sql(180000).unwrap().contains("count(*)"));
        assert_eq!(members_sql(190000), None);
        assert!(!may_list_sql(100000).contains("has_function_privilege"));
        assert!(may_list_sql(110000).contains("has_function_privilege"));
    }

    #[test]
    fn elapsed_time_takes_its_largest_unit() {
        assert_eq!(format_ago(42.0), "42 s");
        assert_eq!(format_ago(3_599.0), "59 min");
        assert_eq!(format_ago(7_200.0), "2 h");
        assert_eq!(format_ago(20.0 * 86_400.0), "20 d");
    }
}
