# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0]

The first release.

### Language (`jlens-core`)

- A complete JSONata 2.2 engine, ported from jsonata-js 2.2.2: paths, predicates, sorting,
  grouping, the parent operator, positional and context bindings, functions and closures,
  partial application, function composition, transforms, regular expressions (ECMAScript
  syntax), and every built-in function. It passes 1,688 of the 1,690 cases of the official test
  suite; the other two are documented overrides.
- Recovering parsing (`parse_recover`) with diagnostics, and a public syntax tree with node ids
  and spans.
- Evaluation audits: a `Tracer` trait, audit trees at the `decisions` or `full` level with
  capture bounds, explanations, JSON Lines event logs, and an optional `tracing` integration.
  Labels and messages for checks come from `/** @label ... @message ... */` annotations.
- `Document`, to convert an input once and evaluate many expressions against it.
- Guardrails: timeouts, depth limits, a nesting limit, and stack checks, so deep recursion
  fails with an error instead of crashing.

### Schemas and editor support (`jlens-schema`)

- Schema inference from sample documents, overrides, and JSON Schema 2020-12 import and export.
- A language service: completion, diagnostics (including unknown fields, with suggestions),
  hover and signature help.

### Command line (`jlens-cli`)

- `jlens eval` (with NDJSON streaming, `--filter`, `--explain`, `--audit` and `--trace-log`),
  `jlens check`, `jlens functions`, `jlens schema infer` and `jlens schema complete`, and
  `jlens ast`.

### Browser (`jlens-wasm` on npm)

- An ES module with TypeScript declarations: `Expression`, `Document`, `LanguageService`,
  `inferSchema`, `parseRecover` and `functions`. Offsets are in UTF-16 code units.
- `examples/web-editor`: a CodeMirror 6 editor with completion, diagnostics, live results,
  explanations and an audit tree viewer.

### Documentation

- The jlens book: a language reference with verified examples, and guides for the CLI, the
  crates, WebAssembly, auditing and schemas, plus the architecture, performance and decision
  records.

[Unreleased]: https://github.com/thkprado/jlens/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/thkprado/jlens/releases/tag/v0.1.0
