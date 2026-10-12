# AGENTS.md

Guidance for coding agents working in this repository. Humans: see [CONTRIBUTING.md](CONTRIBUTING.md).

jlens is a JSONata 2.2 engine in Rust, ported from jsonata-js 2.2.2. It has evaluation audits
and schema-driven editor support, and ships as crates (`jlens-core`, `jlens-schema`,
`jlens-lsp`, `jlens-cli`) and as the npm package `jlens-wasm`. The documentation is an mdBook
in `docs/`.

## Map

| Path | What lives there |
|---|---|
| `crates/jlens-core/src` | `lexer.rs`, `parser.rs` (Pratt, recover mode), `process.rs` (raw tree → `Ast`), `eval.rs` (interpreter), `value.rs` (value and sequence model), `functions/` (library and catalog), `datetime.rs`, `regex.rs`, `signature.rs`, `error.rs` (codes and messages), `audit/` (tracer, collector, explain), `document.rs`, `input.rs` (the `Source` trait, ADR 0010), `stack.rs` |
| `crates/jlens-schema/src` | `shape.rs` (Schema, inference, overrides), `jsonschema.rs`, `typing.rs` (typing walker), `service.rs` (LanguageService) |
| `crates/jlens-regress` | fork of `regress` 0.12.0 with interruptible matches (ADR 0009); keep the diff to upstream small, mark changes `jlens-regress` |
| `crates/jlens-lsp/src` | the language server: `server.rs` (message loop, requests), `config.rs` (`jlens.json`), `text.rs` (positions, incremental sync), `uri.rs` (paths of `file:` URIs) |
| `crates/jlens-cli/src` | the `jlens` binary (clap); `jlens lsp` runs `jlens-lsp` |
| `crates/jlens-wasm` | wasm-bindgen bindings; `src/source.rs` reads `evaluate`'s input object (a JavaScript helper per level); `src/types.d.ts` holds the hand-written TypeScript declarations; `tests/` holds the JS suite (`suite.mjs`), shared by Node and the browser |
| `tests/jsonata-suite` | the vendored official suite (do not edit by hand; `cargo xtask update-suite` replaces it and applies `SUITE_EDITS`) |
| `tests/conformance` | `baseline.txt`, `overrides.json`, `REPORT.md` |
| `docs/src` | the book; `language/functions.md` and `language/errors.md` are generated |
| `docs/examples` | sample documents for the book's examples |
| `xtask/src` | `cargo xtask` commands: conformance, gen-docs, doc-check, differential, fuzz-seed, wasm, wasm-test, serve |
| `benches/` | the benchmark workload; `benches/compare` compares jlens with jsonata-js (npm): speed, the book's examples, and `cargo xtask differential` |
| `fuzz/` | cargo-fuzz targets (nightly, Linux; outside the workspace); corpora seeded by `cargo xtask fuzz-seed`, run weekly by `.github/workflows/fuzz.yml` |
| `examples/web-editor` | the browser demo (CodeMirror 6, no build step) |
| `fixtures/` | sample inputs used by tests and guides |

## Before you finish a change

