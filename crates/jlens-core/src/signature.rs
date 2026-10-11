//! Function signatures such as `<s-nn?:s>`. Ported from jsonata-js `src/signature.js` (MIT).
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
use crate::value::Value;

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

/// One parameter pattern as a set of accepted symbols and a repetition range.
struct Piece<'a> {
    set: &'a str,
    min: usize,
    max: usize,
}

fn piece(pattern: &str) -> Piece<'_> {
    let (set, quantifier) = match pattern.strip_prefix('[') {
        Some(rest) => {
            let close = rest.find(']').unwrap_or(rest.len());
            (&rest[..close], &rest[(close + 1).min(rest.len())..])
        }
        None => pattern.split_at(pattern.len().min(1)),
    };
    let (min, max) = if quantifier.contains('+') {
        (1, usize::MAX)
    } else if quantifier.contains('?') {
        (0, 1)
    } else {
        (1, 1)
    };
    Piece { set, min, max }
}

/// Greedy backtracking match of `pieces` against `symbols` from `pos`, recording the range
/// each piece consumed. Equivalent to the regular expression jsonata-js builds.
fn match_pieces(
    pieces: &[Piece],
    symbols: &[char],
    pos: usize,
    spans: &mut [(usize, usize)],
    anchored: bool,
) -> bool {
    let Some((p, rest)) = pieces.split_first() else {
        return !anchored || pos == symbols.len();
    };
    let mut n = 0;
    while n < p.max && pos + n < symbols.len() && p.set.contains(symbols[pos + n]) {
        n += 1;
    }
    loop {
        if n < p.min {
            return false;
        }
        spans[0] = (pos, pos + n);
        if match_pieces(rest, symbols, pos + n, &mut spans[1..], anchored) {
            return true;
        }
        if n == 0 {
            return false;
        }
        n -= 1;
    }
}

fn array_type_name(subtype: &str) -> &'static str {
    match subtype {
        "a" => "arrays",
        "b" => "booleans",
        "f" => "functions",
        "n" => "numbers",
        "o" => "objects",
        "s" => "strings",
        _ => "values",
    }
}

impl Signature {
    /// Checks `args` against the signature and returns them adjusted: missing context
    /// arguments are filled in and single values are wrapped where an array is expected.
    pub(crate) fn validate<'a>(
        &self,
        args: Vec<Value<'a>>,
        context: &Value<'a>,
    ) -> Result<Vec<Value<'a>>> {
        let symbols: Vec<char> = args.iter().map(Value::signature_symbol).collect();
        let pieces: Vec<Piece> = self.params.iter().map(|p| piece(&p.pattern)).collect();
        let mut spans = vec![(0, 0); pieces.len()];
        if !match_pieces(&pieces, &symbols, 0, &mut spans, true) {
            // Find the first parameter that does not match.
            let mut good_to = 0;
            for index in 0..pieces.len() {
                let mut partial = vec![(0, 0); index + 1];
                if !match_pieces(&pieces[..=index], &symbols, 0, &mut partial, false) {
                    break;
                }
                good_to = partial[index].1;
            }
            let mut err = Error::new("T0410").with("index", good_to + 1);
            if let Some(v) = args.get(good_to).and_then(Value::to_json_detail) {
                err = err.with("value", v);
            }
            return Err(err);
        }

        let mut validated = Vec::with_capacity(args.len());
        let mut arg_index = 0;
        for (param, &(start, end)) in self.params.iter().zip(&spans) {
            let matched = &symbols[start..end];
            if matched.is_empty() {
                if let (true, Some(pattern)) = (param.context, &param.context_pattern) {
                    if piece(pattern).set.contains(context.signature_symbol()) {
                        validated.push(context.clone());
                    } else {
                        let mut err = Error::new("T0411").with("index", arg_index + 1);
                        if let Some(v) = context.to_json_detail() {
                            err = err.with("value", v);
                        }
                        return Err(err);
                    }
                } else {
                    validated.push(args.get(arg_index).cloned().unwrap_or_default());
                    arg_index += 1;
                }
                continue;
            }
            let matched_str: String = matched.iter().collect();
            for &single in matched {
                let mut arg = args.get(arg_index).cloned().unwrap_or_default();
                if param.ty == "a" {
                    if single == 'm' {
                        arg = Value::Undefined;
                    } else {
                        let mut array_ok = true;
                        if let Some(subtype) = &param.subtype {
                            if single != 'a' && matched_str != *subtype {
                                array_ok = false;
                            } else if let (Value::Array(a), Some(want)) =
                                (&arg, subtype.chars().next())
                                && let Some(first) = a.items().first()
                            {
                                let item_type = first.signature_symbol();
                                array_ok = item_type == want
                                    && a.items().iter().all(|v| v.signature_symbol() == item_type);
                            }
                        }
                        if !array_ok {
                            let mut err = Error::new("T0412").with("index", arg_index + 1).with(
                                "type",
                                array_type_name(param.subtype.as_deref().unwrap_or("")),
                            );
                            if let Some(v) = arg.to_json_detail() {
                                err = err.with("value", v);
                            }
                            return Err(err);
                        }
                        if single != 'a' {
                            arg = Value::array(vec![arg], 0);
                        }
                    }
                }
                validated.push(arg);
                arg_index += 1;
            }
        }
        Ok(validated)
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
