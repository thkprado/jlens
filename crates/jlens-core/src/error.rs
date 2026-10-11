//! Errors, error codes and diagnostics.
//!
//! Error codes and message templates follow jsonata-js (`src/jsonata.js`, MIT) so
//! that callers can rely on the same stable codes as other JSONata implementations.

use std::fmt;

use serde::Serialize;
use serde_json::{Map, Value as JsonValue};

use crate::ast::Span;

/// Result type used throughout jlens-core.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error raised while compiling or evaluating a JSONata expression.
///
/// Every error carries a stable JSONata error code (for example `S0201` for a syntax error or
/// `T2001` for a type error), so callers can match on codes rather than on messages. Extra
/// details (such as `token` or `value`) fill in the code's message template.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub struct Error(Box<ErrorInner>);

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ErrorInner {
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span: Option<Span>,
    #[serde(flatten)]
    details: Map<String, JsonValue>,
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Repr<'a> {
            #[serde(flatten)]
            inner: &'a ErrorInner,
            message: String,
        }
        Repr {
            inner: &self.0,
            message: self.message(),
        }
        .serialize(serializer)
    }
}

impl Error {
    /// Creates an error with the given code and no details.
    pub fn new(code: impl Into<String>) -> Self {
        Error(Box::new(ErrorInner {
            code: code.into(),
            position: None,
            span: None,
            details: Map::new(),
        }))
    }

    /// Adds a detail used by the message template (for example `token` or `value`).
    pub fn with(mut self, key: &str, value: impl Into<JsonValue>) -> Self {
        self.0.details.insert(key.to_owned(), value.into());
        self
    }

    /// Sets the location of the error. `position` is the JSONata-compatible position (a UTF-16
    /// offset into the source); `span` is the byte range of the offending text.
    pub(crate) fn located(mut self, source: &str, span: Span, position_byte: usize) -> Self {
        self.0.position = Some(utf16_offset(source, position_byte));
        self.0.span = Some(span);
        self
    }

    /// Sets the location from a node: its byte span and UTF-16 position.
    pub(crate) fn at(mut self, span: Span, position: usize) -> Self {
        self.0.position = Some(position);
        self.0.span = Some(span);
        self
    }

    pub(crate) fn at_if_missing(self, span: Span, position: usize) -> Self {
        if self.0.position.is_none() {
            self.at(span, position)
        } else {
            self
        }
    }

    pub(crate) fn with_token_if_missing(self, token: &str) -> Self {
        if self.0.details.contains_key("token") {
            self
        } else {
            self.with("token", token)
        }
    }

    /// The JSONata error code, such as `S0201`.
    pub fn code(&self) -> &str {
        &self.0.code
    }

    /// The human-readable message, built from the code's template and the error details.
    pub fn message(&self) -> String {
        match template(&self.0.code) {
            Some(template) => populate(template, &self.0.details),
            None => self
                .0
                .details
                .get("message")
                .and_then(JsonValue::as_str)
                .unwrap_or("Unknown error")
                .to_owned(),
        }
    }

    /// Position in the expression source where the error was detected, if known.
    ///
    /// Like jsonata-js, this is an offset in UTF-16 code units, usually pointing just after the
    /// offending token. Use [`Error::span`] for byte ranges.
    pub fn position(&self) -> Option<usize> {
        self.0.position
    }

    /// Byte range of the offending source text, if known.
    pub fn span(&self) -> Option<Span> {
        self.0.span
    }

    /// The token involved in the error, if any.
    pub fn token(&self) -> Option<String> {
        self.0.details.get("token").map(|t| match t {
            JsonValue::String(s) => s.clone(),
            other => other.to_string(),
        })
    }

    /// The value involved in the error, if any.
    pub fn value(&self) -> Option<&JsonValue> {
        self.0.details.get("value")
    }

    /// Any other detail of the error, by name (for example `index` or `functionName`).
    pub fn detail(&self, key: &str) -> Option<&JsonValue> {
        self.0.details.get(key)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.0.code, self.message())?;
        if let Some(pos) = self.0.position {
            write!(f, " (at position {pos})")?;
        }
        Ok(())
    }
}

/// Converts a byte offset into a UTF-16 code unit offset.
pub(crate) fn utf16_offset(source: &str, byte: usize) -> usize {
    let byte = byte.min(source.len());
    let mut end = byte;
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    source[..end].encode_utf16().count()
}

