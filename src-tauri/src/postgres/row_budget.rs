//! Shared UTF-8 cell/row retention caps for dedicated PostgreSQL sockets.

pub(crate) const MAX_CELL_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_ROW_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) fn json_bytes<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|json| json.len())
        .unwrap_or(usize::MAX)
}

pub(crate) fn retained_row_bytes(cells: &[Option<String>], identity: Option<&[String]>) -> usize {
    json_bytes(cells).saturating_add(identity.map(json_bytes).unwrap_or(0))
}

pub(crate) fn truncate_utf8(value: &str, max: usize, reasons: &mut Vec<String>) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    reasons.push("cellBytes".into());
    value[..end].to_string()
}

pub(crate) fn shrink_row(values: &mut [Option<String>], reasons: &mut Vec<String>) {
    while json_bytes(values) > MAX_ROW_BYTES {
        let Some(value) = values
            .iter_mut()
            .rev()
            .find_map(Option::as_mut)
            .filter(|value| !value.is_empty())
        else {
            break;
        };
        let mut end = value.len() / 2;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        reasons.push("rowBytes".into());
    }
}

pub(crate) fn bound_text(value: &mut String, truncated_cells: &mut u64) {
    let mut reasons = Vec::new();
    let truncated = truncate_utf8(value, MAX_CELL_BYTES, &mut reasons);
    if truncated.len() != value.len() {
        *truncated_cells += 1;
    }
    *value = truncated;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_truncation_stays_on_boundary() {
        let mut reasons = Vec::new();
        assert_eq!(truncate_utf8("éé", 3, &mut reasons), "é");
        assert_eq!(reasons, ["cellBytes"]);
    }

    #[test]
    fn oversized_rows_shrink_later_cells_first() {
        let mut values = vec![Some("a".repeat(1024 * 1024)), Some("b".repeat(1024 * 1024))];
        let mut reasons = Vec::new();
        shrink_row(&mut values, &mut reasons);
        assert!(json_bytes(&values) <= MAX_ROW_BYTES);
    }

    #[test]
    fn retained_row_bytes_includes_identity() {
        let cells = vec![Some("pk".repeat(100))];
        let identity = vec!["pk".repeat(100)];
        assert!(retained_row_bytes(&cells, Some(&identity)) > json_bytes(&cells));
        assert_eq!(
            retained_row_bytes(&cells, Some(&identity)),
            json_bytes(&cells) + json_bytes(&identity)
        );
    }

    #[test]
    fn bound_text_caps_wide_primary_key_values() {
        let mut value = "x".repeat(MAX_CELL_BYTES + 50);
        let mut truncated = 0;
        bound_text(&mut value, &mut truncated);
        assert_eq!(value.len(), MAX_CELL_BYTES);
        assert_eq!(truncated, 1);
    }
}
