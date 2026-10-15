// Runs test/extension/index.cjs in VS Code, on the workspace of fixtures/workspace, with the
// jlens built by `cargo build -p jlens-cli`, or the one named by JLENS_SERVER.
//
// With JLENS_VSIX, the path of a package built by `cargo xtask vscode`, the package is installed
// in VS Code instead, as a user installs it, and the tests run with the binary it bundles.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  downloadAndUnzipVSCode,
  resolveCliArgsFromVSCodeExecutablePath,
  runTests,
} from "@vscode/test-electron";

const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, "../../..");
const vsix = process.env.JLENS_VSIX;
const launchArgs = [path.join(repository, "fixtures", "workspace"), "--disable-workspace-trust"];
let extensionDevelopmentPath = path.resolve(here, "..");
let extensionTestsEnv;

if (vsix) {
  const vscodeExecutablePath = await downloadAndUnzipVSCode();
  const extensions = path.resolve(here, "..", ".vscode-test", "extensions-vsix");
  fs.rmSync(extensions, { recursive: true, force: true });
  const [cli, ...args] = resolveCliArgsFromVSCodeExecutablePath(vscodeExecutablePath);
  // The CLI is a batch file on Windows, which only a shell runs.
  const shell = process.platform === "win32";
  const quote = (arg) => (shell ? `"${arg}"` : arg);
  const install = spawnSync(
    quote(cli),
    [...args, `--extensions-dir=${extensions}`, "--install-extension", path.resolve(vsix)].map(quote),
    { stdio: "inherit", shell },
  );
  if (install.status !== 0) {
    throw new Error(`installing ${vsix} failed`);
  }
  launchArgs.push(`--extensions-dir=${extensions}`);
  // The tests run from a development extension, which must not be the one under test.
  extensionDevelopmentPath = path.join(here, "host");
  extensionTestsEnv = { JLENS_VSIX: vsix };
} else {
  launchArgs.push("--disable-extensions");
  extensionTestsEnv = {
    JLENS_SERVER:
      process.env.JLENS_SERVER ??
      path.join(repository, "target", "debug", process.platform === "win32" ? "jlens.exe" : "jlens"),
  };
}

await runTests({
  extensionDevelopmentPath,
  extensionTestsPath: path.join(here, "extension", "index.cjs"),
  launchArgs,
  extensionTestsEnv,
});
