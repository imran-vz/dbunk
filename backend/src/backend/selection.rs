use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionError {
    InvalidUtf8Boundary,
    Unlexable,
}

/// Select SQL using the core lexer. Offsets are UTF-8 byte offsets, never
/// UTF-16 code units. A caret at a statement end or its semicolon chooses that
/// statement; whitespace between statements chooses the following statement.
/// Trailing whitespace/semicolon at EOF chooses the last statement.
pub fn select_sql(
    sql: &str,
    selection: &Range<usize>,
    script: bool,
) -> Result<Option<String>, SelectionError> {
    Ok(select_sql_range(sql, selection, script)?.map(|range| sql[range].to_owned()))
}

/// The exact source range sent for execution, for host-local diagnostics.
/// This uses the same selection rules as `select_sql` without searching for
/// repeated statement text in the editor.
pub fn select_sql_range(
    sql: &str,
    selection: &Range<usize>,
    script: bool,
) -> Result<Option<Range<usize>>, SelectionError> {
    if selection.start > selection.end
        || selection.end > sql.len()
        || !sql.is_char_boundary(selection.start)
        || !sql.is_char_boundary(selection.end)
    {
        return Err(SelectionError::InvalidUtf8Boundary);
    }
    let (text, caret) = if script {
        (sql, None)
    } else if !selection.is_empty() {
        (&sql[selection.clone()], None)
    } else {
        (sql, Some(selection.start))
    };
    let statements = crate::postgres::sql_class::describe_script(text)
        .map_err(|()| SelectionError::Unlexable)?;
    if statements.is_empty() {
        return Ok(None);
    }
    let Some(caret) = caret else {
        return Ok(Some(if script {
            0..sql.len()
        } else {
            selection.clone()
        }));
    };
    let chosen = statements
        .iter()
        .find(|statement| caret <= statement.end)
        .or_else(|| statements.last())
        .expect("non-empty statements");
    Ok(Some(chosen.start..chosen.end))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_ranges_distinguish_duplicate_statements() {
        let sql = "SELECT '🙂'; SELECT '🙂';";
        let second = sql.rfind("SELECT").unwrap();
        let range = select_sql_range(sql, &(second..second), false)
            .unwrap()
            .unwrap();
        assert_eq!(range, second..sql.len() - 1);
        assert_eq!(
            select_sql_range(sql, &(second..sql.len()), false).unwrap(),
            Some(second..sql.len())
        );
        assert_eq!(
            select_sql_range(sql, &(second..second), true).unwrap(),
            Some(0..sql.len())
        );
    }
    #[test]
    fn respects_postgres_quoting_comments_and_utf8() {
        let sql = "SELECT '🙂;x'; /* outer /* nested; */ */ SELECT $$a;b$$; -- end";
        assert_eq!(
            select_sql(sql, &(0..0), false).unwrap().as_deref(),
            Some("SELECT '🙂;x'")
        );
        let end = sql.find(';').unwrap(); // inside literal
        assert_eq!(
            select_sql(sql, &(end..end), false).unwrap().as_deref(),
            Some("SELECT '🙂;x'")
        );
        assert_eq!(
            select_sql(sql, &(sql.len()..sql.len()), false)
                .unwrap()
                .as_deref(),
            Some("SELECT $$a;b$$")
        );
        let unicode = sql.find('🙂').unwrap();
        assert_eq!(
            select_sql(sql, &(unicode + 1..unicode + 1), false),
            Err(SelectionError::InvalidUtf8Boundary)
        );
        assert_eq!(
            select_sql("/* only */; -- comments", &(0..0), false).unwrap(),
            None
        );
    }
    #[test]
    fn selection_and_script_preserve_original_text() {
        let sql = "SELECT 1; SELECT 2;";
        assert_eq!(
            select_sql(sql, &(10..18), false).unwrap().as_deref(),
            Some("SELECT 2")
        );
        assert_eq!(
            select_sql(sql, &(10..18), true).unwrap().as_deref(),
            Some(sql)
        );
        assert_eq!(
            select_sql(sql, &(8..8), false).unwrap().as_deref(),
            Some("SELECT 1")
        );
        assert_eq!(
            select_sql(sql, &(9..9), false).unwrap().as_deref(),
            Some("SELECT 2")
        );
    }
}
