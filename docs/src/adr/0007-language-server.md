# 0007 — A language server on the language service, configured by `jlens.json`

**Status:** Accepted

## Context

The language service (`jlens-schema`) completes, checks and documents expressions against a
schema. The browser reaches it through the WebAssembly package. Other editors (Neovim, Helix,
Emacs, VS Code) speak the Language Server Protocol. They need a server, and a way to know
which schema, variables and host functions apply to which files.

Choices to make:

1. **The protocol library.** `tower-lsp` is asynchronous and brings `tokio`. `lsp-server` and
   `lsp-types`, the libraries of rust-analyzer, are synchronous: a loop over messages on
   channels, with few dependencies. The language service is synchronous and fast (it parses and
   types one expression), so nothing would run concurrently.
2. **The binary.** A separate `jlens-lsp` executable, or a command of the existing CLI.
3. **The configuration.** Settings per file, written in each expression (an annotation such as
   `@schema`), or one file per workspace that maps paths to settings.

## Decision

- **`lsp-server` and `lsp-types`.** The server is a loop over messages. It keeps the open
  documents and one language service per configuration entry, built on first use.
- **A library crate, `jlens-lsp`, run by `jlens lsp`.** Installing the CLI installs the server;
  embedders can run the library on another connection, as the tests do in memory.
- **`jlens.json` at the root of each workspace folder.** It maps glob patterns, relative to its
  folder, to settings: a schema file, variables with their schemas, and host function
  declarations. The first matching pattern applies. Expressions stay pure JSONata, and one file
  configures a whole folder of rules. The same object can come as `initializationOptions`.
- **Reloading.** The server asks the client to watch `**/*.json`. Any change to a JSON file
  reloads the configuration and forgets the built services, so changed schemas are read again,
  and the open files are checked again. Saving a JSON file in the editor does the same, for
  clients that do not watch files.
- **Positions.** UTF-8 when the client offers it, UTF-16 otherwise. Positions past the end of
  a line are clamped to it, and a position inside a character means its start.

## Consequences

- Every editor with an LSP client gets the features of the web editor, with no
  editor-specific code. Only VS Code needs an extension to start the server, which jlens does
  not publish yet.
- Requests are answered one at a time, in order. That is enough while the service answers in
  well under a millisecond. Expensive features would need cancellation, or a worker thread.
- Configuration lives outside the expressions, so the same expression can be checked against
  different schemas in different folders. The cost is a file to maintain next to the rules.
- Reloading on any JSON change is coarse, but cheap: services are rebuilt lazily.
