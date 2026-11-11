//! The language service: completion, diagnostics, hover and signature help for JSONata
//! expressions, driven by a [`Schema`].
//!
//! Offsets and spans are byte offsets into the expression text. Editors that count in UTF-16
//! (browsers) convert at the boundary; the Wasm package does that.

use jlens_core::ast::{BinaryOp, Node, NodeKind, Span};
use jlens_core::signature::Signature;
use jlens_core::{Diagnostic, Severity, parse_recover};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use std::collections::HashSet;

use crate::shape::{Definitions, Field, JsonType, Schema, Sh, Shape, members_of, summary};
use crate::typing::{Event, Scope, Symbols, Ty, Walker};

/// The identifier substituted for the word under the cursor before parsing.
const PLACEHOLDER: &str = "__jlens_cursor__";

const OPERATOR_CHARS: &str = ".[]{}(),@#;:?+-*/%|=<>^&!~\"'`";

/// What a completion item inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    /// A field of the context value.
    Field,
    /// A variable in scope.
    Variable,
    /// A built-in function, or a host function declared with
    /// [`LanguageService::with_function`].
    Function,
    /// `true`, `false`, `null`.
    Keyword,
    /// An operator (`and`, `or`, `in`, ...).
    Operator,
    /// A value of the enumeration (`enum` or `const`) of the left-hand side of `=`, `!=` or
    /// `in`, inserted as a JSONata literal.
    Value,
}

/// One completion suggestion.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    /// The text shown in the list.
    pub label: String,
    /// What is suggested.
    pub kind: CompletionKind,
    /// A short description: the type of a field or variable, the signature of a function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Longer documentation (Markdown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    /// The text to insert in place of the replaced range.
    pub insert_text: String,
}

/// Completion results.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completions {
    /// The byte range the chosen item replaces (the word under the cursor).
    pub range: Span,
    /// The suggestions, best first.
    pub items: Vec<CompletionItem>,
    /// Where the values of an open domain ([`Shape::domain`]) would complete the expression:
    /// the application fetches them and offers them beside `items`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_request: Option<ValueRequest>,
}

/// A request for the values of an open domain, at a position where [`CompletionKind::Value`]
/// items are completed: the right of `=`, `!=` or `in` whose left-hand side has a
/// [`Shape::domain`]. The service stays synchronous; the application fetches the values
/// (filtered by `prefix`, cached as it sees fit) and writes each with [`ValueRequest::literal`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueRequest {
    /// The domain's identifier, as the schema declares it.
    pub domain: String,
    /// The path of the left-hand side in the schema (as [`Schema::shape_at`] reads it, such as
    /// `items[].type`), or `None` when it is not a field of the schema (a variable's shape).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What is typed so far: the word under the cursor, or, inside a string literal, its text
    /// between the opening quote and the cursor (as written, escapes included).
    pub prefix: String,
    /// The byte range a value replaces: the same as [`Completions::range`].
    pub range: Span,
    /// The quote a string value is written with: that of the literal being edited, or `"`.
    pub quote: char,
    /// The fields fixed to literals beside the value, in the order written: the comparisons
    /// joined to this one by `and` (through `or` and parentheses too), within the same predicate
    /// or, outside any, the whole expression. A domain whose values depend on another value
    /// reads it here, such as the group of `categories[group = 'X' and subgroup = '|']`.
    pub conditions: Vec<ValueCondition>,
    /// Whether the value is an element of the array on the right of `in`, where several values
    /// may be inserted, separated by commas.
    pub in_array: bool,
    /// The literals already in that array, which the application need not offer again, as the
    /// service leaves them out of the declared values it completes. Empty outside an array.
    pub listed: Vec<JsonValue>,
}

/// A field compared with literals beside the value a [`ValueRequest`] completes: by `=` (one
/// value) or `in` (the values of its array). Comparisons by `!=`, with a value that is not a
/// literal, or of something that is not a field of the schema are left out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValueCondition {
    /// The field's path in the schema, as [`ValueRequest::path`] writes it.
    pub path: String,
    /// The literal it is compared with, or those of the array it is in.
    pub values: Vec<JsonValue>,
}

impl ValueRequest {
    /// `value` as a JSONata literal, to insert in place of `range`: a string in `quote` when it
    /// can be (JSONata has no `\'` escape, so a string holding `'` keeps double quotes), other
    /// values as JSON.
    ///
    /// ```
    /// use jlens_schema::{LanguageService, Schema};
    /// use serde_json::json;
    ///
    /// let schema = Schema::from_json_schema(&json!({
    ///     "type": "object",
    ///     "properties": {"country": {"type": "string", "x-jlens-domain": "countries"}}
    /// }))
    /// .unwrap();
    /// let service = LanguageService::new(Some(schema));
    /// let request = service.complete("country = 'Br", 13).value_request.unwrap();
    /// assert_eq!(request.domain, "countries");
    /// assert_eq!(request.path.as_deref(), Some("country"));
    /// assert_eq!(request.prefix, "Br");
    /// assert_eq!(request.literal(&json!("Brazil")), "'Brazil'");
    /// ```
    pub fn literal(&self, value: &JsonValue) -> String {
        value_literal(value, self.quote)
    }
}

/// Hover information.
#[derive(Debug, Clone, Serialize)]
pub struct Hover {
    /// The byte range the information is about.
    pub range: Span,
    /// Markdown text.
    pub contents: String,
}

/// Help for the function call around the cursor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureHelp {
    /// The function name, with `$`.
    pub name: String,
    /// The signature, such as `<s-nn?:s>`.
    pub signature: String,
    /// Parameter names, with `$`.
    pub params: Vec<String>,
    /// The parameter being edited (0-based).
    pub active_param: usize,
    /// What the function does.
    pub summary: String,
}

/// A function the host application provides (a `jlens_core::HostFunction`), declared to the
/// language service so that it is completed and documented like a built-in function.
///
/// It deserializes from `{"name", "signature", "params", "summary"}`; all but `name` are
/// optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FunctionDecl {
    /// The name, without the leading `$`.
    pub name: String,
    /// The signature, such as `<n-n?:n>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Parameter names, without `$`, shown by signature help.
    pub params: Vec<String>,
    /// One-line description.
    pub summary: String,
}

impl FunctionDecl {
    /// A declaration with a name and a summary.
    pub fn new(name: impl Into<String>, summary: impl Into<String>) -> Self {
        FunctionDecl {
            name: name.into(),
            summary: summary.into(),
            ..Self::default()
        }
    }

    /// Sets the signature.
    pub fn signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }

    /// Sets the parameter names (without `$`).
    pub fn params<S: Into<String>>(mut self, params: impl IntoIterator<Item = S>) -> Self {
        self.params = params.into_iter().map(Into::into).collect();
        self
    }
}

/// A function as the service presents it: declared or built-in.
struct FunctionView {
    name: String,
    signature: String,
    params: Vec<String>,
    summary: String,
}

