use std::io::{self, Read};
fn main() {
 let mut input=String::new();io::stdin().read_to_string(&mut input).unwrap();
 let value:serde_json::Value=serde_json::from_str(&input).unwrap();
 let sql=value["sql"].as_str().unwrap();
 let options=sqlformat::FormatOptions{indent:sqlformat::Indent::Spaces(2),uppercase:true,lines_between_queries:2};
 let result=sqlformat::format(sql,&sqlformat::QueryParams::None,options);
 println!("{}",serde_json::json!({"output":result}));
}