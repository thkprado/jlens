# Decision records

Architecture decision records (ADRs) capture significant design choices: the context, the
decision, and its consequences. They are numbered and never rewritten. A later ADR supersedes an
earlier one instead.

| # | Decision | Status |
|---|---|---|
| [0001](0001-own-implementation.md) | Implement JSONata ourselves, porting jsonata-js 2.2.2 | Accepted |
| [0002](0002-regress.md) | Use `regress` for ECMAScript-compatible regular expressions | Accepted |
| [0003](0003-value-model.md) | An explicit value model with `Undefined` and sequence flags | Accepted |
| [0004](0004-audit-tracer.md) | Audit evaluations through a tracer interface in the evaluator | Accepted |
| [0005](0005-comment-annotations.md) | Carry audit labels in JSONata doc-comments | Accepted |
| [0006](0006-host-functions.md) | Host functions with a JSON boundary | Accepted |
