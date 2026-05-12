//! Serialise `redis::Value` into a JSON-friendly discriminated union
//! the frontend renders type-aware results from. See Q11 in the
//! grilling session — `nil` / `int` / `status` / `string` /
//! `error` / `array` arms, with strings carrying an encoding tag so
//! non-UTF8 bytes render via hex.

use serde::Serialize;

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SerializedValue {
    Nil,
    Int {
        value: i64,
    },
    Status {
        value: String,
    },
    String {
        value: String,
        /// `utf8` when the bytes decode cleanly to text; `hex` when
        /// they don't (raw bytes hex-encoded with `0x` prefix).
        encoding: String,
    },
    Error {
        value: String,
    },
    Array {
        value: Vec<SerializedValue>,
    },
}

pub fn serialize(value: redis::Value) -> SerializedValue {
    match value {
        redis::Value::Nil => SerializedValue::Nil,
        redis::Value::Int(n) => SerializedValue::Int { value: n },
        redis::Value::SimpleString(s) | redis::Value::VerbatimString { text: s, .. } => {
            SerializedValue::Status { value: s }
        }
        redis::Value::Okay => SerializedValue::Status {
            value: "OK".to_string(),
        },
        redis::Value::BulkString(bytes) => encode_string(bytes),
        redis::Value::Array(values) | redis::Value::Set(values) => SerializedValue::Array {
            value: values.into_iter().map(serialize).collect(),
        },
        redis::Value::Map(entries) => SerializedValue::Array {
            value: entries
                .into_iter()
                .flat_map(|(k, v)| vec![serialize(k), serialize(v)])
                .collect(),
        },
        redis::Value::Double(n) => SerializedValue::String {
            value: n.to_string(),
            encoding: "utf8".to_string(),
        },
        redis::Value::Boolean(b) => SerializedValue::String {
            value: b.to_string(),
            encoding: "utf8".to_string(),
        },
        redis::Value::BigNumber(n) => SerializedValue::String {
            value: n.to_string(),
            encoding: "utf8".to_string(),
        },
        redis::Value::Attribute { data, .. } => serialize(*data),
        redis::Value::Push { data, .. } => SerializedValue::Array {
            value: data.into_iter().map(serialize).collect(),
        },
        redis::Value::ServerError(err) => SerializedValue::Error {
            value: err.details().unwrap_or("server error").to_string(),
        },
    }
}

/// UTF-8 first; on failure, fall back to hex (`0x...` prefix).
pub fn encode_string(bytes: Vec<u8>) -> SerializedValue {
    match String::from_utf8(bytes) {
        Ok(text) if !contains_unprintable(&text) => SerializedValue::String {
            value: text,
            encoding: "utf8".to_string(),
        },
        Ok(text) => SerializedValue::String {
            value: bytes_to_hex(text.as_bytes()),
            encoding: "hex".to_string(),
        },
        Err(err) => SerializedValue::String {
            value: bytes_to_hex(&err.into_bytes()),
            encoding: "hex".to_string(),
        },
    }
}

/// "Looks like usable text" — printable ASCII + common whitespace.
/// Strings with stray NUL bytes or control chars (other than CR/LF/
/// TAB) render better as hex so the viewer doesn't show garbled
/// glyphs.
fn contains_unprintable(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_serializes_to_kind_nil() {
        assert_eq!(serialize(redis::Value::Nil), SerializedValue::Nil);
    }

    #[test]
    fn int_carries_value() {
        assert_eq!(
            serialize(redis::Value::Int(42)),
            SerializedValue::Int { value: 42 }
        );
    }

    #[test]
    fn status_simplestring_returns_status_kind() {
        assert_eq!(
            serialize(redis::Value::SimpleString("OK".to_string())),
            SerializedValue::Status {
                value: "OK".to_string()
            }
        );
    }

    #[test]
    fn okay_variant_returns_ok() {
        assert_eq!(
            serialize(redis::Value::Okay),
            SerializedValue::Status {
                value: "OK".to_string()
            }
        );
    }

    #[test]
    fn utf8_bulk_string_uses_utf8_encoding() {
        let result = serialize(redis::Value::BulkString(b"hello".to_vec()));
        assert_eq!(
            result,
            SerializedValue::String {
                value: "hello".to_string(),
                encoding: "utf8".to_string()
            }
        );
    }

    #[test]
    fn non_utf8_bulk_string_falls_back_to_hex() {
        let result = serialize(redis::Value::BulkString(vec![0xff, 0xfe, 0x00]));
        match result {
            SerializedValue::String { encoding, .. } => assert_eq!(encoding, "hex"),
            other => panic!("expected hex string, got {:?}", other),
        }
    }

    #[test]
    fn array_recursively_serializes() {
        let result = serialize(redis::Value::Array(vec![
            redis::Value::Int(1),
            redis::Value::BulkString(b"two".to_vec()),
        ]));
        match result {
            SerializedValue::Array { value } => {
                assert_eq!(value.len(), 2);
                assert!(matches!(value[0], SerializedValue::Int { value: 1 }));
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    #[test]
    fn map_flattens_to_kv_pairs() {
        let result = serialize(redis::Value::Map(vec![(
            redis::Value::BulkString(b"k".to_vec()),
            redis::Value::BulkString(b"v".to_vec()),
        )]));
        match result {
            SerializedValue::Array { value } => assert_eq!(value.len(), 2),
            other => panic!("expected flattened map, got {:?}", other),
        }
    }
}
