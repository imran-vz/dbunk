use crate::{quote_double, PgDriverOptions};

pub(crate) const READ_ONLY_SESSION_SQL: &str = "SET default_transaction_read_only = on";

/// Ordered post-connect statements shared by every PostgreSQL driver.
pub(crate) fn driver_option_sql(options: &PgDriverOptions, read_only: bool) -> Vec<String> {
    let mut statements = Vec::new();
    if let Some(ms) = options.statement_timeout_ms {
        statements.push(format!("SET statement_timeout = {ms}"));
    }
    if let Some(ms) = options.idle_in_transaction_timeout_ms {
        statements.push(format!("SET idle_in_transaction_session_timeout = {ms}"));
    }
    if let Some(path) = options
        .default_search_path
        .as_ref()
        .filter(|path| !path.is_empty())
    {
        statements.push(format!(
            "SET search_path TO {}",
            path.iter()
                .map(|part| quote_double(part))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(role) = options
        .default_role
        .as_ref()
        .filter(|role| !role.is_empty())
    {
        statements.push(format!("SET ROLE {}", quote_double(role)));
    }
    if read_only {
        statements.push(READ_ONLY_SESSION_SQL.to_string());
    }
    statements
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn options_are_ordered_and_identifiers_are_quoted() {
        let options = PgDriverOptions {
            statement_timeout_ms: Some(1),
            idle_in_transaction_timeout_ms: Some(2),
            default_search_path: Some(vec!["public".into(), "a\"b".into()]),
            default_role: Some("reader".into()),
            ..Default::default()
        };
        assert_eq!(
            driver_option_sql(&options, false),
            vec![
                "SET statement_timeout = 1",
                "SET idle_in_transaction_session_timeout = 2",
                "SET search_path TO \"public\", \"a\"\"b\"",
                "SET ROLE \"reader\""
            ]
        );
        assert_eq!(
            driver_option_sql(&PgDriverOptions::default(), true),
            vec!["SET default_transaction_read_only = on"]
        );
    }
}
