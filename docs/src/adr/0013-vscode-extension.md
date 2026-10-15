# 0013 — The VS Code extension: one package per platform with the release binary

**Status:** Accepted

## Context

`jlens lsp` is a language server (ADR 0007). Neovim, Helix and Emacs start it from a few lines
of configuration, but VS Code starts language servers only from extensions, and most people who
write JSONata use VS Code. The extension has to answer five questions: where the server comes
from, how the user points it to another `jlens`, which language it claims, under which name it
is published, and where its code lives.

The server is the `jlens` binary. The release builds it for five targets (`binaries.yml`): Linux
x86-64 and ARM64 (static, musl), macOS x86-64 and ARM64, Windows x86-64, attested and listed in
`SHA256SUMS`. Three ways to give it to the extension:

- **A `jlens` found on the `PATH`.** The extension stays small, but nothing works until the user
  installs the CLI through another channel, and the server's version drifts from the
  extension's.
- **A download at first start**, from the GitHub release. The extension would need the network
  on first use (not always there, often behind a proxy), and would have to verify the archive
  itself before running it.
- **One VSIX per platform with the release binary**, as rust-analyzer does. Since VS Code 1.61,
  the Visual Studio Marketplace and Open VSX serve each platform its own package (`vsce package
  --target`), and a package without a target to the platforms that have none.

VS Code's targets include `alpine-x64` and `alpine-arm64`. The musl binaries are static, so the
two Linux archives serve both the `linux-` and the `alpine-` targets.

Other JSONata extensions, checked on 2026-10-14: on the Marketplace,
`bigbug.vscode-language-jsonata` (a grammar, about 6,000 installs, last updated in 2023),
`nawed-diwan.jsonata-intellisense` and `Fitmavincent.jsonata-validator`; on Open VSX, none. All
three use the language id `jsonata`, the extension `.jsonata` and the grammar scope
`source.jsonata`, as the editors guide does for Neovim and Helix.

Names, checked on 2026-10-14: the publisher `thkprado` exists on neither registry, and neither
does the extension `thkprado.jlens`.

Open VSX publishes with an access token of an Eclipse account (`ovsx publish`). The Marketplace
publishes with a personal access token of Azure DevOps, but retires global tokens on 2026-12-01
and recommends a Microsoft Entra ID identity instead (`vsce publish --azure-credential`): a
managed identity, which is an Azure resource and needs an Azure subscription.

## Decision

- **One VSIX per release target, with the release binary.** `linux-x64`, `linux-arm64`,
  `alpine-x64`, `alpine-arm64`, `darwin-x64`, `darwin-arm64` and `win32-x64`, each holding the
  `jlens` binary of its archive, as built and attested by the release, never rebuilt. One more
  VSIX, without a target and without a binary, serves the other platforms (Windows on ARM64,
  32-bit ARM Linux) and uses the `jlens` on the `PATH`. No download at first start.
- **The server is found in this order:** the setting `jlens.server.path`, then the bundled
  binary, then `jlens` on the `PATH`. The setting has the `machine` scope and is restricted in
  untrusted workspaces, so that a repository's settings cannot choose which program runs. When
  no server is found, the extension says so once, with a link to the installation page, and
  leaves highlighting working. The server's version, from its `initialize` answer, goes to the
  output channel.
- **The language is `jsonata`**, with the extension `.jsonata`, the alias `JSONata` and the
  grammar scope `source.jsonata`, the same as the other extensions and editors. A file opened in
  VS Code has one language id, so a different id would split JSONata files between extensions.
  `jlens.json` is checked through a JSON Schema contributed with `jsonValidation`.
- **The extension is `thkprado.jlens`**, named `jlens` and displayed as "jlens — JSONata", on
  both registries: the publisher `thkprado` on the Marketplace and the namespace `thkprado` on
  Open VSX, created by the maintainer.
- **One version for all.** The extension has the workspace version and is published by
  `release.yml` after the binaries, as the npm packages are (ADR 0012), skipping a version
  already published, so re-running the job completes a release that failed halfway through.
- **The Marketplace credential is chosen with the publishing job** (roadmap 0.15 #5), among:
  an Entra ID managed identity federated with the release workflow (no long-lived secret, but
  an Azure subscription); a personal access token, which works only until 2026-12-01; or no
  credential, with the maintainer uploading the VSIX files attached to the GitHub release.
  *Chosen on 2026-10-14:* the managed identity, federated with the GitHub environment `vscode`
  of the release job. Open VSX is published to with a token for the first version, which only
  a token can publish, then through trusted publishing (the run's OIDC token), so that neither
  registry keeps a long-lived secret. `CONTRIBUTING.md` says how to set both up.
- **The code lives in `editors/vscode/`**, outside the Cargo workspace: `package.json`, the
  TypeScript client over `vscode-languageclient`, bundled into one file with esbuild, the
  TextMate grammar and the language configuration, and the extension tests
  (`@vscode/test-electron`). A `cargo xtask vscode` command builds the VSIX files from the
  release archives, as `cargo xtask npm-cli` builds the npm packages.

## Consequences

- Installing the extension is enough on the five release platforms: no other step, and the
  server always has the extension's version. Other platforms install the CLI another way.
- Each release publishes eight VSIX files to two registries. Seven of them hold a binary of a
  few megabytes; the alpine packages repeat the Linux ones.
- With `jlens.server.path` or a `jlens` from the `PATH`, the server's version may differ from
  the extension's. The output channel shows it.
- With another JSONata extension installed, VS Code uses both: two grammars for the same scope,
  of which one wins, and diagnostics from each server. The extension's README says to disable
  the other's language features.
- The release needs an Azure subscription for the identity, which costs nothing, and a GitHub
  environment whose rules accept only version tags. Only a workflow run can sign in as the
  identity, so a small workflow (`marketplace-identity.yml`) reads the ID the publisher takes as
  a member.
- VS Code for the Web (the `web` target) is left out. The [WebAssembly
  package](../guides/wasm.md#the-language-service) offers the same language service, and could
  serve it later.
