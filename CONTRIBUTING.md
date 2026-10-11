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
cargo semver-checks -p jlens-core -p jlens-schema  # the API against the last release on crates.io
```

The last check fails on a breaking change while `Cargo.toml` still carries the version of the
last release. A breaking change is fine before 1.0, but it is deliberate: the change that
introduces the first one after a release also raises the minor version in `Cargo.toml` (step 1
of [Releasing](#releasing)), and records the break in the `Changed` section of `CHANGELOG.md`.

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

1. Update the version in `[workspace.package]` and in the `jlens-core`, `jlens-schema` and
   `jlens-lsp` entries of `[workspace.dependencies]` (`Cargo.toml`).
2. Move the `Unreleased` entries of `CHANGELOG.md` under the new version, and open the section
   with a sentence that sums up the release. The section becomes the GitHub release notes;
   preview them with `cargo xtask release-notes X.Y.Z`.
3. Check that everything passes, including the packages:

   ```sh
   cargo package --workspace --exclude jlens-wasm --exclude xtask
   cargo xtask wasm-test --require-browser --example
   (cd crates/jlens-wasm/pkg && npm pack --dry-run)
   ```

4. Commit, tag `vX.Y.Z`, and push the tag. The `Release` workflow first builds the `jlens`
   binaries for Linux, macOS and Windows (CI builds them on each push to `main` too), then
   publishes `jlens-core`, `jlens-schema`, `jlens-lsp` and `jlens-cli` to crates.io, in that
   order, and `jlens-wasm` to npm, then drafts the GitHub release with the notes of the version,
   the binary archives and their `SHA256SUMS`. It needs the repository secrets
   `CARGO_REGISTRY_TOKEN` and `NPM_TOKEN`.

   To publish by hand instead: `cargo publish -p jlens-core`, then `-p jlens-schema`, then
   `-p jlens-lsp`, then `-p jlens-cli`; then `npm publish --access public` in `crates/jlens-wasm/pkg` after
   `cargo xtask wasm`; then
   `cargo xtask release-notes X.Y.Z > notes.md` and
   `gh release create vX.Y.Z --draft --verify-tag --title "jlens X.Y.Z" --notes-file notes.md`.

   If a binary fails, nothing has been published: fix the cause on `main`, then move the tag to
   the fixed commit (`git tag -f vX.Y.Z`, `git push -f origin vX.Y.Z`); re-run the failed jobs
   instead when only a runner failed. If the draft release fails after the packages are
   published, fix the workflow on `main` and run it by hand for the tag:
   `gh workflow run release.yml -f tag=vX.Y.Z`. That run publishes nothing; it builds the
   binaries of the tag and drafts the release.
5. Review the draft on the Releases page, edit it if needed, and publish it. `cargo binstall`
   finds the binaries once the release is published.

## License

By contributing, you agree that your contributions are licensed under the MIT license.
