use super::*;
#[test]
fn full_file_bytes_are_exact_and_cancellation_refuses_before_preparation() {
    let sql = "-- literal NULL vs NULL\r\nSELECT '雪', 9223372036854775807::numeric;\n";
    let token = files::Cancellation::default();
    let prepared = save::prepare_sql(sql, &token).unwrap();
    assert_eq!(prepared.bytes(), sql.as_bytes());
    assert_eq!(prepared.source().scope, files::Scope::CompleteResult);
    token.cancel();
    assert!(save::prepare_sql(sql, &token).is_err());
    assert!(
        save::prepare_sql(
            &"x".repeat(4 * 1024 * 1024 + 1),
            &files::Cancellation::default()
        )
        .is_err()
    );
}
#[test]
fn file_allowance_survives_cancel_until_joined_owner_is_released() {
    let budget = Rc::new(Cell::new(0));
    let lease = Lease::new(budget.clone(), 1024).unwrap();
    let waiter = lease.clone();
    let token = files::Cancellation::default();
    token.cancel();
    drop(lease);
    assert_eq!(budget.get(), 1024);
    drop(waiter);
    assert_eq!(budget.get(), 0);
}
#[test]
fn status_remains_utf8_bounded_without_clipping_short_errors() {
    assert_eq!(bounded_status("Destination exists"), "Destination exists");
    let text = bounded_status(&"雪".repeat(2000));
    assert!(text.len() <= 4096 + " [truncated]".len());
    assert!(text.ends_with(" [truncated]"));
}