/// A call found by the walk, checked once every binding is known.
struct CallSite {
    /// The span of the `$name`.
    span: Span,
    name: String,
    /// The number of arguments, the value applied with `~>` included.
    count: usize,
    /// The span and the types of each argument; `None` for a partial application.
    args: Option<Vec<(Span, Option<Symbols>)>>,
}

/// Completion, diagnostics, hover and signature help for expressions over a schema.
///
/// ```
/// use jlens_schema::{LanguageService, Schema};
/// use serde_json::json;
///
/// let schema = Schema::infer(&[json!({"items": [{"type": "x", "price": 1}]})]);
/// let service = LanguageService::new(Some(schema));
/// let text = "items[type = 'x'].";
/// let completions = service.complete(text, text.len());
/// let labels: Vec<_> = completions.items.iter().map(|i| i.label.as_str()).collect();
/// assert_eq!(labels, ["type", "price"]);
/// ```
#[derive(Debug, Clone, Default)]
pub struct LanguageService {
    pub(crate) schema: Option<Schema>,
    pub(crate) variables: Vec<(String, Option<Schema>)>,
    pub(crate) functions: Vec<FunctionDecl>,
}

/// Lexical situation at an offset.
#[derive(Debug, PartialEq)]
enum Lexical {
    Code,
    /// Inside a comment.
    Comment,
    /// Inside a string literal: where it starts (its opening quote) and where it ends (past its
    /// closing quote, or `None` when it is not closed).
    String(usize, Option<usize>),
    /// Inside a backquoted name that starts at this offset.
    Backquote(usize),
}

fn lexical_state(text: &str, offset: usize) -> Lexical {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < offset.min(bytes.len()) {
        match bytes[i] {
            q @ (b'"' | b'\'') => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= offset {
                    return Lexical::String(start, (i < bytes.len()).then_some(i + 1));
                }
            }
            b'`' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'`' {
                    i += 1;
                }
                if i >= offset {
                    return Lexical::Backquote(start);
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => match text[i + 2..].find("*/") {
                Some(end) if i + 2 + end + 2 <= offset => i += 2 + end + 1,
                _ => return Lexical::Comment,
            },
            _ => {}
        }
        i += 1;
    }
    Lexical::Code
}

fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !OPERATOR_CHARS.contains(c)
}

/// Inserts a field name, quoting it with backquotes when it is not a plain name.
pub(crate) fn field_insert(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().all(is_word_char)
        && !name.starts_with(|c: char| c.is_ascii_digit() || c == '$')
        && !matches!(name, "and" | "or" | "in" | "true" | "false" | "null");
    if plain {
        name.to_owned()
    } else {
        format!("`{name}`")
    }
}

fn shape_docs(shape: &Shape, defs: &Definitions, field: Option<&Field>) -> Option<String> {
    let mut parts = Vec::new();
    // A shape that refers to a definition is described by its own label and description, or
    // by the definition's; the rest is the definition's.
    let first = members_of(shape, defs)
        .0
        .iter()
        .find(|m| !std::ptr::eq(m.shape, shape))
        .map_or(shape, |m| m.shape);
    if let Some(label) = shape.label.as_ref().or(first.label.as_ref()) {
        parts.push(format!("**{label}**"));
    }
    if let Some(d) = shape.description.as_ref().or(first.description.as_ref()) {
        parts.push(d.clone());
    }
    let shape = first;
    if let Some(f) = field
        && !f.required
        && f.presence > 0.0
    {
        parts.push(format!(
            "Optional: present in {:.0}% of the samples.",
            f.presence * 100.0
        ));
    }
    if let Some(format) = &shape.format {
        parts.push(format!("Format: `{format}`"));
    }
    if let Some(domain) = &shape.domain {
        parts.push(format!("Domain: `{domain}`"));
    }
    if let Some(values) = &shape.enumeration {
        let v: Vec<String> = values.iter().map(|v| format!("`{v}`")).collect();
        parts.push(format!("Values: {}", v.join(", ")));
    } else if !shape.examples.is_empty() {
        let v: Vec<String> = shape.examples.iter().map(|v| format!("`{v}`")).collect();
        parts.push(format!("Examples: {}", v.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// The result of walking an expression looking for one node.
#[derive(Default)]
struct Probe<'s> {
    /// The context of the field reference, or the scope of the variable, found.
    ctx: Option<Ty<'s>>,
    scope: Scope<'s>,
    found_variable: bool,
    /// The enumeration of the left-hand side of the comparison whose right-hand side is the
    /// cursor, or that of `in` whose array holds it.
    values: Vec<&'s JsonValue>,
    /// The literals already in that array.
    listed: Vec<JsonValue>,
    /// The shape of that left-hand side that has an open domain.
    domain: Option<&'s Shape>,
    /// Whether the cursor is an element of the array on the right of `in`.
    in_array: bool,
    /// The fields fixed to literals beside that comparison.
    conditions: Vec<ValueCondition>,
}

/// Whether `node` is the placeholder, as a name or a path of one step.
fn is_placeholder(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Name { name } => name == PLACEHOLDER,
        NodeKind::Path { steps, .. } => matches!(steps.as_slice(), [step] if is_placeholder(step)),
        _ => false,
    }
}

/// Whether `node` is a comparison whose right-hand side is the placeholder, or an array holding
/// it on the right of `in`.
fn compares_placeholder(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Binary {
            op: op @ (BinaryOp::Eq | BinaryOp::Ne | BinaryOp::In),
            rhs,
            ..
        } => {
            is_placeholder(rhs)
                || *op == BinaryOp::In
                    && matches!(&rhs.kind, NodeKind::Array { items } if items.iter().any(is_placeholder))
        }
        _ => false,
    }
}

/// The comparisons (`=` and `in`) joined by `and` to the one whose right-hand side is the
/// placeholder: going up from it through `and`, `or` and parentheses around one expression, the
/// other operand of each `and`. Anything else, such as a predicate, a call or `?:`, ends the
/// search. They are compared by address, as nodes of the same tree.
fn sibling_comparisons(root: &Node) -> Vec<*const Node> {
    // Iterative, so that a deep expression cannot overflow the stack: each node with the index
    // of its parent, until the comparison is found.
    let mut nodes: Vec<(&Node, Option<usize>)> = vec![(root, None)];
    let mut found = None;
    let mut i = 0;
    while i < nodes.len() {
        let node = nodes[i].0;
        if compares_placeholder(node) {
            found = Some(i);
            break;
        }
        node.for_each_child(|child| nodes.push((child, Some(i))));
        i += 1;
    }
    let mut out = Vec::new();
    let Some(mut current) = found else {
        return out;
    };
    while let Some(parent) = nodes[current].1 {
        let node = nodes[parent].0;
        match &node.kind {
            NodeKind::Binary {
                op: BinaryOp::And,
                lhs,
                rhs,
            } => {
                let other = if std::ptr::eq(&**lhs, nodes[current].0) {
                    rhs
                } else {
                    lhs
                };
                let mut pending = vec![&**other];
                while let Some(node) = pending.pop() {
                    match &node.kind {
                        NodeKind::Binary {
                            op: BinaryOp::And,
                            lhs,
                            rhs,
                        } => {
                            pending.push(rhs);
                            pending.push(lhs);
                        }
                        NodeKind::Block { exprs, .. } if exprs.len() == 1 => {
                            pending.push(&exprs[0])
                        }
                        NodeKind::Binary {
                            op: BinaryOp::Eq | BinaryOp::In,
                            ..
                        } => out.push(node as *const Node),
                        _ => {}
                    }
                }
            }
            NodeKind::Binary {
                op: BinaryOp::Or, ..
            } => {}
            NodeKind::Block { exprs, .. } if exprs.len() == 1 => {}
            _ => break,
        }
        current = parent;
    }
    out
}

/// The value of a literal node; a number that is an integer is written as one.
fn literal_value(node: &Node) -> Option<JsonValue> {
    match &node.kind {
        NodeKind::String { value } => Some(JsonValue::String(value.clone())),
        // Within ±2^53 an integer-valued double is exact, and `as` cannot saturate.
        NodeKind::Number { value }
            if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_992.0 =>
        {
            Some(JsonValue::from(*value as i64))
        }
        NodeKind::Number { value } => serde_json::Number::from_f64(*value).map(JsonValue::Number),
        NodeKind::Bool { value } => Some(JsonValue::Bool(*value)),
        NodeKind::Null => Some(JsonValue::Null),
        _ => None,
    }
}

/// The values of a type's enumerations, and of its arrays' items, in order and without repeats.
/// A shape that declares no type (a bare `const`) counts too.
fn enumeration<'s>(ty: &Ty<'s>) -> Vec<&'s JsonValue> {
    let mut out: Vec<&'s JsonValue> = Vec::new();
    for shape in ty.shapes.iter().copied().chain(ty.elements().shapes) {
        for value in shape.shape.enumeration.iter().flatten() {
            if !out.contains(&value) {
                out.push(value);
            }
        }
    }
    out
}

