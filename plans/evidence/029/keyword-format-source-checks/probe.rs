#![allow(dead_code)]
#[path = "/Users/imran/projects/Code/dbunk/apps/native/src/sql_format.rs"]
mod sql_format;
use std::io::{Read, Write};
fn main() {
    let mut input=String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let mut out=std::io::stdout().lock();
    for sql in input.split_terminator('\0') {
        match sql_format::format_edits(sql) {
            Ok(edits) => {
                let mut previous=0;
                for edit in edits {
                    out.write_all(sql[previous..edit.range.start].as_bytes()).unwrap();
                    out.write_all(edit.text.as_bytes()).unwrap();
                    previous=edit.range.end;
                }
                out.write_all(sql[previous..].as_bytes()).unwrap();
            },
            Err(e) => write!(out,"!ERROR!{e}").unwrap(),
        }
        out.write_all(&[0]).unwrap();
    }
}
