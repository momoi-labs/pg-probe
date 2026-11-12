//! Wraparound state over time: the multixact member space, the freeze
//! ranking of the databases, the running vacuums, and the freeze state and
//! autovacuum counters of the tables in the oldest databases.
//!
//! The first three are cluster-wide and run on the session the capture
//! already holds, on every sample. The table read is database-local and runs
//! at most once every [`TABLES_EVERY`]; see ADR-0012. When a target is not the
//! connected database, the session is closed before the target opens, so only
//! one connection exists at a time. The next sample opens the session again.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::postgres::connection::{
    parse_text_bool, parse_text_i64, parse_text_string, AdapterError, ConnectedClient,
};
use crate::postgres::sanitize::sanitize_identifier;

const MEMBERS_SOURCE: &str = "postgres.freeze.members";
const DATABASES_SOURCE: &str = "postgres.freeze.databases";
const VACUUMS_SOURCE: &str = "postgres.freeze.vacuums";
const TABLES_SOURCE: &str = "postgres.freeze.tables";
const VISIT_SOURCE: &str = "postgres.freeze.visit";

/// The table read runs at most this often, whatever the sample interval is.
const TABLES_EVERY: Duration = Duration::from_secs(60);
/// Samples tick on a fixed grid and each one takes some time, so the read
/// counts as due a little before the full period.
const TABLES_SLACK: Duration = Duration::from_secs(1);
/// How many tables each ranking keeps per database.
const TOP: usize = 20;
/// The most tables kept under watch after a vacuum ran on them.
const WATCHED: usize = 100;
/// Bounds every statement of a session opened for a table read.
const STATEMENT_TIMEOUT_MS: u64 = 5_000;
/// What the server reports for the query of a session the role may not see.
const HIDDEN: &str = "<insufficient privilege>";
const MXID_SPAN: i64 = 1 << 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TablesStatus {
    Read,
    /// The table read was not due on this sample.
    NotDue,
    /// Not one of the oldest connectable databases.
    NotSelected,
    NotConnectable,
    Failed,
}