/// The first shape of a type, or of its arrays' items, with an open domain.
fn domain_shape<'s>(ty: &Ty<'s>) -> Option<&'s Shape> {
    ty.shapes
        .iter()
        .copied()
        .chain(ty.elements().shapes)
        .find(|shape| shape.domain.is_some())
        .map(|shape| shape.shape)
}

/// The path of `target`, found by identity, among the shapes of `schema`: `a.b[].c`. A shape
/// of a definition is found through the first place that refers to it.
fn shape_path(schema: &Schema, target: &Shape) -> Option<String> {
    // Iterative, so that a deep schema cannot overflow the stack; a recursive one leads back
    // to shapes already seen.
    let mut seen: HashSet<*const Shape> = HashSet::new();
    let mut pending = vec![(&schema.root, String::new())];
    while let Some((shape, path)) = pending.pop() {
        if std::ptr::eq(shape, target) {
            return Some(path);
        }
        for member in members_of(shape, &schema.definitions).0.into_iter().rev() {
            if std::ptr::eq(member.shape, target) {
                return Some(path);
            }
            if !seen.insert(member.shape) {
                continue;
            }
            if let Some(items) = &member.shape.items {
                pending.push((items, format!("{path}[]")));
            }
            for (name, field) in member.shape.fields.iter().rev() {
                let path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                pending.push((&field.shape, path));
            }
        }
    }
    None
}

/// Whether two JSON values are equal as JSONata compares them: numbers by value, so that `2`
/// and `2.0` are the same.
fn same_value(a: &JsonValue, b: &JsonValue) -> bool {
    match (a, b) {
        (JsonValue::Number(x), JsonValue::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// The values a type is declared to take: every shape it may have declares an enumeration
/// (directly, or through the items of its arrays), none of them inferred from samples. `None`
/// when any value is possible, or when an enumeration holds an array or an object, which a
/// literal is not compared with here. An open domain takes values no list holds: `None` too.
fn declared_values<'s>(ty: &Ty<'s>) -> Option<Vec<&'s JsonValue>> {
    fn collect<'s>(shape: Sh<'s>, out: &mut Vec<&'s JsonValue>, depth: usize) -> Option<()> {
        if shape.domain.is_some() {
            return None;
        }
        match &shape.shape.enumeration {
            Some(values) if !shape.enumeration_inferred => {
                for value in values {
                    if value.is_array() || value.is_object() {
                        return None;
                    }
                    if !out.iter().any(|v| same_value(v, value)) {
                        out.push(value);
                    }
                }
                Some(())
            }
            Some(_) => None,
            None if depth < 32 && shape.types == [JsonType::Array] => {
                // One shape for the items: a recursive schema cannot branch on the way down.
                match members_of(shape.shape.items.as_deref()?, shape.defs) {
                    (items, false) if items.len() == 1 => collect(items[0], out, depth + 1),
                    _ => None,
                }
            }
            None => None,
        }
    }
    // A shape without a type (a bare `const`) makes the type partial, as does anything unknown;
    // only the former is accepted.
    if ty.shapes.is_empty() || (ty.partial && ty.shapes.iter().any(|s| !s.types.is_empty())) {
        return None;
    }
    let mut out = Vec::new();
    for shape in &ty.shapes {
        collect(*shape, &mut out, 0)?;
    }
    Some(out)
}

/// `W0206` for each literal on the right of `op` that `values` does not hold: the right-hand
/// side itself, or the items of the array on the right of `in`. A string gets the nearest string
/// value as a suggestion, written with the literal's quotes.
fn value_diagnostics(
    text: &str,
    op: BinaryOp,
    values: &[&JsonValue],
    rhs: &Node,
    out: &mut Vec<Diagnostic>,
) {
    for node in compared_literals(op, rhs) {
        let Some(value) = literal_value(node) else {
            continue;
        };
        if values.iter().any(|v| same_value(v, &value)) {
            continue;
        }
        let source = text.get(node.span.start..node.span.end).unwrap_or_default();
        let quote = if source.starts_with('\'') { '\'' } else { '"' };
        // Only a string has a nearest value: a misspelling; numbers are not near by their digits.
        let strings: Vec<&str> = values.iter().filter_map(|v| v.as_str()).collect();
        let hint = value
            .as_str()
            .and_then(|s| closest(s, &strings))
            .map(|c| {
                let literal = value_literal(&JsonValue::String(c.to_owned()), quote);
                format!("; did you mean `{literal}`?")
            })
            .unwrap_or_default();
        out.push(Diagnostic::new(
            Severity::Warning,
            "W0206",
            node.span,
            &[("value", source), ("hint", &hint)],
        ));
    }
}

/// The types a present value of `ty` may have, as signature symbols: those of the value, an
/// array replaced by its items, as a path yields them and a single item stands alone. `None`
/// when the type is not fully known, or holds arrays of arrays, which a path flattens.
fn present_types(ty: &Ty<'_>) -> Option<(Symbols, String)> {
    let symbols = ty.symbols()?;
    if symbols.items.contains('a') {
        return None;
    }
    let values = symbols.values.chars().filter(|c| *c != 'a');
    let set: String = values.chain(symbols.items.chars()).collect();
    Some((symbols, set))
}

