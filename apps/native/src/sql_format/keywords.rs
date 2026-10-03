//! Classify only scanner-approved words. Phrase precedence follows the pinned
//! Tauri formatter, while protected lexemes never enter keyword matching.
use super::{Token, scanner::Kind};

#[path = "keyword_vocabulary.rs"]
mod vocabulary;

pub(super) fn uppercase_tokens(sql: &str, tokens: &[Token]) -> Vec<bool> {
    let mut uppercase = vec![false; tokens.len()];
    let mut index = 0;
    while index < tokens.len() {
        let matched = vocabulary::GROUPS.iter().find_map(|(phrases, upper)| {
            phrases.iter().find_map(|phrase| {
                match_phrase(sql, tokens, index, phrase).map(|length| (length, *upper))
            })
        });
        if let Some((length, upper)) = matched {
            // Reserved words adjacent to property access are identifiers, even
            // across comments. Protect the entire matched phrase, as Tauri does.
            let qualified = tokens[..index]
                .iter()
                .rev()
                .find(|t| !t.kind.is_comment())
                .is_some_and(|t| t.kind == Kind::Dot)
                || tokens[index + length..]
                    .iter()
                    .find(|t| !t.kind.is_comment())
                    .is_some_and(|t| t.kind == Kind::Dot);
            if upper && !qualified {
                uppercase[index..index + length].fill(true);
            }
            index += length;
        } else {
            index += 1;
        }
    }
    uppercase
}

fn match_phrase(sql: &str, tokens: &[Token], start: usize, phrase: &str) -> Option<usize> {
    let mut length = 0;
    for word in phrase.split(' ') {
        let token = tokens.get(start + length)?;
        if !matches!(token.kind, Kind::Word | Kind::CaseOpen | Kind::CaseClose)
            || !sql[token.start..token.end].eq_ignore_ascii_case(word)
        {
            return None;
        }
        // A phrase cannot span a comment or punctuation. Scanner token gaps
        // contain only ASCII whitespace; require a gap between phrase words.
        if length > 0 && tokens[start + length - 1].end == token.start {
            return None;
        }
        length += 1;
    }
    Some(length)
}
