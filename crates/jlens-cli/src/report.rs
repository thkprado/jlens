//! Human-readable rendering of diagnostics, with the offending source line and a caret.

use std::fmt::Write as _;

use jlens_core::{Diagnostic, Severity};

/// Renders a diagnostic like:
///
/// ```text
/// error[S0202]: Expected "]", got ";"
///   --> <expr>:1:16
///    |
///  1 | Account.Order[0;].Product
///    |                ^
/// ```
pub fn render(source: &str, name: &str, d: &Diagnostic) -> String {
    let severity = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let start = d.span.start.min(source.len());
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |i| start + i);
    let line_text = source[line_start..line_end].trim_end_matches('\r');
    let (line_no, column) = location(source, start);
    let end = d.span.end.clamp(start, line_end);
    let width = source[start..end].chars().count().max(1);
    let gutter = line_no.to_string().len();

    let mut out = String::new();
    let _ = writeln!(out, "{severity}[{}]: {}", d.code, d.message);
    let _ = writeln!(out, "{:gutter$}--> {name}:{line_no}:{column}", "");
    let _ = writeln!(out, "{:gutter$} |", "");
    let _ = writeln!(out, "{line_no} | {line_text}");
    let _ = writeln!(
        out,
        "{:gutter$} | {}{}",
        "",
        " ".repeat(column - 1),
        "^".repeat(width)
    );
    out
}

/// The 1-based line and column of a byte offset in `source`, the column counted in characters.
pub fn location(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let line_start = source[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = source[..line_start].matches('\n').count() + 1;
    let column = source[line_start..offset].chars().count() + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_at_the_offending_token() {
        let ast = jlens_core::parse_recover("Account.Order[0;].Product");
        let text = render("Account.Order[0;].Product", "<expr>", &ast.diagnostics[0]);
        assert_eq!(
            text,
            "error[S0202]: Expected \"]\", got \";\"\n --> <expr>:1:16\n  |\n1 | Account.Order[0;].Product\n  |                ^\n"
        );
    }

    #[test]
    fn handles_later_lines_and_end_of_input() {
        let src = "(\n  $a := 1;\n  $a +\n)";
        let ast = jlens_core::parse_recover(src);
        let text = render(src, "rule.jsonata", &ast.diagnostics[0]);
        assert!(text.contains("--> rule.jsonata:4:1"), "{text}");
    }
}