/// `W0207`, `W0208` and `W0209` for the operands of an order comparison or of arithmetic whose
/// known types the operator never takes, as `T2010`, `T2009`, `T2001` and `T2002` would fail when
/// it runs. An operand that may be undefined passes: the operator takes undefined.
fn operand_diagnostics(
    node: &Node,
    op: BinaryOp,
    lhs: (&Node, &Ty<'_>),
    rhs: (&Node, &Ty<'_>),
    out: &mut Vec<Diagnostic>,
) {
    let left = present_types(lhs.1);
    let right = present_types(rhs.1);
    let order = matches!(
        op,
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    );
    // The types each side must have.
    let takes = |c: char| {
        if order {
            c == 's' || c == 'n'
        } else {
            c == 'n'
        }
    };
    let code = if order { "W0207" } else { "W0209" };
    let mut wrong = false;
    for (side, (node, _), types) in [("left", lhs, &left), ("right", rhs, &right)] {
        if let Some((symbols, set)) = types
            && !set.chars().any(takes)
        {
            wrong = true;
            out.push(Diagnostic::new(
                Severity::Warning,
                code,
                node.span,
                &[
                    ("side", side),
                    ("token", op.symbol()),
                    ("type", &describe(symbols)),
                ],
            ));
        }
    }
    // Strings and numbers are not compared with each other.
    if order
        && !wrong
        && let (Some((l, left)), Some((r, right))) = (&left, &right)
        && !left.chars().any(|c| takes(c) && right.contains(c))
    {
        out.push(Diagnostic::new(
            Severity::Warning,
            "W0208",
            node.span,
            &[
                ("token", op.symbol()),
                ("lhs", &describe(l)),
                ("rhs", &describe(r)),
            ],
        ));
    }
}

/// `W0210` for each literal on the right of `op` (`=`, `!=` or `in`) whose type a present value
/// of `lhs` never has, so that it never equals it: the right-hand side itself, or the items of
/// the array on the right of `in`.
fn equality_diagnostics(
    text: &str,
    op: BinaryOp,
    lhs: &Ty<'_>,
    rhs: &Node,
    out: &mut Vec<Diagnostic>,
) {
    let Some((symbols, set)) = present_types(lhs) else {
        return;
    };
    for node in compared_literals(op, rhs) {
        let Some(value) = literal_value(node) else {
            continue;
        };
        let symbol = match value {
            JsonValue::Null => 'l',
            JsonValue::Bool(_) => 'b',
            JsonValue::Number(_) => 'n',
            JsonValue::String(_) => 's',
            _ => continue,
        };
        if set.contains(symbol) {
            continue;
        }
        let source = text.get(node.span.start..node.span.end).unwrap_or_default();
        out.push(Diagnostic::new(
            Severity::Warning,
            "W0210",
            node.span,
            &[("value", source), ("type", &describe(&symbols))],
        ));
    }
}

/// The literals on the right of `op` that a comparison checks: the right-hand side itself, or
/// the items of the array on the right of `in`. A literal with a predicate or a group,
/// `"a"[false]`, is not that value, nor is an array with one, `["a", "b"][0]`, those items.
fn compared_literals(op: BinaryOp, rhs: &Node) -> Vec<&Node> {
    let plain = |node: &Node| {
        let attrs = &node.attrs;
        attrs.stages.is_empty() && attrs.predicate.is_empty() && attrs.group.is_none()
    };
    let literals: Vec<&Node> = match &rhs.kind {
        NodeKind::Array { items } if op == BinaryOp::In && plain(rhs) => items.iter().collect(),
        NodeKind::Array { .. } => Vec::new(),
        _ => vec![rhs],
    };
    literals.into_iter().filter(|n| plain(n)).collect()
}

/// `value` as a JSONata literal: a string in `quote` (`'` or `"`) when it can be (JSONata has no
/// `\'` escape, so a string holding `'` keeps double quotes), other values as JSON. What
/// [`ValueRequest::literal`] writes.
///
/// ```
/// use serde_json::json;
///
/// assert_eq!(jlens_schema::value_literal(&json!("a\"b"), '\''), r#"'a\"b'"#);
/// assert_eq!(jlens_schema::value_literal(&json!("it's"), '\''), r#""it's""#);
/// assert_eq!(jlens_schema::value_literal(&json!([1, null]), '"'), "[1,null]");
/// ```
pub fn value_literal(value: &JsonValue, quote: char) -> String {
    let json = value.to_string();
    match value {
        // JSONata has no `\'` escape: a string with `'` keeps the double quotes.
        JsonValue::String(s) if quote == '\'' && !s.contains('\'') => {
            format!("'{}'", &json[1..json.len() - 1])
        }
        _ => json,
    }
}

/// Completion items for `values`, but those already `listed`, that start with `prefix` (ignoring
/// case; a string is matched without its quotes).
fn value_items(
    values: &[&JsonValue],
    listed: &[JsonValue],
    prefix: &str,
    quote: char,
) -> Vec<CompletionItem> {
    let lower = prefix.to_lowercase();
    values
        .iter()
        .filter(|v| !listed.iter().any(|l| same_value(l, v)))
        .filter(|v| {
            let text = match v {
                JsonValue::String(s) => s.clone(),
                other => other.to_string(),
            };
            text.to_lowercase().starts_with(&lower)
        })
        .map(|v| {
            let literal = value_literal(v, quote);
            CompletionItem {
                label: literal.clone(),
                kind: CompletionKind::Value,
                detail: None,
                documentation: None,
                insert_text: literal,
            }
        })
        .collect()
}

impl LanguageService {
    /// A service for expressions over documents described by `schema` (or without a schema:
    /// functions and variables are still completed).
    pub fn new(schema: Option<Schema>) -> Self {
        LanguageService {
            schema,
            variables: Vec::new(),
            functions: Vec::new(),
        }
    }

    /// Declares a variable the expression is evaluated with (a binding), optionally with the
    /// shape of its value. A shape that refers to the definitions of a schema
    /// ([`Shape::references`]), as a recursive one does, is declared with that schema by
    /// [`LanguageService::with_variable_schema`]: here its references are unknown.
    pub fn with_variable(mut self, name: impl Into<String>, shape: Option<Shape>) -> Self {
        let schema = shape.map(|root| Schema {
            root,
            ..Schema::default()
        });
        self.variables.push((name.into(), schema));
        self
    }

    /// Declares a variable the expression is evaluated with (a binding), whose value `schema`
    /// describes.
    pub fn with_variable_schema(mut self, name: impl Into<String>, schema: Schema) -> Self {
        self.variables.push((name.into(), Some(schema)));
        self
    }

    /// Declares a host function the expression is evaluated with. It is completed, and shown by
    /// hover and signature help, like a built-in function, and replaces a built-in function of
    /// the same name.
    pub fn with_function(mut self, function: FunctionDecl) -> Self {
        self.functions.retain(|f| f.name != function.name);
        self.functions.push(function);
        self
    }

    /// The declared functions, then the built-in functions they do not replace.
    fn functions(&self) -> Vec<FunctionView> {
        let declared = self.functions.iter().map(|f| FunctionView {
            name: f.name.clone(),
            signature: f.signature.clone().unwrap_or_default(),
            params: f.params.clone(),
            summary: f.summary.clone(),
        });
        let built_in = jlens_core::function_catalog()
            .into_iter()
            .filter(|b| self.functions.iter().all(|f| f.name != b.name))
            .map(|b| FunctionView {
                name: b.name.to_owned(),
                signature: b.signature,
                params: b.params.iter().map(|p| (*p).to_owned()).collect(),
                summary: b.summary.to_owned(),
            });
        declared.chain(built_in).collect()
    }

    fn function(&self, name: &str) -> Option<FunctionView> {
        self.functions().into_iter().find(|f| f.name == name)
    }

    fn root_ty(&self) -> Ty<'_> {
        match &self.schema {
            Some(s) => Ty::of(&s.root, &s.definitions),
            None => Ty::unknown(),
        }
    }

    fn externals(&self) -> Vec<(String, Ty<'_>)> {
        self.variables
            .iter()
            .map(|(n, s)| {
                let ty = s
                    .as_ref()
                    .map_or_else(Ty::unknown, |s| Ty::of(&s.root, &s.definitions));
                (n.clone(), ty)
            })
            .collect()
    }

    /// Walks `root`, calling `visit` for each field reference and variable.
    fn walk<'s>(&'s self, root: &Node, visit: &mut dyn FnMut(Event<'_, 's>)) {
        // Declared functions are external names too: a call to one is not typed as a built-in.
        let mut externals = self.externals();
        externals.extend(
            self.functions
                .iter()
                .map(|f| (f.name.clone(), Ty::unknown())),
        );
        let mut walker = Walker {
            root: self.root_ty(),
            externals: &externals,
            visit,
        };
        let ctx = walker.root.clone();
        walker.walk(root, &ctx, &mut Vec::new());
    }

    /// Suggestions for the word at `offset`.
    pub fn complete(&self, text: &str, offset: usize) -> Completions {
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let empty = |range| Completions {
            range,
            items: Vec::new(),
            value_request: None,
        };
        match lexical_state(text, offset) {
            Lexical::Comment => return empty(Span::new(offset, offset)),
            Lexical::String(start, closed) => {
                // Complete a value of the comparison the literal is the right-hand side of; the
                // whole literal is replaced. The rest of an unclosed literal stays as it is.
                let end = closed.unwrap_or(offset);
                let modified = format!("{}{PLACEHOLDER} {}", &text[..start], &text[end..]);
                let probe = self.probe(&modified, false);
                let quote = if text[start..].starts_with('\'') {
                    '\''
                } else {
                    '"'
                };
                let prefix = &text[start + 1..offset];
                let strings: Vec<&serde_json::Value> = probe
                    .values
                    .iter()
                    .copied()
                    .filter(|v| v.is_string())
                    .collect();
                let range = Span::new(start, end);
                return Completions {
                    range,
                    items: value_items(&strings, &probe.listed, prefix, quote),
                    value_request: self.value_request(&probe, prefix, range, quote),
                };
            }
            Lexical::Backquote(start) => {
                // Complete a quoted field name; the closing quote is replaced too.
                let end = if text[offset..].starts_with('`') {
                    offset + 1
                } else {
                    offset
                };
                let prefix = &text[start + 1..offset];
                let modified = format!("{}{PLACEHOLDER}{}", &text[..start], &text[end..]);
                let probe = self.probe(&modified, false);
                let mut items = self.field_items(probe.ctx.as_ref(), prefix);
                for item in &mut items {
                    item.insert_text = format!("`{}`", item.label);
                }
                return Completions {
                    range: Span::new(start, end),
                    items,
                    value_request: None,
                };
            }
            Lexical::Code => {}
        }

        // The word around the cursor.
        let start = text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word_char(*c))
            .last()
            .map_or(offset, |(i, _)| i);
        let end = offset
            + text[offset..]
                .chars()
                .take_while(|c| is_word_char(*c))
                .map(char::len_utf8)
                .sum::<usize>();
        let prefix = &text[start..offset];
        let range = Span::new(start, end);
        let before = text[..start].trim_end();
        let after_dot = before.ends_with('.') && !before.ends_with("..");
        let previous = before.chars().last();

        // After a complete operand and whitespace, an operator is expected.
        let previous_word: String = before
            .chars()
            .rev()
            .take_while(|c| is_word_char(*c))
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let operator_position = !prefix.starts_with('$')
            && before.len() < start
            && previous.is_some_and(|c| is_word_char(c) || ")]}\"'`".contains(c))
            && !before.ends_with('$')
            && !matches!(previous_word.as_str(), "and" | "or" | "in");
        if operator_position && !after_dot {
            let ops = [
                ("and", "Both conditions hold"),
                ("or", "Either condition holds"),
                ("in", "Value is one of the items of an array"),
            ];
            let items = ops
                .iter()
                .filter(|(op, _)| op.starts_with(prefix))
                .map(|(op, doc)| CompletionItem {
                    label: (*op).into(),
                    kind: CompletionKind::Operator,
                    detail: None,
                    documentation: Some((*doc).into()),
                    insert_text: (*op).into(),
                })
                .collect();
            return Completions {
                range,
                items,
                value_request: None,
            };
        }

        let variable_mode = prefix.starts_with('$');
        let placeholder = if variable_mode {
            format!("${PLACEHOLDER}")
        } else {
            PLACEHOLDER.to_owned()
        };
        let modified = format!("{}{placeholder}{}", &text[..start], &text[end..]);
        let probe = self.probe(&modified, variable_mode);

        let mut items = Vec::new();
        let mut value_request = None;
        if !variable_mode && !after_dot {
            items.extend(value_items(&probe.values, &probe.listed, prefix, '"'));
            value_request = self.value_request(&probe, prefix, range, '"');
        }
        if variable_mode {
            items.extend(self.variable_items(&probe.scope, &prefix[1..]));
            items.extend(self.function_items(&prefix[1..]));
        } else {
            items.extend(self.field_items(probe.ctx.as_ref(), prefix));
            if !after_dot {
                items.extend(self.variable_items(&probe.scope, prefix));
                items.extend(self.function_items(prefix));
                for kw in ["true", "false", "null"] {
                    if kw.starts_with(prefix) && !prefix.is_empty() {
                        items.push(CompletionItem {
                            label: kw.into(),
                            kind: CompletionKind::Keyword,
                            detail: None,
                            documentation: None,
                            insert_text: kw.into(),
                        });
                    }
                }
            }
        }
        Completions {
            range,
            items,
            value_request,
        }
    }

    /// The request for the values of the open domain the probe found, if any.
    fn value_request(
        &self,
        probe: &Probe<'_>,
        prefix: &str,
        range: Span,
        quote: char,
    ) -> Option<ValueRequest> {
        let shape = probe.domain?;
        Some(ValueRequest {
            domain: shape.domain.clone()?,
            path: self
                .schema
                .as_ref()
                .and_then(|schema| shape_path(schema, shape)),
            prefix: prefix.to_owned(),
            range,
            quote,
            conditions: probe.conditions.clone(),
            in_array: probe.in_array,
            listed: probe.listed.clone(),
        })
    }

    /// The condition a comparison beside the cursor sets: a field of the schema compared with a
    /// literal by `=`, or with an array of literals by `in`.
    fn value_condition(&self, op: BinaryOp, lhs: &Ty<'_>, rhs: &Node) -> Option<ValueCondition> {
        let values = match (op, &rhs.kind) {
            (BinaryOp::In, NodeKind::Array { items }) => items
                .iter()
                .map(literal_value)
                .collect::<Option<Vec<_>>>()?,
            (BinaryOp::Eq | BinaryOp::In, _) => vec![literal_value(rhs)?],
            _ => return None,
        };
        if values.is_empty() {
            return None;
        }
        let schema = self.schema.as_ref()?;
        let path = lhs
            .shapes
            .iter()
            .copied()
            .chain(lhs.elements().shapes)
            .find_map(|shape| shape_path(schema, shape.shape))?;
        Some(ValueCondition { path, values })
    }

    /// Parses `modified` (which contains the placeholder) and finds the placeholder's context.
    fn probe<'s>(&'s self, modified: &str, variable: bool) -> Probe<'s> {
        let ast = parse_recover(modified);
        let siblings = if variable {
            Vec::new()
        } else {
            sibling_comparisons(&ast.root)
        };
        let mut probe = Probe::default();
        let mut visit = |event: Event<'_, 's>| match event {
            Event::Field {
                name, ctx, scope, ..
            } if !variable && name == PLACEHOLDER => {
                probe.ctx = Some(ctx.clone());
                probe.scope = scope.clone();
            }
            Event::Variable { name, scope, .. } if variable && name == PLACEHOLDER => {
                probe.scope = scope.clone();
                probe.found_variable = true;
            }
            Event::Compare { node, op, lhs, rhs } if !variable => {
                if siblings.iter().any(|&s| std::ptr::eq(s, node)) {
                    if let Some(condition) = self.value_condition(op, lhs, rhs) {
                        probe.conditions.push(condition);
                    }
                    return;
                }
                let listed = match &rhs.kind {
                    NodeKind::Array { items }
                        if op == BinaryOp::In && items.iter().any(is_placeholder) =>
                    {
                        Some(items.iter().filter_map(literal_value).collect())
                    }
                    _ if is_placeholder(rhs) => Some(Vec::new()),
                    _ => None,
                };
                if let Some(listed) = listed {
                    probe.values = enumeration(lhs);
                    probe.listed = listed;
                    probe.domain = domain_shape(lhs);
                    probe.in_array = matches!(rhs.kind, NodeKind::Array { .. });
                }
            }
            _ => {}
        };
        self.walk(&ast.root, &mut visit);
        if probe.ctx.is_none() && !probe.found_variable && !variable {
            // Could not locate the cursor (very broken input): offer the root fields.
            probe.ctx = Some(self.root_ty());
        }
        probe
    }

    fn field_items(&self, ctx: Option<&Ty<'_>>, prefix: &str) -> Vec<CompletionItem> {
        let Some(ctx) = ctx else {
            return Vec::new();
        };
        let lower = prefix.to_lowercase();
        let mut fields: Vec<(&str, &Field, &Definitions)> = ctx
            .fields()
            .into_iter()
            .filter(|(name, f, _)| !f.shape.hidden && name.to_lowercase().starts_with(&lower))
            .collect();
        // Required fields first, then by how often they occur; ties keep schema order.
        fields.sort_by(|a, b| {
            b.1.required
                .cmp(&a.1.required)
                .then(b.1.presence.total_cmp(&a.1.presence))
        });
        fields
            .into_iter()
            .map(|(name, f, defs)| CompletionItem {
                label: name.to_owned(),
                kind: CompletionKind::Field,
                detail: Some(summary(&f.shape, defs)),
                documentation: shape_docs(&f.shape, defs, Some(f)),
                insert_text: field_insert(name),
            })
            .collect()
    }

    fn variable_items(&self, scope: &Scope<'_>, prefix: &str) -> Vec<CompletionItem> {
        let mut seen: Vec<String> = Vec::new();
        let mut items = Vec::new();
        let externals = self.externals();
        let candidates = scope
            .iter()
            .rev()
            .chain(externals.iter())
            .map(|(n, t)| (n.clone(), t.clone()));
        for (name, ty) in candidates {
            if name.starts_with(prefix) && !seen.contains(&name) && !name.is_empty() {
                let detail = match ty.shapes.as_slice() {
                    [shape] => Some(summary(shape.shape, shape.defs)),
                    _ => None,
                };
                items.push(CompletionItem {
                    label: format!("${name}"),
                    kind: CompletionKind::Variable,
                    detail,
                    documentation: None,
                    insert_text: format!("${name}"),
                });
                seen.push(name);
            }
        }
        for (name, doc) in [("$", "The context value"), ("$$", "The input document")] {
            if prefix.is_empty() || name[1..].starts_with(prefix) {
                items.push(CompletionItem {
                    label: name.into(),
                    kind: CompletionKind::Variable,
                    detail: None,
                    documentation: Some(doc.into()),
                    insert_text: name.into(),
                });
            }
        }
        items
    }

    fn function_items(&self, prefix: &str) -> Vec<CompletionItem> {
        self.functions()
            .into_iter()
            .filter(|f| f.name.starts_with(prefix))
            .map(|f| CompletionItem {
                label: format!("${}", f.name),
                kind: CompletionKind::Function,
                detail: (!f.signature.is_empty()).then(|| f.signature.clone()),
                documentation: (!f.summary.is_empty()).then_some(f.summary),
                insert_text: format!("${}", f.name),
            })
            .collect()
    }

    /// Syntax errors, annotation warnings, references to fields the schema does not have,
    /// literals compared with a field outside its declared `enum` or `const`, calls of unknown
    /// functions, with the wrong number of arguments or with arguments of types the signature
    /// does not accept, and, when variables are declared, references to variables that are not.
    pub fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        let ast = parse_recover(text);
        let mut out = ast.diagnostics.clone();
        // Calls and variables are checked after the walk: a name bound later can be used by a
        // function defined earlier, or by itself.
        let mut calls: Vec<CallSite> = Vec::new();
        let mut variables: Vec<(Span, String)> = Vec::new();
        let mut bound: Vec<String> = Vec::new();
        let mut visit = |event: Event<'_, '_>| match event {
            Event::Field {
                node, name, ctx, ..
            } if self.schema.is_some() && ctx.is_known_objects() && !ctx.has_field(name) => {
                let known: Vec<&str> = ctx.fields().iter().map(|(n, ..)| *n).collect();
                let quoted = text[node.span.start..].starts_with('`');
                let len = name.len() + if quoted { 2 } else { 0 };
                let span = Span::new(node.span.start, (node.span.start + len).min(node.span.end));
                out.push(Diagnostic::new(
                    Severity::Warning,
                    "W0201",
                    span,
                    &[("token", name), ("hint", &did_you_mean(name, &known, ""))],
                ));
            }
            Event::Call {
                node,
                name,
                args,
                types,
            } => calls.push(CallSite {
                span: node.span,
                name: name.to_owned(),
                count: args,
                args: types.map(|t| t.iter().map(|(s, ty)| (*s, ty.symbols())).collect()),
            }),
            Event::Variable { node, name, .. } => variables.push((node.span, name.to_owned())),
            Event::Bind { name } => bound.push(name.to_owned()),
            Event::Compare { node, op, lhs, rhs } => match declared_values(lhs) {
                Some(values) => value_diagnostics(text, op, &values, rhs, &mut out),
                // Two literals compared are left alone: the reader sees both.
                None if !matches!(&node.kind, NodeKind::Binary { lhs, .. }
                    if literal_value(lhs).is_some()) =>
                {
                    equality_diagnostics(text, op, lhs, rhs, &mut out);
                }
                None => {}
            },
            Event::Operands { node, op, lhs, rhs } => {
                operand_diagnostics(node, op, lhs, rhs, &mut out);
            }
            Event::Field { .. } => {}
        };
        self.walk(&ast.root, &mut visit);
        out.extend(self.call_diagnostics(&calls, &bound));
        out.extend(self.variable_diagnostics(&variables, &calls, &bound));
        out.sort_by_key(|d| d.span.start);
        out
    }

    /// `W0202` for calls of functions that are neither built in, declared, nor bound in the
    /// expression, `W0203` for calls with a number of arguments their signature rejects, and
    /// `W0205` for arguments whose known types their signature rejects.
    fn call_diagnostics(&self, calls: &[CallSite], bound: &[String]) -> Vec<Diagnostic> {
        let functions = self.functions();
        let mut out = Vec::new();
        for CallSite {
            span,
            name,
            count,
            args,
        } in calls
        {
            if name.is_empty()
                || name == "$"
                || bound.contains(name)
                || self.variables.iter().any(|(v, _)| v == name)
            {
                continue;
            }
            let Some(function) = functions.iter().find(|f| f.name == *name) else {
                let known: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
                out.push(Diagnostic::new(
                    Severity::Warning,
                    "W0202",
                    *span,
                    &[("token", name), ("hint", &did_you_mean(name, &known, "$"))],
                ));
                continue;
            };
            let Some((min, max)) = arity(&function.signature) else {
                continue;
            };
            if *count < min || max.is_some_and(|max| *count > max) {
                let arguments = |n: usize| match n {
                    1 => "1 argument".to_owned(),
                    n => format!("{n} arguments"),
                };
                let expected = match max {
                    Some(max) if max == min => arguments(max),
                    Some(max) if min == 0 => format!("at most {}", arguments(max)),
                    Some(max) => format!("{min} to {max} arguments"),
                    None => format!("at least {}", arguments(min)),
                };
                out.push(Diagnostic::new(
                    Severity::Warning,
                    "W0203",
                    *span,
                    &[
                        ("token", name),
                        ("expected", &expected),
                        ("count", &count.to_string()),
                    ],
                ));
                continue;
            }
            if let (Some(args), Ok(signature)) = (args, Signature::parse(&function.signature))
                && let Some(index) = mismatch(&signature, args)
                && let (arg_span, Some(symbols)) = &args[index]
            {
                out.push(Diagnostic::new(
                    Severity::Warning,
                    "W0205",
                    *arg_span,
                    &[
                        ("index", &(index + 1).to_string()),
                        ("token", name),
                        ("type", &describe(symbols)),
                        ("signature", &function.signature),
                    ],
                ));
            }
        }
        out
    }

    /// `W0204` for references to variables that are neither declared, bound in the expression,
    /// nor functions. Only when variables are declared: the declarations are then taken to be
    /// complete.
    fn variable_diagnostics(
        &self,
        references: &[(Span, String)],
        calls: &[CallSite],
        bound: &[String],
    ) -> Vec<Diagnostic> {
        if self.variables.is_empty() {
            return Vec::new();
        }
        let functions = self.functions();
        let mut known: Vec<&str> = self.variables.iter().map(|(n, _)| n.as_str()).collect();
        known.extend(bound.iter().map(String::as_str));
        references
            .iter()
            .filter(|(span, name)| {
                !name.is_empty()
                    && name != "$"
                    && !known.contains(&name.as_str())
                    && functions.iter().all(|f| f.name != *name)
                    // A call of an unknown name already has `W0202`.
                    && calls.iter().all(|c| c.span != *span)
            })
            .map(|(span, name)| {
                Diagnostic::new(
                    Severity::Warning,
                    "W0204",
                    *span,
                    &[("token", name), ("hint", &did_you_mean(name, &known, "$"))],
                )
            })
            .collect()
    }

    /// Information about the field, variable or function at `offset`.
    pub fn hover(&self, text: &str, offset: usize) -> Option<Hover> {
        let ast = parse_recover(text);
        let mut best: Option<(Span, String)> = None;
        let mut consider = |span: Span, contents: String| {
            if span.contains(offset)
                && best
                    .as_ref()
                    .is_none_or(|(b, _)| span.end - span.start <= b.end - b.start)
            {
                best = Some((span, contents));
            }
        };
        let mut visit = |event: Event<'_, '_>| match event {
            Event::Field {
                node, name, ctx, ..
            } => {
                let span = Span::new(
                    node.span.start,
                    (node.span.start + name.len()).min(node.span.end),
                );
                let field = ctx
                    .fields()
                    .into_iter()
                    .find(|(n, ..)| *n == name)
                    .map(|(_, f, defs)| (f, defs));
                let contents = match field {
                    Some((f, defs)) => {
                        let mut s = format!("`{name}`: {}", summary(&f.shape, defs));
                        if let Some(docs) = shape_docs(&f.shape, defs, Some(f)) {
                            s.push_str("\n\n");
                            s.push_str(&docs);
                        }
                        s
                    }
                    None if ctx.is_known_objects() => format!("`{name}`: not in the schema"),
                    None => format!("`{name}`"),
                };
                consider(span, contents);
            }
            Event::Variable { node, name, ty, .. } => {
                let contents = match self.function(name) {
                    Some(f) => format!("`${}{}`\n\n{}", f.name, f.signature, f.summary),
                    None => match ty.shapes.as_slice() {
                        [shape] => format!("`${name}`: {}", summary(shape.shape, shape.defs)),
                        _ => format!("`${name}`"),
                    },
                };
                consider(node.span, contents);
            }
            Event::Call { .. }
            | Event::Bind { .. }
            | Event::Compare { .. }
            | Event::Operands { .. } => {}
        };
        self.walk(&ast.root, &mut visit);
        best.map(|(range, contents)| Hover { range, contents })
    }

    /// The function call around `offset` and the argument being edited.
    pub fn signature_help(&self, text: &str, offset: usize) -> Option<SignatureHelp> {
        let offset = offset.min(text.len());
        let bytes = text.as_bytes();
        let mut depth = 0i32;
        let mut commas = 0;
        let mut i = offset;
        while i > 0 {
            i -= 1;
            if lexical_state(text, i) != Lexical::Code {
                continue;
            }
            match bytes[i] {
                b')' | b']' | b'}' => depth += 1,
                b'[' | b'{' if depth > 0 => depth -= 1,
                b'[' | b'{' => return None,
                b'(' if depth > 0 => depth -= 1,
                b'(' => {
                    let name_end = text[..i].trim_end().len();
                    let name_start = text[..name_end]
                        .char_indices()
                        .rev()
                        .take_while(|(_, c)| is_word_char(*c))
                        .last()
                        .map_or(name_end, |(j, _)| j);
                    let name = text[name_start..name_end].strip_prefix('$')?;
                    let f = self.function(name)?;
                    return Some(SignatureHelp {
                        name: format!("${name}"),
                        signature: f.signature,
                        params: f.params.iter().map(|p| format!("${p}")).collect(),
                        active_param: commas,
                        summary: f.summary,
                    });
                }
                b',' if depth == 0 => commas += 1,
                _ => {}
            }
        }
        None
    }
}