impl TablesStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::NotDue => "not_due",
            Self::NotSelected => "not_selected",
            Self::NotConnectable => "not_connectable",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseFreeze {
    pub database_name: String,
    pub is_template: bool,
    pub allows_connections: bool,
    pub frozenxid_age: i64,
    /// Raw `datminmxid`. None when the server has none or it is invalid.
    pub minmxid: Option<i64>,
    /// `mxid_age(datminmxid)`, from 9.5.
    pub minmxid_age: Option<i64>,
    /// 1 for the database that holds the oldest multixact.
    pub minmxid_rank: Option<i64>,
    pub tables_status: TablesStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableFreeze {
    pub database_name: String,
    pub schema_name: String,
    pub relation_name: String,
    /// The table that owns this TOAST table, as `schema.table`.
    pub toast_of: Option<String>,
    pub frozenxid_age: i64,
    pub minmxid: Option<i64>,
    pub minmxid_age: Option<i64>,
    pub autovacuum_count: Option<i64>,
    pub last_autovacuum: Option<String>,
    pub vacuum_count: Option<i64>,
    pub last_vacuum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningVacuum {
    pub database_name: String,
    /// `schema.table`. None when a manual vacuum names no single table.
    pub relation: Option<String>,
    pub autovacuum: bool,
    pub wraparound: bool,
    /// `query_start` as PostgreSQL text. Tells one try from the next.
    pub started_at: String,
    pub running_seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreezeObservation {
    /// Segment files in `pg_multixact/members`. None when not read.
    pub member_segment_files: Option<i64>,
    pub databases: Vec<DatabaseFreeze>,
    pub tables: Vec<TableFreeze>,
    pub vacuums: Vec<RunningVacuum>,
}

/// The session the capture holds between samples.
pub struct Session {
    pub client: ConnectedClient,
    pub version: i32,
    /// The connected database, when the server told it.
    pub database: Option<String>,
}

/// What the freeze reads remember from one sample to the next.
#[derive(Default)]
pub struct FreezeState {
    /// Whether the role may list `pg_multixact/members`. Asked once.
    may_list_members: Option<bool>,
    /// Tables seen under a vacuum, per database. They stay in the table read
    /// until the capture ends, so a table that keeps failing keeps its
    /// counter series when it leaves the rankings.
    watched: BTreeMap<String, BTreeSet<String>>,
    tables_read: Option<Instant>,
}

impl FreezeState {
    /// The state for the samples after the first, which has just read the
    /// tables.
    pub fn after_first_sample() -> Self {
        Self {
            tables_read: Some(Instant::now()),
            ..Self::default()
        }
    }

    fn tables_due(&self) -> bool {
        self.tables_read
            .is_none_or(|read| read.elapsed() + TABLES_SLACK >= TABLES_EVERY)
    }

    fn watch(&mut self, vacuums: &[RunningVacuum]) {
        for vacuum in vacuums {
            let Some(relation) = &vacuum.relation else {
                continue;
            };
            if self.watched.values().map(BTreeSet::len).sum::<usize>() >= WATCHED {
                return;
            }
            self.watched
                .entry(vacuum.database_name.clone())
                .or_default()
                .insert(relation.clone());
        }
    }
}

/// Reads the freeze state for one sample. `session` is left empty when a
/// table read had to leave the connected database.
pub fn collect(
    session: &mut Option<Session>,
    state: &mut FreezeState,
    outcomes: &mut Vec<ComponentOutcome>,
) -> FreezeObservation {
    let mut freeze = FreezeObservation::default();
    let Some(current) = session.as_ref() else {
        return freeze;
    };
    let version = current.version;

    freeze.member_segment_files = collect_members(&current.client, version, state, outcomes);
    freeze.vacuums = collect_step(VACUUMS_SOURCE, "freeze_vacuums_failed", outcomes, |o| {
        read_vacuums(&current.client, o)
    });
    state.watch(&freeze.vacuums);
    freeze.databases = collect_step(DATABASES_SOURCE, "freeze_databases_failed", outcomes, |o| {
        read_databases(&current.client, version, o)
    });

    if !state.tables_due() {
        return freeze;
    }
    state.tables_read = Some(Instant::now());
    let mut targets = targets(&freeze.databases);
    // The connected database goes first: it needs no new connection.
    let connected = current.database.clone();
    targets.sort_by_key(|target| Some(target) != connected.as_ref());
    for database in freeze.databases.iter_mut() {
        database.tables_status = if targets.contains(&database.database_name) {
            TablesStatus::Failed
        } else if database.allows_connections {
            TablesStatus::NotSelected
        } else {
            TablesStatus::NotConnectable
        };
    }
    let elsewhere: Vec<_> = targets
        .iter()
        .map(|target| {
            current
                .client
                .read_only_target(target, STATEMENT_TIMEOUT_MS)
        })
        .collect();

    for (target, elsewhere) in targets.iter().zip(elsewhere) {
        let watched = state.watched.get(target);
        let started_at = now_utc();
        let read = if Some(target) == connected.as_ref() {
            match session.as_ref() {
                Some(current) => read_tables(&current.client, target, version, watched, outcomes),
                None => Err(AdapterError::ConnectionFailed),
            }
        } else {
            // Close the held session before the target opens.
            *session = None;
            elsewhere
                .open(VISIT_SOURCE, outcomes)
                .and_then(|client| read_tables(&client, target, version, watched, outcomes))
        };
        match read {
            Ok(tables) => {
                outcomes.push(ComponentOutcome::collected(
                    TABLES_SOURCE,
                    started_at,
                    now_utc(),
                    now_utc(),
                ));
                freeze.tables.extend(tables);
                if let Some(database) = freeze
                    .databases
                    .iter_mut()
                    .find(|database| &database.database_name == target)
                {
                    database.tables_status = TablesStatus::Read;
                }
            }
            Err(_) => outcomes.push(
                ComponentOutcome::failed(
                    TABLES_SOURCE,
                    CollectionStatus::IoError,
                    "freeze_tables_failed",
                    None,
                )
                .with_timing(started_at, now_utc()),
            ),
        }
    }
    freeze
}

fn collect_step<T>(
    source: &'static str,
    error_code: &'static str,
    outcomes: &mut Vec<ComponentOutcome>,
    read: impl FnOnce(&mut Vec<ComponentOutcome>) -> Result<Vec<T>, AdapterError>,
) -> Vec<T> {
    let started_at = now_utc();
    match read(outcomes) {
        Ok(rows) => {
            outcomes.push(ComponentOutcome::collected(
                source,
                started_at,
                now_utc(),
                now_utc(),
            ));
            rows
        }
        Err(_) => {
            outcomes.push(
                ComponentOutcome::failed(source, CollectionStatus::IoError, error_code, None)
                    .with_timing(started_at, now_utc()),
            );
            Vec::new()
        }
    }
}

/// Asks first whether the role may list the directory, so a refusal never
/// lands in the server log. Before 11 `pg_ls_dir` checks for a superuser in
/// its code, whatever the grants say.
fn may_list_sql(version: i32) -> &'static str {
    if version >= 110000 {
        "SELECT current_setting('is_superuser') = 'on' \
            OR has_function_privilege('pg_ls_dir(text)', 'EXECUTE')"
    } else {
        "SELECT current_setting('is_superuser') = 'on'"
    }
}

const MEMBERS_SQL: &str =
    "SELECT count(*) FROM pg_ls_dir('pg_multixact/members') f WHERE f ~ '^[0-9A-F]+$'";

fn collect_members(
    client: &ConnectedClient,
    version: i32,
    state: &mut FreezeState,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Option<i64> {
    let started_at = now_utc();
    let skipped = |status, code| {
        ComponentOutcome::failed(MEMBERS_SOURCE, status, code, None)
            .with_timing(started_at, now_utc())
    };
    // From 19 the member offsets are 64-bit and the space has no limit.
    if version >= 190000 {
        outcomes.push(skipped(
            CollectionStatus::Unsupported,
            "not_in_server_version",
        ));
        return None;
    }
    if state.may_list_members.is_none() {
        state.may_list_members = client
            .exec_bounded_row(may_list_sql(version), MEMBERS_SOURCE, outcomes)
            .ok()
            .and_then(|row| parse_text_bool(&row[0]).ok());
    }
    match state.may_list_members {
        Some(true) => {}
        // Not a failure of the capture: the role is what the operator chose.
        Some(false) => {
            outcomes.push(skipped(
                CollectionStatus::Unsupported,
                "members_not_permitted",
            ));
            return None;
        }
        None => {
            outcomes.push(skipped(CollectionStatus::IoError, "freeze_members_failed"));
            return None;
        }
    }
    let files = client
        .exec_bounded_row(MEMBERS_SQL, MEMBERS_SOURCE, outcomes)
        .ok()
        .and_then(|row| parse_text_i64(&row[0]).ok());
    match files {
        Some(files) => {
            outcomes.push(ComponentOutcome::collected(
                MEMBERS_SOURCE,
                started_at,
                now_utc(),
                now_utc(),
            ));
            Some(files)
        }
        None => {
            outcomes.push(skipped(CollectionStatus::IoError, "freeze_members_failed"));
            None
        }
    }
}

const fn has_minmxid(version: i32) -> bool {
    version >= 90300
}

const fn has_mxid_age(version: i32) -> bool {
    version >= 90500
}

fn databases_sql(version: i32) -> String {
    let minmxid = if has_minmxid(version) {
        "datminmxid::text::bigint"
    } else {
        "NULL::bigint"
    };
    let age = if has_mxid_age(version) {
        "mxid_age(datminmxid)"
    } else {
        "NULL::integer"
    };
    format!(
        "SELECT datname, datistemplate, datallowconn, age(datfrozenxid), {minmxid}, {age} \
FROM pg_database ORDER BY datname"
    )
}

fn optional_i64(raw: &[u8]) -> Option<i64> {
    if raw.is_empty() {
        None
    } else {
        parse_text_i64(raw).ok()
    }
}

fn optional_text(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        None
    } else {
        parse_text_string(raw).ok()
    }
}

/// A multixact id of 0 is invalid.
fn valid_mxid(raw: &[u8]) -> Option<i64> {
    optional_i64(raw).filter(|mxid| *mxid != 0)
}

fn read_databases(
    client: &ConnectedClient,
    version: i32,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<DatabaseFreeze>, AdapterError> {
    let rows = client.exec_bounded_rows(&databases_sql(version), DATABASES_SOURCE, outcomes)?;
    let mut databases = Vec::with_capacity(rows.rows.len());
    for row in rows.rows {
        databases.push(DatabaseFreeze {
            database_name: sanitize_identifier(&parse_text_string(&row[0])?).value,
            is_template: parse_text_bool(&row[1])?,
            allows_connections: parse_text_bool(&row[2])?,
            frozenxid_age: parse_text_i64(&row[3])?,
            minmxid: valid_mxid(&row[4]),
            minmxid_age: optional_i64(&row[5]),
            minmxid_rank: None,
            tables_status: TablesStatus::NotDue,
        });
    }
    rank_by_minmxid(&mut databases);
    Ok(databases)
}

/// Whether multixact `a` is older than `b`. The ids wrap at 2^32, so the
/// comparison is on the signed distance. The server keeps all live ids within
/// 2^31 of each other, which makes this sound.
fn mxid_precedes(a: i64, b: i64) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

/// Ranks the databases from the oldest multixact, without an age: before 9.5
/// there is no `mxid_age`. The distance from the oldest value gives a total
/// order.
fn rank_by_minmxid(databases: &mut [DatabaseFreeze]) {
    let Some(oldest) = databases
        .iter()
        .filter_map(|database| database.minmxid)
        .reduce(|oldest, mxid| {
            if mxid_precedes(mxid, oldest) {
                mxid
            } else {
                oldest
            }
        })
    else {
        return;
    };
    let mut distances: Vec<i64> = databases
        .iter()
        .filter_map(|database| database.minmxid)
        .map(|mxid| (mxid - oldest).rem_euclid(MXID_SPAN))
        .collect();
    distances.sort_unstable();
    for database in databases {
        database.minmxid_rank = database.minmxid.and_then(|mxid| {
            let distance = (mxid - oldest).rem_euclid(MXID_SPAN);
            let rank = distances.iter().position(|other| *other == distance)?;
            i64::try_from(rank + 1).ok()
        });
    }
}

/// The databases whose tables are read: the oldest connectable one by xid
/// age, and the oldest connectable one by multixact when it is another.
fn targets(databases: &[DatabaseFreeze]) -> Vec<String> {
    let connectable = || {
        databases
            .iter()
            // A name the sanitizer replaced cannot be connected to.
            .filter(|database| database.allows_connections && !database.database_name.contains('<'))
    };
    // On a tie a template loses, and one database serves both rankings.
    let by_xid =
        connectable().max_by_key(|database| (database.frozenxid_age, !database.is_template));
    let oldest_rank = connectable()
        .filter_map(|database| database.minmxid_rank)
        .min();
    let by_mxid = connectable()
        .filter(|database| database.minmxid_rank == oldest_rank && oldest_rank.is_some())
        .max_by_key(|database| {
            (
                Some(&database.database_name) == by_xid.map(|oldest| &oldest.database_name),
                !database.is_template,
            )
        });
    let mut targets = Vec::new();
    for database in [by_xid, by_mxid].into_iter().flatten() {
        if !targets.contains(&database.database_name) {
            targets.push(database.database_name.clone());
        }
    }
    targets
}

/// The vacuums in progress, and the sessions whose query the role may not
/// see, which have no start time either. Only the table name leaves the
/// query text.
const VACUUMS_SQL: &str = "\
SELECT datname, query, query_start::text, \
    GREATEST(EXTRACT(EPOCH FROM (now() - query_start)), 0)::bigint \
FROM pg_stat_activity \
WHERE pid <> pg_backend_pid() \
    AND ((query_start IS NOT NULL AND (query LIKE 'autovacuum: VACUUM%' \
            OR (state = 'active' AND query ~* '^\\s*vacuum\\M'))) \
        OR query = '<insufficient privilege>')";

fn read_vacuums(
    client: &ConnectedClient,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<RunningVacuum>, AdapterError> {
    let rows = client.exec_bounded_rows(VACUUMS_SQL, VACUUMS_SOURCE, outcomes)?;
    let mut vacuums = Vec::new();
    let mut hidden = false;
    for row in rows.rows {
        let query = parse_text_string(&row[1])?;
        if query == HIDDEN {
            hidden = true;
            continue;
        }
        let Some(parsed) = parse_vacuum(&query) else {
            continue;
        };
        vacuums.push(RunningVacuum {
            database_name: sanitize_identifier(&parse_text_string(&row[0])?).value,
            relation: parsed.relation,
            autovacuum: parsed.autovacuum,
            wraparound: parsed.wraparound,
            started_at: parse_text_string(&row[2])?,
            running_seconds: parse_text_i64(&row[3])?,
        });
    }
    if hidden {
        // An empty list must not read as "no vacuum is running".
        outcomes.push(ComponentOutcome::failed(
            VACUUMS_SOURCE,
            CollectionStatus::Unsupported,
            "activity_not_visible",
            None,
        ));
    }
    Ok(vacuums)
}

struct ParsedVacuum {
    relation: Option<String>,
    autovacuum: bool,
    wraparound: bool,
}

/// Reads the table out of the text of a vacuum. An autovacuum worker reports
/// `autovacuum: VACUUM [ANALYZE] schema.table [(to prevent wraparound)]`. A
/// manual vacuum has its options first, then at most one plain table name
/// that this reader accepts.
fn parse_vacuum(query: &str) -> Option<ParsedVacuum> {
    let (autovacuum, rest) = match query.strip_prefix("autovacuum: VACUUM") {
        Some(rest) => (true, rest),
        None => {
            let trimmed = query.trim_start();
            let head = trimmed.get(..6)?;
            if !head.eq_ignore_ascii_case("vacuum") {
                return None;
            }
            (false, &trimmed[6..])
        }
    };
    let wraparound = autovacuum && rest.trim_end().ends_with("(to prevent wraparound)");
    let mut rest = rest.trim_start();
    if !autovacuum && rest.starts_with('(') {
        rest = rest[rest.find(')')? + 1..].trim_start();
    }
    let mut words = rest
        .split(|c: char| c.is_whitespace() || c == ';')
        .filter(|word| !word.is_empty())
        .skip_while(|word| {
            ["full", "freeze", "verbose", "analyze"]
                .iter()
                .any(|option| word.eq_ignore_ascii_case(option))
        });
    let relation = words.next().and_then(|word| {
        // A column list or a second table makes the target ambiguous.
        let more = words.next().is_some_and(|next| !next.starts_with('('));
        let plain = !word.contains([',', '(', ')']);
        if !autovacuum && (more || !plain) {
            return None;
        }
        // Each part has its own length limit.
        let parts: Vec<_> = word.split('.').map(sanitize_identifier).collect();
        (parts.len() <= 2 && parts.iter().all(|part| !part.redacted)).then(|| {
            parts
                .into_iter()
                .map(|part| part.value)
                .collect::<Vec<_>>()
                .join(".")
        })
    });
    Some(ParsedVacuum {
        relation,
        autovacuum,
        wraparound,
    })
}

/// The tables with the oldest xid, the tables with the oldest multixact, and
/// the watched ones, in one pass over `pg_class`. Catalogs and TOAST tables
/// count. The multixact order is the distance from the `datminmxid` of the
/// database, which is never ahead of a table.
fn tables_sql(version: i32, watched: Option<&BTreeSet<String>>) -> String {
    let (minmxid, by_mxid) = if has_minmxid(version) {
        (
            "c.relminmxid::text::bigint",
            format!(
                "UNION (SELECT rel.oid FROM rel, \
    (SELECT datminmxid::text::bigint AS base FROM pg_database \
        WHERE datname = current_database()) db \
    WHERE rel.minmxid <> 0 \
    ORDER BY (rel.minmxid - db.base + {MXID_SPAN}) % {MXID_SPAN} LIMIT {TOP}) "
            ),
        )
    } else {
        ("NULL::bigint", String::new())
    };
    let age = if has_mxid_age(version) {
        "mxid_age(c.relminmxid)"
    } else {
        "NULL::integer"
    };
    // Watched names passed the identifier sanitizer, which rejects quotes.
    let watched = match watched.filter(|names| !names.is_empty()) {
        Some(names) => {
            let list = |qualified: bool| {
                names
                    .iter()
                    .filter(|name| name.contains('.') == qualified)
                    .map(|name| format!("'{name}'"))
                    .chain(["''".to_owned()])
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            // A manual vacuum may name its table without the schema.
            format!(
                "UNION (SELECT oid FROM rel WHERE nspname || '.' || relname IN ({}) \
    OR relname IN ({})) ",
                list(true),
                list(false)
            )
        }
        None => String::new(),
    };
    format!(
        "\
WITH rel AS ( \
    SELECT c.oid, n.nspname, c.relname, c.relkind, age(c.relfrozenxid) AS xid_age, \
        {minmxid} AS minmxid, {age} AS mxid_age \
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
    WHERE c.relkind IN ('r', 'm', 't') \
), picked AS ( \
    (SELECT oid FROM rel ORDER BY xid_age DESC LIMIT {TOP}) \
    {by_mxid}{watched}\
) \
SELECT r.nspname, r.relname, \
    CASE WHEN o.relname IS NOT NULL THEN o.nspname || '.' || o.relname END, \
    r.xid_age, r.minmxid, r.mxid_age, \
    s.autovacuum_count, s.last_autovacuum::text, s.vacuum_count, s.last_vacuum::text \
FROM rel r \
JOIN picked p ON p.oid = r.oid \
LEFT JOIN (SELECT t.reltoastrelid, tn.nspname, t.relname FROM pg_class t \
    JOIN pg_namespace tn ON tn.oid = t.relnamespace WHERE t.reltoastrelid <> 0) o \
    ON r.relkind = 't' AND o.reltoastrelid = r.oid \
LEFT JOIN pg_stat_all_tables s ON s.relid = r.oid \
ORDER BY r.xid_age DESC"
    )
}

fn read_tables(
    client: &ConnectedClient,
    database: &str,
    version: i32,
    watched: Option<&BTreeSet<String>>,
    outcomes: &mut Vec<ComponentOutcome>,
) -> Result<Vec<TableFreeze>, AdapterError> {
    let rows = client.exec_bounded_rows(&tables_sql(version, watched), TABLES_SOURCE, outcomes)?;
    let mut tables = Vec::with_capacity(rows.rows.len());
    for row in rows.rows {
        tables.push(TableFreeze {
            database_name: database.to_owned(),
            schema_name: sanitize_identifier(&parse_text_string(&row[0])?).value,
            relation_name: sanitize_identifier(&parse_text_string(&row[1])?).value,
            toast_of: optional_text(&row[2]).map(|owner| sanitize_identifier(&owner).value),
            frozenxid_age: parse_text_i64(&row[3])?,
            minmxid: valid_mxid(&row[4]),
            minmxid_age: optional_i64(&row[5]),
            autovacuum_count: optional_i64(&row[6]),
            last_autovacuum: optional_text(&row[7]),
            vacuum_count: optional_i64(&row[8]),
            last_vacuum: optional_text(&row[9]),
        });
    }
    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(name: &str, xid_age: i64, minmxid: Option<i64>, connects: bool) -> DatabaseFreeze {
        DatabaseFreeze {
            database_name: name.to_owned(),
            is_template: false,
            allows_connections: connects,
            frozenxid_age: xid_age,
            minmxid,
            minmxid_age: None,
            minmxid_rank: None,
            tables_status: TablesStatus::NotDue,
        }
    }

    #[test]
    fn the_multixact_rank_follows_the_wrap() {
        // 4294967000 is older than 5: the ids wrapped in between.
        let mut databases = vec![
            database("after_wrap", 0, Some(5), true),
            database("before_wrap", 0, Some(4_294_967_000), true),
            database("no_mxid", 0, None, true),
            database("newest", 0, Some(900), true),
        ];
        rank_by_minmxid(&mut databases);
        let ranks: Vec<_> = databases.iter().map(|d| d.minmxid_rank).collect();
        assert_eq!(ranks, [Some(2), Some(1), None, Some(3)]);
    }

    #[test]
    fn targets_are_the_oldest_connectable_databases() {
        let mut databases = vec![
            database("template0", 900, Some(1), false),
            database("app", 500, Some(30), true),
            database("logs", 100, Some(20), true),
        ];
        rank_by_minmxid(&mut databases);
        // template0 is the oldest on both counts and cannot be read.
        assert_eq!(targets(&databases), ["app", "logs"]);
        databases[2].minmxid = Some(40);
        rank_by_minmxid(&mut databases);
        assert_eq!(targets(&databases), ["app"]);
        // A tie on both counts is one target.
        databases[2].minmxid = Some(30);
        databases[2].frozenxid_age = 500;
        rank_by_minmxid(&mut databases);
        assert_eq!(targets(&databases).len(), 1);
    }

    #[test]
    fn an_autovacuum_gives_its_table_and_reason() {
        let wraparound =
            parse_vacuum("autovacuum: VACUUM pg_toast.pg_toast_16384 (to prevent wraparound)")
                .unwrap();
        assert_eq!(
            wraparound.relation.as_deref(),
            Some("pg_toast.pg_toast_16384")
        );
        assert!(wraparound.autovacuum && wraparound.wraparound);
        let plain = parse_vacuum("autovacuum: VACUUM ANALYZE public.orders").unwrap();
        assert_eq!(plain.relation.as_deref(), Some("public.orders"));
        assert!(!plain.wraparound);
    }

    #[test]
    fn a_manual_vacuum_gives_one_plain_table_or_none() {
        let relation = |query| parse_vacuum(query).unwrap().relation;
        assert_eq!(
            relation("VACUUM (FREEZE, VERBOSE) public.orders;").as_deref(),
            Some("public.orders")
        );
        assert_eq!(
            relation("vacuum freeze analyze orders").as_deref(),
            Some("orders")
        );
        assert_eq!(relation("VACUUM"), None);
        assert_eq!(relation("VACUUM orders, items"), None);
        assert_eq!(relation("VACUUM \"Odd Name\""), None);
        assert!(!parse_vacuum("VACUUM orders").unwrap().autovacuum);
        assert!(parse_vacuum("SELECT 'vacuum'").is_none());
    }

    #[test]
    fn the_table_query_fits_the_server_version() {
        let old = tables_sql(90300, None);
        assert!(old.contains("c.relminmxid::text::bigint"));
        assert!(old.contains("NULL::integer AS mxid_age"));
        assert!(old.contains("LIMIT 20"));
        assert!(!old.contains("mxid_age(c.relminmxid)"));
        assert!(tables_sql(90500, None).contains("mxid_age(c.relminmxid)"));
        assert!(!tables_sql(90200, None).contains("relminmxid"));
        let watched = BTreeSet::from(["public.orders".to_owned()]);
        let sql = tables_sql(90300, Some(&watched));
        assert!(sql.contains("IN ('public.orders', '') OR relname IN ('')"));
    }

    #[test]
    fn the_table_read_waits_for_its_period() {
        assert!(FreezeState::default().tables_due());
        assert!(!FreezeState::after_first_sample().tables_due());
    }

    #[test]
    fn a_vacuumed_table_stays_watched() {
        let mut state = FreezeState::default();
        state.watch(&[RunningVacuum {
            database_name: "app".to_owned(),
            relation: Some("public.orders".to_owned()),
            autovacuum: true,
            wraparound: true,
            started_at: "2026-01-01 00:00:00+00".to_owned(),
            running_seconds: 4,
        }]);
        assert!(state.watched["app"].contains("public.orders"));
    }
}
