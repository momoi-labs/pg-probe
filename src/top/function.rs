//! The function panel of `pg-probe top`: the definition and call statistics
//! of the user functions a query calls, in the spirit of psql's `\df+` and
//! `\sf`. It runs one catalog query, only when the operator asks for it.

use ratatui::text::{Line, Span};
use tui_kiso::{Page, Theme};

use crate::postgres::{AdapterError, ConnectedClient};

const SOURCE: &str = "postgres.top.function";
/// Upper bound on names looked up and functions returned for one query.
const MAX_FUNCTIONS: usize = 20;

/// One function definition with its statistics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FunctionDef {
    pub schema: String,
    pub name: String,
    pub arguments: String,
    pub result: String,
    pub language: String,
    pub volatility: String,
    pub security_definer: bool,
    pub owner: String,
    pub cost: String,
    pub rows: String,
    pub calls: Option<u64>,
    pub total_ms: Option<f64>,
    pub self_ms: Option<f64>,
    pub track_functions: String,
    pub source: String,
    pub library: String,
}

/// Names that may be function calls in a query: unquoted identifiers, with
/// an optional schema, followed by `(`. Built-in functions are filtered out
/// later by the catalog query. Names are folded to lower case, as the
/// server folds unquoted identifiers.
pub fn candidate_names(query: &str) -> Vec<String> {
    let chars: Vec<char> = query.chars().collect();
    let mut names: Vec<String> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c == '\'' {
            // Skip string literals, so text inside them is never a call.
            index += 1;
            while index < chars.len() && chars[index] != '\'' {
                index += 1;
            }
            index += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '_' | '$'))
            {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            let mut next = index;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            let is_call = chars.get(next) == Some(&'(');
            let followed_by_dot = chars.get(index) == Some(&'.');
            if is_call && !followed_by_dot {
                let name = word.to_lowercase();
                if !names.contains(&name) && names.len() < MAX_FUNCTIONS {
                    names.push(name);
                }
            }
            continue;
        }
        index += 1;
    }
    names
}

/// Builds the catalog query for the given names. Returns `None` when there
/// is nothing to look up. Every name holds only `[a-z0-9_$]`, so it can be
/// written as a literal without escaping.
fn functions_sql(names: &[String]) -> Option<String> {
    let names: Vec<&String> = names
        .iter()
        .filter(|name| {
            !name.is_empty()
                && name.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'$')
                })
        })
        .collect();
    if names.is_empty() {
        return None;
    }
    let list = names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "\
SELECT n.nspname, p.proname, pg_get_function_arguments(p.oid), \
    pg_get_function_result(p.oid), l.lanname, p.provolatile, p.prosecdef, \
    pg_get_userbyid(p.proowner), p.procost, p.prorows, \
    s.calls, s.total_time, s.self_time, current_setting('track_functions'), \
    p.prosrc, p.probin \
FROM pg_proc p \
JOIN pg_namespace n ON n.oid = p.pronamespace \
JOIN pg_language l ON l.oid = p.prolang \
LEFT JOIN pg_stat_user_functions s ON s.funcid = p.oid \
WHERE p.proname IN ({list}) \
  AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
ORDER BY n.nspname, p.proname, p.oid \
LIMIT {MAX_FUNCTIONS}"
    ))
}

/// Looks up the user functions that `query` may call, on a client connected
/// to the database the query runs in.
pub fn fetch(client: &ConnectedClient, query: &str) -> Result<Vec<FunctionDef>, AdapterError> {
    let Some(sql) = functions_sql(&candidate_names(query)) else {
        return Ok(Vec::new());
    };
    let rows = client.exec_bounded_rows(&sql, SOURCE, &mut Vec::new())?;
    Ok(rows
        .rows
        .into_iter()
        .map(|row| {
            let text = |index: usize| {
                row.get(index)
                    .map(|value| super::text_of(value))
                    .unwrap_or_default()
            };
            FunctionDef {
                schema: text(0),
                name: text(1),
                arguments: text(2),
                result: text(3),
                language: text(4),
                volatility: match text(5).as_str() {
                    "i" => "IMMUTABLE",
                    "s" => "STABLE",
                    _ => "VOLATILE",
                }
                .to_owned(),
                security_definer: text(6) == "t",
                owner: text(7),
                cost: text(8),
                rows: text(9),
                calls: text(10).parse().ok(),
                total_ms: text(11).parse().ok(),
                self_ms: text(12).parse().ok(),
                track_functions: text(13),
                source: row
                    .get(14)
                    .map(|value| source_text(value))
                    .unwrap_or_default(),
                library: text(15),
            }
        })
        .collect())
}

/// Decodes function source for display. Unlike query text it keeps line
/// breaks, but tabs become spaces and other control characters are
/// dropped, so the source cannot send escape sequences to the terminal.
fn source_text(value: &[u8]) -> String {
    String::from_utf8_lossy(value)
        .replace('\t', "    ")
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect()
}

