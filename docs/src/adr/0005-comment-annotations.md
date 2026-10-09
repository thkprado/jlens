# 0005 — Carry audit labels in JSONata doc-comments

**Status:** Accepted

## Context

Business users need audit output in their own terms. They want to read "Authorized customer:
failed", not "binary `in` at 12..48 returned false". The original DSL had `label` and `message`
fields on every condition. JSONata has no syntax for metadata, and adding new syntax would make
expressions unportable to other JSONata implementations.

## Decision

Annotations are written as JSONata block comments that start with `/**`. Each annotation
attaches to the sub-expression that immediately follows it:

```jsonata
/** @label "Authorized customer" @message "Login must be on the approved list" */
customer.login in $approved
```

- The lexer keeps comments as trivia, with spans. The parser attaches a doc-comment to the next
  AST node it produces.
- Recognised tags are `@label`, `@message` and `@id`. Tag values are JSON strings. Unknown tags
  produce a diagnostic warning, not an error.
- Plain `/* ... */` comments remain comments and carry no meaning.

## Consequences

- Annotated expressions are still valid JSONata, and every other implementation ignores the
  annotations.
- Labels appear in the `AuditTree`, in explanations, and in editor hovers.
- Annotation placement depends on the parse. The annotation documentation will specify precisely
  which node a comment attaches to, including for operators and for `and`/`or` chains.
