#!/usr/bin/env node
// The `jlens` command of the npm package jlens-cli (ADR 0012). It runs the release binary held
// by the platform package that npm installed for this machine, with the terminal inherited, and
// ends as the binary ended: with its exit code, or by the same signal.
"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");

const platformPackage = `jlens-cli-${process.platform}-${process.arch}`;
const executable = process.platform === "win32" ? "jlens.exe" : "jlens";

let binary;
try {
  binary = path.join(path.dirname(require.resolve(`${platformPackage}/package.json`)), executable);
} catch {
  process.stderr.write(
    `jlens: the package ${platformPackage}, which holds the jlens binary for this platform, ` +
      "is not installed.\n" +
      "It is an optional dependency of jlens-cli: an install with --omit=optional or " +
      "--no-optional leaves it out, and platforms without a release binary have none.\n" +
      "Other ways to install jlens: https://thkprado.github.io/jlens/getting-started/installation.html\n",
  );
  process.exit(2);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  process.stderr.write(`jlens: cannot run ${binary}: ${result.error.message}\n`);
  process.exit(2);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