/// The function panel's page for one definition, `width` cells wide.
pub fn page(t: &Theme, function: &FunctionDef, width: usize) -> Page {
    let mut page = Page::default();
    let field = |page: &mut Page, key: &str, value: Vec<Span<'static>>| {
        for line in super::fact_lines(t, width, 0, 16, key, value) {
            page.line(line);
        }
    };
    let plain = |text: String| vec![Span::styled(text, t.text())];

    page.line(Line::styled(
        format!("{}.{}", function.schema, function.name),
        t.strong(),
    ));
    page.blank();
    field(&mut page, "Arguments", plain(or_none(&function.arguments)));
    field(&mut page, "Returns", plain(function.result.clone()));
    field(&mut page, "Language", plain(function.language.clone()));
    field(&mut page, "Volatility", plain(function.volatility.clone()));
    let security = if function.security_definer {
        "definer"
    } else {
        "invoker"
    };
    field(&mut page, "Security", plain(security.to_owned()));
    field(&mut page, "Owner", plain(function.owner.clone()));
    field(
        &mut page,
        "Cost / rows",
        plain(format!("{} / {}", function.cost, function.rows)),
    );
    page.blank();

    match (function.calls, function.total_ms, function.self_ms) {
        (Some(calls), Some(total_ms), Some(self_ms)) => {
            field(&mut page, "Calls", plain(group_digits(calls)));
            field(
                &mut page,
                "Total time",
                vec![
                    Span::styled(super::format_duration(total_ms / 1000.0), t.text()),
                    Span::styled(
                        format!(
                            "   (self {}, the rest in queries it runs)",
                            super::format_duration(self_ms / 1000.0)
                        ),
                        t.muted(),
                    ),
                ],
            );
            if calls > 0 {
                field(
                    &mut page,
                    "Mean time",
                    plain(super::format_duration(total_ms / 1000.0 / calls as f64)),
                );
            }
        }
        _ if function.track_functions == "none" => page.line(Line::styled(
            "Function stats off (track_functions = none)",
            t.info(),
        )),
        _ => page.line(Line::styled(
            "No calls recorded since the last stats reset",
            t.muted(),
        )),
    }

    page.blank();
    page.section(t, width, "Source", None);
    let source = if matches!(function.language.as_str(), "c" | "internal") {
        vec![format!(
            "{} function: symbol {} in {}",
            function.language,
            function.source.trim(),
            or_none(&function.library)
        )]
    } else {
        let source = function.source.trim_matches('\n');
        let digits = source.lines().count().to_string().len();
        source
            .lines()
            .enumerate()
            .map(|(number, line)| format!("{:>digits$}  {line}", number + 1))
            .collect()
    };
    super::code_block(t, &mut page, &source, width);
    page
}

fn or_none(value: &str) -> String {
    if value.is_empty() {
        "(none)".to_owned()
    } else {
        value.to_owned()
    }
}

fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;
    use tui_kiso::Scroll;

    #[test]
    fn calls_are_found_outside_string_literals() {
        let query =
            "SELECT * FROM fnc_conhecimento_importarxml_gerarcte(1, 1, '4326 fake(1)', 40) \
                     WHERE x IN (SELECT public.Other_Fn (2)) AND count(*) > 0";
        assert_eq!(
            candidate_names(query),
            [
                "fnc_conhecimento_importarxml_gerarcte",
                "in",
                "other_fn",
                "count"
            ]
        );
    }

    #[test]
    fn only_plain_lowercase_names_reach_the_sql() {
        assert!(functions_sql(&["x'); DROP TABLE t; --".to_owned()]).is_none());
        let sql = functions_sql(&["fnc_a".to_owned(), "count".to_owned()]).unwrap();
        assert!(sql.contains("p.proname IN ('fnc_a', 'count')"));
        assert!(sql.contains("NOT IN ('pg_catalog', 'information_schema')"));
    }

    #[test]
    fn source_keeps_lines_but_drops_escape_sequences() {
        assert_eq!(
            source_text(b"BEGIN\n\tRETURN;\x1b[2J\nEND"),
            "BEGIN\n    RETURN;[2J\nEND"
        );
    }

    #[test]
    fn stats_off_is_explained() {
        let function = FunctionDef {
            track_functions: "none".to_owned(),
            ..FunctionDef::default()
        };
        let theme = Theme::truecolor(kiso_tokens::GOLD_DARK);
        let page = page(&theme, &function, 60);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 62, 20));
        page.view(&theme, &Scroll::default())
            .render(Rect::new(0, 0, 60, 20), &mut buffer);
        let text: String = (0..20)
            .map(|y| (0..62).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
            .collect();
        assert!(text.contains("track_functions = none"), "{text}");
    }

    #[test]
    fn digits_are_grouped_in_thousands() {
        assert_eq!(group_digits(18_442), "18,442");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000_000), "1,000,000");
    }
}
