# 0012 — The CLI on npm: a launcher package and one package per platform

**Status:** Accepted

## Context

The `jlens` command comes from the release archives, `cargo binstall` and `cargo install`. Many
of the people who write JSONata work in JavaScript projects, where tools are installed with npm
(`npm install --save-dev`, `npx`). The npm package `jlens-wasm` is a library: it has no `bin`.

Three ways to put the command on npm:

- **A second CLI, in JavaScript, over `jlens-wasm`.** It would have to reproduce the contract of
  ADR 0008 (envelopes, exit codes, standard output for data, never waiting on a terminal) and
  `jlens lsp`, and keep doing so for every change of `jlens-cli`. Two implementations of one
  contract drift.
- **`jlens-cli` compiled to WASI**, run by Node.js. One package for every platform, but Node.js
  prints an experimental warning for WASI, the interpreter is slower than the native binary, and
  file and terminal access go through a sandbox that the CLI would have to be adapted to.
- **The release binary, one package per platform**, as esbuild and Biome do: a small package
  with the command, whose `optionalDependencies` list one package per platform. npm installs only
  the one whose `os` and `cpu` match the machine.

The release builds five binaries (`binaries.yml`): Linux x86-64 and ARM64 (static, musl), macOS
x86-64 and ARM64, Windows x86-64. They are attested and listed in `SHA256SUMS`.

Names on npm, checked on 2026-10-13: `jlens` belongs to an unrelated package (a JSON reader
from 2018, whose command is `jl`); `jlens-cli`, `jlens-cli-linux-x64`, `jlens-cli-linux-arm64`,
`jlens-cli-darwin-x64`, `jlens-cli-darwin-arm64` and `jlens-cli-win32-x64` are free; the scope
`@jlens` does not exist.

## Decision

- **Six packages, named after the crate.** `jlens-cli` holds the command `jlens`; its
  `optionalDependencies` are the five platform packages `jlens-cli-<os>-<cpu>`, with `os` and
  `cpu` as Node.js names them (`process.platform`, `process.arch`): `linux-x64`, `linux-arm64`,
  `darwin-x64`, `darwin-arm64`, `win32-x64`. They are unscoped, like `jlens-wasm`, so no npm
  organization is needed. The command runs with `npx jlens-cli` or, once installed,
  `jlens`.
- **The binary is the release binary.** Each platform package holds the binary of its archive,
  as built and attested by the release, never rebuilt, with `LICENSE` and a short `README.md`.
  The Linux packages need no `libc` field: the musl binaries are static and run on glibc and
  musl systems alike.
- **One version for all.** Every package has the workspace version, and `jlens-cli` pins its
  platform packages to that exact version.
- **No install scripts.** The command is a small Node.js launcher (`bin/jlens.js`): it resolves
  the binary of `jlens-cli-${process.platform}-${process.arch}`, runs it with `spawnSync` and
  standard input, output and error inherited, and exits with the binary's exit code, or ends
  with the same signal. The binary sees the real terminal, so ADR 0008 holds through the
  launcher. When the platform package is missing (an unsupported platform, or an install with
  `--omit=optional`), the launcher prints which package it looked for and the other ways to
  install, on standard error, and exits with code 2.
- **Generated, then packed in CI.** An `xtask` command builds the six package directories from
  the archives: the binaries out of the archives, `package.json` from the workspace version.
  CI packs them and installs the packed packages on Linux, macOS and Windows, then runs
  `jlens --version`.
- **Published by `release.yml` with provenance**, after the binaries and next to `jlens-wasm`:
  the platform packages first, `jlens-cli` last, so that a published `jlens-cli` never points to
  a missing package. The job skips the packages whose version is already on npm, so re-running
  it completes a release that failed halfway through.

## Consequences

- `npm install -D jlens-cli` and `npx jlens-cli` give JavaScript projects the same `jlens` as
  every other channel, with no second implementation to keep in step.
- Each run of the command starts Node.js before the binary, some tens of milliseconds. Programs
  that run `jlens` many times can call the binary directly
  (`node_modules/jlens-cli-<os>-<cpu>/jlens`) or install it through another channel.
- Windows on ARM64 and other platforms without a release binary get the launcher's error, as
  they get no archive today. A new target in `binaries.yml` adds one platform package.
- npm and the other channels install the same command in different directories. With more than
  one installed, the first on the `PATH` runs, whatever its version; inside npm scripts and
  `npx`, a project's own `jlens-cli` comes first. The installation page recommends one channel
  and shows how to check which `jlens` runs.
- The release publishes five more packages, and `NPM_TOKEN` must be allowed to create them for
  the first release.
- The `jlens` name on npm stays with its owner. If it becomes free, it could later become an
  alias of `jlens-cli`.
