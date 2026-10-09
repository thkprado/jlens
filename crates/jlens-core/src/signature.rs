//! Function signatures such as `<s-nn?:s>`. Ported from jsonata-js `src/signature.js`
//! (© IBM Corp., MIT).
//!
//! A signature lists one symbol per parameter:
//!
//! | Symbol | Type |
//! |---|---|
//! | `s` `n` `b` `l` `o` | string, number, boolean, null, object |
//! | `a` | array (a non-array argument is wrapped in one) |
//! | `f` | function |
//! | `j` | any JSON value |
//! | `x` | any value |
//! | `(sn)` | a choice of types |
//!
//! Modifiers follow a symbol: `?` (optional), `+` (one or more), `-` (use the context value
//! when the argument is missing). `a<n>` and `f<n:n>` add a type parameter. The return type
//! after `:` is accepted but not checked, as in the reference implementation.

use serde::Serialize;

use crate::error::{Error, Result};

/// A parsed function signature.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Signature {
    /// The signature text, including the angle brackets.
    pub definition: String,
    /// The parameters, in order.
    pub params: Vec<Param>,
}

/// One parameter of a [`Signature`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Param {
    /// The type symbol (`s`, `n`, `a`, ...) or a choice such as `(sn)`.
    pub ty: String,
    /// The argument-type symbols this parameter accepts, as a regular-expression fragment over
    /// type symbols (for example `[sm]?`). `m` stands for a missing (undefined) argument.
    pub pattern: String,
    /// When the argument is missing, the context value is used instead (`-` modifier).
    pub context: bool,
    /// The pattern the context value must match when it is substituted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_pattern: Option<String>,
    /// The parameter expects an array; a single value is wrapped in an array.
    pub array: bool,
    /// Type parameter of `a<...>` or `f<...>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
}

impl Signature {
    /// Parses a signature. On error, the error's `offset` detail is the character offset in
    /// `definition` where the problem was found.
    pub fn parse(definition: &str) -> Result<Self> {
        let chars: Vec<char> = definition.chars().collect();
        let mut params: Vec<Param> = Vec::new();
        let mut position = 1;
        while position < chars.len() {
            let symbol = chars[position];
            if symbol == ':' {
                break;
            }
            match symbol {
                's' | 'n' | 'b' | 'l' | 'o' => {
                    params.push(Param::new(symbol, format!("[{symbol}m]")))
                }
                'a' => {
                    let mut p = Param::new('a', "[asnblfom]".into());
                    p.array = true;
                    params.push(p);
                }
                'f' => params.push(Param::new('f', "f".into())),
                'j' => params.push(Param::new('j', "[asnblom]".into())),
                'x' => params.push(Param::new('x', "[asnblfom]".into())),
                '-' => {
                    if let Some(prev) = params.last_mut() {
                        prev.context = true;
                        prev.context_pattern = Some(prev.pattern.clone());
                        prev.pattern.push('?');
                    }
                }
                '?' | '+' => {
                    if let Some(prev) = params.last_mut() {
                        prev.pattern.push(symbol);
                    }
                }
                '(' => {
                    let end = closing(&chars, position, '(', ')');
                    let choice: String = chars[position + 1..end.min(chars.len())].iter().collect();
                    if choice.contains('<') {
                        return Err(Error::new("S0402")
                            .with("value", choice)
                            .with("offset", position));
                    }
                    params.push(Param {
                        ty: format!("({choice})"),
                        pattern: format!("[{choice}m]"),
                        ..Param::new('x', String::new())
                    });
                    position = end;
                }
                '<' => {
                    let prev_ty = params.last().map(|p| p.ty.clone()).unwrap_or_default();
                    if prev_ty == "a" || prev_ty == "f" {
                        let end = closing(&chars, position, '<', '>');
                        let subtype: String =
                            chars[position + 1..end.min(chars.len())].iter().collect();
                        if let Some(prev) = params.last_mut() {
                            prev.subtype = Some(subtype);
                        }
                        position = end;
                    } else {
                        let value = if prev_ty.is_empty() {
                            serde_json::Value::Null
                        } else {
                            prev_ty.into()
                        };
                        return Err(Error::new("S0401")
                            .with("value", value)
                            .with("offset", position));
                    }
                }
                _ => {}
            }
            position += 1;
        }
        Ok(Signature {
            definition: definition.to_owned(),
            params,
        })
    }
}

impl Param {
    fn new(symbol: char, pattern: String) -> Self {
        Param {
            ty: symbol.to_string(),
            pattern,
            context: false,
            context_pattern: None,
            array: false,
            subtype: None,
        }
    }
}

/// Position of the bracket that closes the one at `start`, or `chars.len()` if unbalanced.
fn closing(chars: &[char], start: usize, open: char, close: char) -> usize {
    let mut depth = 1;
    let mut position = start;
    while position < chars.len() {
        position += 1;
        match chars.get(position) {
            Some(&c) if c == close => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Some(&c) if c == open => depth += 1,
            _ => {}
        }
    }
    position
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_subtypes() {
        let sig = Signature::parse("<s-nn?a<n>(sb)f<n:n>:s>").unwrap();
        let summary: Vec<_> = sig
            .params
            .iter()
            .map(|p| {
                (
                    p.ty.as_str(),
                    p.pattern.as_str(),
                    p.context,
                    p.subtype.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("s", "[sm]?", true, None),
                ("n", "[nm]", false, None),
                ("n", "[nm]?", false, None),
                ("a", "[asnblfom]", false, Some("n")),
                ("(sb)", "[sbm]", false, None),
                ("f", "f", false, Some("n:n")),
            ]
        );
    }

    #[test]
    fn rejects_invalid_type_parameters() {
        assert_eq!(Signature::parse("<n<s>>").unwrap_err().code(), "S0401");
        assert_eq!(Signature::parse("<(sa<n>)>").unwrap_err().code(), "S0402");
    }
}
