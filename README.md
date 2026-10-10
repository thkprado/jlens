# jlens

A [JSONata](https://jsonata.org/) engine written in Rust. It filters, queries and transforms
JSON documents, and it can **explain** how a result was reached. It ships as:

- **`jlens-core`**: the Rust crate (with `jlens-schema` for metadata and autocomplete);
- **`jlens`**: the command-line tool;
- **`jlens-wasm`**: an ES module for the browser.

> **Status: the JSONata engine is complete (phase 6 of 8 done).** jlens passes 1,688 of the
> 1,690 cases of the official JSONata 2.2.2 test suite; the other two are documented
> overrides. Evaluations can be audited and explained, and schemas inferred from samples drive
> completion and diagnostics. The browser WebAssembly package (phase 7) is next. See the
> [roadmap](docs/src/architecture/roadmap.md) and the current
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

## Quick start

```sh
cargo install --path crates/jlens-cli

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
| `crates/jlens-cli` | `jlens` binary |
| `crates/jlens-wasm` | Browser bindings (`wasm-pack build --target web`) |
| `tests/jsonata-suite` | Vendored official test suite (jsonata-js 2.2.2, MIT) |
| `tests/conformance` | Baseline, overrides and conformance report |
| `fixtures` | Sample documents and expressions used in tests and guides |
| `docs` | The jlens book (mdBook) and decision records |
| `xtask` | Repository automation (`cargo xtask`) |

## Development

```sh
cargo test --workspace                 # unit tests + conformance baseline (regressions only)
cargo xtask conformance                # full suite run, writes tests/conformance/REPORT.md
cargo clippy --workspace --all-targets
mdbook build docs                      # requires mdbook
wasm-pack build crates/jlens-wasm --target web
```

## License

MIT. The vendored JSONata test suite and the documentation adapted from JSONata are © IBM Corp.
and JSONata contributors, also under the MIT license.
