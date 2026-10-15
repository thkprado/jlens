// Runs test/extension/index.cjs in VS Code, on the workspace of fixtures/workspace, with the
// jlens built by `cargo build -p jlens-cli`, or the one named by JLENS_SERVER.

import path from "node:path";
import { fileURLToPath } from "node:url";
import { runTests } from "@vscode/test-electron";

const here = path.dirname(fileURLToPath(import.meta.url));
const repository = path.resolve(here, "../../..");
const server =
  process.env.JLENS_SERVER ??
  path.join(repository, "target", "debug", process.platform === "win32" ? "jlens.exe" : "jlens");

await runTests({
  extensionDevelopmentPath: path.resolve(here, ".."),
  extensionTestsPath: path.join(here, "extension", "index.cjs"),
  launchArgs: [
    path.join(repository, "fixtures", "workspace"),
    "--disable-extensions",
    "--disable-workspace-trust",
  ],
  extensionTestsEnv: { JLENS_SERVER: server },
});
