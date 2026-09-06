//! Fixture CLI calls the actual native recognizer without building/launching Tauri.
#[path = "../../../src-tauri/src/postgres/schema_compare/expression.rs"]
mod expression;

use std::io::Read;

fn main() {
    let columns: Vec<String> = std::env::args().skip(1).collect();
    let names: Vec<&str> = columns.iter().map(String::as_str).collect();
    let mut text = String::new();
    std::io::stdin().take((expression::MAX_EXPRESSION_BYTES + 1) as u64)
        .read_to_string(&mut text).expect("UTF-8 expression");
    assert!(expression::supported_scalar("pg_catalog", "int4"));
    println!("{}", if expression::comparable(&text, &names) {
        "comparable"
    } else { "notComparable" });
}
