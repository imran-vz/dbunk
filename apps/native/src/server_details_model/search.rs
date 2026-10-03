//! Normalize the bounded needle once; scan captured fields without cloning or
//! lowercasing server values. KMP keeps each field scan linear in its size.

pub(super) struct Search {
    needle: Vec<char>,
    prefix: Vec<usize>,
}

impl Search {
    pub(super) fn new(query: &str) -> Self {
        let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        let mut prefix = vec![0; needle.len()];
        let mut matched = 0;
        for index in 1..needle.len() {
            while matched > 0 && needle[index] != needle[matched] {
                matched = prefix[matched - 1];
            }
            if needle[index] == needle[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self { needle, prefix }
    }

    pub(super) fn matches(&self, text: &str) -> bool {
        if self.needle.is_empty() {
            return true;
        }
        let mut matched = 0;
        for ch in text.chars().flat_map(char::to_lowercase) {
            while matched > 0 && ch != self.needle[matched] {
                matched = self.prefix[matched - 1];
            }
            if ch == self.needle[matched] {
                matched += 1;
                if matched == self.needle.len() {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_lowercase_and_overlapping_prefixes_match_without_field_copies() {
        assert!(Search::new("İSTANBUL").matches("prefix İstanbul suffix"));
        assert!(Search::new("aab").matches("aaaab"));
        assert!(Search::new("界").matches("名字界"));
        assert!(Search::new("").matches(""));
        assert!(!Search::new("abc").matches("ab"));
        assert!(!Search::new("straße").matches("STRASSE"));
    }
}
