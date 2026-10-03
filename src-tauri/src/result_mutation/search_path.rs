//! Captured search_path parsing for unqualified native query targets. The
//! split follows PostgreSQL's `SplitIdentifierString` for list GUCs; anything
//! it cannot reproduce exactly (non-ASCII unquoted text, over-long names,
//! malformed quoting) is refused instead of approximated.

const MAX_IDENTIFIER_BYTES: usize = 63;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SearchPathSchemas {
    /// Explicit schema names, excluding `$user` and temporary schemas.
    pub(crate) schemas: Vec<String>,
    /// `$user` expands to the current role, which SET ROLE can change
    /// without a ParameterStatus report.
    pub(crate) includes_user: bool,
}

pub(crate) fn split_search_path(value: &str) -> Option<SearchPathSchemas> {
    let mut parsed = SearchPathSchemas::default();
    let bytes = value.as_bytes();
    let mut index = 0;
    let skip_space = |index: &mut usize| {
        while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
            *index += 1;
        }
    };
    skip_space(&mut index);
    if index == bytes.len() {
        return Some(parsed);
    }
    loop {
        let name = if bytes.get(index) == Some(&b'"') {
            let mut name = Vec::new();
            index += 1;
            loop {
                match bytes.get(index) {
                    None => return None,
                    Some(b'"') if bytes.get(index + 1) == Some(&b'"') => {
                        name.push(b'"');
                        index += 2;
                    }
                    Some(b'"') => {
                        index += 1;
                        break;
                    }
                    Some(byte) => {
                        name.push(*byte);
                        index += 1;
                    }
                }
            }
            String::from_utf8(name).ok()?
        } else {
            let start = index;
            while index < bytes.len() && bytes[index] != b',' && !bytes[index].is_ascii_whitespace()
            {
                index += 1;
            }
            let raw = &value[start..index];
            if raw.is_empty() || !raw.is_ascii() || raw.contains('"') {
                return None;
            }
            raw.to_ascii_lowercase()
        };
        if name.is_empty() || name.len() > MAX_IDENTIFIER_BYTES {
            return None;
        }
        if name == "$user" {
            parsed.includes_user = true;
        } else if name != "pg_temp" && !name.starts_with("pg_temp_") {
            parsed.schemas.push(name);
        }
        skip_space(&mut index);
        match bytes.get(index) {
            None => return Some(parsed),
            Some(b',') => {
                index += 1;
                skip_space(&mut index);
            }
            Some(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_postgres_list_gucs() {
        let parsed = split_search_path("\"$user\", public").unwrap();
        assert!(parsed.includes_user);
        assert_eq!(parsed.schemas, ["public"]);
        let parsed = split_search_path(" App ,\"Mixed\"\"Case\",\"a,b\", pg_temp").unwrap();
        assert!(!parsed.includes_user);
        assert_eq!(parsed.schemas, ["app", "Mixed\"Case", "a,b"]);
        assert_eq!(split_search_path("$user").unwrap().schemas.len(), 0);
        assert_eq!(split_search_path("").unwrap(), SearchPathSchemas::default());
        assert_eq!(split_search_path("\"東京\"").unwrap().schemas, ["東京"]);
    }

    #[test]
    fn refuses_what_it_cannot_reproduce() {
        for value in [
            "public,",
            ",public",
            "public,,app",
            "\"unterminated",
            "\"\"",
            "東京",
            "a b",
            "a\"b",
            &"x".repeat(64),
        ] {
            assert_eq!(split_search_path(value), None, "{value:?}");
        }
        assert!(split_search_path(&"x".repeat(63)).is_some());
    }
}