/// Fills a message template: `{{{key}}}` inserts the raw detail, `{{key}}` its JSON form
/// (`undefined` when the detail is absent), exactly like jsonata-js.
fn populate(template: &str, details: &Map<String, JsonValue>) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let triple = rest[start..].starts_with("{{{");
        let (open, close) = if triple { (3, "}}}") } else { (2, "}}") };
        let Some(len) = rest[start + open..].find(close) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = &rest[start + open..start + open + len];
        match (details.get(key), triple) {
            (Some(JsonValue::String(s)), true) => out.push_str(s),
            (Some(v), _) => out.push_str(&v.to_string()),
            (None, _) => out.push_str("undefined"),
        }
        rest = &rest[start + open + len + close.len()..];
    }
    out.push_str(rest);
    out
}

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The expression cannot be evaluated.
    Error,
    /// The expression can be evaluated, but something looks wrong (for example a misplaced
    /// annotation).
    Warning,
}

/// A problem found while parsing, with its location in the source.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    /// How serious the problem is.
    pub severity: Severity,
    /// Stable code: a JSONata error code (`S0xxx`) or a jlens warning code (`W0xxx`).
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// Byte range of the offending source text.
    pub span: Span,
}

impl Diagnostic {
    /// A diagnostic with a catalog code; `details` fill the code's message template (string
    /// details are inserted as JSON strings by `{{name}}` and as raw text by `{{{name}}}`).
    pub fn new(severity: Severity, code: &str, span: Span, details: &[(&str, &str)]) -> Self {
        let mut d = Diagnostic::warning(code, span, details);
        d.severity = severity;
        d
    }

    pub(crate) fn warning(code: &str, span: Span, details: &[(&str, &str)]) -> Self {
        let mut map = Map::new();
        for (k, v) in details {
            map.insert((*k).to_owned(), JsonValue::String((*v).to_owned()));
        }
        Diagnostic {
            severity: Severity::Warning,
            code: code.to_owned(),
            message: populate(template(code).unwrap_or("{{{message}}}"), &map),
            span,
        }
    }
}

impl From<&Error> for Diagnostic {
    fn from(error: &Error) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code: error.code().to_owned(),
            message: error.message(),
            span: error.span().unwrap_or_default(),
        }
    }
}

/// Error category, derived from the first letter of the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Category {
    /// `S` — static (syntax) errors, raised while compiling.
    Static,
    /// `T` — type errors, raised while evaluating.
    Type,
    /// `D` — dynamic errors, raised while evaluating.
    Dynamic,
    /// `U` — errors raised by user-defined guardrails (limits, cancellation), and by the
    /// `jlens` command-line interface for input that is not valid JSON (`U2001`).
    Guardrail,
    /// `W` — jlens warnings; they never stop evaluation.
    Warning,
}

/// One entry of the error catalog.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CatalogEntry {
    /// The stable code, such as `S0201`.
    pub code: &'static str,
    /// The message template; `{{name}}` placeholders are filled from the error details.
    pub template: &'static str,
}

impl CatalogEntry {
    /// The category of this code.
    pub fn category(&self) -> Category {
        match self.code.as_bytes()[0] {
            b'S' => Category::Static,
            b'T' => Category::Type,
            b'D' => Category::Dynamic,
            b'U' => Category::Guardrail,
            _ => Category::Warning,
        }
    }
}

/// Every error and warning code jlens can report, with its message template.
pub fn catalog() -> &'static [CatalogEntry] {
    CATALOG
}

fn template(code: &str) -> Option<&'static str> {
    CATALOG.iter().find(|e| e.code == code).map(|e| e.template)
}

macro_rules! catalog {
    ($($code:literal => $template:literal,)*) => {
        static CATALOG: &[CatalogEntry] = &[$(CatalogEntry { code: $code, template: $template },)*];
    };
}

