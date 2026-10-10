# Roadmap

jlens is built in phases. A phase is complete only when its documentation is written.

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
| 8 | Documentation consolidation, benchmarks, publishing | Doc examples verified; link check green | **done** (publishing pending) |

Phases 4 and 5 can run in parallel once phase 3 is done. Phase 5 only needs the parser from
phase 1 to start.

## After 0.1

Ideas that came up during development, not commitments:

- Custom functions registered from JavaScript in the Wasm package. Rust applications already
  have [host functions](../guides/crate.md#host-functions) ([ADR 0006](../adr/0006-host-functions.md)).
- Lazy conversion of input documents, so that evaluations which read a small part of a large
  document do not pay for converting all of it.
- Asynchronous functions (jsonata-js allows functions that return promises).
- A language server (LSP) built on `jlens-schema`, for editors other than the browser.