/// The fewest and the most arguments a signature accepts (`None`: no limit), or `None` when
/// there is no signature to check against. Parameters marked `?` are optional, and so are those
/// marked `-`, which take the context value; `+` takes any number.
fn arity(signature: &str) -> Option<(usize, Option<usize>)> {
    if signature.is_empty() {
        return None;
    }
    let params = Signature::parse(signature).ok()?.params;
    let min = params.iter().filter(|p| !p.pattern.ends_with('?')).count();
    let variadic = params.iter().any(|p| p.pattern.ends_with('+'));
    Some((min, (!variadic).then_some(params.len())))
}

/// One parameter of a signature: the symbols it accepts, how many arguments it takes, and the
/// type its array items must have.
struct Piece<'p> {
    set: &'p str,
    min: usize,
    max: usize,
    items: Option<char>,
}

impl Piece<'_> {
    /// Whether an argument of these types may be accepted (unknown types always may).
    fn accepts(&self, arg: Option<&Symbols>) -> bool {
        let Some(arg) = arg else {
            return true;
        };
        // A path yields the items of an array, and a single item stands alone.
        let mut values = arg.values.chars().chain(arg.items.chars());
        let mut items = arg
            .items
            .chars()
            .chain(arg.values.chars().filter(|c| *c != 'a'));
        values.any(|c| self.set.contains(c))
            && self.items.is_none_or(|want| items.any(|c| c == want))
    }
}

