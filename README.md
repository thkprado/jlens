# jlens

A [JSONata](https://jsonata.org/) engine written in Rust. It filters, queries and transforms
JSON documents, and it can **explain** how a result was reached. It ships as:

- **`jlens-core`**: the Rust crate (with `jlens-schema` for metadata and autocomplete);
- **`jlens`**: the command-line tool, which includes a language server for editors
  (`jlens lsp`);
- **`jlens-wasm`**: an ES module for the browser (npm package `jlens-wasm`).

> **Status: version 0.2.0.** jlens passes 1,688 of the 1,690 cases of the
> official JSONata 2.2.2 test suite; the other two are documented overrides. Evaluations can be
> audited and explained, schemas inferred from samples drive completion and diagnostics, and
> everything is available in the browser as a WebAssembly ES module. See the
> [changelog](CHANGELOG.md), the [roadmap](docs/src/architecture/roadmap.md), the
> [performance measurements](docs/src/architecture/performance.md) and the current
> [conformance report](tests/conformance/REPORT.md).

## Highlights

- Full JSONata 2.2 semantics, including every built-in function, checked against the official
  test suite.
- Audit trees and explanations of evaluations (`jlens eval --explain`). Business labels go in
  ordinary comments, so expressions stay portable:

  ```jsonata
  /** @label "Minimum order grand total" */
  $sum(items.(unitPrice * quantity)) > 50000
  ```

- Structural metadata inferred from sample documents (JSON Schema 2020-12), which drives
  completion, diagnostics and hover (`jlens schema infer`, `jlens check --schema`).
- A browser ES module with TypeScript types. The `examples/web-editor` demo is a JSONata editor
  with completion, live results, explanations and an audit tree viewer.

## Quick start

```sh
cargo install jlens-cli                 # or: cargo install --path crates/jlens-cli

jlens eval -f fixtures/shipping-rule.jsonata -i fixtures/online-store-order.json --explain
jlens eval --ndjson --filter -i fixtures/orders.ndjson '$sum(items.(unitPrice * quantity)) > 50000'
jlens schema infer fixtures/online-store-order.json -o schema.json
jlens check --schema schema.json -f fixtures/shipping-rule.jsonata
```

## Repository layout

| Path | Contents |
|---|---|
| `crates/jlens-core` | Language engine and audit subsystem |
| `crates/jlens-schema` | Metadata inference and language service |
| `crates/jlens-lsp` | Language server (`jlens lsp`) |
| `crates/jlens-cli` | `jlens` binary |
| `crates/jlens-wasm` | Browser bindings (`cargo xtask wasm`) |
| `tests/jsonata-suite` | Vendored official test suite (jsonata-js 2.2.2, MIT) |
| `tests/conformance` | Baseline, overrides and conformance report |
| `fixtures` | Sample documents and expressions used in tests and guides |
| `examples/web-editor` | Browser demo: JSONata editor with autocomplete, explanations and audit trees |
| `benches` | Benchmark workload, and comparisons with jsonata-js (`benches/compare`) |
| `docs` | The jlens book (mdBook) and decision records |
| `xtask` | Repository automation (`cargo xtask`) |

## Development

```sh
cargo test --workspace                 # unit tests + conformance baseline (regressions only)
cargo xtask conformance                # full suite run, writes tests/conformance/REPORT.md
cargo clippy --workspace --all-targets
mdbook build docs                      # requires mdbook
cargo xtask wasm                       # browser package in crates/jlens-wasm/pkg (needs wasm-bindgen-cli)
cargo xtask wasm-test                  # its tests, in Node.js and a headless browser
cargo xtask serve                      # serve the repository; open /examples/web-editor/
cargo xtask doc-check                  # run the book's examples, check its links
cargo bench -p jlens-core              # benchmarks
```

[CONTRIBUTING.md](CONTRIBUTING.md) has the full list of checks and the release process.

## License

MIT. The vendored JSONata test suite, and the code and documentation ported or adapted from
JSONata, are also under the MIT license; see [LICENSE](LICENSE).
