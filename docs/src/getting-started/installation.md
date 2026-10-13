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
gh attestation verify jlens-0.14.0-x86_64-unknown-linux-musl.tar.gz --repo thkprado/jlens
```

Elsewhere, build it from source with Rust 1.88 or later:

```sh
cargo install jlens-cli                          # from crates.io
cargo install --git https://github.com/thkprado/jlens jlens-cli
cargo install --path crates/jlens-cli            # from a checkout
```

```text
$ jlens --version
jlens 0.14.0
```

On macOS and Linux, Homebrew installs the same binary from the tap
[`thkprado/homebrew-jlens`](https://github.com/thkprado/homebrew-jlens) (from 0.14.0 on), and
`brew upgrade` follows the releases:

```sh
brew install thkprado/jlens/jlens
```

On Windows, Scoop installs it from the bucket
[`thkprado/scoop-jlens`](https://github.com/thkprado/scoop-jlens) (from 0.14.0 on), and
`scoop update jlens` follows the releases:

```sh
scoop bucket add jlens https://github.com/thkprado/scoop-jlens
scoop install jlens/jlens
```

In a JavaScript project, the same binary comes from npm, in the package `jlens-cli` (from
0.14.0 on):

```sh
npx jlens-cli --version              # run it without installing
npm install --save-dev jlens-cli     # or add it to the project; npm scripts then run `jlens`
```

`jlens-cli` runs the release binary for your platform, which npm installs from one of its
optional dependencies, `jlens-cli-<os>-<cpu>`, with no install script. Each run starts Node.js
first, which adds some tens of milliseconds. An install with `--omit=optional` leaves the binary
out, and so does a platform without a release binary (Windows on ARM64, for one): `jlens` then
says which package it looked for and exits with code 2.

With Docker or Podman, the container image `ghcr.io/thkprado/jlens` (from 0.14.0 on) runs the
static Linux binary, for `linux/amd64` and `linux/arm64`, with nothing else in the image. It is
tagged with each version (`0.14.0`), each minor version (`0.14`) and `latest`. Give it `-i` to
read standard input, or mount the current directory on `/work` to read files:

```sh
echo '{"a": [1, 2]}' | docker run --rm -i ghcr.io/thkprado/jlens eval '$sum(a)'
docker run --rm -v "$PWD:/work" ghcr.io/thkprado/jlens eval -f rule.jsonata -i order.json
```

The image runs as an unprivileged user (UID 65532): add `--user "$(id -u):$(id -g)"` for files
that only you can read. It has a build attestation too:

```sh
gh attestation verify oci://ghcr.io/thkprado/jlens:0.14.0 --repo thkprado/jlens
```

Install the command through one channel. With more than one, the first `jlens` on the `PATH`
runs, whatever its version; inside npm scripts and `npx`, the project's own `jlens-cli` comes
first. To see which one runs:

```sh
jlens --version
which -a jlens       # Linux and macOS: every jlens on the PATH, the one that runs first
where.exe jlens      # Windows
```

The [CLI guide](../guides/cli.md) describes every command.

## Rust crates

```toml
[dependencies]
jlens-core = "0.14"    # the language: parsing, evaluation, functions, auditing
jlens-schema = "0.14"  # optional: schema inference and the language service
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
