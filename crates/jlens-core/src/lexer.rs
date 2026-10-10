//! The tokenizer. Ported from the `tokenizer` function in jsonata-js `src/parser.js` (MIT).
//!
//! Tokens are produced on demand because `/` is ambiguous: in operand position it starts a
//! regular expression, in operator position it is division. The parser tells the lexer which
//! position it is in (`infix`). Differences from the reference implementation:
//!
//! * every token carries its byte [`Span`] (the reference only records its end);
//! * doc-comments (`/** ... */`) are kept and attached to the following token;
//! * `\uXXXX` escapes forming a UTF-16 surrogate pair are combined into one character, and a
//!   lone surrogate becomes U+FFFD, since Rust strings cannot hold unpaired surrogates.

use crate::annotation::DocComment;
use crate::ast::Span;
use crate::error::{Error, Result};

/// Literal values `true`, `false` and `null`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub(crate) enum Lit {
    Bool(bool),
    Null,
}

/// What a token is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokKind {
    Name(String),
    Variable(String),
    Str(String),
    Number(f64),
    Value(Lit),
    Regex { pattern: String, flags: String },
    Op(&'static str),
}

/// A token with its location and any doc-comments that precede it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub kind: TokKind,
    pub span: Span,
    pub docs: Vec<DocComment>,
}

/// Operators known to the tokenizer. `!` and `~` are not operators on their own, but they
/// terminate names (the parser reports them as unknown operators).
const SINGLE_CHAR_OPS: &[&str] = &[
    ".", "[", "]", "{", "}", "(", ")", ",", "@", "#", ";", ":", "?", "+", "-", "*", "/", "%", "|",
    "=", "<", ">", "^", "&", "!", "~",
];

const DOUBLE_CHAR_OPS: &[&str] = &["..", ":=", "!=", ">=", "<=", "**", "~>", "?:", "??"];

fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b)
}

fn is_operator_char(b: u8) -> bool {
    SINGLE_CHAR_OPS.iter().any(|op| op.as_bytes()[0] == b)
}

