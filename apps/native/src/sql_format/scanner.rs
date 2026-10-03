use super::{FormatError, MAX_DEPTH, MAX_INPUT_BYTES, MAX_TOKENS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Word,
    String,
    QuotedIdentifier,
    Number,
    Parameter,
    Operator,
    Dot,
    Colon,
    Comma,
    Semicolon,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    CaseOpen,
    CaseClose,
    LineComment,
    BlockComment,
}
impl Kind {
    pub(super) fn is_comment(self) -> bool {
        matches!(self, Self::LineComment | Self::BlockComment)
    }
}
#[derive(Debug)]
pub(super) struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub depth: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Parenthesis,
    Bracket,
    Case,
}

pub(super) fn scan(sql: &str) -> Result<Vec<Token>, FormatError> {
    if sql.len() > MAX_INPUT_BYTES {
        return Err(FormatError::InputLimit);
    }
    if sql.contains('\0') {
        return Err(FormatError::InvalidToken);
    }
    let bytes = sql.as_bytes();
    let mut tokens: Vec<Token> = Vec::with_capacity(sql.len().min(MAX_TOKENS));
    let mut stack = [Frame::Parenthesis; MAX_DEPTH];
    let mut depth = 0;
    let mut i = 0;
    while i < bytes.len() {
        if ascii_space(bytes[i]) {
            i += 1;
            continue;
        }
        if !bytes[i].is_ascii() && sql[i..].chars().next().is_some_and(char::is_whitespace) {
            return Err(FormatError::UnsupportedWhitespace);
        }
        if tokens.len() == MAX_TOKENS {
            return Err(FormatError::TokenLimit);
        }
        let start = i;
        let kind = if bytes[i..].starts_with(b"--") {
            i += 2;
            while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                i += 1;
            }
            Kind::LineComment
        } else if bytes[i..].starts_with(b"/*") {
            i = block_comment(bytes, i)?;
            Kind::BlockComment
        } else if matches!(
            bytes[i],
            b'e' | b'E' | b'n' | b'N' | b'b' | b'B' | b'x' | b'X'
        ) && bytes.get(i + 1) == Some(&b'\'')
        {
            let prefix = bytes[i].to_ascii_lowercase();
            i = if prefix == b'e' {
                quoted(bytes, i + 1, b'\'', true)?
            } else if prefix == b'n' {
                ordinary_string(bytes, i + 1)?
            } else {
                quoted(bytes, i + 1, b'\'', false)?
            };
            Kind::String
        } else if matches!(bytes[i], b'u' | b'U')
            && bytes.get(i + 1) == Some(&b'&')
            && bytes.get(i + 2).is_some_and(|b| matches!(b, b'\'' | b'"'))
        {
            let quote = bytes[i + 2];
            i = quoted(bytes, i + 2, quote, false)?;
            if quote == b'\'' {
                Kind::String
            } else {
                Kind::QuotedIdentifier
            }
        } else if bytes[i] == b'\'' {
            i = ordinary_string(bytes, i)?;
            Kind::String
        } else if bytes[i] == b'"' {
            i = quoted(bytes, i, b'"', false)?;
            Kind::QuotedIdentifier
        } else if bytes[i] == b'$' {
            let (end, kind) = dollar(sql, i)?;
            i = end;
            kind
        } else if identifier_start(bytes[i]) {
            i = identifier_end(sql, i)?;
            let qualified = tokens
                .iter()
                .rev()
                .find(|t| !t.kind.is_comment())
                .is_some_and(|t| t.kind == Kind::Dot);
            let word = &sql[start..i];
            if !qualified && word.eq_ignore_ascii_case("case") {
                Kind::CaseOpen
            } else if !qualified
                && word.eq_ignore_ascii_case("end")
                && depth > 0
                && stack[depth - 1] == Frame::Case
            {
                Kind::CaseClose
            } else {
                Kind::Word
            }
        } else if bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i = number(bytes, i)?;
            Kind::Number
        } else if bytes[i] == b':' {
            i += 1;
            if bytes.get(i).is_some_and(|b| matches!(b, b':' | b'=')) {
                i += 1;
                Kind::Operator
            } else if bytes.get(i).is_some_and(|b| identifier_start(*b)) {
                i = identifier_end(sql, i)?;
                Kind::Parameter
            } else {
                Kind::Colon
            }
        } else if operator(bytes[i]) {
            i += 1;
            while i < bytes.len() && operator(bytes[i]) {
                if bytes[i..].starts_with(b"--") || bytes[i..].starts_with(b"/*") {
                    break;
                }
                i += 1;
            }
            // PostgreSQL strips trailing +/- from multi-character operators
            // unless the run contains a permitted nonstandard operator character.
            if !bytes[start..i].iter().any(|b| b"~!@#%^&|`?".contains(b)) {
                while i > start + 1 && matches!(bytes[i - 1], b'+' | b'-') {
                    i -= 1;
                }
            }
            Kind::Operator
        } else {
            i += 1;
            match bytes[start] {
                b'(' => Kind::OpenParen,
                b')' => Kind::CloseParen,
                b'[' => Kind::OpenBracket,
                b']' => Kind::CloseBracket,
                b',' => Kind::Comma,
                b';' => Kind::Semicolon,
                b'.' => Kind::Dot,
                _ => return Err(FormatError::InvalidToken),
            }
        };
        let closing = match kind {
            Kind::CloseParen => Some(Frame::Parenthesis),
            Kind::CloseBracket => Some(Frame::Bracket),
            Kind::CaseClose => Some(Frame::Case),
            _ => None,
        };
        if let Some(frame) = closing {
            if depth == 0 || stack[depth - 1] != frame {
                return Err(FormatError::Unbalanced);
            }
            depth -= 1;
        }
        tokens.push(Token {
            start,
            end: i,
            kind,
            depth,
        });
        let opening = match kind {
            Kind::OpenParen => Some(Frame::Parenthesis),
            Kind::OpenBracket => Some(Frame::Bracket),
            Kind::CaseOpen => Some(Frame::Case),
            _ => None,
        };
        if let Some(frame) = opening {
            if depth == MAX_DEPTH {
                return Err(FormatError::DepthLimit);
            }
            stack[depth] = frame;
            depth += 1;
        }
    }
    if depth != 0 {
        return Err(FormatError::Unbalanced);
    }
    Ok(tokens)
}
fn ascii_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}
fn identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || !byte.is_ascii()
}
fn identifier_end(sql: &str, mut i: usize) -> Result<usize, FormatError> {
    let bytes = sql.as_bytes();
    while i < bytes.len() {
        if bytes[i].is_ascii() {
            if !(bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'$')) {
                break;
            }
            i += 1;
        } else {
            let c = sql[i..].chars().next().ok_or(FormatError::InvalidToken)?;
            if c.is_whitespace() {
                return Err(FormatError::UnsupportedWhitespace);
            }
            i += c.len_utf8();
        }
    }
    Ok(i)
}
fn quoted(bytes: &[u8], start: usize, quote: u8, escapes: bool) -> Result<usize, FormatError> {
    let mut i = start + 1;
    while i < bytes.len() {
        if escapes && bytes[i] == b'\\' {
            i += 2;
        } else if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
            } else {
                return Ok(i + 1);
            }
        } else {
            i += 1;
        }
    }
    Err(FormatError::Incomplete)
}
fn ordinary_string(bytes: &[u8], start: usize) -> Result<usize, FormatError> {
    // This also conservatively refuses ambiguous backslash-bearing continuation
    // segments after E strings instead of assuming a session/continuation mode.
    match (
        quoted(bytes, start, b'\'', false),
        quoted(bytes, start, b'\'', true),
    ) {
        (Ok(normal), Ok(escaped)) if normal == escaped => Ok(normal),
        (Err(_), Err(_)) => Err(FormatError::Incomplete),
        _ => Err(FormatError::AmbiguousString),
    }
}
fn block_comment(bytes: &[u8], start: usize) -> Result<usize, FormatError> {
    let mut depth = 1;
    let mut i = start + 2;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            depth += 1;
            if depth > MAX_DEPTH {
                return Err(FormatError::DepthLimit);
            }
            i += 2;
        } else if bytes[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Ok(i);
            }
        } else {
            i += 1;
        }
    }
    Err(FormatError::Incomplete)
}
fn dollar(sql: &str, start: usize) -> Result<(usize, Kind), FormatError> {
    let bytes = sql.as_bytes();
    let mut i = start + 1;
    if bytes.get(i).is_some_and(u8::is_ascii_digit) {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if bytes
            .get(i)
            .is_some_and(|b| identifier_start(*b) || *b == b'$')
        {
            return Err(FormatError::InvalidToken);
        }
        return Ok((i, Kind::Parameter));
    }
    while i < bytes.len() && bytes[i] != b'$' {
        if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' {
            i += 1;
        } else if !bytes[i].is_ascii() {
            let c = sql[i..].chars().next().ok_or(FormatError::InvalidToken)?;
            if c.is_whitespace() {
                return Err(FormatError::UnsupportedWhitespace);
            }
            i += c.len_utf8();
        } else {
            break;
        }
    }
    if bytes.get(i) == Some(&b'$') {
        let delimiter = &sql[start..=i];
        let body = i + 1;
        let end = sql[body..].find(delimiter).ok_or(FormatError::Incomplete)?;
        Ok((body + end + delimiter.len(), Kind::String))
    } else if i > start + 1 {
        Ok((i, Kind::Parameter))
    } else {
        Err(FormatError::InvalidToken)
    }
}
fn number(bytes: &[u8], start: usize) -> Result<usize, FormatError> {
    let mut i = start;
    if bytes.get(i) == Some(&b'0')
        && bytes
            .get(i + 1)
            .is_some_and(|b| matches!(b, b'x' | b'X' | b'o' | b'O' | b'b' | b'B'))
    {
        i += 2;
        let digits = i;
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_hexdigit() || *b == b'_')
        {
            i += 1;
        }
        if i == digits {
            return Err(FormatError::InvalidToken);
        }
    } else {
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
        {
            i += 1;
        }
        if bytes.get(i) == Some(&b'.') && bytes.get(i + 1) != Some(&b'.') {
            i += 1;
            while bytes
                .get(i)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
            {
                i += 1;
            }
        }
        if bytes.get(i).is_some_and(|b| matches!(b, b'e' | b'E')) {
            i += 1;
            if bytes.get(i).is_some_and(|b| matches!(b, b'+' | b'-')) {
                i += 1;
            }
            let digits = i;
            while bytes
                .get(i)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
            {
                i += 1;
            }
            if i == digits {
                return Err(FormatError::InvalidToken);
            }
        }
    }
    if bytes
        .get(i)
        .is_some_and(|b| identifier_start(*b) || *b == b'$')
    {
        return Err(FormatError::InvalidToken);
    }
    Ok(i)
}
fn operator(byte: u8) -> bool {
    b"+-*/<>=~!@#%^&|`?".contains(&byte)
}
