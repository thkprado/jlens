# Introduction

**jlens** evaluates [JSONata](https://jsonata.org/) expressions against JSON documents. You can
use it to filter, query and transform payloads, and to express business rules whose outcome must
be explained.

It ships in three forms that share a single engine:

| Distribution | Package | Use it for |
|---|---|---|
| Rust crate | `jlens-core` (+ `jlens-schema`) | Embedding the engine in Rust services |
| Command line | `jlens` binary | Scripts, pipelines, NDJSON filtering, debugging expressions |
| WebAssembly | `jlens-wasm` (npm package, ES module) | Browsers: evaluation, editors with autocomplete, audit viewers |

What sets jlens apart from other JSONata implementations:

- **Explainable evaluation.** jlens can record an *audit tree* of how an expression reached its
  result: which comparisons passed or failed, which items a filter rejected, and which values
  were bound. You can attach business labels through ordinary JSONata comments
  ([annotations](language/annotations.md)), so the expression stays valid in any other JSONata
  implementation.
- **Schema-driven assistance.** jlens infers structural metadata from sample documents. It then
  uses that metadata to offer completion, diagnostics and hover information for expressions
  being written.
- **Conformance first.** The engine is checked against the official JSONata test suite (2.2.2)
  on every change; see [Conformance testing](architecture/conformance.md).

> **Status:** jlens 0.1 implements all of JSONata 2.2 (1,688 of the 1,690 official test cases,
> with the other two documented), evaluation audits, schema-driven editor support, the
> command line and the browser package. See the [roadmap](architecture/roadmap.md).
