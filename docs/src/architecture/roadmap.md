# Roadmap

## How 0.1 was built

jlens was built in phases. A phase was complete only when its documentation was written.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 0 | Workspace, CI, documentation skeleton, conformance harness | CI green; conformance report | **done** |
| 1 | Lexer, Pratt parser, AST post-processing, spans, recover mode, annotations; `jlens ast` / `jlens check` | Syntax groups of the suite and the parser-recovery cases pass | **done** |
| 2 | Value model and evaluator (paths, sequences, predicates, operators, lambdas, closures, tail calls, sort, group-by, parent operator, transform, limits) | Suite groups that don't depend on functions pass | **done** |
| 3 | Function library, signatures, regex, date/time and number formatting | 100% of the suite minus documented overrides | **done** |
| 4 | Evaluation logging and auditing (tracer, audit tree, levels, annotations, explanations) | Snapshot tests; under 3% overhead with the tracer off | **done** |
| 5 | `jlens-schema`: inference, overrides, JSON Schema, static typing, completion/diagnostics/hover | Table-driven language-service tests | **done** |
| 6 | Full CLI | CLI snapshot tests | **done** |
| 7 | WebAssembly package (browser ESM), TypeScript types, web editor example | Headless browser tests; working demo | **done** |
| 8 | Documentation consolidation, benchmarks, publishing | Doc examples verified; link check green; published to crates.io and npm | **done** (0.1.0) |

## Releases

The [changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md) lists every change.

| Version | Highlights |
|---|---|
| 0.1.0 | The first release: the engine, audits, schemas and the language service, the CLI and the WebAssembly package |
| 0.1.1 | Examples, fixtures and the web editor use an online store order |
| 0.2.0 | Host functions in Rust and WebAssembly ([ADR 0006](../adr/0006-host-functions.md)), and their declarations in the language service; the web editor's example selector |
| Unreleased | The language server `jlens lsp` and `jlens.json` ([ADR 0007](../adr/0007-language-server.md), [editors guide](../guides/editors.md)) |

## Ideas

Not commitments, roughly from the smallest to the largest:

- **A VS Code extension** that starts `jlens lsp` for `.jsonata` files, with a TextMate grammar
  for syntax highlighting. The other editors with an LSP client need no extension.
- **Host functions that call back** the JSONata functions they receive as arguments, through a
  callable handle. This extends the host function API of ADR 0006 without changing the
  functions already written against it.
- **Lazy conversion of input documents**, so that evaluations that read a small part of a large
  document do not pay for converting all of it. This changes the value model
  ([ADR 0003](../adr/0003-value-model.md)).
- **Asynchronous functions**, as in jsonata-js, whose functions may return promises. This needs
  an evaluator that can suspend, and an ADR first. Until then, a host function that returns a
  promise fails with `U1002`.