pub(crate) struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    docs: Vec<DocComment>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            pos: 0,
            docs: Vec::new(),
        }
    }

    /// Skips the rest of the input.
    pub fn finish(&mut self) {
        self.pos = self.src.len();
        self.docs.clear();
    }

    /// Doc-comments seen after the last token (they have nothing to attach to).
    pub fn take_trailing_docs(&mut self) -> Vec<DocComment> {
        std::mem::take(&mut self.docs)
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.src.as_bytes().get(at).copied()
    }

    fn error(&self, code: &str, start: usize, pos: usize) -> Error {
        Error::new(code).located(self.src, Span::new(start, pos.max(start)), pos)
    }

    fn token(&mut self, kind: TokKind, start: usize) -> Option<Token> {
        Some(Token {
            kind,
            span: Span::new(start, self.pos),
            docs: std::mem::take(&mut self.docs),
        })
    }

    /// Returns the next token, or `None` at the end of the input.
    ///
    /// `infix` is true when the parser expects an operator, in which case `/` means division.
    pub fn next(&mut self, infix: bool) -> Result<Option<Token>> {
        let len = self.src.len();
        loop {
            while self.byte(self.pos).is_some_and(is_whitespace) {
                self.pos += 1;
            }
            if self.byte(self.pos) == Some(b'/') && self.byte(self.pos + 1) == Some(b'*') {
                let start = self.pos;
                let Some(close) = self.src[start + 2..].find("*/") else {
                    self.pos = len;
                    return Err(self.error("S0106", start, start));
                };
                let end = start + 2 + close + 2;
                let text = &self.src[start..end];
                if text.starts_with("/**") && text != "/**/" {
                    self.docs.push(DocComment {
                        span: Span::new(start, end),
                        body_start: start + 3,
                        body: self.src[start + 3..end - 2].to_owned(),
                    });
                }
                self.pos = end;
                continue;
            }
            break;
        }
        if self.pos >= len {
            return Ok(None);
        }
        let start = self.pos;
        let c = self.src.as_bytes()[start];

        if !infix && c == b'/' {
            self.pos += 1;
            let (pattern, flags) = self.scan_regex(start)?;
            return Ok(self.token(TokKind::Regex { pattern, flags }, start));
        }

        let rest = &self.src[start..];
        if let Some(op) = DOUBLE_CHAR_OPS.iter().find(|op| rest.starts_with(**op)) {
            self.pos += 2;
            return Ok(self.token(TokKind::Op(op), start));
        }
        if let Some(op) = SINGLE_CHAR_OPS.iter().find(|op| op.as_bytes()[0] == c) {
            self.pos += 1;
            return Ok(self.token(TokKind::Op(op), start));
        }

        if c == b'"' || c == b'\'' {
            let value = self.scan_string(c)?;
            return Ok(self.token(TokKind::Str(value), start));
        }

        if c.is_ascii_digit() {
            let end = scan_number(self.src.as_bytes(), start);
            let text = &self.src[start..end];
            let value: f64 = text.parse().unwrap_or(f64::INFINITY);
            if !value.is_finite() {
                // Consume the number, so that a recovering parser moves past it.
                self.pos = end;
                return Err(self.error("S0102", start, start).with("token", text));
            }
            self.pos = end;
            return Ok(self.token(TokKind::Number(value), start));
        }

        if c == b'`' {
            return match self.src[start + 1..].find('`') {
                Some(close) => {
                    let name = self.src[start + 1..start + 1 + close].to_owned();
                    self.pos = start + 1 + close + 1;
                    Ok(self.token(TokKind::Name(name), start))
                }
                None => {
                    self.pos = len;
                    Err(self.error("S0105", start, len))
                }
            };
        }

        let mut end = start;
        while let Some(b) = self.byte(end) {
            if is_whitespace(b) || is_operator_char(b) {
                break;
            }
            end += 1;
        }
        self.pos = end;
        let word = &self.src[start..end];
        let kind = if let Some(var) = word.strip_prefix('$') {
            TokKind::Variable(var.to_owned())
        } else {
            match word {
                "or" => TokKind::Op("or"),
                "in" => TokKind::Op("in"),
                "and" => TokKind::Op("and"),
                "true" => TokKind::Value(Lit::Bool(true)),
                "false" => TokKind::Value(Lit::Bool(false)),
                "null" => TokKind::Value(Lit::Null),
                _ => TokKind::Name(word.to_owned()),
            }
        };
        Ok(self.token(kind, start))
    }

    /// Scans a regex after its opening `/`, which has already been consumed.
    fn scan_regex(&mut self, token_start: usize) -> Result<(String, String)> {
        let bytes = self.src.as_bytes();
        let start = self.pos;
        let mut depth: i32 = 0;
        while self.pos < bytes.len() {
            let c = bytes[self.pos];
            if c == b'/' && depth == 0 {
                let mut backslashes = 0;
                while self.pos > backslashes && bytes[self.pos - backslashes - 1] == b'\\' {
                    backslashes += 1;
                }
                if backslashes % 2 == 0 {
                    let pattern = &self.src[start..self.pos];
                    if pattern.is_empty() {
                        return Err(self.error("S0301", token_start, self.pos));
                    }
                    self.pos += 1;
                    let flags_start = self.pos;
                    while matches!(self.byte(self.pos), Some(b'i' | b'm')) {
                        self.pos += 1;
                    }
                    return Ok((
                        pattern.to_owned(),
                        self.src[flags_start..self.pos].to_owned(),
                    ));
                }
            }
            let escaped = self.pos > 0 && bytes[self.pos - 1] == b'\\';
            if matches!(c, b'(' | b'[' | b'{') && !escaped {
                depth += 1;
            }
            if matches!(c, b')' | b']' | b'}') && !escaped {
                depth -= 1;
            }
            self.pos += 1;
        }
        Err(self.error("S0302", token_start, self.pos))
    }

    /// Scans a string literal; the opening quote is at `self.pos`.
    fn scan_string(&mut self, quote: u8) -> Result<String> {
        let token_start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        let src = self.src;
        while self.pos < src.len() {
            let c = src[self.pos..].chars().next().expect("in bounds");
            if c == '\\' {
                self.pos += 1;
                let Some(esc) = src[self.pos..].chars().next() else {
                    break;
                };
                match esc {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let Some(unit) = hex4(&src[self.pos + 1..]) else {
                            return Err(self.error("S0104", token_start, self.pos));
                        };
                        self.pos += 4;
                        let ch = if (0xD800..0xDC00).contains(&unit) {
                            // High surrogate: combine with a following \uDC00-\uDFFF escape.
                            let next = &src[(self.pos + 1).min(src.len())..];
                            match next.strip_prefix("\\u").and_then(hex4) {
                                Some(low) if (0xDC00..0xE000).contains(&low) => {
                                    self.pos += 6;
                                    let cp = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                                    char::from_u32(cp).unwrap_or('\u{FFFD}')
                                }
                                _ => '\u{FFFD}',
                            }
                        } else {
                            char::from_u32(unit).unwrap_or('\u{FFFD}')
                        };
                        out.push(ch);
                    }
                    other => {
                        return Err(self
                            .error("S0103", token_start, self.pos)
                            .with("token", other.to_string()));
                    }
                }
                self.pos += 1;
            } else if c as u32 == quote as u32 {
                self.pos += 1;
                return Ok(out);
            } else {
                out.push(c);
                self.pos += c.len_utf8();
            }
        }
        self.pos = src.len();
        Err(self.error("S0101", token_start, src.len()))
    }
}