catalog! {
    "S0101" => "String literal must be terminated by a matching quote",
    "S0102" => "Number out of range: {{token}}",
    "S0103" => "Unsupported escape sequence: \\{{token}}",
    "S0104" => "The escape sequence \\u must be followed by 4 hex digits",
    "S0105" => "Quoted property name must be terminated with a backquote (`)",
    "S0106" => "Comment has no closing tag",
    "S0201" => "Syntax error: {{token}}",
    "S0202" => "Expected {{value}}, got {{token}}",
    "S0203" => "Expected {{value}} before end of expression",
    "S0204" => "Unknown operator: {{token}}",
    "S0205" => "Unexpected token: {{token}}",
    "S0206" => "Unknown expression type: {{token}}",
    "S0207" => "Unexpected end of expression",
    "S0208" => "Parameter {{value}} of function definition must be a variable name (start with $)",
    "S0209" => "A predicate cannot follow a grouping expression in a step",
    "S0210" => "Each step can only have one grouping expression",
    "S0211" => "The symbol {{token}} cannot be used as a unary operator",
    "S0212" => "The left side of := must be a variable name (start with $)",
    "S0213" => "The literal value {{value}} cannot be used as a step within a path expression",
    "S0214" => "The right side of {{token}} must be a variable name (start with $)",
    "S0215" => "A context variable binding must precede any predicates on a step",
    "S0216" => "A context variable binding must precede the 'order-by' clause on a step",
    "S0217" => "The object representing the 'parent' cannot be derived from this expression",
    "S0301" => "Empty regular expressions are not allowed",
    "S0302" => "No terminating / in regular expression",
    "S0402" => "Choice groups containing parameterized types are not supported",
    "S0401" => "Type parameters can only be applied to functions and arrays",
    "S0500" => "Attempted to evaluate an expression containing syntax error(s)",
    "T0410" => "Argument {{index}} of function {{token}} does not match function signature",
    "T0411" => "Context value is not a compatible type with argument {{index}} of function {{token}}",
    "T0412" => "Argument {{index}} of function {{token}} must be an array of {{type}}",
    "D1001" => "Number out of range: {{value}}",
    "D1002" => "Cannot negate a non-numeric value: {{value}}",
    "T1003" => "Key in object structure must evaluate to a string; got: {{value}}",
    "D1004" => "Regular expression matches zero length string",
    "T1005" => "Attempted to invoke a non-function. Did you mean ${{{token}}}?",
    "T1006" => "Attempted to invoke a non-function",
    "T1007" => "Attempted to partially apply a non-function. Did you mean ${{{token}}}?",
    "T1008" => "Attempted to partially apply a non-function",
    "D1009" => "Multiple key definitions evaluate to same key: {{value}}",
    "T1010" => "The matcher function argument passed to function {{token}} does not return the correct object structure",
    "D1011" => "Stack overflow. Check for non-terminating recursive function.  Consider rewriting as tail-recursive",
    "D1012" => "Evaluation timeout after {{value}} milliseconds. Check for infinite loop",
    "D1013" => "Object property names starting with _jsonata_ are reserved for internal use: {{value}}",
    "T2001" => "The left side of the {{token}} operator must evaluate to a number",
    "T2002" => "The right side of the {{token}} operator must evaluate to a number",
    "T2003" => "The left side of the range operator (..) must evaluate to an integer",
    "T2004" => "The right side of the range operator (..) must evaluate to an integer",
    "D2005" => "The left side of := must be a variable name (start with $)",
    "T2006" => "The right side of the function application operator ~> must be a function",
    "T2007" => "Type mismatch when comparing values {{value}} and {{value2}} in order-by clause",
    "T2008" => "The expressions within an order-by clause must evaluate to numeric or string values",
    "T2009" => "The values {{value}} and {{value2}} either side of operator {{token}} must be of the same data type",
    "T2010" => "The expressions either side of operator {{token}} must evaluate to numeric or string values",
    "T2011" => "The insert/update clause of the transform expression must evaluate to an object: {{value}}",
    "T2012" => "The delete clause of the transform expression must evaluate to a string or array of strings: {{value}}",
    "T2013" => "The transform expression clones the input object using the $clone() function.  This has been overridden in the current scope by a non-function.",
    "D2014" => "The size of the sequence allocated by the range operator (..) must not exceed 1e7.  Attempted to allocate {{value}}.",
    "D2015" => "The maximum sequence length of {{value}} was exceeded.",
    "D2016" => "The size of the string requested by the $pad function must not exceed 1e7.  Attempted to allocate {{value}}.",
    "D3001" => "Attempting to invoke string function on Infinity or NaN",
    "D3010" => "Second argument of replace function cannot be an empty string",
    "D3011" => "Fourth argument of replace function must evaluate to a positive number",
    "D3012" => "Attempted to replace a matched string with a non-string value",
    "D3020" => "Third argument of split function must evaluate to a positive number",
    "D3030" => "Unable to cast value to a number: {{value}}",
    "D3040" => "Third argument of match function must evaluate to a positive number",
    "D3050" => "The second argument of reduce function must be a function with at least two arguments",
    "D3060" => "The sqrt function cannot be applied to a negative number: {{value}}",
    "D3061" => "The power function has resulted in a value that cannot be represented as a JSON number: base={{value}}, exponent={{exp}}",
    "D3070" => "The single argument form of the sort function can only be applied to an array of strings or an array of numbers.  Use the second argument to specify a comparison function",
    "D3080" => "The picture string must only contain a maximum of two sub-pictures",
    "D3081" => "The sub-picture must not contain more than one instance of the 'decimal-separator' character",
    "D3082" => "The sub-picture must not contain more than one instance of the 'percent' character",
    "D3083" => "The sub-picture must not contain more than one instance of the 'per-mille' character",
    "D3084" => "The sub-picture must not contain both a 'percent' and a 'per-mille' character",
    "D3085" => "The mantissa part of a sub-picture must contain at least one character that is either an 'optional digit character' or a member of the 'decimal digit family'",
    "D3086" => "The sub-picture must not contain a passive character that is preceded by an active character and that is followed by another active character",
    "D3087" => "The sub-picture must not contain a 'grouping-separator' character that appears adjacent to a 'decimal-separator' character",
    "D3088" => "The sub-picture must not contain a 'grouping-separator' at the end of the integer part",
    "D3089" => "The sub-picture must not contain two adjacent instances of the 'grouping-separator' character",
    "D3090" => "The integer part of the sub-picture must not contain a member of the 'decimal digit family' that is followed by an instance of the 'optional digit character'",
    "D3091" => "The fractional part of the sub-picture must not contain an instance of the 'optional digit character' that is followed by a member of the 'decimal digit family'",
    "D3092" => "A sub-picture that contains a 'percent' or 'per-mille' character must not contain a character treated as an 'exponent-separator'",
    "D3093" => "The exponent part of the sub-picture must comprise only of one or more characters that are members of the 'decimal digit family'",
    "D3100" => "The radix of the formatBase function must be between 2 and 36.  It was given {{value}}",
    "D3110" => "The argument of the toMillis function must be an ISO 8601 formatted timestamp. Given {{value}}",
    "D3120" => "Syntax error in expression passed to function eval: {{value}}",
    "D3121" => "Dynamic error evaluating the expression passed to function eval: {{value}}",
    "D3130" => "Formatting or parsing an integer as a sequence starting with {{value}} is not supported by this implementation",
    "D3131" => "In a decimal digit pattern, all digits must be from the same decimal group",
    "D3132" => "Unknown component specifier {{value}} in date/time picture string",
    "D3133" => "The 'name' modifier can only be applied to months and days in the date/time picture string, not {{value}}",
    "D3134" => "The timezone integer format specifier cannot have more than four digits",
    "D3135" => "No matching closing bracket ']' in date/time picture string",
    "D3136" => "The date/time picture string is missing specifiers required to parse the timestamp",
    "D3137" => "{{{message}}}",
    "D3138" => "The $single() function expected exactly 1 matching result.  Instead it matched more.",
    "D3139" => "The $single() function expected exactly 1 matching result.  Instead it matched 0.",
    "D3140" => "Malformed URL passed to ${{{functionName}}}(): {{value}}",
    "D3141" => "{{{message}}}",
    "U1001" => "{{{message}}}",
    "U1002" => "The function ${{{token}}} returned a promise: asynchronous functions are not supported",
    "U1003" => "The evaluation was cancelled",
    "U2001" => "The input is not valid JSON: {{{message}}}",
    "W0101" => "This annotation is not attached to an expression and will be ignored",
    "W0102" => "Unknown annotation tag @{{{tag}}}; expected @label, @message or @id",
    "W0103" => "The value of annotation tag @{{{tag}}} should be a JSON string literal",
    "W0201" => "Field {{token}} is not in the schema{{{hint}}}",
    "W0202" => "Unknown function ${{{token}}}{{{hint}}}",
    "W0203" => "Function ${{{token}}} takes {{{expected}}} but is given {{{count}}}",
    "W0204" => "Variable ${{{token}}} is not declared{{{hint}}}",
    "W0205" => "Argument {{{index}}} of ${{{token}}} is {{{type}}}, which does not match the signature {{{signature}}}",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_follow_jsonata_templates() {
        let err = Error::new("S0202").with("token", ";").with("value", "]");
        assert_eq!(err.message(), r#"Expected "]", got ";""#);
        let err = Error::new("T1005").with("token", "sum");
        assert_eq!(
            err.message(),
            "Attempted to invoke a non-function. Did you mean $sum?"
        );
        assert_eq!(
            Error::new("S0211").message(),
            "The symbol undefined cannot be used as a unary operator"
        );
    }

    #[test]
    fn positions_are_utf16_offsets() {
        assert_eq!(utf16_offset("aé𝄞b", 1), 1);
        assert_eq!(utf16_offset("aé𝄞b", 3), 2);
        assert_eq!(utf16_offset("aé𝄞b", 7), 4);
    }

    #[test]
    fn catalog_codes_are_unique() {
        let mut codes: Vec<_> = catalog().iter().map(|e| e.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), catalog().len());
    }
}
