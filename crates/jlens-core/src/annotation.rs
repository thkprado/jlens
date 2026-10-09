//! Audit annotations written as JSONata doc-comments.
//!
//! ```jsonata
//! /** @label "Authorized customer" @message "Login must be on the approved list" */
//! customer.login in $approved
//! ```
//!
//! A doc-comment starts with `/**` (but is not the empty comment `/**/`). Free text before the
//! first tag becomes the description. Tags are `@label`, `@message` and `@id`, each followed by
//! a JSON string literal. Other implementations see an ordinary comment, so annotated
//! expressions stay portable. See ADR 0005.

use serde::Serialize;

use crate::ast::Span;
use crate::error::Diagnostic;

/// Business metadata attached to a sub-expression through a doc-comment.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Annotation {
    /// Short name shown in audit trees and explanations (`@label`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Longer explanation of what the expression checks (`@message`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Stable identifier for referring to this check from other systems (`@id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Free text written before the first tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Byte span of the comment(s) this annotation was read from.
    pub span: Span,
}

impl Annotation {
    /// Merges a later annotation into this one; fields set in `later` win.
    pub(crate) fn merge(&mut self, later: Annotation) {
        if later.label.is_some() {
            self.label = later.label;
        }
        if later.message.is_some() {
            self.message = later.message;
        }
        if later.id.is_some() {
            self.id = later.id;
        }
        if later.description.is_some() {
            self.description = later.description;
        }
        self.span = self.span.to(later.span);
    }
}

/// A doc-comment as found by the lexer: the full span and the text between `/**` and `*/`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DocComment {
    pub span: Span,
    pub body_start: usize,
    pub body: String,
}

/// Parses a doc-comment into an annotation, reporting malformed tags as warnings.
pub(crate) fn parse_doc(doc: &DocComment, warnings: &mut Vec<Diagnostic>) -> Annotation {
    let mut annotation = Annotation {
        span: doc.span,
        ..Annotation::default()
    };
    let text = doc.body.as_str();
    let mut description = String::new();
    let mut i = 0;
    let bytes = text.as_bytes();

    // Free text before the first tag.
    while i < bytes.len() {
        if bytes[i] == b'@'
            && (i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'*')
        {
            break;
        }
        i += 1;
    }
    for line in text[..i].lines() {
        let line = line.trim().trim_start_matches('*').trim();
        if !line.is_empty() {
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str(line);
        }
    }
    if !description.is_empty() {
        annotation.description = Some(description);
    }

    // Tags.
    while i < bytes.len() {
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        let tag_start = i;
        i += 1;
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let tag = &text[name_start..i];
        let tag_span = Span::new(doc.body_start + tag_start, doc.body_start + i);
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        let value = if i < bytes.len() && bytes[i] == b'"' {
            match json_string_end(&bytes[i..]) {
                Some(len) => {
                    let literal = &text[i..i + len];
                    i += len;
                    serde_json::from_str::<String>(literal).ok()
                }
                None => None,
            }
        } else {
            None
        };
        let value = match value {
            Some(v) => v,
            None => {
                // Fall back to the bare text up to the end of the line or the next tag.
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'@' {
                    i += 1;
                }
                let bare = text[start..i]
                    .trim()
                    .trim_end_matches('*')
                    .trim()
                    .to_owned();
                if matches!(tag, "label" | "message" | "id") {
                    warnings.push(Diagnostic::warning("W0103", tag_span, &[("tag", tag)]));
                }
                bare
            }
        };
        match tag {
            "label" => annotation.label = Some(value),
            "message" => annotation.message = Some(value),
            "id" => annotation.id = Some(value),
            _ => warnings.push(Diagnostic::warning("W0102", tag_span, &[("tag", tag)])),
        }
    }
    annotation
}

/// Length of the JSON string literal at the start of `bytes` (including both quotes).
fn json_string_end(bytes: &[u8]) -> Option<usize> {
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> (Annotation, Vec<Diagnostic>) {
        let doc = DocComment {
            span: Span::new(0, body.len() + 5),
            body_start: 3,
            body: body.to_owned(),
        };
        let mut warnings = Vec::new();
        let a = parse_doc(&doc, &mut warnings);
        (a, warnings)
    }

    #[test]
    fn reads_tags_and_description() {
        let (a, w) = parse(
            "\n * Checks the customer.\n * @label \"Authorized customer\"\n * @message \"Login \\\"must\\\" match\" @id \"R-1\"\n ",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(a.description.as_deref(), Some("Checks the customer."));
        assert_eq!(a.label.as_deref(), Some("Authorized customer"));
        assert_eq!(a.message.as_deref(), Some("Login \"must\" match"));
        assert_eq!(a.id.as_deref(), Some("R-1"));
    }

    #[test]
    fn warns_on_unknown_tags_and_bare_values() {
        let (a, w) = parse(" @label Total above limit @severity \"high\" ");
        assert_eq!(a.label.as_deref(), Some("Total above limit"));
        let codes: Vec<_> = w.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["W0103", "W0102"]);
    }
}
