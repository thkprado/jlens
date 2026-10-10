# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Language server: `jlens lsp` (crate `jlens-lsp`) offers completion, diagnostics, hover and
  signature help to editors. A `jlens.json` per workspace folder maps file patterns to a
  schema, variables and host functions. See the editors guide and ADR 0007.
- Reproducible evaluations (`jlens-core`): `Options::now` fixes the timestamp of `$now()` and
  `$millis()`, and `Options::seed` seeds `$random()` and `$shuffle()`.
- `jlens eval --json` prints one JSON envelope per evaluation on standard output:
  `{"ok":true,"result":…}`, `{"ok":true,"undefined":true}` or `{"ok":false,"error":{…}}`,
  with the error's code, message, position, line and column. An input document that is not
  valid JSON is reported with the new code `U2001`. See ADR 0008.

### Changed

- `Options` (`jlens-core`) is `#[non_exhaustive]`, so new options can be added without a
  breaking change. Build it with `Options::new().timeout(..).max_depth(..)` instead of a struct
  literal; its fields stay public.

## [0.2.0]

### Added

- Host functions (`jlens-core`): the application provides its own functions through
  `Bindings::function`, with an optional signature that is checked like a built-in function's.
  They work with higher-order functions, partial application and `~>`, and take precedence over
  built-in functions of the same name. See ADR 0006.
- Language service (`jlens-schema`): `LanguageService::with_function` declares host functions
  (`FunctionDecl`), which completion, hover and signature help show like built-in functions.
- WebAssembly: JavaScript functions in the bindings, and `Expression.registerFunction`, are
  called as host functions. Their exceptions are rethrown as is; a function that returns a
  promise fails with the new code `U1002`. `LanguageService` takes host function declarations
  as a third argument.
- Web editor: an example selector, with the samples of try.jsonata.org (Invoice, Address, Schema,
  Library and an adapted Bindings) next to the store order rule. The chosen example is kept in
  the address (`#invoice`).

### Changed

- Language service: a call to a declared variable or host function that has the name of a
  built-in function is no longer typed as the built-in's result.
- WebAssembly: bindings that are a function, rather than an object, are a `TypeError`.

## [0.1.1]

No change to the language or the APIs.

### Changed

- The examples in the API documentation (`jlens-core` annotations, the `jlens-schema` overview),
  the book, the fixtures and the web editor use an online store order.
- `cargo xtask wasm-test` prints the browser's console when the browser tests hang.

### Fixed

- Web editor: the text cursor is visible in dark mode.

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

[Unreleased]: https://github.com/thkprado/jlens/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/thkprado/jlens/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/thkprado/jlens/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/thkprado/jlens/releases/tag/v0.1.0
