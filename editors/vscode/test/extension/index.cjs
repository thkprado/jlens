// The extension's tests, run inside VS Code by test/run-extension.mjs, on fixtures/workspace:
// the language server answers through the extension, and jlens.json is checked by its schema.

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const vscode = require("vscode");

const folder = () => vscode.workspace.workspaceFolders[0].uri;
const file = (relative) => vscode.Uri.joinPath(folder(), relative);

/** Calls `f` until it returns a truthy value: the server answers once it has started. */
async function until(what, f, timeout = 30000) {
  const end = Date.now() + timeout;
  for (;;) {
    const value = await f();
    if (value) {
      return value;
    }
    if (Date.now() > end) {
      throw new Error(`timed out waiting for ${what}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

async function open(uri) {
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);
  return document;
}

async function hover(uri, line, character) {
  const hovers = await vscode.commands.executeCommand(
    "vscode.executeHoverProvider",
    uri,
    new vscode.Position(line, character),
  );
  return hovers
    .flatMap((h) => h.contents)
    .map((c) => (typeof c === "string" ? c : c.value))
    .join("\n");
}

const tests = {
  async "diagnostics come from the server, configured by jlens.json"() {
    const uri = file("rules/mistakes.jsonata");
    await open(uri);
    const diagnostics = await until("diagnostics", () => {
      const found = vscode.languages.getDiagnostics(uri);
      return found.length > 0 && found;
    });
    // The same codes as `jlens check` and the server's tests find in this fixture.
    assert.deepEqual(diagnostics.map((d) => d.code).sort(), ["W0201", "W0202", "W0204"]);
  },

  async "completion offers the fields of the schema"() {
    const uri = file("rules/discount.jsonata");
    await open(uri);
    // After `unit` in `items[unitPrice > …`.
    const fields = await until("a completion", async () => {
      const list = await vscode.commands.executeCommand(
        "vscode.executeCompletionItemProvider",
        uri,
        new vscode.Position(0, 15),
      );
      const labels = list.items
        .filter((i) => i.kind === vscode.CompletionItemKind.Field)
        .map((i) => i.label);
      return labels.length > 0 && labels;
    });
    assert.deepEqual(fields, ["unitPrice"]);
  },

  async "hover shows a declared host function"() {
    const uri = file("rules/discount.jsonata");
    await open(uri);
    const text = await until("a hover", () => hover(uri, 0, 36));
    assert.match(text, /The discount rate of a product type/);
  },

  async "the restart command starts the server again"() {
    await vscode.commands.executeCommand("jlens.restartServer");
    const text = await until("a hover", () => hover(file("rules/discount.jsonata"), 0, 2));
    assert.match(text, /\$sum/);
  },

  async "jlens.json is checked by its JSON Schema"() {
    // The editors guide's example, then a pattern with a misspelled setting on line 9.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "jlens-vscode-"));
    try {
      const config = path.join(dir, "jlens.json");
      fs.writeFileSync(
        config,
        `{
  "rules/**/*.jsonata": {
    "schema": "schemas/order.schema.json",
    "variables": {"minimum": {"type": "number"}, "region": null},
    "functions": [
      {"name": "shipping", "signature": "<ns:n>", "params": ["weight", "region"],
       "summary": "Shipping cost of a weight to a region"}
    ]
  },
  "**/*.jsonata": {"schemas": "order.schema.json"}
}
`,
      );
      const uri = vscode.Uri.file(config);
      await open(uri);
      await until("the JSON diagnostics", () => vscode.languages.getDiagnostics(uri).length > 0);
      // Let a late diagnostic of the valid part arrive, if there were one.
      await new Promise((resolve) => setTimeout(resolve, 1000));
      const lines = vscode.languages.getDiagnostics(uri).map((d) => d.range.start.line);
      assert.deepEqual(lines, [9]);
      await vscode.commands.executeCommand("workbench.action.closeActiveEditor");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  },
};

if (process.env.JLENS_VSIX) {
  tests["the installed package bundles an executable server"] = async () => {
    const extension = vscode.extensions.getExtension("thkprado.jlens");
    assert.ok(extension, "the extension is installed");
    const exe = process.platform === "win32" ? "jlens.exe" : "jlens";
    // VS Code keeps the mode that `cargo xtask vscode` packed.
    fs.accessSync(path.join(extension.extensionPath, "server", exe), fs.constants.X_OK);
  };
}

exports.run = async function run() {
  // The setting has the `machine` scope: only user settings can give it. An installed package
  // runs without it, with its bundled server.
  const server = process.env.JLENS_VSIX ? undefined : process.env.JLENS_SERVER;
  await vscode.workspace
    .getConfiguration("jlens")
    .update("server.path", server, vscode.ConfigurationTarget.Global);
  const failures = [];
  for (const [name, test] of Object.entries(tests)) {
    try {
      await test();
      console.log(`ok - ${name}`);
    } catch (e) {
      console.log(`not ok - ${name}\n${e.stack ?? e}`);
      failures.push(name);
    }
  }
  if (failures.length > 0) {
    throw new Error(`${failures.length} extension test(s) failed: ${failures.join(", ")}`);
  }
};
