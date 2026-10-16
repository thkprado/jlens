# AGENTS.md — VS Code extension

Guidance for working on the VS Code extension (ADR 0013; outside the Cargo workspace). The
repository-wide rules are in the root [AGENTS.md](../../AGENTS.md).

## Map

- The client is in `src/extension.ts`, bundled by esbuild (`npm run build`; `npm run check`
  type-checks it).
- The `jlens.json` schema is in `schemas/`, the TextMate grammar in `syntaxes/`, and
  `language-configuration.json` sits at the root.

## Testing

- `test/grammar.test.mjs` compares `test/fixtures/*.jsonata` with their `.snap` files
  (`npm test -- --update` rewrites them) and tokenizes the suite and the book.
- `npm run test:extension` runs `test/extension/` in VS Code on `fixtures/workspace`, with
  `target/debug/jlens` (`cargo build -p jlens-cli` first) or `JLENS_SERVER`.

## Packaging

`cargo xtask vscode <archives>` packs one VSIX per target into `target/vscode` (pack on Linux or
macOS: Windows loses the executable bit), and `JLENS_VSIX=<vsix> node test/run-extension.mjs`
installs one and runs the same test on its bundled binary.