/// Parses the (up to) four hex digits after `\u`. Like jsonata-js, fewer than four digits are
/// accepted at the very end of the input.
fn hex4(s: &str) -> Option<u32> {
    let digits: String = s.chars().take(4).collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(&digits, 16).ok()
}

/// Matches `(0|[1-9][0-9]*)(\.[0-9]+)?([Ee][-+]?[0-9]+)?` starting at `start` (which is a digit)
/// and returns the end offset.
fn scan_number(bytes: &[u8], start: usize) -> usize {
    let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
    let mut i = start;
    if bytes[i] == b'0' {
        i += 1;
    } else {
        while digit(i) {
            i += 1;
        }
    }
    if bytes.get(i) == Some(&b'.') && digit(i + 1) {
        i += 1;
        while digit(i) {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if digit(j) {
            while digit(j) {
                j += 1;
            }
            i = j;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokKind> {
        let mut lexer = Lexer::new(src);
        let mut out = Vec::new();
        let mut infix = false;
        while let Some(tok) = lexer.next(infix).unwrap() {
            infix = !matches!(tok.kind, TokKind::Op(op) if op != ")" && op != "]" && op != "}");
            out.push(tok.kind);
        }
        out
    }

    #[test]
    fn operators_names_and_literals() {
        assert_eq!(
            kinds("Account.Order[0].`Product Name` != $x and true"),
            vec![
                TokKind::Name("Account".into()),
                TokKind::Op("."),
                TokKind::Name("Order".into()),
                TokKind::Op("["),
                TokKind::Number(0.0),
                TokKind::Op("]"),
                TokKind::Op("."),
                TokKind::Name("Product Name".into()),
                TokKind::Op("!="),
                TokKind::Variable("x".into()),
                TokKind::Op("and"),
                TokKind::Value(Lit::Bool(true)),
            ]
        );
    }

    #[test]
    fn numbers_and_ranges() {
        assert_eq!(
            kinds("[1..10, 2.5e3, 0.5]"),
            vec![
                TokKind::Op("["),
                TokKind::Number(1.0),
                TokKind::Op(".."),
                TokKind::Number(10.0),
                TokKind::Op(","),
                TokKind::Number(2500.0),
                TokKind::Op(","),
                TokKind::Number(0.5),
                TokKind::Op("]"),
            ]
        );
    }

    #[test]
    fn regex_versus_division() {
        assert_eq!(
            kinds("a / /b[/]c/i"),
            vec![
                TokKind::Name("a".into()),
                TokKind::Op("/"),
                TokKind::Regex {
                    pattern: "b[/]c".into(),
                    flags: "i".into()
                },
            ]
        );
    }

    #[test]
    fn string_escapes_and_surrogates() {
        assert_eq!(
            kinds(r#""a\"bé𝄞\uD800""#),
            vec![TokKind::Str("a\"bé𝄞\u{FFFD}".into())]
        );
    }

    #[test]
    fn doc_comments_attach_to_next_token() {
        let mut lexer = Lexer::new("/* plain */ /** @label \"x\" */ a /**/ b");
        let a = lexer.next(false).unwrap().unwrap();
        assert_eq!(a.docs.len(), 1);
        assert_eq!(a.docs[0].body, " @label \"x\" ");
        let b = lexer.next(true).unwrap().unwrap();
        assert!(b.docs.is_empty());
    }

    #[test]
    fn lexer_errors() {
        let code = |src: &str| {
            let mut lexer = Lexer::new(src);
            loop {
                match lexer.next(false) {
                    Ok(Some(_)) => {}
                    Ok(None) => return String::new(),
                    Err(e) => return e.code().to_owned(),
                }
            }
        };
        assert_eq!(code("'abc"), "S0101");
        assert_eq!(code("1e999"), "S0102");
        assert_eq!(code(r#""\q""#), "S0103");
        assert_eq!(code(r#""\u12x4""#), "S0104");
        assert_eq!(code("`abc"), "S0105");
        assert_eq!(code("/* abc"), "S0106");
        assert_eq!(code("//"), "S0301");
        assert_eq!(code("/abc"), "S0302");
    }
}