Run what your change touches; CI runs all of it:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets            # CI sets RUSTFLAGS=-D warnings
cargo test --workspace                            # unit, snapshot, guide and property tests, conformance baseline
cargo xtask gen-docs --check                      # after changing the function catalog or error messages
cargo xtask doc-check                             # after touching docs/ or behaviour the book shows
cargo clippy -p jlens-wasm --target wasm32-unknown-unknown
cargo xtask wasm-test                             # after touching jlens-wasm or anything it exposes
```

`cargo xtask wasm-test` needs:
- `wasm-bindgen-cli`, at exactly the version in `Cargo.lock`;
- Node.js 20+;
- a Chromium-based browser for the browser half (`JLENS_BROWSER`).

## Rules

- **Behave like jsonata-js 2.2.2.** When unsure, run the expression in jsonata-js
  (`benches/compare` has it installed after `npm install`) and match it.
  - Deliberate differences must be documented in `docs/src/language/compatibility.md`.
  - If a difference affects the suite, it must also be recorded in
    `tests/conformance/overrides.json`, with a `reason`.
- **Weigh compatibility against precision, correctness and performance.** They are goals of
  the project too. Before copying a jsonata-js behaviour, ask what it costs:
  - Copying favours compatibility when the cost stays in unusual input or error paths, and users
    may rely on getting the same answer from both engines.
  - Diverging favours jlens when copying would lose precision (an exact result replaced by
    JavaScript's rounding), give a result that is wrong by the JSONata documentation or exposes
    a JavaScript internal (a leaked flag), or slow down a common path. Measure with `benches/`
    when a change touches one.
  - Record the decision and its reason in `compatibility.md`, both when jlens differs and when
    it reproduces a jsonata-js bug on purpose. A divergence the differential tests can meet
    also needs a narrow rule in `xtask/src/differential.rs`.
- **The conformance baseline only grows.** A case in `tests/conformance/baseline.txt` that starts
  failing is a regression. After fixing cases, run `cargo xtask conformance --update-baseline`.
- **Errors use JSONata codes** (`S0xxx`, `T`, `D`), defined in `error.rs`.
  - Limits use `U1001` with a `message`.
  - Warnings use `W` codes.
  - Never match on messages in code or tests; match on codes.
- **Never panic or overflow the stack on user input.**
  - Recursive code over syntax trees or values goes through `jlens_core::stack::{grow,
    exhausted}`.
  - The parser caps nesting at 1,000 levels.
  - New features need cases in the property tests (`crates/*/tests/robustness.rs`). Run them
    longer with `PROPTEST_CASES=100000`.
- **Evaluations never mutate their input.** A `Document` is shared between evaluations; only
  freshly built values may be modified, as the transform operator does on its `$clone` copy.
- **Offsets:**
  - Rust APIs use byte offsets.
  - Error `position` is a UTF-16 offset, as in jsonata-js.
  - The Wasm package converts every offset to UTF-16, in both directions (`convert.rs`).
- **No tracer, no cost.** Keep tracing hooks off the untraced hot path in `eval.rs`; see
  ADR 0004.
- **`unsafe` is forbidden** (workspace lint `unsafe_code = "forbid"`). `missing_docs` warns, which
  CI turns into an error: document every public item.
- **Documentation is part of the change.**
  - JSONata examples in the book are `jsonata-example` blocks, which `doc-check` runs. Their
    format is documented in `xtask/src/docs.rs`.
  - Rust snippets in the crate guide live in `crates/jlens-core/tests/guide.rs`, between
    `ANCHOR` comments.
  - Never edit the generated pages by hand: run `cargo xtask gen-docs`.
  - Significant design decisions get an ADR in `docs/adr/`.
- **Snapshots** (`insta`): review every changed `.snap` file. Update them with
  `INSTA_UPDATE=always cargo test -p <crate>`, then read the diff.
- **Wasm API changes** need four updates:
  - the Rust bindings;
  - `src/types.d.ts` (the classes are declared there with `skip_typescript`);
  - `tests/suite.mjs` and `tests/types.ts`;
  - `docs/src/guides/wasm.md`.
- **All project artifacts are in English**: code, comments, docs and commit messages.

## Gotchas

- On the maintainer's Windows machine, `python` is not on PATH; use `uv run python`. Git Bash
  rewrites some paths; prefer the dedicated tools or PowerShell when that bites.
- Floating point: in examples, round results that would print as `206.70000000000002`.
- `wasm-opt` comes from the installed binaryen, or from npm `binaryen` through `npx`.
  Without either, the package is about 45% larger.
- The browser tests run Chrome or Edge with `--headless=new --dump-dom` and virtual time
  (`--virtual-time-budget`), which does not always advance during synchronous work: a check that
  waits for wall-clock time (a timeout) can hang the page. Keep such checks out of the browser
  runner (`env.virtualTime` in `crates/jlens-wasm/tests/suite.mjs`). Each run is bounded and
  retried once, and a page that never finishes prints its last console lines (the case that hung).

## Releasing

Follow the checklist in [CONTRIBUTING.md](CONTRIBUTING.md#releasing). Pushing a `v*` tag runs
`.github/workflows/release.yml`, which publishes to crates.io and npm. That is irreversible:
never create or push a tag, or run `cargo publish` / `npm publish`, without the maintainer's
explicit confirmation.
