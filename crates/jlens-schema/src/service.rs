//! The language service: completion, diagnostics, hover and signature help for JSONata
//! expressions, driven by a [`Schema`].
//!
//! Offsets and spans are byte offsets into the expression text. Editors that count in UTF-16
//! (browsers) convert at the boundary; the Wasm package does that.

use jlens_core::ast::{Node, Span};
use jlens_core::{Diagnostic, Severity, parse_recover};
use serde::Serialize;

use crate::shape::{Field, Schema, Shape};
use crate::typing::{Event, Scope, Ty, Walker};

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
    /// A built-in function.
    Function,
    /// `true`, `false`, `null`.
    Keyword,
    /// An operator (`and`, `or`, `in`, ...).
    Operator,
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
pub struct Completions {
    /// The byte range the chosen item replaces (the word under the cursor).
    pub range: Span,
    /// The suggestions, best first.
    pub items: Vec<CompletionItem>,
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
    schema: Option<Schema>,
    variables: Vec<(String, Option<Shape>)>,
}

/// Lexical situation at an offset.
#[derive(Debug, PartialEq)]
enum Lexical {
    Code,
    /// Inside a string literal or a comment.
    Literal,
    /// Inside a backquoted name that starts at this offset.
    Backquote(usize),
}

fn lexical_state(text: &str, offset: usize) -> Lexical {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < offset.min(bytes.len()) {
        match bytes[i] {
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= offset {
                    return Lexical::Literal;
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
                _ => return Lexical::Literal,
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
fn field_insert(name: &str) -> String {
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

fn shape_docs(shape: &Shape, field: Option<&Field>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(label) = &shape.label {
        parts.push(format!("**{label}**"));
    }
    if let Some(d) = &shape.description {
        parts.push(d.clone());
    }
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
}

impl LanguageService {
    /// A service for expressions over documents described by `schema` (or without a schema:
    /// functions and variables are still completed).
    pub fn new(schema: Option<Schema>) -> Self {
        LanguageService {
            schema,
            variables: Vec::new(),
        }
    }

    /// Declares a variable the expression is evaluated with (a binding), optionally with the
    /// shape of its value.
    pub fn with_variable(mut self, name: impl Into<String>, shape: Option<Shape>) -> Self {
        self.variables.push((name.into(), shape));
        self
    }

    fn root_ty(&self) -> Ty<'_> {
        match &self.schema {
            Some(s) => Ty::of(&s.root),
            None => Ty::unknown(),
        }
    }

    fn externals(&self) -> Vec<(String, Ty<'_>)> {
        self.variables
            .iter()
            .map(|(n, s)| (n.clone(), s.as_ref().map_or_else(Ty::unknown, Ty::of)))
            .collect()
    }

    /// Walks `root`, calling `visit` for each field reference and variable.
    fn walk<'s>(&'s self, root: &Node, visit: &mut dyn FnMut(Event<'_, 's>)) {
        let externals = self.externals();
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
        };
        match lexical_state(text, offset) {
            Lexical::Literal => return empty(Span::new(offset, offset)),
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
            return Completions { range, items };
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
        if variable_mode {
            items.extend(self.variable_items(&probe.scope, &prefix[1..]));
            items.extend(function_items(&prefix[1..]));
        } else {
            items.extend(self.field_items(probe.ctx.as_ref(), prefix));
            if !after_dot {
                items.extend(self.variable_items(&probe.scope, prefix));
                items.extend(function_items(prefix));
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
        Completions { range, items }
    }

    /// Parses `modified` (which contains the placeholder) and finds the placeholder's context.
    fn probe<'s>(&'s self, modified: &str, variable: bool) -> Probe<'s> {
        let ast = parse_recover(modified);
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
        let mut fields: Vec<(&str, &Field)> = ctx
            .fields()
            .into_iter()
            .filter(|(name, f)| !f.shape.hidden && name.to_lowercase().starts_with(&lower))
            .collect();
        // Required fields first, then by how often they occur; ties keep schema order.
        fields.sort_by(|a, b| {
            b.1.required
                .cmp(&a.1.required)
                .then(b.1.presence.total_cmp(&a.1.presence))
        });
        fields
            .into_iter()
            .map(|(name, f)| CompletionItem {
                label: name.to_owned(),
                kind: CompletionKind::Field,
                detail: Some(f.shape.type_summary()),
                documentation: shape_docs(&f.shape, Some(f)),
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
                    [shape] => Some(shape.type_summary()),
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

    /// Syntax errors, annotation warnings, and references to fields the schema does not have.
    pub fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        let ast = parse_recover(text);
        let mut out = ast.diagnostics.clone();
        if self.schema.is_some() {
            let mut visit = |event: Event<'_, '_>| {
                if let Event::Field {
                    node, name, ctx, ..
                } = event
                    && ctx.is_known_objects()
                    && ctx.fields().iter().all(|(n, _)| *n != name)
                {
                    let known: Vec<&str> = ctx.fields().iter().map(|(n, _)| *n).collect();
                    let hint = match closest(name, &known) {
                        Some(s) => format!("; did you mean `{s}`?"),
                        None => String::new(),
                    };
                    let quoted = text[node.span.start..].starts_with('`');
                    let len = name.len() + if quoted { 2 } else { 0 };
                    let span =
                        Span::new(node.span.start, (node.span.start + len).min(node.span.end));
                    out.push(Diagnostic::new(
                        Severity::Warning,
                        "W0201",
                        span,
                        &[("token", name), ("hint", &hint)],
                    ));
                }
            };
            self.walk(&ast.root, &mut visit);
        }
        out.sort_by_key(|d| d.span.start);
        out
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
                    .find(|(n, _)| *n == name)
                    .map(|(_, f)| f);
                let contents = match field {
                    Some(f) => {
                        let mut s = format!("`{name}`: {}", f.shape.type_summary());
                        if let Some(docs) = shape_docs(&f.shape, Some(f)) {
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
                let contents = match jlens_core::function_catalog()
                    .iter()
                    .find(|f| f.name == name)
                {
                    Some(f) => format!("`${}{}`\n\n{}", f.name, f.signature, f.summary),
                    None => match ty.shapes.as_slice() {
                        [shape] => format!("`${name}`: {}", shape.type_summary()),
                        _ => format!("`${name}`"),
                    },
                };
                consider(node.span, contents);
            }
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
                    let f = jlens_core::function_catalog()
                        .into_iter()
                        .find(|f| f.name == name)?;
                    return Some(SignatureHelp {
                        name: format!("${name}"),
                        signature: f.signature.clone(),
                        params: f.params.iter().map(|p| format!("${p}")).collect(),
                        active_param: commas,
                        summary: f.summary.to_owned(),
                    });
                }
                b',' if depth == 0 => commas += 1,
                _ => {}
            }
        }
        None
    }
}

fn function_items(prefix: &str) -> Vec<CompletionItem> {
    jlens_core::function_catalog()
        .into_iter()
        .filter(|f| f.name.starts_with(prefix))
        .map(|f| CompletionItem {
            label: format!("${}", f.name),
            kind: CompletionKind::Function,
            detail: Some(f.signature.clone()),
            documentation: Some(f.summary.to_owned()),
            insert_text: format!("${}", f.name),
        })
        .collect()
}

/// The candidate within edit distance 2 of `name`, if any.
fn closest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
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
