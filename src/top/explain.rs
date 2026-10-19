//! The explain popup of `pg-probe top`: the estimated plan of a running
//! query. It runs `EXPLAIN` without `ANALYZE`, so the query is planned but
//! never executed, and only when the operator asks for it.

use crate::postgres::ConnectedClient;

/// Statements `EXPLAIN` can plan. Anything else, including `EXPLAIN` itself,
/// is refused before it reaches the server.
const EXPLAINABLE: [&str; 7] = [
    "select", "with", "insert", "update", "delete", "values", "table",
];

/// Checks that `query` can be explained and returns the statement to send.
/// `query` is the raw text from `pg_stat_activity`, with its line breaks, and
/// `size_limit` is `track_activity_query_size`: the server cuts longer text.
pub fn statement(query: &str, size_limit: usize) -> Result<String, String> {
    let query = query.trim().trim_end_matches(';').trim_end();
    if query.is_empty() {
        return Err("There is no query to explain.".to_owned());
    }
    if size_limit > 0 && query.len() >= size_limit.saturating_sub(1) {
        return Err(format!(
            "The query text may be cut at track_activity_query_size ({size_limit} bytes), \
             so it cannot be planned."
        ));
    }
    if has_parameters(query) {
        return Err(
            "The query uses parameters ($1, $2, ...) whose values are not visible, \
             so it cannot be planned."
                .to_owned(),
        );
    }
    let keyword = first_keyword(query);
    if !EXPLAINABLE.contains(&keyword.as_str()) {
        return Err(format!(
            "Only SELECT, INSERT, UPDATE, DELETE, VALUES, TABLE, and WITH can be explained, \
             not {}.",
            keyword.to_uppercase()
        ));
    }
    Ok(format!("EXPLAIN {query}"))
}

/// Plans the statement on a client connected to the query's database.
pub fn fetch(client: &ConnectedClient, statement: &str) -> Result<Vec<String>, String> {
    client.single_statement_text_rows(statement)
}

/// The first word of the statement, after leading space, comments, and
/// parentheses, in lower case.
fn first_keyword(query: &str) -> String {
    let mut rest = query;
    loop {
        let trimmed = rest.trim_start().trim_start_matches('(');
        if let Some(after) = trimmed.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, tail)| tail);
        } else if let Some(after) = trimmed.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        } else if trimmed.len() != rest.len() {
            rest = trimmed;
        } else {
            break;
        }
    }
    rest.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_lowercase()
}

/// Whether the statement has a `$n` placeholder outside string literals.
fn has_parameters(query: &str) -> bool {
    let chars: Vec<char> = query.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\'' => {
                index += 1;
                while index < chars.len() && chars[index] != '\'' {
                    index += 1;
                }
            }
            '$' if chars.get(index + 1).is_some_and(char::is_ascii_digit) => {
                // `x$1` is part of an identifier, not a placeholder.
                let in_identifier = index > 0
                    && (chars[index - 1].is_ascii_alphanumeric() || chars[index - 1] == '_');
                if !in_identifier {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_statements_are_prefixed_with_explain() {
        assert_eq!(
            statement("SELECT * FROM t WHERE id = 1;\n", 1024).unwrap(),
            "EXPLAIN SELECT * FROM t WHERE id = 1"
        );
    }

    #[test]
    fn leading_comments_keep_their_line_breaks() {
        let query = "WITH q_target AS ( -- comment\n SELECT 1) SELECT * FROM q_target";
        assert_eq!(first_keyword(query), "with");
        assert_eq!(first_keyword("/* app */ -- x\n (select 1)"), "select");
        assert!(statement(query, 1024).unwrap().contains("-- comment\n"));
    }

    #[test]
    fn analyze_and_other_commands_are_refused() {
        assert!(statement("EXPLAIN ANALYZE DELETE FROM t", 1024).is_err());
        assert!(statement("VACUUM t", 1024).is_err());
        assert!(statement("DO $$ BEGIN END $$", 1024).is_err());
    }

    #[test]
    fn parameters_are_refused_but_literals_and_identifiers_are_not() {
        assert!(statement("SELECT * FROM t WHERE id = $1", 1024).is_err());
        assert!(statement("SELECT '$1' AS price, col$1 FROM t", 1024).is_ok());
    }

    #[test]
    fn text_at_the_size_limit_is_treated_as_cut() {
        let query = format!("SELECT {}", "x".repeat(1100));
        assert!(statement(&query, 1024)
            .unwrap_err()
            .contains("track_activity_query_size"));
    }
}
