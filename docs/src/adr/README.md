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
| [0006](0006-host-functions.md) | Host functions with a JSON boundary | Accepted; extended by [0011](0011-host-callbacks.md) |
| [0007](0007-language-server.md) | A language server on the language service, configured by `jlens.json` | Accepted |
| [0008](0008-cli-machine-contract.md) | A machine contract for the CLI: `--json` envelopes, exit codes, stdout for data | Accepted |
| [0009](0009-bounded-regex.md) | Regular expressions stop within the evaluation's limits, through a fork of `regress` | Accepted; the fork made permanent by [0020](0020-own-regress-fork.md) |
| [0010](0010-lazy-input.md) | Input documents converted lazily, one level at a time | Accepted |
| [0011](0011-host-callbacks.md) | Host functions call the functions they receive, through a handle that borrows the call | Accepted |
| [0012](0012-npm-distribution.md) | The CLI on npm: a launcher package and one package per platform | Accepted |
| [0013](0013-vscode-extension.md) | The VS Code extension: one package per platform with the release binary | Accepted |
| [0014](0014-python-package.md) | The Python package: PyO3 bindings that mirror the WebAssembly package | Accepted |
| [0016](0016-fields-read.md) | The fields an expression reads: an abstract evaluation that gives an upper bound, and its union | Accepted |
| [0017](0017-result-builder.md) | Results written straight into another representation, through a builder the core walks | Accepted |
| [0018](0018-step-limit.md) | A deterministic step limit: each evaluated node and callback is a step, fast paths charge what they skip, `U1006` | Accepted |
| [0019](0019-documents-from-rust-values.md) | Documents serialized straight from Rust values, as `serde_json::to_value` and `Document::new` would build them | Accepted |
| [0020](0020-own-regress-fork.md) | `jlens-regress` is jlens's own fork of `regress`: upstream fixes ported, not rebased | Accepted |
| [0021](0021-java-package.md) | The Java package: a C API called through Panama, for Java 25 | Accepted |
