# AGENTS.md — jlens-wasm

Guidance for working in the Wasm package. The repository-wide rules are in the root
[AGENTS.md](../../AGENTS.md).

## Testing

`cargo xtask wasm-test` needs:
- `wasm-bindgen-cli`, at exactly the version in `Cargo.lock`;
- Node.js 20+;
- a Chromium-based browser for the browser half (`JLENS_BROWSER`).

## Rules

- **Wasm API changes** need four updates:
  - the Rust bindings;
  - `src/types.d.ts` (the classes are declared there with `skip_typescript`);
  - `tests/suite.mjs` and `tests/types.ts`;
  - `docs/src/guides/wasm.md`.

## Gotchas

- `wasm-opt` comes from the installed binaryen, or from npm `binaryen` through `npx`.
  Without either, the package is about 45% larger.
- The browser tests run Chrome or Edge with `--headless=new --dump-dom` and virtual time
  (`--virtual-time-budget`), which does not always advance during synchronous work: a check that
  waits for wall-clock time (a timeout) can hang the page. Keep such checks out of the browser
  runner (`env.virtualTime` in `tests/suite.mjs`). Each run is bounded and retried once, and a
  page that never finishes prints its last console lines (the case that hung).
