# Roadmap

jlens is built in phases. A phase is complete only when its documentation is written.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 0 | Workspace, CI, documentation skeleton, conformance harness | CI green; conformance report | **done** |
| 1 | Lexer, Pratt parser, AST post-processing, spans, recover mode, annotations; `jlens ast` / `jlens check` | Syntax groups of the suite and the parser-recovery cases pass | **done** |
| 2 | Value model and evaluator (paths, sequences, predicates, operators, lambdas, closures, tail calls, sort, group-by, parent operator, transform, limits) | Suite groups that don't depend on functions pass | planned |
| 3 | Function library, signatures, regex, date/time and number formatting | 100% of the suite minus documented overrides | planned |
| 4 | Evaluation logging and auditing (tracer, audit tree, levels, annotations, explanations) | Snapshot tests; under 3% overhead with the tracer off | planned |
| 5 | `jlens-schema`: inference, overrides, JSON Schema, static typing, completion/diagnostics/hover | Table-driven language-service tests | planned |
| 6 | Full CLI | CLI snapshot tests | planned |
| 7 | WebAssembly package (browser ESM), TypeScript types, web editor example | Headless browser tests; working demo | planned |
| 8 | Documentation consolidation, benchmarks, publishing | Doc examples verified; link check green | planned |

Phases 4 and 5 can run in parallel once phase 3 is done. Phase 5 only needs the parser from
phase 1 to start.
