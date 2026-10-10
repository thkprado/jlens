# Contributing to jlens

## Setting up

You need Rust 1.88 or later (stable). Some tasks need more:

| Task | Needs |
|---|---|
| The WebAssembly package | the `wasm32-unknown-unknown` target; `wasm-bindgen-cli` at the version in `Cargo.lock`; Node.js 20+ |
| Browser tests | Chrome, Chromium or Edge (`JLENS_BROWSER` to choose) |
| The book | `mdbook` |
| Comparisons with jsonata-js | Node.js and `npm install` in `benches/compare` |

## Checks

CI runs the following; run them before sending a change:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets         # warnings are errors in CI
cargo test --workspace                         # includes the conformance baseline
cargo xtask gen-docs --check                   # generated pages are up to date
cargo xtask doc-check                          # the book's examples run, its links resolve
cargo xtask wasm-test                          # the browser package, in Node.js and a browser
```

Other useful tasks:

```sh
cargo xtask conformance                        # full suite run, writes tests/conformance/REPORT.md
cargo bench -p jlens-core                      # criterion benchmarks
PROPTEST_CASES=100000 cargo test -p jlens-core --test robustness
cargo xtask serve                              # serve the repository, for examples/web-editor
(cd benches/compare && npm install && node compare.mjs && node doc-examples.mjs)
```

## Guidelines

- **Conformance first.** jlens follows jsonata-js 2.2.2. A difference must be deliberate,
  explained in `docs/src/language/compatibility.md`, and, if it affects the suite, recorded in
  `tests/conformance/overrides.json` with a reason. The baseline in
  `tests/conformance/baseline.txt` may only grow.
- **Errors have codes.** New errors reuse the JSONata codes, or `U1001` with a message for
  limits; warnings use `W` codes (`crates/jlens-core/src/error.rs`).
- **Documentation is part of the change.** Examples in the book use `jsonata-example` blocks,
  which `cargo xtask doc-check` runs. Rust snippets in the crate guide come from
  `crates/jlens-core/tests/guide.rs`. Significant design decisions get a record in `docs/adr/`.
- **Commits** describe why, not only what.

## Releasing

1. Update the version in `[workspace.package]` and in the `jlens-core` and `jlens-schema`
   entries of `[workspace.dependencies]` (`Cargo.toml`).
2. Move the `Unreleased` entries of `CHANGELOG.md` under the new version.
3. Check that everything passes, including the packages:

   ```sh
   cargo package --workspace --exclude jlens-wasm --exclude xtask
   cargo xtask wasm-test --require-browser --example
   (cd crates/jlens-wasm/pkg && npm pack --dry-run)
   ```

4. Commit, tag `vX.Y.Z`, and push the tag. The `Release` workflow publishes `jlens-core`,
   `jlens-schema` and `jlens-cli` to crates.io, in that order, and `jlens-wasm` to npm. It needs
   the repository secrets `CARGO_REGISTRY_TOKEN` and `NPM_TOKEN`.

   To publish by hand instead: `cargo publish -p jlens-core`, then `-p jlens-schema`, then
   `-p jlens-cli`; then `npm publish --access public` in `crates/jlens-wasm/pkg` after
   `cargo xtask wasm`.

## License

By contributing, you agree that your contributions are licensed under the MIT license.