/// The index of the first argument whose known types `signature` cannot accept, as jsonata-js
/// finds it when it reports `T0410` or `T0412`, or `None`.
fn mismatch(signature: &Signature, args: &[(Span, Option<Symbols>)]) -> Option<usize> {
    let pieces: Vec<Piece> = signature
        .params
        .iter()
        .map(|p| {
            let (set, quantifier) = match p.pattern.strip_prefix('[') {
                Some(rest) => rest.split_once(']').unwrap_or((rest, "")),
                None => p.pattern.split_at(p.pattern.len().min(1)),
            };
            let (min, max) = match quantifier {
                q if q.contains('+') => (1, usize::MAX),
                q if q.contains('?') => (0, 1),
                _ => (1, 1),
            };
            let items = match (p.array, p.subtype.as_deref().and_then(|s| s.chars().next())) {
                (true, Some(c)) if "snblo".contains(c) => Some(c),
                _ => None,
            };
            Piece {
                set,
                min,
                max,
                items,
            }
        })
        .collect();
    let args: Vec<Option<&Symbols>> = args.iter().map(|(_, s)| s.as_ref()).collect();
    if match_pieces(&pieces, &args, 0, true).is_some() {
        return None;
    }
    // The end of the longest run of parameters that match from the start.
    let mut good_to = 0;
    for index in 0..pieces.len() {
        match match_pieces(&pieces[..=index], &args, 0, false) {
            Some(end) => good_to = end,
            None => break,
        }
    }
    args.get(good_to).copied().flatten().map(|_| good_to)
}

