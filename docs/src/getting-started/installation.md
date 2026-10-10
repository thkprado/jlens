# Installation

jlens comes in three forms that share one engine. Install the ones you need.

## Command line

The `jlens` binary is built from source with Rust 1.88 or later:

```sh
cargo install jlens-cli                          # from crates.io
cargo install --git https://github.com/thkprado/jlens jlens-cli
cargo install --path crates/jlens-cli            # from a checkout
```

```text
$ jlens --version
jlens 0.1.0
```

The [CLI guide](../guides/cli.md) describes every command.

## Rust crates

```toml
[dependencies]
jlens-core = "0.1"     # the language: parsing, evaluation, functions, auditing
jlens-schema = "0.1"   # optional: schema inference and the language service
serde_json = "1"
```

`jlens-core` depends on `serde_json`, `indexmap`, `regress` and a few small crates. The
optional `tracing` feature adds [`tracing`](https://docs.rs/tracing) spans for evaluations; see
the [audit guide](../guides/audit.md). The [crate guide](../guides/crate.md) shows how to use
the API.

## Browser and JavaScript

The WebAssembly package is the npm package `jlens-wasm`, an ES module:

```sh
npm install jlens-wasm
```

To build it from a checkout instead, you need:
- the `wasm32-unknown-unknown` Rust target;
- the `wasm-bindgen` CLI, at the version in `Cargo.lock`.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <version in Cargo.lock> --locked
cargo xtask wasm          # writes crates/jlens-wasm/pkg
```

The [WebAssembly guide](../guides/wasm.md) covers loading the module in browsers, bundlers and
Node.js.

## This book

The documentation is an [mdBook](https://rust-lang.github.io/mdBook/):

```sh
cargo install mdbook
mdbook serve docs        # http://localhost:3000
cargo xtask doc-check    # runs the examples and checks the links
```
