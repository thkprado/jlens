# Architecture overview

```text
                ┌───────────────────────── jlens-core ──────────────────────────┐
source ──► lexer ──► Pratt parser ──► raw tree ──► process ──► Ast (Expression)  │
                │      (recover mode, annotations)    (paths, stages, %,         │
                │                                      tail calls)       │      │
                │                                                        ▼      │
input JSON ──► Value / Document ───────────────────────────────────► evaluator  │
                │                                    function library ──┘  │    │
                │                                    Tracer events ◄───────┘    │
                │                                       │                       │
                │                     AuditCollector ◄──┴──► JsonlTracer, ...   │
                └──────────────────────────┬───────────────────────────────────┘
                                           │ result, AuditTree, Explanation
        jlens-schema: Schema (inference, JSON Schema) ─► typing ─► LanguageService
        jlens-cli: the `jlens` binary        jlens-wasm: the ES module for browsers
```

## Crates

| Crate | Contents | Depends on |
|---|---|---|
| `jlens-core` | Lexer, parser, syntax tree, evaluator, function library, errors, audit subsystem | serde_json, indexmap, jlens-regress (a fork of regress), ryu-js |
| `jlens-schema` | Structural metadata (`Schema`), inference, JSON Schema import and export, static typing of expressions, the language service | jlens-core |
| `jlens-cli` | The `jlens` command: `eval`, `check`, `functions`, `schema`, `ast` | jlens-core, jlens-schema, clap |
| `jlens-wasm` | WebAssembly bindings: `Expression`, `Document`, `LanguageService`, … with TypeScript declarations | jlens-core, jlens-schema, wasm-bindgen |
| `xtask` | Repository automation: conformance, generated docs, doc checks, the Wasm package | jlens-core |

The core crate has no dependency on the others. Everything a tool needs from the language is
public there:
- the syntax tree, with node ids and spans;
- recovering parsing;
- the function catalog;
- the tracer events.

## From source to result

1. **Lexing and parsing** ([Parsing](parser.md)). The lexer produces tokens with byte spans and
   collects documentation comments. A Pratt parser, ported from jsonata-js, builds a raw tree.
   In recover mode it records errors and keeps going, which editors need. Annotations are
   attached to the subexpression that follows them.
2. **Processing**. The raw tree becomes the `Ast`:
   - path steps and their stages (filters, sorting, `#`, `@`) are laid out;
   - parent references (`%`) are resolved to ancestor slots;
   - tail calls are marked.
   The result is immutable and shared by all evaluations of the expression.
3. **Evaluation** ([Evaluation](evaluator.md)). A tree-walking interpreter over the `Value`
   model. Values are reference counted, with sequences flagged as in jsonata-js; see
   [ADR 0003](../adr/0003-value-model.md). Functions are values. Tail calls run in a
   trampoline. Guardrails bound the depth and the time.
4. **Tracing**. When a tracer is attached, the evaluator reports events: entering and leaving a
   node, operands, short circuits, branches, bindings and filters. The audit collector turns
   them into an `AuditTree`, and `explain` summarizes the tree. See
   [ADR 0004](../adr/0004-audit-tracer.md).

## Tooling

`jlens-schema` builds on recovering parsing ([Schema and language service](language-service.md)):

- `Schema` records the shape of sample documents and round-trips through JSON Schema.
- The *typing walker* follows an expression the way the evaluator would. It computes what
  each part of the expression refers to in the schema: paths, filters, variables and function
  results.
- The `LanguageService` answers completion, diagnostics, hover and signature help from that
  information. A placeholder is substituted at the cursor so that incomplete expressions parse.

The CLI and the WebAssembly package are thin layers over these crates. The WebAssembly package
converts offsets between UTF-8 bytes (Rust) and UTF-16 code units (JavaScript) at the boundary.

## Quality

- [Conformance testing](conformance.md): the official JSONata test suite runs on every
  `cargo test`, against a baseline that may only improve. Property tests check that nothing
  panics.
- [Performance](performance.md): criterion benchmarks and a comparison with jsonata-js.
- Documentation checks: `cargo xtask doc-check` runs every example in this book. The
  generated pages (functions, errors) are checked against the code.

The design decisions are recorded in the [decision records](../adr/README.md).