/// Greedy backtracking match of `pieces` against `args` from `pos`, as jsonata-js matches its
/// signature regular expression; returns where the match ends.
fn match_pieces(
    pieces: &[Piece],
    args: &[Option<&Symbols>],
    pos: usize,
    anchored: bool,
) -> Option<usize> {
    let Some((p, rest)) = pieces.split_first() else {
        return (!anchored || pos == args.len()).then_some(pos);
    };
    let mut n = 0;
    while n < p.max && pos + n < args.len() && p.accepts(args[pos + n]) {
        n += 1;
    }
    loop {
        if n < p.min {
            return None;
        }
        if let Some(end) = match_pieces(rest, args, pos + n, anchored) {
            return Some(end);
        }
        if n == 0 {
            return None;
        }
        n -= 1;
    }
}

/// `a string`, `an array of numbers or strings`, `a number or null`...
fn describe(symbols: &Symbols) -> String {
    let plural = |c| match c {
        's' => "strings",
        'n' => "numbers",
        'b' => "booleans",
        'l' => "nulls",
        'o' => "objects",
        _ => "arrays",
    };
    let parts: Vec<String> = symbols
        .values
        .chars()
        .map(|c| match c {
            's' => "a string".to_owned(),
            'n' => "a number".to_owned(),
            'b' => "a boolean".to_owned(),
            'l' => "null".to_owned(),
            'o' => "an object".to_owned(),
            _ => {
                let items: Vec<&str> = symbols.items.chars().map(plural).collect();
                format!("an array of {}", items.join(" or "))
            }
        })
        .collect();
    parts.join(" or ")
}

/// `; did you mean `{prefix}{candidate}`?` for the closest candidate, or nothing.
fn did_you_mean(name: &str, candidates: &[&str], prefix: &str) -> String {
    match closest(name, candidates) {
        Some(s) => format!("; did you mean `{prefix}{s}`?"),
        None => String::new(),
    }
}

/// The candidate within edit distance 2 of `name`, if any.
pub(crate) fn closest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    fn distance(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.to_lowercase().chars().collect();
        let b: Vec<char> = b.to_lowercase().chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.iter().enumerate() {
            let mut cur = vec![i + 1; b.len() + 1];
            for (j, cb) in b.iter().enumerate() {
                cur[j + 1] = (prev[j] + usize::from(ca != cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1);
            }
            prev = cur;
        }
        prev[b.len()]
    }
    candidates
        .iter()
        .map(|c| (distance(name, c), *c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}
