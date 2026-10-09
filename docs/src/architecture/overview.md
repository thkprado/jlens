# Architecture overview

```text
source ──► lexer ──► Pratt parser ──► raw AST ──► process (paths, stages, %, tail calls) ──► AST
                                                                                        │
input JSON ──► Value ──────────────────────────────────────────────► evaluator ◄────────┘
                                                                         │  ▲
                                                           Tracer events │  │ function registry
                                                                         ▼  │
                                                                result + optional AuditTree
```

The workspace has four crates:

- **`jlens-core`** holds the language: lexer, parser, AST, evaluator, function library, errors,
  and the tracer and audit subsystem.
- **`jlens-schema`** holds structural metadata and the language service (completion,
  diagnostics, hover). It depends on the core parser's recover mode.
- **`jlens-cli`** is the `jlens` binary.
- **`jlens-wasm`** provides the browser bindings, built with `wasm-bindgen`.

Later phases will fill in the detailed chapters on each stage. The design decisions recorded so
far are in the [decision records](../adr/README.md).
