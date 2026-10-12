# Installation

jlens comes in three forms that share one engine. Install the ones you need.

## Command line

Each [GitHub release](https://github.com/thkprado/jlens/releases) has the `jlens` binary for
Linux (x86-64 and ARM64, static), macOS (Intel and Apple silicon) and Windows (x86-64), with a
`SHA256SUMS` file. Download the archive for your platform and put `jlens` on your `PATH`, or let
[`cargo binstall`](https://github.com/cargo-bins/cargo-binstall) do it:

```sh
cargo binstall jlens-cli
```

From 0.6.0 on, each archive has a build attestation: a signed statement that the release
workflow of this repository built it, from which commit. Check one with the GitHub CLI:

```sh
gh attestation verify jlens-0.10.0-x86_64-unknown-linux-musl.tar.gz --repo thkprado/jlens
```

Elsewhere, build it from source with Rust 1.88 or later:

```sh
cargo install jlens-cli                          # from crates.io
cargo install --git https://github.com/thkprado/jlens jlens-cli
cargo install --path crates/jlens-cli            # from a checkout
```

```text
$ jlens --version
jlens 0.10.0
```

The [CLI guide](../guides/cli.md) describes every command.

## Rust crates

```toml
[dependencies]
jlens-core = "0.10"    # the language: parsing, evaluation, functions, auditing
jlens-schema = "0.10"  # optional: schema inference and the language service
serde_json = "1"
```

`jlens-core` depends on `serde_json`, `indexmap`, `jlens-regress` (a fork of `regress`) and a
few small crates. The
optional `tracing` feature adds [`tracing`](https://docs.rs/tracing) spans for evaluations; see
the [audit guide](../guides/audit.md). The [crate guide](../guides/crate.md) shows how to use
the API.

## Browser and JavaScript

The WebAssembly package is the npm package `jlens-wasm`, an ES module:

```sh
npm install jlens-wasm
```

From 0.6.0 on, the package is published with
[provenance](https://docs.npmjs.com/generating-provenance-statements): its page on npmjs.com
links it to the commit and the workflow run that built it, and `npm audit signatures` checks it
in a project that depends on it.

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
